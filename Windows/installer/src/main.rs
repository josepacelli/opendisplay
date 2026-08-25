//! windows-installer entry point: one-shot elevated first-run setup
//! (driver install, Scheduled Task registration, tray autostart) and the
//! uninstall path, per design.md's `windows-installer` component (FIX6,
//! Verifier gap 1 — Blocker, installer half).
//!
//! Dispatches on a single argv flag: `install` (the default, used with no
//! argument at all — the first-run flow from `windows-tray`'s
//! `ui::first_run::WindowsInstallerLauncher` launches this binary with no
//! arguments) or `uninstall` (invoked from Windows' "Apps & features").
//! Every binary this installer registers/removes is assumed to sit
//! alongside `windows-installer.exe` in the same install directory —
//! `Cargo.toml`'s package names (`core`, `tray`) are each binary's actual
//! `.exe` name.

mod autostart;
mod driver_install;
mod scheduled_task;
mod uninstall;

#[cfg(windows)]
fn main() {
    std::process::exit(windows_impl::run(std::env::args().nth(1).as_deref()));
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod windows_impl {
    use crate::driver_install::SetupApiInterfaceCheck;
    use std::path::{Path, PathBuf};

    /// The directory this executable lives in — every sibling file
    /// (driver package, `core.exe`, `tray.exe`) is resolved relative to
    /// it, per this module's doc comment.
    fn install_dir() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Runs the `install`/`uninstall` flow named by `arg` (`None` or any
    /// other value defaults to `install`), returning the process exit
    /// code.
    pub fn run(arg: Option<&str>) -> i32 {
        match arg {
            Some("uninstall") => run_uninstall(),
            _ => run_install(),
        }
    }

    fn run_install() -> i32 {
        let dir = install_dir();

        if let Err(err) = crate::driver_install::install_driver(
            &dir.join("opendisplay-idd.inf"),
            &crate::driver_install::PnpUtilInstaller,
            &SetupApiInterfaceCheck,
        ) {
            eprintln!("windows-installer: driver install failed: {err}");
            return 1;
        }

        if let Err(err) = crate::scheduled_task::register(
            &crate::scheduled_task::ComTaskScheduler,
            &dir.join("core.exe"),
        ) {
            eprintln!("windows-installer: Scheduled Task registration failed: {err}");
            return 1;
        }

        if let Err(err) = crate::autostart::register(
            &crate::autostart::RegistryAutostartRegistrar,
            &dir.join("tray.exe"),
        ) {
            eprintln!("windows-installer: tray autostart registration failed: {err}");
            return 1;
        }

        0
    }

    fn run_uninstall() -> i32 {
        match crate::uninstall::uninstall(
            &crate::uninstall::PnpUtilDriverRemover,
            &crate::uninstall::ComScheduledTaskRemover,
            &crate::uninstall::RegistryAutostartRemover,
            &SetupApiInterfaceCheck,
        ) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("windows-installer: uninstall failed: {err}");
                1
            }
        }
    }
}
