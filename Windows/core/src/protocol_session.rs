//! The `hello`/`welcome` handshake, per `PROTOCOL.md` §6.
//!
//! `windows-core` plays the *sender* role: it waits for the receiver's
//! `hello` (the first message on every new connection), replies with
//! `welcome`, and — if the peer's `pv` is below `MIN_SUPPORTED_PEER` —
//! additionally sends `updateRequired` and refuses to stream, per the
//! spec's Edge Cases.
//!
//! Generic over `Read + Write` rather than tied to `TcpStream`, so the
//! handshake logic is exercised in tests against an in-memory fake stream
//! instead of a live socket.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// The receiver's `hello` payload, per `PROTOCOL.md` §6.1.
#[derive(Debug, Deserialize)]
struct HelloWire {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(rename = "pixelsWide")]
    pixels_wide: u32,
    #[serde(rename = "pixelsHigh")]
    pixels_high: u32,
    scale: f32,
    device: Option<String>,
    id: Option<String>,
    pv: Option<u32>,
}

#[derive(Debug, Serialize)]
struct WelcomeWire {
    #[serde(rename = "type")]
    msg_type: &'static str,
    pv: u32,
    min: u32,
}

#[derive(Debug, Serialize)]
struct PingWire {
    #[serde(rename = "type")]
    msg_type: &'static str,
}

/// Builds the outbound `ping` payload windows-core sends every 2 seconds
/// while connected (spec WSEND-05, `PROTOCOL.md` §6.2/Appendix A). No
/// optional telemetry fields (`drops`/`encDrops`/`netDrops`/...) are
/// populated — `PROTOCOL.md`'s ping table marks all of them optional, and
/// this sender does not yet track those stats. Whether/when to call this is
/// `session_state::should_send_ping`'s decision; this function only builds
/// the wire bytes.
pub fn build_ping_message() -> Vec<u8> {
    serde_json::to_vec(&PingWire {
        msg_type: protocol::wire_message::PING,
    })
    .expect("PingWire always serializes")
}

#[derive(Debug, Serialize)]
struct UpdateRequiredWire {
    #[serde(rename = "type")]
    msg_type: &'static str,
    target: &'static str,
    store: String,
    message: String,
}

/// The receiver's reported panel, straight from `hello` (before any
/// `DisplaySpec` derivation — that's T15's job).
#[derive(Debug, Clone, PartialEq)]
pub struct PanelReport {
    pub pixels_wide: u32,
    pub pixels_high: u32,
    pub scale: f32,
    pub device: Option<String>,
    pub id: Option<String>,
}

/// The result of a completed handshake.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// The peer's `hello.pv`, defaulted per `COMPATIBILITY.md` §2.
    pub peer_pv: u32,
    /// The lower of the peer's `pv` and this sender's own
    /// `protocol::WIRE_PROTOCOL_VERSION` — the effective version this
    /// session speaks.
    pub negotiated_pv: u32,
    pub panel: PanelReport,
}

/// Errors that can occur during the handshake.
#[derive(Debug)]
pub enum HandshakeError {
    Io(io::Error),
    MalformedHello(serde_json::Error),
    UnexpectedFirstMessage { got: String },
    Frame(protocol::FrameError),
    /// The peer's `pv` is below `protocol::MIN_SUPPORTED_PEER`. `welcome`
    /// and `updateRequired` have already been sent by the time this is
    /// returned; the caller MUST NOT proceed to streaming for this session.
    PeerBelowMinSupported { peer_pv: u32 },
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeError::Io(err) => write!(f, "handshake I/O error: {err}"),
            HandshakeError::MalformedHello(err) => write!(f, "malformed hello: {err}"),
            HandshakeError::UnexpectedFirstMessage { got } => {
                write!(f, "expected \"hello\" as the first message, got {got:?}")
            }
            HandshakeError::Frame(err) => write!(f, "handshake framing error: {err}"),
            HandshakeError::PeerBelowMinSupported { peer_pv } => {
                write!(f, "peer pv {peer_pv} is below the sender's minimum supported peer")
            }
        }
    }
}

impl std::error::Error for HandshakeError {}

/// Reads bytes from `stream` until one complete length-prefixed frame (per
/// `PROTOCOL.md` §3) has arrived, and returns its payload.
fn read_one_frame<S: Read>(stream: &mut S) -> Result<Vec<u8>, HandshakeError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match protocol::decode_frame(&buf) {
            Ok(Some(frame)) => return Ok(frame.payload.to_vec()),
            Ok(None) => {
                let n = stream.read(&mut chunk).map_err(HandshakeError::Io)?;
                if n == 0 {
                    return Err(HandshakeError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed before a full frame arrived",
                    )));
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(err) => return Err(HandshakeError::Frame(err)),
        }
    }
}

fn write_frame<S: Write>(stream: &mut S, payload: &[u8]) -> Result<(), HandshakeError> {
    let framed = protocol::encode_frame(payload).map_err(HandshakeError::Frame)?;
    stream.write_all(&framed).map_err(HandshakeError::Io)
}

/// Performs the `hello`/`welcome` handshake on an already-connected
/// `stream` (see `transport::dial`, T12), per `PROTOCOL.md` §6.
pub fn handshake<S: Read + Write>(stream: &mut S) -> Result<Session, HandshakeError> {
    let hello_bytes = read_one_frame(stream)?;
    let hello: HelloWire =
        serde_json::from_slice(&hello_bytes).map_err(HandshakeError::MalformedHello)?;

    if hello.msg_type != "hello" {
        return Err(HandshakeError::UnexpectedFirstMessage { got: hello.msg_type });
    }

    let peer_pv = hello.pv.unwrap_or(protocol::ASSUMED_WHEN_ABSENT);

    let welcome = WelcomeWire {
        msg_type: protocol::wire_message::WELCOME,
        pv: protocol::WIRE_PROTOCOL_VERSION,
        min: protocol::MIN_SUPPORTED_PEER,
    };
    write_frame(
        stream,
        serde_json::to_string(&welcome)
            .expect("WelcomeWire always serializes")
            .as_bytes(),
    )?;

    if peer_pv < protocol::MIN_SUPPORTED_PEER {
        let update_required = UpdateRequiredWire {
            msg_type: protocol::wire_message::UPDATE_REQUIRED,
            target: "ios",
            store: "https://apps.apple.com/app/opendisplay".to_string(),
            message: "Please update the OpenDisplay app on your device.".to_string(),
        };
        write_frame(
            stream,
            serde_json::to_string(&update_required)
                .expect("UpdateRequiredWire always serializes")
                .as_bytes(),
        )?;
        return Err(HandshakeError::PeerBelowMinSupported { peer_pv });
    }

    Ok(Session {
        peer_pv,
        negotiated_pv: peer_pv.min(protocol::WIRE_PROTOCOL_VERSION),
        panel: PanelReport {
            pixels_wide: hello.pixels_wide,
            pixels_high: hello.pixels_high,
            scale: hello.scale,
            device: hello.device,
            id: hello.id,
        },
    })
}

/// One parsed in-session control message (`PROTOCOL.md` §6.1, receiver ->
/// sender). `Ignored` covers every type this sender doesn't act on —
/// `pencil`/`proximity` (out of scope, spec Touch/cursor AC 4) and any
/// future/unknown type — so unrecognized messages are a deliberate no-op,
/// never a parse error, per `PROTOCOL.md` Appendix A ("ignore every control
/// message it does not care about").
#[derive(Debug, Clone, PartialEq)]
pub enum ControlMessage {
    Touch { phase: crate::input::TouchPhase, x_norm: f64, y_norm: f64 },
    Scroll { dx: f64, dy: f64 },
    Sleeping,
    Closing,
    Ignored,
}

#[derive(Debug, Deserialize)]
struct ControlWire {
    #[serde(rename = "type")]
    msg_type: String,
    phase: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    dx: Option<f64>,
    dy: Option<f64>,
}

fn parse_touch_phase(phase: Option<&str>) -> crate::input::TouchPhase {
    match phase {
        Some("began") => crate::input::TouchPhase::Began,
        Some("ended") => crate::input::TouchPhase::Ended,
        Some("cancelled") => crate::input::TouchPhase::Cancelled,
        // "moved", absent, or unrecognized: never changes button state
        // (see `input::next_touch_action`), the safest default.
        _ => crate::input::TouchPhase::Moved,
    }
}

/// Parses one in-session control message payload. Never fails on an
/// unrecognized `type` (that's `ControlMessage::Ignored`, not an error) —
/// only malformed JSON is a parse error.
pub fn parse_control_message(payload: &[u8]) -> Result<ControlMessage, serde_json::Error> {
    let wire: ControlWire = serde_json::from_slice(payload)?;
    Ok(match wire.msg_type.as_str() {
        t if t == protocol::wire_message::TOUCH => ControlMessage::Touch {
            phase: parse_touch_phase(wire.phase.as_deref()),
            x_norm: wire.x.unwrap_or(0.0),
            y_norm: wire.y.unwrap_or(0.0),
        },
        t if t == protocol::wire_message::SCROLL => {
            ControlMessage::Scroll { dx: wire.dx.unwrap_or(0.0), dy: wire.dy.unwrap_or(0.0) }
        }
        t if t == protocol::wire_message::SLEEPING => ControlMessage::Sleeping,
        t if t == protocol::wire_message::CLOSING => ControlMessage::Closing,
        // pencil, proximity, ping/pong/hello/welcome echoes, and anything
        // else this sender doesn't act on in-session.
        _ => ControlMessage::Ignored,
    })
}

/// Applies one parsed control message: `touch`/`scroll` drive `injector`
/// (and `touch_is_down`'s persisted click state); `sleeping`/`closing`
/// become the matching `session_state::SessionEvent` for the caller to feed
/// into `session_state::transition`; `Ignored` touches nothing and returns
/// `None`.
pub fn apply_control_message(
    message: &ControlMessage,
    display: &crate::display_spec::DisplaySpec,
    injector: &mut dyn crate::input::InputInjector,
    touch_is_down: &mut bool,
) -> Option<crate::session_state::SessionEvent> {
    match message {
        ControlMessage::Touch { phase, x_norm, y_norm } => {
            crate::input::inject_touch(display, injector, touch_is_down, *phase, *x_norm, *y_norm);
            None
        }
        ControlMessage::Scroll { dx, dy } => {
            crate::input::inject_scroll(injector, *dx, *dy);
            None
        }
        ControlMessage::Sleeping => Some(crate::session_state::SessionEvent::ReceivedSleeping),
        ControlMessage::Closing => Some(crate::session_state::SessionEvent::ReceivedClosing),
        ControlMessage::Ignored => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// An in-memory stand-in for the dialed `TcpStream`, so the handshake
    /// is exercised without a live socket (per the task: "exercised via an
    /// injected fake peer").
    struct FakeStream {
        incoming: Cursor<Vec<u8>>,
        outgoing: Vec<u8>,
    }

    impl FakeStream {
        fn with_incoming(bytes: Vec<u8>) -> Self {
            Self { incoming: Cursor::new(bytes), outgoing: Vec::new() }
        }
    }

    impl Read for FakeStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.incoming.read(buf)
        }
    }

    impl Write for FakeStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.outgoing.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn hello_frame(json: &str) -> Vec<u8> {
        protocol::encode_frame(json.as_bytes()).unwrap()
    }

    /// Decodes every frame packed into `buf`, returning each payload as a
    /// JSON value.
    fn decode_all_frames(mut buf: &[u8]) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        while let Ok(Some(frame)) = protocol::decode_frame(buf) {
            out.push(serde_json::from_slice(frame.payload).unwrap());
            buf = &buf[frame.consumed..];
        }
        out
    }

    #[test]
    fn successful_handshake_yields_session_with_negotiated_pv_and_panel_dimensions() {
        let hello = hello_frame(
            r#"{"type":"hello","pixelsWide":1668,"pixelsHigh":2388,"scale":2.0,"device":"iPad","id":"00008030-ABC","pv":3}"#,
        );
        let mut stream = FakeStream::with_incoming(hello);

        let session = handshake(&mut stream).expect("compatible peer completes the handshake");

        assert_eq!(session.peer_pv, 3);
        assert_eq!(session.negotiated_pv, 3.min(protocol::WIRE_PROTOCOL_VERSION));
        assert_eq!(
            session.panel,
            PanelReport {
                pixels_wide: 1668,
                pixels_high: 2388,
                scale: 2.0,
                device: Some("iPad".to_string()),
                id: Some("00008030-ABC".to_string()),
            }
        );
    }

    #[test]
    fn hello_with_absent_pv_defaults_to_assumed_when_absent() {
        let hello = hello_frame(
            r#"{"type":"hello","pixelsWide":750,"pixelsHigh":1334,"scale":2.0}"#,
        );
        let mut stream = FakeStream::with_incoming(hello);

        let session = handshake(&mut stream).expect("pre-pv-2 peer still completes the handshake");

        assert_eq!(session.peer_pv, protocol::ASSUMED_WHEN_ABSENT);
    }

    #[test]
    fn successful_handshake_writes_a_welcome_frame_with_this_senders_pv_and_min() {
        let hello = hello_frame(
            r#"{"type":"hello","pixelsWide":1668,"pixelsHigh":2388,"scale":2.0,"pv":3}"#,
        );
        let mut stream = FakeStream::with_incoming(hello);

        handshake(&mut stream).expect("compatible peer completes the handshake");

        let frames = decode_all_frames(&stream.outgoing);
        assert_eq!(frames.len(), 1, "only welcome, no updateRequired, for a compatible peer");
        assert_eq!(frames[0]["type"], "welcome");
        assert_eq!(frames[0]["pv"], protocol::WIRE_PROTOCOL_VERSION);
        assert_eq!(frames[0]["min"], protocol::MIN_SUPPORTED_PEER);
    }

    #[test]
    fn peer_below_min_supported_peer_receives_update_required_and_session_ends_without_streaming() {
        // The real MIN_SUPPORTED_PEER is 1, so this injects a fake peer
        // advertising pv 0 to exercise the below-floor path, per the
        // task's note ("since the real constant is 1").
        let hello = hello_frame(
            r#"{"type":"hello","pixelsWide":750,"pixelsHigh":1334,"scale":2.0,"pv":0}"#,
        );
        let mut stream = FakeStream::with_incoming(hello);

        let result = handshake(&mut stream);

        match result {
            Err(HandshakeError::PeerBelowMinSupported { peer_pv }) => assert_eq!(peer_pv, 0),
            other => panic!("expected PeerBelowMinSupported, got {other:?}"),
        }

        let frames = decode_all_frames(&stream.outgoing);
        assert_eq!(frames.len(), 2, "welcome, then updateRequired");
        assert_eq!(frames[0]["type"], "welcome");
        assert_eq!(frames[1]["type"], "updateRequired");
        assert_eq!(frames[1]["target"], "ios");
    }

    // --- build_ping_message: spec WSEND-05. ---

    #[test]
    fn build_ping_message_produces_the_ping_wire_type() {
        let bytes = build_ping_message();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["type"], "ping");
    }

    // --- parse_control_message / apply_control_message: FIX3, WSEND-22. ---

    #[test]
    fn a_pencil_message_parses_to_ignored() {
        let msg = parse_control_message(br#"{"type":"pencil","phase":"down","x":0.5,"y":0.5}"#)
            .expect("recognized but out-of-scope type never errors");
        assert_eq!(msg, ControlMessage::Ignored);
    }

    #[test]
    fn a_proximity_message_parses_to_ignored() {
        let msg = parse_control_message(br#"{"type":"proximity"}"#).expect("never errors");
        assert_eq!(msg, ControlMessage::Ignored);
    }

    #[test]
    fn an_unrecognized_type_parses_to_ignored_not_an_error() {
        let msg = parse_control_message(br#"{"type":"somethingFromTheFuture"}"#)
            .expect("unknown type is a no-op, not an error");
        assert_eq!(msg, ControlMessage::Ignored);
    }

    #[test]
    fn a_touch_message_parses_its_phase_and_position() {
        let msg = parse_control_message(br#"{"type":"touch","phase":"began","x":0.25,"y":0.75}"#)
            .unwrap();
        assert_eq!(
            msg,
            ControlMessage::Touch {
                phase: crate::input::TouchPhase::Began,
                x_norm: 0.25,
                y_norm: 0.75,
            }
        );
    }

    #[test]
    fn a_scroll_message_parses_its_deltas() {
        let msg = parse_control_message(br#"{"type":"scroll","dx":1.0,"dy":-2.0}"#).unwrap();
        assert_eq!(msg, ControlMessage::Scroll { dx: 1.0, dy: -2.0 });
    }

    #[test]
    fn a_sleeping_message_parses_to_sleeping() {
        assert_eq!(
            parse_control_message(br#"{"type":"sleeping"}"#).unwrap(),
            ControlMessage::Sleeping
        );
    }

    #[test]
    fn a_closing_message_parses_to_closing() {
        assert_eq!(
            parse_control_message(br#"{"type":"closing"}"#).unwrap(),
            ControlMessage::Closing
        );
    }

    struct FakeInjector {
        moves: Vec<(i32, i32)>,
        downs: u32,
        ups: u32,
        scrolls: Vec<(i32, i32)>,
    }

    impl FakeInjector {
        fn new() -> Self {
            Self { moves: Vec::new(), downs: 0, ups: 0, scrolls: Vec::new() }
        }
    }

    impl crate::input::InputInjector for FakeInjector {
        fn move_cursor(&mut self, x: i32, y: i32) {
            self.moves.push((x, y));
        }
        fn mouse_down(&mut self) {
            self.downs += 1;
        }
        fn mouse_up(&mut self) {
            self.ups += 1;
        }
        fn scroll(&mut self, horizontal: i32, vertical: i32) {
            self.scrolls.push((horizontal, vertical));
        }
    }

    fn test_display() -> crate::display_spec::DisplaySpec {
        crate::display_spec::DisplaySpec {
            width_px: 1000,
            height_px: 2000,
            scale_factor: 2.0,
            orientation: crate::display_spec::Orientation::Portrait,
        }
    }

    #[test]
    fn applying_an_ignored_message_leaves_touch_state_and_injector_untouched() {
        let mut injector = FakeInjector::new();
        let mut is_down = false;

        let event = apply_control_message(
            &ControlMessage::Ignored,
            &test_display(),
            &mut injector,
            &mut is_down,
        );

        assert_eq!(event, None);
        assert!(injector.moves.is_empty());
        assert_eq!(injector.downs, 0);
        assert_eq!(injector.ups, 0);
        assert!(injector.scrolls.is_empty());
        assert!(!is_down);
    }

    #[test]
    fn applying_a_touch_message_dispatches_through_input_inject_touch() {
        let mut injector = FakeInjector::new();
        let mut is_down = false;
        let message = ControlMessage::Touch {
            phase: crate::input::TouchPhase::Began,
            x_norm: 0.5,
            y_norm: 0.5,
        };

        let event =
            apply_control_message(&message, &test_display(), &mut injector, &mut is_down);

        assert_eq!(event, None);
        assert_eq!(injector.moves, vec![(500, 1000)]);
        assert_eq!(injector.downs, 1);
        assert!(is_down);
    }

    #[test]
    fn applying_a_scroll_message_dispatches_through_input_inject_scroll() {
        let mut injector = FakeInjector::new();
        let mut is_down = false;
        let message = ControlMessage::Scroll { dx: 40.0, dy: 0.0 };

        apply_control_message(&message, &test_display(), &mut injector, &mut is_down);

        assert_eq!(injector.scrolls, vec![(crate::input::WHEEL_DELTA, 0)]);
    }

    #[test]
    fn applying_sleeping_yields_the_received_sleeping_session_event() {
        let mut injector = FakeInjector::new();
        let mut is_down = false;

        let event = apply_control_message(
            &ControlMessage::Sleeping,
            &test_display(),
            &mut injector,
            &mut is_down,
        );

        assert_eq!(event, Some(crate::session_state::SessionEvent::ReceivedSleeping));
    }

    #[test]
    fn applying_closing_yields_the_received_closing_session_event() {
        let mut injector = FakeInjector::new();
        let mut is_down = false;

        let event = apply_control_message(
            &ControlMessage::Closing,
            &test_display(),
            &mut injector,
            &mut is_down,
        );

        assert_eq!(event, Some(crate::session_state::SessionEvent::ReceivedClosing));
    }
}
