//! First-run "driver not installed" flow (T28): detects a "driver
//! missing" `Error` from windows-core and offers a single action that
//! launches `windows-installer`, per spec WSEND-13/WSEND-15.
//!
//! Per the Test Coverage Matrix, tray UI rendering is OS-bound (Tests:
//! none) — manual verification on a clean VM is the only gate. The
//! offer/decline state below is kept plain so it stays legible even
//! though it isn't unit-tested for this task.

use ipc::CoreToTray;

/// The message text `windows-core` sends as a `CoreToTray::Error` to
/// report the virtual-display driver missing or failing to load.
///
/// SPEC_DEVIATION: same reasoning as `ui::status`'s `NOT_ELEVATED_MESSAGE`
/// — the already-committed `ipc` schema (T3) has no dedicated "driver
/// missing" variant, so it is recognized by this message-content
/// convention on `Error` rather than changing that schema, which is
/// outside this task's file scope (`Windows/tray/src/ui/first_run.rs`
/// only).
pub const DRIVER_MISSING_MESSAGE: &str = "driver not installed";

/// The state this task's first-run flow renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstRunState {
    /// The driver is missing: offer the single "set up virtual display"
    /// action.
    OfferInstall,
    /// The user declined the offer this session; the app stays usable
    /// (spec Driver-install AC 3: "remain open... explain that a virtual
    /// display cannot be created yet"), but setup remains incomplete.
    SetupIncompleteDeclined,
}

/// Tracks whether the install offer is currently showing, driven purely by
/// the `CoreToTray` stream. Holds no across-launch persistence, so a fresh
/// process start always begins with no state and re-evaluates the next
/// message it receives — satisfying spec Driver-install AC 3's "re-offer
/// the install on next launch" without any extra bookkeeping.
#[derive(Debug, Default)]
pub struct FirstRunFlow {
    state: Option<FirstRunState>,
}

impl FirstRunFlow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies an incoming `CoreToTray` message. This task's Done when:
    /// "the prompt appears exactly when core reports the driver missing,
    /// and not otherwise" — only the recognized [`DRIVER_MISSING_MESSAGE`]
    /// `Error` changes state; every other message (including an unrelated
    /// `Error`, a `Status`, or a `DeviceList`) leaves it untouched, so a
    /// same-session decline is never silently cleared by an unrelated
    /// update.
    pub fn apply(&mut self, msg: &CoreToTray) {
        if let CoreToTray::Error { message } = msg {
            if message == DRIVER_MISSING_MESSAGE {
                self.state = Some(FirstRunState::OfferInstall);
            }
        }
    }

    /// The current state to render, or `None` when there is nothing to
    /// offer (the driver is present, or no relevant status has arrived
    /// yet).
    pub fn state(&self) -> Option<FirstRunState> {
        self.state
    }

    /// Records that the user declined the offer: the app stays open in
    /// `SetupIncompleteDeclined` for the rest of this session (this task's
    /// Done when: "declining leaves the app usable in 'setup incomplete'
    /// state"). A later process launch starts a brand new `FirstRunFlow`
    /// with no memory of this decline, which is what re-offers it (spec AC
    /// 3) — there is deliberately no persisted "permanently dismissed"
    /// flag.
    pub fn decline(&mut self) {
        if self.state == Some(FirstRunState::OfferInstall) {
            self.state = Some(FirstRunState::SetupIncompleteDeclined);
        }
    }
}

/// Abstraction over "launch windows-installer", isolating the OS process
/// spawn so a caller could unit-test acceptance flow without launching a
/// real process — no such test exists for this task (Tests: none, tray
/// UI), but the isolation matches the pattern used throughout this
/// feature (e.g. T24's `PipeConnector`).
pub trait InstallerLauncher {
    fn launch(&self) -> std::io::Result<()>;
}

/// Accepts the offer: launches `windows-installer` via `launcher`.
pub fn accept(launcher: &dyn InstallerLauncher) -> std::io::Result<()> {
    launcher.launch()
}

/// The real launcher, spawning the installer executable installed
/// alongside the tray binary. Not exercised by an automated gate on this
/// host (no Rust toolchain, no installer binary to spawn) — verified
/// manually on a clean VM per the Driver-install story's Independent Test.
#[cfg(windows)]
pub struct WindowsInstallerLauncher;

#[cfg(windows)]
impl InstallerLauncher for WindowsInstallerLauncher {
    fn launch(&self) -> std::io::Result<()> {
        std::process::Command::new("windows-installer.exe").spawn()?;
        Ok(())
    }
}
