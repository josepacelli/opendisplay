//! Tray icon + device picker UI (T25).
//!
//! Renders `windows-core`'s discovered-device list (`CoreToTray::DeviceList`,
//! delivered over `ipc_client`, T24) as a tray popup menu and turns a
//! device selection into a `TrayToCore::Connect` message for `windows-core`
//! to act on, per spec WSEND-01/WSEND-08's Independent Tests ("select a
//! device from a list").
//!
//! Per the Test Coverage Matrix, tray icon/UI rendering is an OS-bound
//! module (Tests: none) — manual verification against a live tray session
//! is the only gate. The list-update/selection bookkeeping below is kept
//! as plain state so it stays legible even though it isn't unit-tested for
//! this task; the actual menu rendering is native Win32
//! (`Shell_NotifyIcon` + a popup menu), isolated in `windows_impl` the same
//! way `core::cursor`/`core::input` isolate their OS calls.

use ipc::{CoreToTray, DiscoveredDevice, TrayToCore};

/// The device picker's state: the most recently received device list.
/// Owned by the tray's event loop and updated on every `DeviceList`
/// message (this task's "the list updates live as `DeviceList` messages
/// arrive").
#[derive(Debug, Default)]
pub struct DevicePicker {
    devices: Vec<DiscoveredDevice>,
}

impl DevicePicker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies an incoming `CoreToTray` message to the picker's state.
    /// Only `DeviceList` affects the picker; `Status` and `Error` are
    /// handled by `ui::status` and `ui::first_run` respectively.
    pub fn apply(&mut self, msg: &CoreToTray) {
        if let CoreToTray::DeviceList(devices) = msg {
            self.devices = devices.clone();
        }
    }

    /// The devices currently shown in the picker menu, in receipt order.
    pub fn devices(&self) -> &[DiscoveredDevice] {
        &self.devices
    }

    /// Turns a user's menu click on `device_id` into the `TrayToCore`
    /// message `windows-core` acts on (this task's "selecting a device
    /// sends `TrayToCore::Connect`"). Returns `None` if `device_id` is no
    /// longer in the current list — e.g. it disconnected between the menu
    /// opening and the click landing — so a stale click never sends a
    /// `Connect` for a device that isn't there anymore.
    pub fn select(&self, device_id: &str) -> Option<TrayToCore> {
        self.devices
            .iter()
            .find(|d| d.id == device_id)
            .map(|d| TrayToCore::Connect {
                device_id: d.id.clone(),
            })
    }
}

/// Native tray icon + popup menu rendering, backed by Win32
/// `Shell_NotifyIconW` and `TrackPopupMenuEx`. OS UI surface with no
/// automated-test ROI per the Test Coverage Matrix ("tray icon/UI
/// rendering" -> none); not exercised by an automated gate on this host
/// (no Rust/Win32 toolchain) — verified manually against a live tray
/// session per WSEND-01/WSEND-08.
#[cfg(windows)]
pub mod windows_impl {
    use super::DevicePicker;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NOTIFYICONDATAW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, SetForegroundWindow,
        TrackPopupMenuEx, HICON, IDI_APPLICATION, MF_STRING, TPM_LEFTALIGN, TPM_RETURNCMD,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Registers the tray icon for `hwnd`, using `callback_message` as the
    /// window message `Shell_NotifyIconW` posts back on icon
    /// clicks/right-clicks (the standard Win32 tray-icon pattern).
    pub fn add_tray_icon(hwnd: HWND, callback_message: u32) -> windows::core::Result<()> {
        unsafe {
            let icon: HICON = LoadIconW(None, IDI_APPLICATION)?;
            let mut data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                uCallbackMessage: callback_message,
                hIcon: icon,
                ..Default::default()
            };
            let tip = wide("OpenDisplay");
            data.szTip[..tip.len()].copy_from_slice(&tip);
            Shell_NotifyIconW(NIM_ADD, &data).ok()?;
        }
        Ok(())
    }

    pub fn remove_tray_icon(hwnd: HWND) -> windows::core::Result<()> {
        unsafe {
            let data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                ..Default::default()
            };
            Shell_NotifyIconW(NIM_DELETE, &data).ok()?;
        }
        Ok(())
    }

    /// Builds and tracks a popup menu listing `picker`'s current devices,
    /// returning the selected device's id (if any) so the caller can turn
    /// it into a `TrayToCore::Connect` via `DevicePicker::select`.
    pub fn show_menu(hwnd: HWND, picker: &DevicePicker) -> windows::core::Result<Option<String>> {
        unsafe {
            let menu = CreatePopupMenu()?;
            let devices = picker.devices();
            for (index, device) in devices.iter().enumerate() {
                let label = wide(&device.name);
                AppendMenuW(menu, MF_STRING, index + 1, PCWSTR(label.as_ptr()))?;
            }

            let mut cursor = POINT::default();
            GetCursorPos(&mut cursor)?;
            let _ = SetForegroundWindow(hwnd);
            let selected_index = TrackPopupMenuEx(
                menu,
                (TPM_LEFTALIGN | TPM_RETURNCMD).0,
                cursor.x,
                cursor.y,
                hwnd,
                None,
            );
            let _ = DestroyMenu(menu);

            let index = selected_index.0 as usize;
            Ok(if index >= 1 && index <= devices.len() {
                Some(devices[index - 1].id.clone())
            } else {
                None
            })
        }
    }

    #[allow(dead_code)]
    fn _unused(_: LPARAM, _: WPARAM) -> LRESULT {
        LRESULT(0)
    }
}
