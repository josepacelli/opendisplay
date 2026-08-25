//! A named pipe server, ACL-restricted to the current user's SID, that
//! emits `CoreToTray` messages and validates+handles incoming `TrayToCore`
//! messages — the elevated side of the tray<->core privilege boundary
//! (design.md's Risks & Concerns: "named pipe IPC crosses a privilege
//! boundary").
//!
//! Split the same way the rest of `windows-core` is: message
//! validation/interpretation and the device-list/state bookkeeping it
//! drives are plain, unit-testable Rust; the actual named-pipe creation and
//! ACL restriction are OS calls, verified manually per the Test Coverage
//! Matrix's "highest test type required by any layer it touches" rule
//! (same treatment T20 gives `SendInput`).

use crate::session_state::SessionState;
use crate::transport::{usb, wifi};
use ipc::{CoreToTray, DiscoveredDevice as IpcDevice, Transport as IpcTransport, TrayToCore};

/// What the server should do in response to one incoming line from the
/// tray. `windows-core` treats every line as untrusted input (crosses the
/// unprivileged -> elevated boundary, design.md's Error Handling
/// Strategy), so a malformed line becomes `RejectedMalformed`, never an
/// action.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerAction {
    Connect { device_id: String },
    Disconnect,
    OpenLogFolder,
    /// The line was malformed or an unexpected shape; it was rejected, not
    /// acted on.
    RejectedMalformed { raw_line: String },
}

/// Parses one raw line into the [`ServerAction`] it authorizes. A line that
/// fails to parse as `TrayToCore` — malformed JSON or a well-formed but
/// unexpected shape — becomes `RejectedMalformed`, never a `Connect`,
/// `Disconnect`, or `OpenLogFolder` action (design.md: "A malformed
/// TrayToCore message is rejected and logged, never acted on").
pub fn handle_incoming_line(raw_line: &str) -> ServerAction {
    match ipc::from_line::<TrayToCore>(raw_line) {
        Ok(TrayToCore::Connect { device_id }) => ServerAction::Connect { device_id },
        Ok(TrayToCore::Disconnect) => ServerAction::Disconnect,
        Ok(TrayToCore::OpenLogFolder) => ServerAction::OpenLogFolder,
        Err(_) => ServerAction::RejectedMalformed {
            raw_line: raw_line.to_string(),
        },
    }
}

/// A side effect [`IpcServer::apply`] asks the runtime to carry out. Kept
/// as data (not a direct call) so `apply`'s bookkeeping is testable without
/// a live dial, a live session, or a live log writer.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Dial and hand off to the session state machine (T14) for this
    /// device.
    DialDevice { device_id: String },
    /// Tear down the current session.
    TeardownSession,
    /// Open the log folder via the log writer (T22).
    OpenLogFolder,
    /// Log the rejected line — never acted on beyond logging.
    LogRejectedMessage { raw_line: String },
}

/// The server's bookkeeping: which device (if any) is the current target.
/// Owns no I/O itself — `apply` only decides what should happen next.
pub struct IpcServer {
    pub current_device_id: Option<String>,
}

impl IpcServer {
    pub fn new() -> Self {
        Self {
            current_device_id: None,
        }
    }

    /// Applies one already-parsed [`ServerAction`], updating bookkeeping
    /// and returning the [`Effect`]s the runtime must carry out —
    /// `Connect`/`Disconnect`/`OpenLogFolder` drive the session state
    /// machine (T14) and log writer (T22) this way.
    pub fn apply(&mut self, action: ServerAction) -> Vec<Effect> {
        match action {
            ServerAction::Connect { device_id } => {
                self.current_device_id = Some(device_id.clone());
                vec![Effect::DialDevice { device_id }]
            }
            ServerAction::Disconnect => {
                self.current_device_id = None;
                vec![Effect::TeardownSession]
            }
            ServerAction::OpenLogFolder => vec![Effect::OpenLogFolder],
            ServerAction::RejectedMalformed { raw_line } => {
                vec![Effect::LogRejectedMessage { raw_line }]
            }
        }
    }
}

impl Default for IpcServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds the `CoreToTray::DeviceList` from the currently discovered WiFi
/// (T10) and USB (T11) devices. A WiFi device with no `id` (pre-`pv`-2
/// receiver, `PROTOCOL.md` §2.1) is left out — the tray has nothing to
/// `Connect` it with.
pub fn build_device_list(
    wifi_devices: &[wifi::DiscoveredDevice],
    usb_devices: &[usb::DiscoveredDevice],
) -> CoreToTray {
    let mut devices: Vec<IpcDevice> = Vec::new();
    for d in usb_devices {
        devices.push(IpcDevice {
            id: d.id.clone(),
            name: d.id.clone(),
            transport: IpcTransport::Usb,
        });
    }
    for d in wifi_devices {
        if let Some(id) = &d.id {
            devices.push(IpcDevice {
                id: id.clone(),
                name: d.name.clone(),
                transport: IpcTransport::Wifi,
            });
        }
    }
    CoreToTray::DeviceList(devices)
}

/// Builds the `CoreToTray::Status` message for the current session state,
/// per design.md's Data Models. `device`/`transport` describe the target
/// selected by the last `Connect`, if any; `connected` reflects whether the
/// session machine (T14) is in `Connected`.
pub fn build_status(current_device_id: &Option<String>, state: Option<SessionState>) -> CoreToTray {
    CoreToTray::Status {
        device: current_device_id.clone(),
        transport: None,
        connected: matches!(state, Some(SessionState::Connected)),
        stats: None,
    }
}

/// The real named-pipe server, ACL-restricted to the current user's SID,
/// per design.md's Risks & Concerns mitigation ("Pipe ACL restricted to
/// the current user's SID"). Not exercised by any automated gate on this
/// host (no Rust toolchain, no OS pipe to open) — see the Test Coverage
/// Matrix's manual-verification note for OS-bound code. Every non-trivial
/// branch this module needs (message validation, device-list/state
/// bookkeeping) lives above this line and is unit-tested without it.
#[cfg(windows)]
pub mod windows_impl {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, LocalFree, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_WAIT,
    };

    /// The named pipe's ACL: full access to the current logon session's
    /// owner (`OW`/`BA` in SDDL terms restrict it to the account that owns
    /// the process token — the current user), denying everyone else. Using
    /// an owner-relative SDDL string (`D:(A;;GA;;;OW)`) rather than a
    /// hard-coded SID means it always resolves to whichever user account
    /// `windows-core` is running as, without querying the SID separately.
    const PIPE_SECURITY_DESCRIPTOR_SDDL: &str = "D:(A;;GA;;;OW)";

    fn build_security_attributes() -> windows::core::Result<SECURITY_ATTRIBUTES> {
        let sddl: Vec<u16> = PIPE_SECURITY_DESCRIPTOR_SDDL
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )?;
        }
        Ok(SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        })
    }

    /// Opens the named pipe `pipe_name` (e.g. `\\.\pipe\opendisplay-core`)
    /// with the user-SID-restricted ACL above.
    pub fn create_pipe(pipe_name: &str) -> windows::core::Result<HANDLE> {
        let attributes = build_security_attributes()?;
        let wide_name: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide_name.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                Some(&attributes),
            )
        };
        unsafe {
            let _ = LocalFree(HLOCAL(attributes.lpSecurityDescriptor));
        }
        if handle.is_invalid() {
            Err(windows::core::Error::from_win32())
        } else {
            Ok(handle)
        }
    }

    /// Closes a pipe handle opened by [`create_pipe`]. The connect/read/
    /// write loop itself (accepting a client, reading newline-delimited
    /// `TrayToCore` lines via `super::handle_incoming_line`, writing
    /// `CoreToTray` lines back) is a runtime concern wired in by whatever
    /// drives `windows-core`'s event loop — not modeled here, same as
    /// `session_state`'s retry timer.
    pub fn close_pipe(handle: HANDLE) {
        unsafe {
            let _ = CloseHandle(handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Message validation: malformed input is rejected, never acted on. ---

    #[test]
    fn a_connect_line_yields_a_connect_action() {
        let line = ipc::to_line(&TrayToCore::Connect {
            device_id: "00008030-ABC".into(),
        });
        assert_eq!(
            handle_incoming_line(&line),
            ServerAction::Connect { device_id: "00008030-ABC".into() }
        );
    }

    #[test]
    fn a_disconnect_line_yields_a_disconnect_action() {
        let line = ipc::to_line(&TrayToCore::Disconnect);
        assert_eq!(handle_incoming_line(&line), ServerAction::Disconnect);
    }

    #[test]
    fn an_open_log_folder_line_yields_an_open_log_folder_action() {
        let line = ipc::to_line(&TrayToCore::OpenLogFolder);
        assert_eq!(handle_incoming_line(&line), ServerAction::OpenLogFolder);
    }

    #[test]
    fn a_malformed_line_is_rejected_not_acted_on() {
        let action = handle_incoming_line("not json at all");
        assert_eq!(
            action,
            ServerAction::RejectedMalformed { raw_line: "not json at all".to_string() }
        );
    }

    #[test]
    fn a_well_formed_but_unexpected_shape_is_rejected_not_acted_on() {
        let action = handle_incoming_line(r#"{"unexpectedField": 42}"#);
        assert!(matches!(action, ServerAction::RejectedMalformed { .. }));
    }

    // --- apply(): Connect/Disconnect/OpenLogFolder drive state + effects. ---

    #[test]
    fn applying_connect_sets_the_current_device_and_requests_a_dial() {
        let mut server = IpcServer::new();

        let effects = server.apply(ServerAction::Connect { device_id: "A".into() });

        assert_eq!(effects, vec![Effect::DialDevice { device_id: "A".into() }]);
        assert_eq!(server.current_device_id, Some("A".to_string()));
    }

    #[test]
    fn applying_disconnect_clears_the_current_device_and_requests_teardown() {
        let mut server = IpcServer::new();
        server.apply(ServerAction::Connect { device_id: "A".into() });

        let effects = server.apply(ServerAction::Disconnect);

        assert_eq!(effects, vec![Effect::TeardownSession]);
        assert_eq!(server.current_device_id, None);
    }

    #[test]
    fn applying_open_log_folder_requests_it_without_changing_the_current_device() {
        let mut server = IpcServer::new();
        server.apply(ServerAction::Connect { device_id: "A".into() });

        let effects = server.apply(ServerAction::OpenLogFolder);

        assert_eq!(effects, vec![Effect::OpenLogFolder]);
        assert_eq!(server.current_device_id, Some("A".to_string()));
    }

    #[test]
    fn applying_a_rejected_malformed_action_only_logs_never_dials_or_tears_down() {
        let mut server = IpcServer::new();

        let effects = server.apply(ServerAction::RejectedMalformed { raw_line: "junk".into() });

        assert_eq!(effects, vec![Effect::LogRejectedMessage { raw_line: "junk".into() }]);
        assert_eq!(server.current_device_id, None, "a malformed message must never be acted on");
    }

    // --- build_device_list: merges wifi + usb into the wire shape. ---

    #[test]
    fn build_device_list_merges_usb_and_wifi_devices_with_correct_transport_tags() {
        let wifi_devices = vec![wifi::DiscoveredDevice {
            id: Some("W1".into()),
            name: "Jose's iPad".into(),
            address: "192.168.1.42:9000".parse().unwrap(),
            pv: 3,
        }];
        let usb_devices = vec![usb::DiscoveredDevice { id: "U1".into() }];

        let list = build_device_list(&wifi_devices, &usb_devices);

        match list {
            CoreToTray::DeviceList(devices) => {
                assert_eq!(devices.len(), 2);
                assert!(devices.iter().any(|d| d.id == "U1" && d.transport == IpcTransport::Usb));
                assert!(devices.iter().any(|d| d.id == "W1"
                    && d.name == "Jose's iPad"
                    && d.transport == IpcTransport::Wifi));
            }
            other => panic!("expected DeviceList, got {other:?}"),
        }
    }

    #[test]
    fn build_device_list_excludes_a_wifi_device_with_no_id() {
        let wifi_devices = vec![wifi::DiscoveredDevice {
            id: None,
            name: "Old iPhone".into(),
            address: "192.168.1.50:9000".parse().unwrap(),
            pv: 1,
        }];

        let list = build_device_list(&wifi_devices, &[]);

        match list {
            CoreToTray::DeviceList(devices) => assert!(devices.is_empty()),
            other => panic!("expected DeviceList, got {other:?}"),
        }
    }
}
