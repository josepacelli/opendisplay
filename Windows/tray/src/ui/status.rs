//! Status display (T26): renders `CoreToTray::Status`/`Error` messages and
//! the pipe-connection outcome from `ipc_client` (T24) as a small set of
//! distinct tray states, per design.md's Error Handling Strategy
//! ("`windows-core` not running or not elevated... shows it, instead of a
//! picker that silently does nothing").
//!
//! Per the Test Coverage Matrix, tray UI rendering is OS-bound (Tests:
//! none) — manual verification against a live session is the only gate.
//! The classification logic below is kept plain so it stays legible even
//! though it isn't unit-tested for this task.

use crate::ipc_client::ConnectOutcome;
use ipc::{CoreToTray, SessionStats, Transport};

/// The message text `windows-core` sends as a `CoreToTray::Error` to
/// report its own startup elevation self-check failing (`core::main`'s
/// `BootstrapStatus::NotElevated`, T9).
///
/// SPEC_DEVIATION: the `Windows/ipc` schema (T3, already committed) has no
/// dedicated "not elevated" variant — only `DeviceList`/`Status`/`Error`.
/// Recognizing it via this message-content convention on `Error` is the
/// only way to surface it distinctly without changing that already-shipped
/// schema, which this task does not list as a file to touch.
/// Reason: adding an `ipc` schema variant would touch `Windows/ipc/src/lib.rs`,
/// outside T26's `Where` (`Windows/tray/src/ui/status.rs`).
pub const NOT_ELEVATED_MESSAGE: &str = "not elevated";

/// The distinct states this task's status view renders (this task's Done
/// when: "Connected/disconnected/not-elevated/core-not-running states each
/// render distinctly").
#[derive(Debug, Clone, PartialEq)]
pub enum TrayStatus {
    /// The pipe to windows-core is missing or unreadable.
    CoreNotRunning,
    /// windows-core is reachable but reported failing its elevation
    /// self-check.
    NotElevated,
    /// Reachable, elevated, but no active session.
    Disconnected,
    /// An active session, with the device/transport/stats windows-core
    /// last reported.
    Connected {
        device: Option<String>,
        transport: Option<Transport>,
        stats: Option<SessionStats>,
    },
}

/// Classifies an `ipc_client::connect` outcome that failed to even reach
/// windows-core, before any `CoreToTray` message could be read. Returns
/// `None` for a live connection — the status is then driven by
/// [`status_for_message`] instead.
pub fn status_for_connect_failure<S>(outcome: &ConnectOutcome<S>) -> Option<TrayStatus> {
    match outcome {
        ConnectOutcome::CoreNotRunning => Some(TrayStatus::CoreNotRunning),
        ConnectOutcome::Connected(_) => None,
    }
}

/// Classifies one `CoreToTray` message into the status the tray renders.
/// `DeviceList` doesn't affect status (handled by `ui::picker`), so it maps
/// to `None` — callers keep the previously rendered status unchanged. An
/// `Error` whose message isn't the recognized [`NOT_ELEVATED_MESSAGE`] also
/// maps to `None`: this task's scope is the four states named in its Done
/// when, not every possible error string (e.g. "driver missing" is
/// `ui::first_run`'s concern, T28).
pub fn status_for_message(msg: &CoreToTray) -> Option<TrayStatus> {
    match msg {
        CoreToTray::DeviceList(_) => None,
        CoreToTray::Status {
            device,
            transport,
            connected,
            stats,
        } => Some(if *connected {
            TrayStatus::Connected {
                device: device.clone(),
                transport: *transport,
                stats: stats.clone(),
            }
        } else {
            TrayStatus::Disconnected
        }),
        CoreToTray::Error { message } if message == NOT_ELEVATED_MESSAGE => {
            Some(TrayStatus::NotElevated)
        }
        CoreToTray::Error { .. } => None,
    }
}

/// Native rendering of [`TrayStatus`] as the tray icon's tooltip text, via
/// `Shell_NotifyIconW(NIM_MODIFY)`. OS UI surface with no automated-test
/// ROI per the Test Coverage Matrix; not exercised by an automated gate on
/// this host (no Rust/Win32 toolchain) — verified manually against a live
/// session.
#[cfg(windows)]
pub mod windows_impl {
    use super::TrayStatus;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{Shell_NotifyIconW, NOTIFYICONDATAW, NIF_TIP, NIM_MODIFY};

    fn tooltip_text(status: &TrayStatus) -> String {
        match status {
            TrayStatus::CoreNotRunning => "OpenDisplay: core not running".to_string(),
            TrayStatus::NotElevated => "OpenDisplay: not elevated — setup incomplete".to_string(),
            TrayStatus::Disconnected => "OpenDisplay: no device connected".to_string(),
            TrayStatus::Connected { device, .. } => match device {
                Some(name) => format!("OpenDisplay: connected to {name}"),
                None => "OpenDisplay: connected".to_string(),
            },
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Updates the tray icon (added by `ui::picker`'s `add_tray_icon`) to
    /// reflect `status`, per this task's "each render distinctly".
    pub fn render(hwnd: HWND, status: &TrayStatus) -> windows::core::Result<()> {
        let tip = wide(&tooltip_text(status));
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_TIP,
            ..Default::default()
        };
        let len = tip.len().min(data.szTip.len());
        data.szTip[..len].copy_from_slice(&tip[..len]);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data).ok() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- status_for_connect_failure ---

    #[test]
    fn a_core_not_running_outcome_yields_core_not_running_status() {
        let outcome: ConnectOutcome<()> = ConnectOutcome::CoreNotRunning;
        assert_eq!(status_for_connect_failure(&outcome), Some(TrayStatus::CoreNotRunning));
    }

    #[test]
    fn a_connected_outcome_yields_no_status_change() {
        let outcome = ConnectOutcome::Connected(());
        assert_eq!(status_for_connect_failure(&outcome), None);
    }

    // --- status_for_message ---

    #[test]
    fn a_device_list_message_yields_no_status_change() {
        assert_eq!(status_for_message(&CoreToTray::DeviceList(vec![])), None);
    }

    #[test]
    fn a_connected_status_message_yields_connected_with_its_fields() {
        let msg = CoreToTray::Status {
            device: Some("iPad".to_string()),
            transport: Some(Transport::Wifi),
            connected: true,
            stats: None,
        };
        assert_eq!(
            status_for_message(&msg),
            Some(TrayStatus::Connected {
                device: Some("iPad".to_string()),
                transport: Some(Transport::Wifi),
                stats: None,
            })
        );
    }

    #[test]
    fn a_disconnected_status_message_yields_disconnected() {
        let msg = CoreToTray::Status { device: None, transport: None, connected: false, stats: None };
        assert_eq!(status_for_message(&msg), Some(TrayStatus::Disconnected));
    }

    #[test]
    fn a_not_elevated_error_message_yields_not_elevated_status() {
        let msg = CoreToTray::Error { message: NOT_ELEVATED_MESSAGE.to_string() };
        assert_eq!(status_for_message(&msg), Some(TrayStatus::NotElevated));
    }

    #[test]
    fn an_unrecognized_error_message_yields_no_status_change() {
        let msg = CoreToTray::Error { message: "something else".to_string() };
        assert_eq!(status_for_message(&msg), None);
    }
}
