//! windows-tray's named-pipe client: connects to windows-core's IPC pipe
//! (`Windows/core/src/ipc_server.rs`, T23) and exposes its `CoreToTray`
//! message stream.
//!
//! Design Risk mitigation ("`windows-core` not running or not elevated" —
//! design.md's Error Handling Strategy): a missing or unreadable pipe must
//! surface as an explicit "core not running" state, never a picker that
//! silently shows an empty list. The actual pipe connect
//! (`\\.\pipe\opendisplay-core`) is a Windows OS call, so it is isolated
//! behind the `PipeConnector` trait — `connect()`'s classification decision
//! (connected vs. core not running) is plain, unit-testable Rust that never
//! touches a real pipe, same pattern as `windows-core`'s
//! `TokenIntegrityCheck` (T9) and `ipc_server::handle_incoming_line` (T23).

use ipc::{CoreToTray, IpcError};
use std::io::BufRead;

/// The pipe name `windows-core`'s `ipc::serve` (T23) listens on, per
/// design.md's IPC schema ("Local IPC (tray <-> core)").
pub const PIPE_NAME: &str = r"\\.\pipe\opendisplay-core";

/// The outcome of attempting to reach `windows-core` over the pipe.
#[derive(Debug)]
pub enum ConnectOutcome<S> {
    /// The pipe connected; `S` is the live stream callers read `CoreToTray`
    /// messages from via [`read_message`].
    Connected(S),
    /// The pipe is missing or unreadable — `windows-core` is not running,
    /// or is running but not elevated. Never conflated with "no devices".
    CoreNotRunning,
}

/// Abstraction over "open a connection to the named pipe at `pipe_name`",
/// so [`connect`]'s classification decision is testable without an OS call.
pub trait PipeConnector {
    type Stream;
    fn connect(&self, pipe_name: &str) -> std::io::Result<Self::Stream>;
}

/// Attempts to connect to `windows-core`'s pipe via `connector`, classifying
/// the result per design.md's "core not running / not elevated" handling —
/// this task's Done when ("a missing or unreadable pipe yields an explicit
/// 'core not running' state, not a silent empty list").
pub fn connect<C: PipeConnector>(connector: &C, pipe_name: &str) -> ConnectOutcome<C::Stream> {
    match connector.connect(pipe_name) {
        Ok(stream) => ConnectOutcome::Connected(stream),
        Err(_) => ConnectOutcome::CoreNotRunning,
    }
}

/// Reads and parses the next `CoreToTray` message from `reader`'s newline-
/// delimited JSON, per `Windows/ipc`'s schema (T3). Returns `None` at
/// end-of-stream (the pipe closed) rather than an error — an ended stream
/// is an ordinary disconnect, not a malformed message. This is how a
/// successful connection's live `CoreToTray` stream (this task's other Done
/// when) is surfaced to callers, independent of the OS-specific stream type
/// `PipeConnector::Stream` resolves to.
pub fn read_message<R: BufRead>(reader: &mut R) -> Option<Result<CoreToTray, IpcError>> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(ipc::from_line(line.trim_end())),
        Err(_) => None,
    }
}

/// The real named-pipe connector, backed by the Windows API. A named pipe
/// is addressable as an ordinary file path (`\\.\pipe\...`), so
/// `std::fs::OpenOptions` opens it via `CreateFileW` with no pipe-specific
/// Win32 call needed on the client side. Not exercised by an automated gate
/// on this host (no Rust toolchain, no real pipe to open) — verified
/// manually alongside `ipc_server`'s pipe (T23).
#[cfg(windows)]
pub struct WindowsPipeConnector;

#[cfg(windows)]
impl PipeConnector for WindowsPipeConnector {
    type Stream = std::fs::File;

    fn connect(&self, pipe_name: &str) -> std::io::Result<Self::Stream> {
        std::fs::OpenOptions::new().read(true).write(true).open(pipe_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::Transport;
    use std::io::Cursor;

    struct FailingConnector;
    impl PipeConnector for FailingConnector {
        type Stream = Cursor<Vec<u8>>;
        fn connect(&self, _pipe_name: &str) -> std::io::Result<Self::Stream> {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no such pipe"))
        }
    }

    struct SucceedingConnector;
    impl PipeConnector for SucceedingConnector {
        type Stream = Cursor<Vec<u8>>;
        fn connect(&self, _pipe_name: &str) -> std::io::Result<Self::Stream> {
            Ok(Cursor::new(Vec::new()))
        }
    }

    #[test]
    fn a_missing_pipe_yields_an_explicit_core_not_running_state() {
        let outcome = connect(&FailingConnector, PIPE_NAME);
        assert!(
            matches!(outcome, ConnectOutcome::CoreNotRunning),
            "a missing/unreadable pipe must classify as CoreNotRunning, not silently succeed"
        );
    }

    #[test]
    fn a_successful_connect_surfaces_the_stream_as_connected() {
        let outcome = connect(&SucceedingConnector, PIPE_NAME);
        assert!(matches!(outcome, ConnectOutcome::Connected(_)));
    }

    #[test]
    fn read_message_parses_a_device_list_line_from_the_stream() {
        let msg = CoreToTray::DeviceList(vec![ipc::DiscoveredDevice {
            id: "00008030-ABC".into(),
            name: "Jose's iPad".into(),
            transport: Transport::Wifi,
        }]);
        let mut reader = Cursor::new(format!("{}\n", ipc::to_line(&msg)).into_bytes());

        let parsed = read_message(&mut reader)
            .expect("a message is present on the stream")
            .expect("the line parses as a valid CoreToTray message");

        assert_eq!(parsed, msg);
    }

    #[test]
    fn read_message_returns_none_at_end_of_stream() {
        let mut reader = Cursor::new(Vec::new());
        assert!(read_message(&mut reader).is_none());
    }

    #[test]
    fn read_message_surfaces_a_malformed_line_as_an_error_not_a_panic() {
        let mut reader = Cursor::new(b"not json at all\n".to_vec());
        let result = read_message(&mut reader).expect("a line is present");
        assert!(result.is_err());
    }
}
