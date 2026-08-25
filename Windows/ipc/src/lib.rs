//! Tray <-> core local IPC schema crate.
//!
//! Defines the newline-delimited JSON message schema exchanged between
//! `windows-tray` (unprivileged) and `windows-core` (elevated) over a
//! named pipe, per `design.md`'s Data Models section. This is a purely
//! local schema — distinct from, and never mixed with, the network wire
//! protocol in `Windows/protocol`.

use serde::{Deserialize, Serialize};

/// Which transport a device is reachable/connected over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transport {
    Wifi,
    Usb,
}

/// A device windows-core has discovered (over WiFi and/or USB), as shown
/// to the tray's device picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: String,
    pub transport: Transport,
}

/// Live session statistics shown in the tray's status view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionStats {
    pub fps: f32,
    pub bitrate_kbps: u32,
}

/// Messages windows-core emits to windows-tray.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CoreToTray {
    DeviceList(Vec<DiscoveredDevice>),
    Status {
        device: Option<String>,
        transport: Option<Transport>,
        connected: bool,
        stats: Option<SessionStats>,
    },
    Error {
        message: String,
    },
}

/// Messages windows-tray sends to windows-core. windows-core MUST treat
/// every value of this type as untrusted input (it crosses the
/// unprivileged -> elevated boundary) — see design.md's Risks & Concerns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TrayToCore {
    Connect { device_id: String },
    Disconnect,
    OpenLogFolder,
}

/// An explicit, non-panicking error for a line that failed to parse as an
/// IPC message — either malformed JSON or JSON of an unexpected shape.
#[derive(Debug)]
pub struct IpcError(serde_json::Error);

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "malformed IPC message: {}", self.0)
    }
}

impl std::error::Error for IpcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl From<serde_json::Error> for IpcError {
    fn from(err: serde_json::Error) -> Self {
        IpcError(err)
    }
}

/// Serializes an IPC message to a single JSON line (no trailing newline;
/// the pipe writer is responsible for framing lines with `\n`).
pub fn to_line<T: Serialize>(msg: &T) -> String {
    serde_json::to_string(msg).expect("IPC message types are always JSON-serializable")
}

/// Parses one line of newline-delimited JSON into an IPC message.
/// Returns `Err(IpcError)` for malformed or unexpected JSON rather than
/// panicking, so the caller (e.g. `windows-core`'s `ipc::serve`) can reject
/// and log it instead of acting on it.
pub fn from_line<T: for<'de> Deserialize<'de>>(line: &str) -> Result<T, IpcError> {
    serde_json::from_str(line).map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_to_tray_device_list_round_trips() {
        let msg = CoreToTray::DeviceList(vec![DiscoveredDevice {
            id: "00008030-ABC".into(),
            name: "Jose's iPad".into(),
            transport: Transport::Wifi,
        }]);

        let line = to_line(&msg);
        let decoded: CoreToTray = from_line(&line).expect("valid message decodes");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn core_to_tray_status_round_trips_with_all_fields_present() {
        let msg = CoreToTray::Status {
            device: Some("00008030-ABC".into()),
            transport: Some(Transport::Usb),
            connected: true,
            stats: Some(SessionStats {
                fps: 60.0,
                bitrate_kbps: 8000,
            }),
        };

        let line = to_line(&msg);
        let decoded: CoreToTray = from_line(&line).expect("valid message decodes");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn core_to_tray_status_round_trips_with_no_device_connected() {
        let msg = CoreToTray::Status {
            device: None,
            transport: None,
            connected: false,
            stats: None,
        };

        let line = to_line(&msg);
        let decoded: CoreToTray = from_line(&line).expect("valid message decodes");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn core_to_tray_error_round_trips() {
        let msg = CoreToTray::Error {
            message: "driver not installed".into(),
        };

        let line = to_line(&msg);
        let decoded: CoreToTray = from_line(&line).expect("valid message decodes");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn tray_to_core_connect_round_trips() {
        let msg = TrayToCore::Connect {
            device_id: "00008030-ABC".into(),
        };

        let line = to_line(&msg);
        let decoded: TrayToCore = from_line(&line).expect("valid message decodes");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn tray_to_core_disconnect_round_trips() {
        let msg = TrayToCore::Disconnect;

        let line = to_line(&msg);
        let decoded: TrayToCore = from_line(&line).expect("valid message decodes");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn tray_to_core_open_log_folder_round_trips() {
        let msg = TrayToCore::OpenLogFolder;

        let line = to_line(&msg);
        let decoded: TrayToCore = from_line(&line).expect("valid message decodes");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn malformed_json_line_yields_an_explicit_error() {
        let result: Result<TrayToCore, IpcError> = from_line("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn well_formed_but_unexpected_json_shape_yields_an_explicit_error() {
        // Valid JSON, but not a shape any TrayToCore variant matches.
        let result: Result<TrayToCore, IpcError> = from_line(r#"{"unexpectedField": 42}"#);
        assert!(result.is_err());
    }

    #[test]
    fn parsing_a_malformed_line_never_panics() {
        let outcome = std::panic::catch_unwind(|| {
            let _: Result<TrayToCore, IpcError> = from_line("{ this is not valid json");
        });
        assert!(outcome.is_ok(), "from_line must return Err, not panic, on malformed input");
    }
}
