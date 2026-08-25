//! windows-installer's `uninstall()` (T32): removes the driver package,
//! the Scheduled Task, and the tray autostart entry, then verifies no
//! orphaned virtual display remains, per spec Driver-install AC 6
//! (WSEND-18).
//!
//! Composes the removal half of each of T29–T31's installed pieces. Per
//! the Test Coverage Matrix, uninstall is OS-bound (Tests: none) — manual
//! verification via Windows "Apps & features" is the only gate. Each OS
//! call is isolated behind a trait, same pattern as the rest of this
//! feature; `uninstall()` itself is a pure composition over those traits.

use crate::driver_install::DeviceInterfaceCheck;

/// What went wrong during uninstall — kept per-component so a caller can
/// report exactly what didn't clean up, in service of spec Driver-install
/// AC 6 ("leaving no orphaned virtual display behind").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallError {
    DriverRemovalFailed { detail: String },
    ScheduledTaskRemovalFailed { detail: String },
    AutostartRemovalFailed { detail: String },
    /// Every removal step reported success, but the device interface is
    /// still present — an orphaned virtual display (this task's Done
    /// when: "no orphaned virtual display remains").
    OrphanedDeviceInterfaceRemains,
}

impl std::fmt::Display for UninstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UninstallError::DriverRemovalFailed { detail } => {
                write!(f, "failed to remove the driver package: {detail}")
            }
            UninstallError::ScheduledTaskRemovalFailed { detail } => {
                write!(f, "failed to remove the windows-core Scheduled Task: {detail}")
            }
            UninstallError::AutostartRemovalFailed { detail } => {
                write!(f, "failed to remove the windows-tray autostart entry: {detail}")
            }
            UninstallError::OrphanedDeviceInterfaceRemains => write!(
                f,
                "the driver's device interface is still present after uninstall \
                 (orphaned virtual display)"
            ),
        }
    }
}

impl std::error::Error for UninstallError {}

/// Abstraction over "remove the installed opendisplay-idd driver package".
pub trait DriverRemover {
    fn remove(&self) -> Result<(), String>;
}

/// Abstraction over "delete the named Scheduled Task".
pub trait ScheduledTaskRemover {
    fn remove(&self, task_name: &str) -> Result<(), String>;
}

/// Abstraction over "delete the named per-user autostart entry".
pub trait AutostartRemover {
    fn remove(&self, value_name: &str) -> Result<(), String>;
}

/// Removes the driver package, the Scheduled Task, and the tray autostart
/// entry (in that order), then verifies via `interface_check` that no
/// orphaned virtual display remains — this task's Done when ("Device
/// Manager shows no opendisplay-idd device, Task Scheduler shows no
/// windows-core task, and no autostart entry remains for windows-tray").
pub fn uninstall(
    driver: &dyn DriverRemover,
    scheduled_task: &dyn ScheduledTaskRemover,
    autostart: &dyn AutostartRemover,
    interface_check: &dyn DeviceInterfaceCheck,
) -> Result<(), UninstallError> {
    driver
        .remove()
        .map_err(|detail| UninstallError::DriverRemovalFailed { detail })?;

    scheduled_task
        .remove(crate::scheduled_task::TASK_NAME)
        .map_err(|detail| UninstallError::ScheduledTaskRemovalFailed { detail })?;

    autostart
        .remove(crate::autostart::AUTOSTART_VALUE_NAME)
        .map_err(|detail| UninstallError::AutostartRemovalFailed { detail })?;

    if interface_check.is_available() {
        return Err(UninstallError::OrphanedDeviceInterfaceRemains);
    }

    Ok(())
}

/// The real driver-package remover, backed by `pnputil /enum-drivers` (to
/// find the published OEM inf name for `opendisplay-idd.inf`) followed by
/// `pnputil /delete-driver ... /uninstall /force` — the same lookup
/// `Windows/driver/install-dev.ps1`'s `Uninstall-DriverPackage` performs
/// (T8), ported to Rust for the production installer. Not exercised by an
/// automated gate on this host (no Rust toolchain, no `pnputil` to call) —
/// verified manually via "Apps & features".
#[cfg(windows)]
pub struct PnpUtilDriverRemover;

#[cfg(windows)]
impl DriverRemover for PnpUtilDriverRemover {
    fn remove(&self) -> Result<(), String> {
        windows_impl::remove_driver_package()
    }
}

/// The real Scheduled Task remover, backed by `ITaskFolder::DeleteTask`.
#[cfg(windows)]
pub struct ComScheduledTaskRemover;

#[cfg(windows)]
impl ScheduledTaskRemover for ComScheduledTaskRemover {
    fn remove(&self, task_name: &str) -> Result<(), String> {
        windows_impl::delete_scheduled_task(task_name).map_err(|e| e.message().to_string())
    }
}

/// The real autostart remover, backed by `RegDeleteValueW` on the per-user
/// `Run` key `autostart::RegistryAutostartRegistrar` wrote to.
#[cfg(windows)]
pub struct RegistryAutostartRemover;

#[cfg(windows)]
impl AutostartRemover for RegistryAutostartRemover {
    fn remove(&self, value_name: &str) -> Result<(), String> {
        windows_impl::delete_run_value(value_name).map_err(|e| e.message().to_string())
    }
}

#[cfg(windows)]
mod windows_impl {
    use windows::core::{BSTR, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, HKEY_CURRENT_USER, KEY_SET_VALUE,
    };
    use windows::Win32::System::TaskScheduler::{ITaskService, TaskScheduler as TaskSchedulerComClass};

    /// Runs `pnputil /enum-drivers`, finds the published OEM inf name for
    /// `opendisplay-idd.inf`, and deletes it — mirroring
    /// `install-dev.ps1`'s `Uninstall-DriverPackage` parsing logic.
    pub(super) fn remove_driver_package() -> Result<(), String> {
        let enum_output = std::process::Command::new("pnputil.exe")
            .arg("/enum-drivers")
            .output()
            .map_err(|e| format!("failed to run pnputil /enum-drivers: {e}"))?;
        let listing = String::from_utf8_lossy(&enum_output.stdout);

        let oem_inf_name = find_published_name_for(&listing, "opendisplay-idd.inf");
        let Some(oem_inf_name) = oem_inf_name else {
            // Not installed; nothing to do — uninstall is idempotent
            // (design.md's `install()`/`uninstall()` "both idempotent").
            return Ok(());
        };

        let status = std::process::Command::new("pnputil.exe")
            .arg("/delete-driver")
            .arg(&oem_inf_name)
            .arg("/uninstall")
            .arg("/force")
            .status()
            .map_err(|e| format!("failed to run pnputil /delete-driver: {e}"))?;
        if !status.success() {
            return Err(format!(
                "pnputil /delete-driver exited with {:?}",
                status.code()
            ));
        }
        Ok(())
    }

    /// Parses `pnputil /enum-drivers`' text output for the block whose
    /// "Original Name" matches `original_inf_name`, returning its
    /// "Published Name" (e.g. `oem12.inf`) — the identifier
    /// `/delete-driver` requires.
    fn find_published_name_for(listing: &str, original_inf_name: &str) -> Option<String> {
        let mut current_published: Option<&str> = None;
        for line in listing.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("Published Name:") {
                current_published = Some(value.trim());
            } else if let Some(value) = line.strip_prefix("Original Name:") {
                if value.trim().eq_ignore_ascii_case(original_inf_name) {
                    return current_published.map(str::to_string);
                }
            }
        }
        None
    }

    pub(super) fn delete_scheduled_task(task_name: &str) -> windows::core::Result<()> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let service: ITaskService =
                CoCreateInstance(&TaskSchedulerComClass, None, CLSCTX_INPROC_SERVER)?;
            service.Connect(
                &windows::core::VARIANT::default(),
                &windows::core::VARIANT::default(),
                &windows::core::VARIANT::default(),
                &windows::core::VARIANT::default(),
            )?;
            let root_folder = service.GetFolder(&BSTR::from("\\"))?;
            // DeleteTask on a task that doesn't exist returns an error in
            // some Task Scheduler versions; uninstall treats "already
            // gone" the same as "removed" (idempotent, per design.md).
            match root_folder.DeleteTask(&BSTR::from(task_name), 0) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.to_hresult() => {
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
    }

    pub(super) fn delete_run_value(value_name: &str) -> windows::core::Result<()> {
        const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(std::iter::once(0)).collect()
        }
        unsafe {
            let subkey = wide(RUN_KEY_PATH);
            let mut hkey = Default::default();
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                0,
                KEY_SET_VALUE,
                &mut hkey,
            )
            .ok()?;
            let name = wide(value_name);
            let result = RegDeleteValueW(hkey, PCWSTR(name.as_ptr())).ok();
            let _ = RegCloseKey(hkey);
            // Already absent is not a failure — uninstall is idempotent.
            match result {
                Ok(()) => Ok(()),
                Err(e) if e.code() == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.to_hresult() => {
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
    }
}
