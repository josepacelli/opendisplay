//! Composes every module wired: bootstrap (`main.rs`), discovery
//! (`transport::{wifi,usb}`), the IPC pipe server (`ipc_server`), and the
//! per-device dial -> handshake -> display -> capture -> encode ->
//! frame_sender -> input -> cursor pipeline, driven by
//! `session_state`'s retry loop (FIX1, Verifier gap 1 — Blocker: no story
//! was reachable without this).
//!
//! **Threading model**: one OS thread per long-running I/O loop (WiFi
//! discovery, USB discovery, the IPC pipe's accept loop, and — while a
//! device is targeted — one session-driving thread per `Connect`),
//! coordinated through `std::sync::{Arc, Mutex}` shared state. A
//! multi-thread `tokio::runtime::Runtime` is created once and leaked for
//! the process lifetime (`std::mem::forget`) so its `Handle` can be cloned
//! onto any thread that needs to drive `idevice`'s async usbmuxd client —
//! USB discovery and every USB dial attempt each get their own clone.
//!
//! **Not exercised by any automated gate** — this is OS/hardware-bound
//! orchestration, same Test Coverage Matrix bucket as
//! `capture`/`encode`/`display`. `base64_encode`/`encode_cursor_event`
//! below are the pure-glue exception carved out by this task's Tests line
//! and are unit-tested.

/// Base64 (standard alphabet, `=`-padded) encode, per `PROTOCOL.md` §6.2's
/// `cursorImg.png` ("the base64 of a PNG"). Hand-rolled rather than adding
/// a dependency for one call site.
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(ALPHABET[(n >> 18 & 0x3F) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6 & 0x3F) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[(n & 0x3F) as usize] as char } else { '=' });
    }
    out
}

/// Builds the wire JSON payload for one `cursor::CursorEvent`, per
/// `PROTOCOL.md` §6.2's `cursor { x?, y?, v }` / `cursorImg { nw, nh, ax,
/// ay, png }` shapes.
pub fn encode_cursor_event(event: &crate::cursor::CursorEvent) -> Vec<u8> {
    use crate::cursor::CursorEvent;
    let value = match event {
        CursorEvent::Cursor { position, visible } => {
            let mut obj = serde_json::json!({
                "type": "cursor",
                "v": if *visible { 1 } else { 0 },
            });
            if let Some((x, y)) = position {
                obj["x"] = serde_json::json!(x);
                obj["y"] = serde_json::json!(y);
            }
            obj
        }
        CursorEvent::CursorImg { png, width_norm, height_norm, hotspot_x_norm, hotspot_y_norm } => {
            serde_json::json!({
                "type": "cursorImg",
                "nw": width_norm,
                "nh": height_norm,
                "ax": hotspot_x_norm,
                "ay": hotspot_y_norm,
                "png": base64_encode(png),
            })
        }
    };
    serde_json::to_vec(&value).expect("cursor event always serializes")
}

#[cfg(windows)]
mod windows_impl {
    use super::encode_cursor_event;
    use crate::display_spec::DisplaySpec;
    use crate::protocol_session::HandshakeError;
    use crate::session_state::{SessionState, TerminalReason};
    use crate::transport::usb::UsbAttachSource;
    use crate::transport::{dial, usb, wifi};
    use std::collections::BTreeSet;
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// A currently dial-targeted device's control handle, held so a later
    /// `Connect` to a *different* device (FIX4) can end it before starting
    /// a new one, and so `Disconnect` can end it directly.
    struct SessionHandle {
        teardown: Arc<AtomicBool>,
        join: std::thread::JoinHandle<()>,
    }

    struct SharedState {
        wifi: Mutex<Vec<wifi::DiscoveredDevice>>,
        usb: Mutex<BTreeSet<String>>,
        ipc: Mutex<crate::ipc_server::IpcServer>,
        session: Mutex<Option<SessionHandle>>,
        /// The current tray connection's write half, if one is connected —
        /// `None` between tray launches. Shared so a session thread can
        /// push a `Status` update the moment it changes, not only when the
        /// tray happens to poll.
        writer: Mutex<Option<Arc<Mutex<std::fs::File>>>>,
        log: crate::log::RotatingLogFile,
    }

    fn send_message(writer: &Arc<Mutex<std::fs::File>>, msg: &ipc::CoreToTray) {
        let line = format!("{}\n", ipc::to_line(msg));
        if let Ok(mut f) = writer.lock() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn send_to_tray(shared: &SharedState, msg: &ipc::CoreToTray) {
        if let Some(writer) = shared.writer.lock().unwrap().clone() {
            send_message(&writer, msg);
        }
    }

    fn snapshot_device_list(shared: &SharedState) -> ipc::CoreToTray {
        let wifi = shared.wifi.lock().unwrap().clone();
        let usb: Vec<usb::DiscoveredDevice> = shared
            .usb
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .map(|id| usb::DiscoveredDevice { id })
            .collect();
        crate::ipc_server::build_device_list(&wifi, &usb)
    }

    fn publish_status(shared: &SharedState, device_id: &str, connected: bool, transport: Option<ipc::Transport>) {
        send_to_tray(
            shared,
            &ipc::CoreToTray::Status {
                device: Some(device_id.to_string()),
                transport,
                connected,
                stats: None,
            },
        );
    }

    /// Runs one background thread that browses `_opensidecar._tcp` (T10)
    /// and merges resolved records into `shared.wifi`. Merges rather than
    /// replaces on each tick — `wifi::discover` only reports records
    /// resolved *since the last call* (mDNS gives no periodic re-announce
    /// signal this sender relies on), so replacing the list wholesale each
    /// tick would drop devices that resolved once and haven't changed
    /// since.
    fn spawn_wifi_discovery(shared: Arc<SharedState>) {
        std::thread::spawn(move || {
            let mut source = match wifi::MdnsSdBrowseSource::new() {
                Ok(s) => s,
                Err(_) => return,
            };
            loop {
                let discovered = wifi::discover(&mut source);
                if !discovered.is_empty() {
                    let mut list = shared.wifi.lock().unwrap();
                    for device in discovered {
                        match list.iter_mut().find(|existing| existing.name == device.name) {
                            Some(existing) => *existing = device,
                            None => list.push(device),
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }

    /// Runs one background thread that polls usbmuxd (T11) and folds
    /// attach/detach events into `shared.usb`'s persistent known-device
    /// set — `usb::discover`'s fold only covers events from one poll, so
    /// the accumulation across polls happens here instead of by calling it
    /// per tick (same reasoning as `spawn_wifi_discovery`).
    fn spawn_usb_discovery(shared: Arc<SharedState>, tokio_handle: tokio::runtime::Handle) {
        std::thread::spawn(move || {
            let mut source = match usb::IdeviceUsbAttachSource::new(tokio_handle) {
                Ok(s) => s,
                Err(_) => return,
            };
            loop {
                for event in source.poll_events() {
                    let mut known = shared.usb.lock().unwrap();
                    match event {
                        usb::UsbEvent::Attached { device_id } => {
                            known.insert(device_id);
                        }
                        usb::UsbEvent::Detached { device_id } => {
                            known.remove(&device_id);
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }

    /// Why one pass through a connected session's inner loop ended.
    enum SessionOutcome {
        /// An ordinary connection drop, a read/framing error, or the
        /// receiver's `sleeping` message — the outer `drive_session` loop
        /// redials after `session_state::RETRY_INTERVAL_SECS`.
        Retry,
        /// The receiver's `closing` message, or the dial-refusal floor was
        /// reached — no further auto-retry for this device.
        Terminal(TerminalReason),
        /// `teardown` was set externally (`Disconnect`, or a `Connect` to a
        /// different device, FIX4) — stop immediately, no retry, no status
        /// push (the caller that set `teardown` owns reporting).
        ExternallyStopped,
    }

    /// Runs one connected session end to end: display create, capture,
    /// encode, frame send, control-message read/dispatch, ping, and cursor
    /// forwarding, until the connection drops, the receiver ends it, or
    /// `teardown` is set. `stream` must already have a short read timeout
    /// set (see `try_read_frame`'s doc comment) so this loop can interleave
    /// the read with the rest of the tick instead of blocking on it.
    fn run_connected_session(
        mut stream: std::net::TcpStream,
        display: DisplaySpec,
        display_handle: crate::display::VirtualDisplayHandle,
        teardown: &AtomicBool,
    ) -> SessionOutcome {
        let write_stream = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => return SessionOutcome::Retry,
        };
        let mut sender = crate::frame_sender::FrameSender::new(write_stream);

        let mut capture_stream = match crate::capture::start(&display) {
            Ok(c) => c,
            Err(_) => return SessionOutcome::Retry,
        };
        let mut encoder = match crate::encode::start(&display) {
            Ok(e) => e,
            Err(_) => return SessionOutcome::Retry,
        };
        let mut injector = crate::input::RealInputInjector;
        let mut touch_is_down = false;
        let mut read_buf: Vec<u8> = Vec::new();
        let mut state = SessionState::new_connected();
        let mut last_ping_at: Option<Instant> = None;
        let mut cursor_snapshot: Option<crate::cursor::CursorSnapshotHandle> = None;

        loop {
            if teardown.load(Ordering::SeqCst) {
                let _ = display_handle.destroy();
                return SessionOutcome::ExternallyStopped;
            }

            if let Ok(frame) = capture_stream.next_frame(16) {
                if let Ok(Some(encoded)) = encoder.encode(frame) {
                    let _ = sender.send(&encoded.data);
                }
            }
            // A capture timeout/error just means no new frame this tick —
            // desktop duplication legitimately has nothing new to report
            // when the screen hasn't changed; the loop keeps going so
            // control messages, pings, and cursor updates are never
            // starved by an idle desktop.

            match crate::protocol_session::try_read_frame(&mut stream, &mut read_buf) {
                Ok(Some(payload)) => {
                    if let Ok(msg) = crate::protocol_session::parse_control_message(&payload) {
                        if let Some(event) = crate::protocol_session::apply_control_message(
                            &msg,
                            &display,
                            &mut injector,
                            &mut touch_is_down,
                        ) {
                            state = crate::session_state::transition(state, event);
                            match state {
                                SessionState::Terminal { reason } => {
                                    let _ = display_handle.destroy();
                                    return SessionOutcome::Terminal(reason);
                                }
                                SessionState::Paused => {
                                    let _ = display_handle.destroy();
                                    return SessionOutcome::Retry;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    // Connection dropped or a framing error: ordinary
                    // drop, per session_state's Dropped transition.
                    let _ = display_handle.destroy();
                    return SessionOutcome::Retry;
                }
            }

            let now = Instant::now();
            if crate::session_state::should_send_ping(state, last_ping_at, now) {
                let _ = sender.send(&crate::protocol_session::build_ping_message());
                last_ping_at = Some(now);
            }

            if let Ok((events, snapshot)) = crate::cursor::poll(&display, cursor_snapshot.take()) {
                cursor_snapshot = Some(snapshot);
                for event in &events {
                    let _ = sender.send(&encode_cursor_event(event));
                }
            }
        }
    }

    /// Drives one device end to end: resolve -> dial -> handshake ->
    /// display -> the connected session loop -> redial on an ordinary
    /// drop, per `session_state`'s documented transitions (FIX1's Done
    /// when: "the retry loop driving reconnects").
    fn drive_session(
        target_id: String,
        shared: Arc<SharedState>,
        teardown: Arc<AtomicBool>,
        tokio_handle: tokio::runtime::Handle,
    ) {
        let mut dialer = dial::RealDialer::new(tokio_handle);
        let mut consecutive_refusals = 0u32;

        loop {
            if teardown.load(Ordering::SeqCst) {
                return;
            }

            let device = {
                let wifi = shared.wifi.lock().unwrap().clone();
                let usb: Vec<usb::DiscoveredDevice> = shared
                    .usb
                    .lock()
                    .unwrap()
                    .iter()
                    .cloned()
                    .map(|id| usb::DiscoveredDevice { id })
                    .collect();
                dial::select_preferred_transport(&wifi, &usb, &target_id)
            };
            let Some(device) = device else {
                std::thread::sleep(Duration::from_secs_f64(crate::session_state::RETRY_INTERVAL_SECS));
                continue;
            };
            let transport = match &device {
                dial::DiscoveredDevice::Wifi(_) => ipc::Transport::Wifi,
                dial::DiscoveredDevice::Usb(_) => ipc::Transport::Usb,
            };

            match dial::dial(&device, &mut dialer) {
                Ok(mut stream) => match crate::protocol_session::handshake(&mut stream) {
                    Ok(session) => {
                        consecutive_refusals = 0;
                        let display = crate::display_spec::derive_display_spec(&session.panel);
                        let display_handle = match crate::display::VirtualDisplayHandle::create(&display) {
                            Ok(h) => h,
                            Err(_) => {
                                std::thread::sleep(Duration::from_secs_f64(
                                    crate::session_state::RETRY_INTERVAL_SECS,
                                ));
                                continue;
                            }
                        };
                        shared
                            .log
                            .append(format!("session started for {target_id}\n").as_bytes())
                            .ok();
                        publish_status(&shared, &target_id, true, Some(transport));

                        // 20ms: short enough that control messages/pings/
                        // cursor updates aren't meaningfully delayed, long
                        // enough not to busy-spin the thread between real
                        // reads.
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));

                        let outcome = run_connected_session(stream, display, display_handle, &teardown);
                        publish_status(&shared, &target_id, false, Some(transport));

                        match outcome {
                            SessionOutcome::Terminal(reason) => {
                                shared
                                    .log
                                    .append(format!("session for {target_id} ended: {reason:?}\n").as_bytes())
                                    .ok();
                                return;
                            }
                            SessionOutcome::ExternallyStopped => return,
                            SessionOutcome::Retry => {
                                std::thread::sleep(Duration::from_secs_f64(
                                    crate::session_state::RETRY_INTERVAL_SECS,
                                ));
                                continue;
                            }
                        }
                    }
                    Err(HandshakeError::PeerBelowMinSupported { .. }) => return,
                    Err(_) => {
                        std::thread::sleep(Duration::from_secs_f64(crate::session_state::RETRY_INTERVAL_SECS));
                        continue;
                    }
                },
                Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
                    consecutive_refusals += 1;
                    if consecutive_refusals >= crate::session_state::REFUSALS_BEFORE_ANOTHER_SENDER {
                        shared
                            .log
                            .append(format!("session for {target_id} ended: another sender connected\n").as_bytes())
                            .ok();
                        return;
                    }
                    std::thread::sleep(Duration::from_secs_f64(crate::session_state::RETRY_INTERVAL_SECS));
                }
                Err(_) => {
                    std::thread::sleep(Duration::from_secs_f64(crate::session_state::RETRY_INTERVAL_SECS));
                }
            }
        }
    }

    /// Ends the currently dial-targeted device's session (if any),
    /// blocking until its thread has actually exited before returning —
    /// so a following `DialDevice` never races the old session's
    /// `VirtualDisplayHandle::destroy()` against a new `::create()` (the
    /// driver models exactly one virtual monitor, per the P1 constraint).
    fn teardown_current_session(shared: &SharedState) {
        if let Some(previous) = shared.session.lock().unwrap().take() {
            previous.teardown.store(true, Ordering::SeqCst);
            let _ = previous.join.join();
        }
    }

    fn apply_effect(
        effect: crate::ipc_server::Effect,
        shared: &Arc<SharedState>,
        tokio_handle: &tokio::runtime::Handle,
    ) {
        use crate::ipc_server::Effect;
        match effect {
            Effect::DialDevice { device_id } => {
                let teardown = Arc::new(AtomicBool::new(false));
                let join = {
                    let shared = shared.clone();
                    let teardown = teardown.clone();
                    let tokio_handle = tokio_handle.clone();
                    let target = device_id.clone();
                    std::thread::spawn(move || drive_session(target, shared, teardown, tokio_handle))
                };
                *shared.session.lock().unwrap() = Some(SessionHandle { teardown, join });
            }
            Effect::TeardownSession => teardown_current_session(shared),
            Effect::OpenLogFolder => {
                let _ = std::process::Command::new("explorer.exe")
                    .arg(crate::log::default_log_dir())
                    .spawn();
            }
            Effect::LogRejectedMessage { raw_line } => {
                let _ = shared.log.append(format!("rejected malformed IPC line: {raw_line}\n").as_bytes());
            }
        }
    }

    fn handle_client(file: std::fs::File, shared: Arc<SharedState>, tokio_handle: tokio::runtime::Handle) {
        let write_file = match file.try_clone() {
            Ok(f) => f,
            Err(_) => return,
        };
        let writer = Arc::new(Mutex::new(write_file));
        *shared.writer.lock().unwrap() = Some(writer.clone());

        send_message(&writer, &snapshot_device_list(&shared));
        let current_device_id = shared.ipc.lock().unwrap().current_device_id.clone();
        send_message(
            &writer,
            &crate::ipc_server::build_status(&current_device_id, None),
        );

        let mut reader = std::io::BufReader::new(file);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let action = crate::ipc_server::handle_incoming_line(line.trim_end());
                    let effects = shared.ipc.lock().unwrap().apply(action);
                    for effect in effects {
                        apply_effect(effect, &shared, &tokio_handle);
                    }
                }
            }
        }

        // This client disconnected; stop pushing status to it. Its
        // session (if any) keeps running independent of the tray
        // connection — the tray reconnecting later gets a fresh snapshot.
        let mut current = shared.writer.lock().unwrap();
        if matches!(&*current, Some(existing) if Arc::ptr_eq(existing, &writer)) {
            *current = None;
        }
    }

    /// The real entry point: starts the log writer, WiFi+USB discovery,
    /// and the IPC pipe accept loop on a `Ready` bootstrap (FIX1's Done
    /// when). Never returns under normal operation.
    pub fn run() {
        let log = crate::log::RotatingLogFile::new(crate::log::default_log_dir(), "windows-core", 5 * 1024 * 1024);
        let shared = Arc::new(SharedState {
            wifi: Mutex::new(Vec::new()),
            usb: Mutex::new(BTreeSet::new()),
            ipc: Mutex::new(crate::ipc_server::IpcServer::new()),
            session: Mutex::new(None),
            writer: Mutex::new(None),
            log,
        });

        let tokio_rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        let tokio_handle = tokio_rt.handle().clone();
        // Leaked deliberately: this runtime must outlive every thread that
        // clones its Handle for the rest of the process's life (USB
        // discovery, every session's dial_usb) — there is no natural point
        // to shut it down before process exit.
        std::mem::forget(tokio_rt);

        spawn_wifi_discovery(shared.clone());
        spawn_usb_discovery(shared.clone(), tokio_handle.clone());

        loop {
            match crate::ipc_server::windows_impl::accept_client(crate::ipc_server::PIPE_NAME) {
                Ok(file) => {
                    let shared = shared.clone();
                    let tokio_handle = tokio_handle.clone();
                    std::thread::spawn(move || handle_client(file, shared, tokio_handle));
                }
                Err(_) => {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }
}

#[cfg(windows)]
pub use windows_impl::run;

#[cfg(test)]
mod tests {
    use super::*;

    // --- base64_encode: PROTOCOL.md §6.2's cursorImg.png. ---

    #[test]
    fn empty_input_encodes_to_an_empty_string() {
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn a_length_matching_a_multiple_of_3_needs_no_padding() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn a_length_one_short_of_a_multiple_of_3_gets_one_padding_char() {
        assert_eq!(base64_encode(b"Ma"), "TWE=");
    }

    #[test]
    fn a_length_two_short_of_a_multiple_of_3_gets_two_padding_chars() {
        assert_eq!(base64_encode(b"M"), "TQ==");
    }

    #[test]
    fn a_longer_payload_encodes_correctly() {
        assert_eq!(base64_encode(b"hello world"), "aGVsbG8gd29ybGQ=");
    }

    // --- encode_cursor_event: PROTOCOL.md §6.2's cursor/cursorImg shapes. ---

    #[test]
    fn a_visible_cursor_event_encodes_type_v_and_position() {
        let event = crate::cursor::CursorEvent::Cursor { position: Some((0.25, 0.75)), visible: true };

        let value: serde_json::Value = serde_json::from_slice(&encode_cursor_event(&event)).unwrap();

        assert_eq!(value["type"], "cursor");
        assert_eq!(value["v"], 1);
        assert_eq!(value["x"], 0.25);
        assert_eq!(value["y"], 0.75);
    }

    #[test]
    fn a_hidden_cursor_event_encodes_v_zero_with_no_position_fields() {
        let event = crate::cursor::CursorEvent::Cursor { position: None, visible: false };

        let value: serde_json::Value = serde_json::from_slice(&encode_cursor_event(&event)).unwrap();

        assert_eq!(value["type"], "cursor");
        assert_eq!(value["v"], 0);
        assert!(value.get("x").is_none());
        assert!(value.get("y").is_none());
    }

    #[test]
    fn a_cursor_img_event_encodes_base64_png_and_normalized_geometry() {
        let event = crate::cursor::CursorEvent::CursorImg {
            png: b"hello world".to_vec(),
            width_norm: 0.1,
            height_norm: 0.2,
            hotspot_x_norm: 0.3,
            hotspot_y_norm: 0.4,
        };

        let value: serde_json::Value = serde_json::from_slice(&encode_cursor_event(&event)).unwrap();

        assert_eq!(value["type"], "cursorImg");
        assert_eq!(value["png"], "aGVsbG8gd29ybGQ=");
        // Compared as f32 -> f64 (not bare f64 literals): 0.1f32 widened to
        // f64 isn't bit-identical to the f64 literal 0.1, since json!()
        // widens the struct's f32 fields the same way.
        assert_eq!(value["nw"], serde_json::json!(0.1_f32));
        assert_eq!(value["nh"], serde_json::json!(0.2_f32));
        assert_eq!(value["ax"], serde_json::json!(0.3_f32));
        assert_eq!(value["ay"], serde_json::json!(0.4_f32));
    }
}
