//! windows-installer's driver-install half of `install()` (T29): installs
//! the (test-signed in dev / attestation-signed in release) opendisplay-idd
//! package via `pnputil`, then verifies the device interface becomes
//! available before returning success, per spec Driver-install AC 1-2
//! (single elevation, verify load) and AC 4 (specific, actionable
//! failure).
//!
//! Reuses `Windows/driver/install-dev.ps1`'s package shape (T8): the same
//! INF path is handed to `pnputil /add-driver ... /install` a release
//! build's installer would use, minus that script's dev-only test-signing
//! step (spec Driver-install AC 5: no test-signing mode required).
//!
//! Per the Test Coverage Matrix, driver install/PnP is OS-bound (Tests:
//! none) — manual verification on a clean Windows 11 VM with Secure Boot
//! on is the only gate. `pnputil`/SetupAPI calls are isolated behind
//! traits, same pattern as `core::display`'s device-interface open (T16).

use std::path::Path;

/// A specific, actionable install failure, per spec Driver-install AC 4
/// ("SHALL surface a specific, actionable error (not a generic failure)").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverInstallError {
    /// The INF/catalog package at the given path could not be found.
    PackageNotFound { path: String },
    /// `pnputil /add-driver` rejected the package (exit code carried for
    /// diagnostics) — e.g. corrupt package or an invalid/missing
    /// signature.
    PnpUtilRejected { exit_code: i32 },
    /// `pnputil` reported success, but the device interface never became
    /// available — the package installed but the driver failed to load.
    DeviceInterfaceNeverAppeared,
}

impl std::fmt::Display for DriverInstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriverInstallError::PackageNotFound { path } => {
                write!(f, "driver package not found at '{path}' — reinstall the app")
            }
            DriverInstallError::PnpUtilRejected { exit_code } => write!(
                f,
                "Windows rejected the driver package (pnputil exit code {exit_code}) — \
                 it may be corrupt or improperly signed"
            ),
            DriverInstallError::DeviceInterfaceNeverAppeared => write!(
                f,
                "the driver package installed but its device interface never became \
                 available — it may have failed to load"
            ),
        }
    }
}

impl std::error::Error for DriverInstallError {}

/// Abstraction over "run `pnputil /add-driver <inf> /install`", isolating
/// the process spawn so the install/verify sequencing below could be
/// unit-tested independent of a real `pnputil` call. No test exists for
/// this task (Tests: none, OS-bound per the Test Coverage Matrix), but the
/// isolation matches the pattern used throughout this feature.
pub trait DriverPackageInstaller {
    /// Returns `pnputil`'s process exit code (0 = success).
    fn install(&self, inf_path: &Path) -> std::io::Result<i32>;
}

/// Abstraction over "is opendisplay-idd's device interface currently
/// available", isolating the SetupAPI enumeration call.
pub trait DeviceInterfaceCheck {
    fn is_available(&self) -> bool;
}

/// Installs the driver package at `inf_path` via `installer`, then
/// verifies it loaded via `interface_check` before returning success —
/// this task's Done when ("a successful install is followed by a
/// verified-loaded device interface before returning").
pub fn install_driver(
    inf_path: &Path,
    installer: &dyn DriverPackageInstaller,
    interface_check: &dyn DeviceInterfaceCheck,
) -> Result<(), DriverInstallError> {
    if !inf_path.exists() {
        return Err(DriverInstallError::PackageNotFound {
            path: inf_path.display().to_string(),
        });
    }

    let exit_code = installer
        .install(inf_path)
        .map_err(|_| DriverInstallError::PnpUtilRejected { exit_code: -1 })?;
    if exit_code != 0 {
        return Err(DriverInstallError::PnpUtilRejected { exit_code });
    }

    if !interface_check.is_available() {
        return Err(DriverInstallError::DeviceInterfaceNeverAppeared);
    }

    Ok(())
}

/// The real installer, backed by `pnputil.exe /add-driver ... /install`.
/// Not exercised by an automated gate on this host (no Rust toolchain, no
/// `pnputil` to call) — verified manually on a clean Windows 11 VM.
#[cfg(windows)]
pub struct PnpUtilInstaller;

#[cfg(windows)]
impl DriverPackageInstaller for PnpUtilInstaller {
    fn install(&self, inf_path: &Path) -> std::io::Result<i32> {
        let status = std::process::Command::new("pnputil.exe")
            .arg("/add-driver")
            .arg(inf_path)
            .arg("/install")
            .status()?;
        Ok(status.code().unwrap_or(-1))
    }
}

/// The real device-interface check, backed by `SetupDiGetClassDevsW`
/// against `GUID_DEVINTERFACE_OPENDISPLAY_IDD`.
#[cfg(windows)]
pub struct SetupApiInterfaceCheck;

#[cfg(windows)]
impl DeviceInterfaceCheck for SetupApiInterfaceCheck {
    fn is_available(&self) -> bool {
        windows_impl::device_interface_present().unwrap_or(false)
    }
}

#[cfg(windows)]
mod windows_impl {
    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
        DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
    };

    /// `GUID_DEVINTERFACE_OPENDISPLAY_IDD` from `Windows/driver/Ioctl.h` —
    /// mirrors `core::display`'s constant of the same name exactly (shared
    /// contract, hand-kept in sync per `[[memory:AD-002]]`'s pattern, since
    /// this crate has no dependency on `core`).
    const DEVINTERFACE_OPENDISPLAY_IDD: GUID = GUID::from_values(
        0x6e1b6f9b,
        0x6b0b,
        0x4e2a,
        [0x9c, 0x36, 0x9b, 0x7e, 0x6e, 0x6e, 0x7a, 0x11],
    );

    pub(super) fn device_interface_present() -> windows::core::Result<bool> {
        unsafe {
            let device_info = SetupDiGetClassDevsW(
                Some(&DEVINTERFACE_OPENDISPLAY_IDD),
                PCWSTR::null(),
                None,
                DIGCF_DEVICEINTERFACE | DIGCF_PRESENT,
            )?;
            let mut interface_data = SP_DEVICE_INTERFACE_DATA {
                cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            let present = SetupDiEnumDeviceInterfaces(
                device_info,
                None,
                &DEVINTERFACE_OPENDISPLAY_IDD,
                0,
                &mut interface_data,
            )
            .is_ok();
            let _ = SetupDiDestroyDeviceInfoList(device_info);
            Ok(present)
        }
    }
}
