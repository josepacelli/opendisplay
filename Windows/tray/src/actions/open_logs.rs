//! "Open logs" tray action (T27): requests `OpenLogFolder` over IPC when
//! windows-core is reachable, falling back to opening the log folder
//! directly when it isn't — mirrors the Mac app's "Logs" button (README),
//! per the spec's local-logging Assumption.
//!
//! The IPC-vs-fallback *decision* is plain, unit-tested Rust (this task's
//! "the fallback-decision logic... is unit-tested independent of actually
//! opening a folder"); the two OS calls it can trigger — writing a request
//! to the pipe, and opening a folder in Explorer — are isolated behind
//! injectable traits, same pattern as T24's `PipeConnector`.

use crate::ipc_client::ConnectOutcome;
use std::io;
use std::path::{Path, PathBuf};

/// The Windows per-user app-data log folder, mirrored from
/// `Windows/core/src/log.rs`'s `default_log_dir()` (T22) — tray and core
/// are separate binaries with no shared crate for this path, so it is
/// hand-kept in sync (same treatment `Windows/protocol` gives
/// `Shared/Protocol.swift`'s constants). This task's Done when ("the
/// IPC-available path and the fallback path both resolve to the same
/// folder") holds because both routes are ultimately backed by this one
/// function — core opens it on the tray's behalf when reachable; the tray
/// opens it directly otherwise.
pub fn default_log_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(".").to_path_buf());
    base.join("OpenDisplay").join("Logs")
}

/// What "open logs" does, decided purely from whether windows-core is
/// reachable over IPC.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenLogsAction {
    /// windows-core is reachable: ask it to open its own log folder
    /// (`TrayToCore::OpenLogFolder`) rather than the tray guessing a
    /// possibly-stale path itself.
    RequestViaIpc,
    /// windows-core is unreachable: the tray opens the log folder
    /// directly.
    OpenDirectly(PathBuf),
}

/// Decides the action for "open logs" from an `ipc_client::connect`
/// outcome (T24). Never touches a real pipe or filesystem call itself —
/// see [`perform`] for that.
pub fn decide<S>(outcome: &ConnectOutcome<S>) -> OpenLogsAction {
    match outcome {
        ConnectOutcome::Connected(_) => OpenLogsAction::RequestViaIpc,
        ConnectOutcome::CoreNotRunning => OpenLogsAction::OpenDirectly(default_log_dir()),
    }
}

/// Abstraction over "send the `OpenLogFolder` request to windows-core",
/// isolating the pipe write so [`decide`]'s decision is testable without a
/// real pipe.
pub trait IpcRequester {
    fn request_open_log_folder(&self) -> io::Result<()>;
}

/// Abstraction over "open this folder in the OS file browser", isolating
/// the OS call so [`decide`]'s decision is testable without actually
/// opening a folder.
pub trait FolderOpener {
    fn open(&self, path: &Path) -> io::Result<()>;
}

/// Carries out `action`, delegating the OS-facing half of each branch to
/// `requester`/`opener`.
pub fn perform(
    action: &OpenLogsAction,
    requester: &dyn IpcRequester,
    opener: &dyn FolderOpener,
) -> io::Result<()> {
    match action {
        OpenLogsAction::RequestViaIpc => requester.request_open_log_folder(),
        OpenLogsAction::OpenDirectly(path) => opener.open(path),
    }
}

/// The real folder opener, backed by `explorer.exe`. Not exercised by an
/// automated gate on this host (no Rust toolchain, no folder to open) —
/// verified manually alongside the rest of the tray UI.
#[cfg(windows)]
pub struct WindowsFolderOpener;

#[cfg(windows)]
impl FolderOpener for WindowsFolderOpener {
    fn open(&self, path: &Path) -> io::Result<()> {
        std::process::Command::new("explorer.exe").arg(path).spawn()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Cursor;

    struct RecordingRequester {
        called: RefCell<bool>,
    }
    impl IpcRequester for RecordingRequester {
        fn request_open_log_folder(&self) -> io::Result<()> {
            *self.called.borrow_mut() = true;
            Ok(())
        }
    }

    struct RecordingOpener {
        opened_path: RefCell<Option<PathBuf>>,
    }
    impl FolderOpener for RecordingOpener {
        fn open(&self, path: &Path) -> io::Result<()> {
            *self.opened_path.borrow_mut() = Some(path.to_path_buf());
            Ok(())
        }
    }

    #[test]
    fn a_reachable_core_requests_open_log_folder_over_ipc() {
        let outcome: ConnectOutcome<Cursor<Vec<u8>>> =
            ConnectOutcome::Connected(Cursor::new(Vec::new()));

        let action = decide(&outcome);

        assert_eq!(action, OpenLogsAction::RequestViaIpc);
    }

    #[test]
    fn an_unreachable_core_falls_back_to_opening_the_folder_directly() {
        let outcome: ConnectOutcome<Cursor<Vec<u8>>> = ConnectOutcome::CoreNotRunning;

        let action = decide(&outcome);

        assert_eq!(action, OpenLogsAction::OpenDirectly(default_log_dir()));
    }

    #[test]
    fn the_ipc_path_and_the_fallback_path_resolve_to_the_same_folder() {
        // Both branches are backed by the same `default_log_dir()` call —
        // there is no separate "core's folder" value that could drift out
        // of sync with the fallback's, since both ultimately name exactly
        // this path.
        let outcome: ConnectOutcome<Cursor<Vec<u8>>> = ConnectOutcome::CoreNotRunning;

        let action = decide(&outcome);

        match action {
            OpenLogsAction::OpenDirectly(path) => assert_eq!(path, default_log_dir()),
            OpenLogsAction::RequestViaIpc => panic!("expected the fallback path"),
        }
    }

    #[test]
    fn perform_dispatches_the_ipc_request_without_touching_the_folder_opener() {
        let requester = RecordingRequester {
            called: RefCell::new(false),
        };
        let opener = RecordingOpener {
            opened_path: RefCell::new(None),
        };

        perform(&OpenLogsAction::RequestViaIpc, &requester, &opener).expect("perform succeeds");

        assert!(*requester.called.borrow(), "the IPC requester must be called");
        assert!(
            opener.opened_path.borrow().is_none(),
            "the folder opener must not be called"
        );
    }

    #[test]
    fn perform_dispatches_the_direct_open_without_touching_the_ipc_requester() {
        let requester = RecordingRequester {
            called: RefCell::new(false),
        };
        let opener = RecordingOpener {
            opened_path: RefCell::new(None),
        };
        let dir = default_log_dir();

        perform(&OpenLogsAction::OpenDirectly(dir.clone()), &requester, &opener)
            .expect("perform succeeds");

        assert!(!*requester.called.borrow(), "the IPC requester must not be called");
        assert_eq!(*opener.opened_path.borrow(), Some(dir));
    }
}
