//! Per-user autostart registration for windows-tray (T31): registers
//! windows-tray to start unprivileged at user logon, per design.md's
//! "Approach confirmed with the user" (option C: unprivileged tray +
//! elevated core) — the tray needs only an ordinary per-user autostart
//! entry, not a Scheduled Task, since it never needs elevation.
//!
//! Per the Test Coverage Matrix, autostart registration is OS-bound
//! (Tests: none) — manual verification (log off/on, confirm Medium
//! integrity) is the only gate. The registry write is isolated behind a
//! trait, same pattern as `driver_install`'s `DriverPackageInstaller` and
//! `scheduled_task`'s `TaskScheduler`.

use std::path::Path;

/// The value name windows-installer writes under the per-user `Run` key.
pub const AUTOSTART_VALUE_NAME: &str = "OpenDisplay Tray";

/// A specific registration failure, so a caller can report something more
/// useful than "registration failed".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutostartError {
    /// The per-user `Run` registry key could not be opened or created.
    RegistryKeyUnavailable,
    /// The value write itself failed.
    WriteFailed { detail: String },
}

impl std::fmt::Display for AutostartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutostartError::RegistryKeyUnavailable => {
                write!(f, "could not open the per-user autostart registry key")
            }
            AutostartError::WriteFailed { detail } => {
                write!(f, "failed to write the autostart entry: {detail}")
            }
        }
    }
}

impl std::error::Error for AutostartError {}

/// Abstraction over "register `executable_path` to autostart, unprivileged,
/// at user logon", isolating the registry write so registration could be
/// exercised without a real registry call. No test exists for this task
/// (Tests: none, OS-bound per the Test Coverage Matrix), but the isolation
/// matches the pattern used throughout this feature.
pub trait AutostartRegistrar {
    fn register(&self, value_name: &str, executable_path: &Path) -> Result<(), AutostartError>;
}

/// Registers `executable_path` (windows-tray's binary) to autostart, per
/// this task's Done when ("after registration, windows-tray starts at the
/// next logon without elevation").
pub fn register(
    registrar: &dyn AutostartRegistrar,
    executable_path: &Path,
) -> Result<(), AutostartError> {
    registrar.register(AUTOSTART_VALUE_NAME, executable_path)
}

/// The real registration, backed by a per-user `HKEY_CURRENT_USER` registry
/// value under `Software\Microsoft\Windows\CurrentVersion\Run` — the
/// standard unprivileged per-user autostart mechanism (deliberately not a
/// Scheduled Task: the tray runs at ordinary Medium integrity and never
/// needs `RunLevel=HighestAvailable`, unlike `scheduled_task`'s
/// registration for windows-core). Not exercised by an automated gate on
/// this host (no Rust toolchain, no registry to write) — verified manually
/// by logging off/on and confirming the tray icon appears at Medium
/// integrity.
#[cfg(windows)]
pub struct RegistryAutostartRegistrar;

#[cfg(windows)]
impl AutostartRegistrar for RegistryAutostartRegistrar {
    fn register(&self, value_name: &str, executable_path: &Path) -> Result<(), AutostartError> {
        windows_impl::write_run_value(value_name, executable_path)
            .map_err(|e| AutostartError::WriteFailed { detail: e.message() })
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::path::Path;
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Writes `value_name` = `executable_path` under the per-user `Run`
    /// key, creating the key if it doesn't already exist (it always does
    /// on a normal Windows install, but `RegCreateKeyExW` opens-or-creates
    /// in one call either way).
    pub(super) fn write_run_value(
        value_name: &str,
        executable_path: &Path,
    ) -> windows::core::Result<()> {
        unsafe {
            let subkey = wide(RUN_KEY_PATH);
            let mut hkey = Default::default();
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                0,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut hkey,
                None,
            )
            .ok()?;

            let name = wide(value_name);
            let data = wide(&executable_path.to_string_lossy());
            let data_bytes = std::slice::from_raw_parts(
                data.as_ptr() as *const u8,
                data.len() * std::mem::size_of::<u16>(),
            );
            let result = RegSetValueExW(hkey, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(data_bytes));
            let _ = RegCloseKey(hkey);
            result.ok()
        }
    }
}
