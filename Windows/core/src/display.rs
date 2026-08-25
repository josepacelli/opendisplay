//! Opens `opendisplay-idd`'s device interface and issues
//! `IOCTL_OPENDISPLAY_{CREATE,RESIZE,DESTROY}_DISPLAY`, per the shared
//! contract in `Windows/driver/Ioctl.h`.
//!
//! This module is a thin wrapper over Windows device-interface/IOCTL calls
//! (`SetupDiGetClassDevsW`/`SetupDiEnumDeviceInterfaces` to resolve the
//! device path, `CreateFileW` to open it, `DeviceIoControl` to issue each
//! IOCTL) — per the Test Coverage Matrix, this class of code is verified
//! manually against real Windows 11 hardware, not by an automated gate. It
//! is not exercised by any test in this repository; every call below is
//! unverified on this macOS host (no Rust/WDK toolchain, no real driver to
//! open).
//!
//! `IOCTL_OPENDISPLAY_*_DISPLAY`'s codes and `OPENDISPLAY_DISPLAY_PARAMS`
//! layout below MUST match `Windows/driver/Ioctl.h` exactly — this is the
//! Rust side of that shared contract, hand-ported the same way
//! `Windows/protocol` hand-ports `Shared/Protocol.swift` (`[[memory:AD-002]]`'s
//! pattern applied to a second cross-language boundary).

use crate::display_spec::DisplaySpec;

#[cfg(windows)]
use windows::core::GUID;

/// `GUID_DEVINTERFACE_OPENDISPLAY_IDD` from `Windows/driver/Ioctl.h`.
#[cfg(windows)]
const DEVINTERFACE_OPENDISPLAY_IDD: GUID = GUID::from_values(
    0x6e1b6f9b,
    0x6b0b,
    0x4e2a,
    [0x9c, 0x36, 0x9b, 0x7e, 0x6e, 0x6e, 0x7a, 0x11],
);

// `CTL_CODE` inputs, mirroring `Windows/driver/Ioctl.h` exactly.
const FILE_DEVICE_UNKNOWN: u32 = 0x0000_0022;
const METHOD_BUFFERED: u32 = 0;
const FILE_WRITE_DATA: u32 = 0x0002;

/// Mirrors the `CTL_CODE` Win32 macro so the three IOCTL constants below are
/// computed the same way the driver's header computes them, instead of
/// being hand-copied as magic numbers that could silently drift.
const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 2) | method
}

const IOCTL_OPENDISPLAY_CREATE_DISPLAY: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x800, METHOD_BUFFERED, FILE_WRITE_DATA);
const IOCTL_OPENDISPLAY_RESIZE_DISPLAY: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x801, METHOD_BUFFERED, FILE_WRITE_DATA);
const IOCTL_OPENDISPLAY_DESTROY_DISPLAY: u32 =
    ctl_code(FILE_DEVICE_UNKNOWN, 0x802, METHOD_BUFFERED, FILE_WRITE_DATA);

/// `PROTOCOL.md`/`DisplaySpec` carry no refresh rate (neither `hello` nor
/// `design.md`'s `DisplaySpec` model one), but the driver's
/// `OPENDISPLAY_DISPLAY_PARAMS` requires one. 60Hz is the documented
/// default used to fill that gap — not a spec-defined value.
const DEFAULT_REFRESH_HZ: u32 = 60;

/// Mirrors `Windows/driver/Ioctl.h`'s `OPENDISPLAY_DISPLAY_PARAMS` layout
/// exactly (`repr(C)`, same field order/types) — this is the buffer
/// `DeviceIoControl` packs for `CREATE`/`RESIZE`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct OpenDisplayDisplayParams {
    width_px: u32,
    height_px: u32,
    refresh_hz: u32,
    scale_factor: f32,
}

impl From<&DisplaySpec> for OpenDisplayDisplayParams {
    fn from(spec: &DisplaySpec) -> Self {
        OpenDisplayDisplayParams {
            width_px: spec.width_px,
            height_px: spec.height_px,
            refresh_hz: DEFAULT_REFRESH_HZ,
            scale_factor: spec.scale_factor,
        }
    }
}

/// A handle to the (single, per the P1 constraint) virtual monitor created
/// through `opendisplay-idd`'s device interface. Identifies the
/// adapter/output `capture::start` (T17) targets.
#[cfg(windows)]
pub struct VirtualDisplayHandle {
    device: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
        SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
        SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    };
    use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE, OPEN_EXISTING,
    };
    use windows::Win32::System::IO::DeviceIoControl;

    /// Resolves `opendisplay-idd`'s device path via
    /// `SetupDiGetClassDevsW`/`SetupDiEnumDeviceInterfaces` and opens it,
    /// per the module's shared IOCTL contract.
    fn open_device_interface() -> windows::core::Result<HANDLE> {
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
            let enumerated = SetupDiEnumDeviceInterfaces(
                device_info,
                None,
                &DEVINTERFACE_OPENDISPLAY_IDD,
                0,
                &mut interface_data,
            );
            if enumerated.is_err() {
                let _ = SetupDiDestroyDeviceInfoList(device_info);
                return Err(windows::core::Error::from_win32());
            }

            // First call learns the required buffer size; a too-small
            // buffer is the documented way to size the second call.
            let mut required_size = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(
                device_info,
                &interface_data,
                None,
                0,
                Some(&mut required_size),
                None,
            );

            let mut buf = vec![0u8; required_size as usize];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            SetupDiGetDeviceInterfaceDetailW(
                device_info,
                &interface_data,
                Some(detail),
                required_size,
                None,
                None,
            )?;
            let device_path = PCWSTR((*detail).DevicePath.as_ptr());

            let handle = CreateFileW(
                device_path,
                (windows::Win32::Storage::FileSystem::FILE_GENERIC_READ
                    | windows::Win32::Storage::FileSystem::FILE_GENERIC_WRITE)
                    .0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            );

            let _ = SetupDiDestroyDeviceInfoList(device_info);

            match handle {
                Ok(h) if h != INVALID_HANDLE_VALUE => Ok(h),
                _ => Err(windows::core::Error::from_win32()),
            }
        }
    }

    fn issue_ioctl(
        device: HANDLE,
        code: u32,
        params: Option<&OpenDisplayDisplayParams>,
    ) -> windows::core::Result<()> {
        unsafe {
            let (in_ptr, in_len) = match params {
                Some(p) => (
                    p as *const OpenDisplayDisplayParams as *const std::ffi::c_void,
                    std::mem::size_of::<OpenDisplayDisplayParams>() as u32,
                ),
                None => (std::ptr::null(), 0),
            };
            let mut bytes_returned = 0u32;
            let ok = DeviceIoControl(
                device,
                code,
                Some(in_ptr),
                in_len,
                None,
                0,
                Some(&mut bytes_returned),
                None,
            );
            if ok.is_ok() {
                Ok(())
            } else {
                Err(windows::core::Error::from(GetLastError().to_hresult()))
            }
        }
    }

    impl super::VirtualDisplayHandle {
        pub fn create(spec: &DisplaySpec) -> windows::core::Result<Self> {
            let device = open_device_interface()?;
            let params = OpenDisplayDisplayParams::from(spec);
            issue_ioctl(device, super::IOCTL_OPENDISPLAY_CREATE_DISPLAY, Some(&params))?;
            Ok(Self { device })
        }

        pub fn resize(&self, spec: &DisplaySpec) -> windows::core::Result<()> {
            let params = OpenDisplayDisplayParams::from(spec);
            issue_ioctl(
                self.device,
                super::IOCTL_OPENDISPLAY_RESIZE_DISPLAY,
                Some(&params),
            )
        }

        pub fn destroy(self) -> windows::core::Result<()> {
            issue_ioctl(self.device, super::IOCTL_OPENDISPLAY_DESTROY_DISPLAY, None)
        }
    }

    impl Drop for super::VirtualDisplayHandle {
        fn drop(&mut self) {
            // Best-effort: IOCTL_OPENDISPLAY_DESTROY_DISPLAY is idempotent
            // on a repeated call (T7), so this is a safe fallback for a
            // caller that dropped the handle without calling `destroy()`
            // explicitly. Errors are unobservable from a destructor.
            let _ = issue_ioctl(self.device, super::IOCTL_OPENDISPLAY_DESTROY_DISPLAY, None);
            unsafe {
                let _ = CloseHandle(self.device);
            }
        }
    }
}
