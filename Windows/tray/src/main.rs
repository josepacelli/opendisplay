//! windows-tray entry point.
//!
//! Wires `ipc_client` (T24), the device picker/status/first-run UI
//! modules (T25-T28), and `actions::open_logs` (T27) into an actual
//! running tray icon: a hidden message-only window owns the icon, a
//! background thread owns the blocking IPC connect/read loop and posts
//! each `CoreToTray` message to the window via `PostMessageW` (FIX6,
//! Verifier gap 1 — Blocker, tray half).

mod actions;
mod ipc_client;
mod ui;

#[cfg(windows)]
fn main() {
    windows_impl::run();
}

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
mod windows_impl {
    use crate::actions::open_logs;
    use crate::ipc_client::{self, ConnectOutcome, WindowsPipeConnector, PIPE_NAME};
    use crate::ui::first_run::{self, FirstRunFlow, FirstRunState};
    use crate::ui::picker::{self, DevicePicker};
    use crate::ui::status::{self, TrayStatus};
    use ipc::{CoreToTray, DiscoveredDevice, TrayToCore};
    use std::io::{BufReader, Write};
    use std::sync::{Arc, Mutex};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    /// Posted by `Shell_NotifyIconW` on a click against the tray icon
    /// (registered as the icon's `uCallbackMessage`, per `ui::picker`'s
    /// `add_tray_icon`).
    const WM_TRAYICON: u32 = WM_APP + 1;
    /// Posted by the IPC reader thread with a boxed `CoreToTray` in
    /// `wparam`, so the window (the only thread allowed to touch `APP`
    /// and re-render the icon) applies it.
    const WM_CORE_MESSAGE: u32 = WM_APP + 2;

    const ID_OPEN_LOGS: usize = 9001;
    const ID_QUIT: usize = 9002;
    const ID_INSTALL: usize = 9003;
    const ID_NOT_NOW: usize = 9004;

    /// `HWND` is a bare pointer, so `windows-rs` leaves it `!Send`. A
    /// window handle is safe to hand to another thread in practice — Win32
    /// messaging functions (`PostMessageW` in particular) are documented
    /// as callable from any thread — so this wrapper asserts that intent
    /// explicitly at the one or two points this module actually crosses a
    /// thread boundary with it (the static `APP` and the IPC reader
    /// thread), rather than at every call site.
    #[derive(Clone, Copy)]
    struct SendHwnd(HWND);
    unsafe impl Send for SendHwnd {}
    unsafe impl Sync for SendHwnd {}
    impl SendHwnd {
        /// A method call (not a `.0` field projection) so Rust 2021's
        /// disjoint closure captures can't narrow a `move` closure's
        /// capture down to the bare `HWND` field — which would silently
        /// defeat this wrapper's whole purpose.
        fn get(self) -> HWND {
            self.0
        }
    }

    struct AppState {
        hwnd: SendHwnd,
        picker: DevicePicker,
        status: Option<TrayStatus>,
        first_run: FirstRunFlow,
        /// The current IPC connection's write half, if `windows-core` is
        /// reachable right now — `None` between connections.
        writer: Option<Arc<Mutex<std::fs::File>>>,
    }

    // WndProc always runs on the thread that created `hwnd` (this
    // process's only UI thread); the IPC reader thread only ever reaches
    // `AppState` through `set_writer`, which takes the same lock.
    static APP: Mutex<Option<AppState>> = Mutex::new(None);

    pub fn run() {
        let hwnd = unsafe { create_message_window() };
        *APP.lock().unwrap() = Some(AppState {
            hwnd: SendHwnd(hwnd),
            picker: DevicePicker::new(),
            status: None,
            first_run: FirstRunFlow::new(),
            writer: None,
        });

        let _ = picker::windows_impl::add_tray_icon(hwnd, WM_TRAYICON);
        spawn_ipc_reader(hwnd);

        let mut msg = MSG::default();
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn create_message_window() -> HWND {
        let class_name = wide("OpenDisplayTrayWindow");
        let hmodule = GetModuleHandleW(None).unwrap_or_default();
        let hinstance = windows::Win32::Foundation::HINSTANCE::from(hmodule);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        RegisterClassW(&wc);
        // HWND_MESSAGE: a message-only window needs no visible surface —
        // the tray icon itself is the only UI surface this process shows.
        CreateWindowExW(
            Default::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            hinstance,
            None,
        )
        .unwrap_or_default()
    }

    extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_TRAYICON => {
                let mouse_msg = (lparam.0 as u32) & 0xFFFF;
                if mouse_msg == WM_LBUTTONUP || mouse_msg == WM_RBUTTONUP {
                    on_tray_click(hwnd);
                }
                LRESULT(0)
            }
            WM_CORE_MESSAGE => {
                // Safety: only `post_core_message` below produces this
                // message, always with a pointer from `Box::into_raw` of
                // exactly this type.
                let boxed = unsafe { Box::from_raw(wparam.0 as *mut CoreToTray) };
                on_core_message(*boxed);
                LRESULT(0)
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    // --- Tray click -> popup menu -> action. ---

    fn on_tray_click(hwnd: HWND) {
        let offer = {
            let app = APP.lock().unwrap();
            app.as_ref().unwrap().first_run.state()
        };

        if let Some(state) = offer {
            handle_first_run_click(hwnd, state);
            return;
        }

        let devices = {
            let app = APP.lock().unwrap();
            app.as_ref().unwrap().picker.devices().to_vec()
        };
        let items: Vec<(usize, String)> = devices
            .iter()
            .enumerate()
            .map(|(i, d)| (i + 1, d.name.clone()))
            .chain([(ID_OPEN_LOGS, "Open Logs".to_string()), (ID_QUIT, "Quit".to_string())])
            .collect();

        match unsafe { show_popup_menu(hwnd, &items) } {
            Some(ID_OPEN_LOGS) => handle_open_logs(),
            Some(ID_QUIT) => unsafe {
                PostQuitMessage(0);
            },
            Some(id) if id >= 1 && id <= devices.len() => {
                let device: &DiscoveredDevice = &devices[id - 1];
                send_to_core(&TrayToCore::Connect { device_id: device.id.clone() });
            }
            _ => {}
        }
    }

    fn handle_first_run_click(hwnd: HWND, state: FirstRunState) {
        let label = match state {
            FirstRunState::OfferInstall => "Set up virtual display",
            FirstRunState::SetupIncompleteDeclined => "Set up virtual display (setup incomplete)",
        };
        let items = [(ID_INSTALL, label.to_string()), (ID_NOT_NOW, "Not now".to_string())];
        match unsafe { show_popup_menu(hwnd, &items) } {
            Some(ID_INSTALL) => {
                let _ = first_run::accept(&first_run::WindowsInstallerLauncher);
            }
            Some(ID_NOT_NOW) => {
                let mut app = APP.lock().unwrap();
                app.as_mut().unwrap().first_run.decline();
            }
            _ => {}
        }
    }

    struct NoopIpcRequester;
    impl open_logs::IpcRequester for NoopIpcRequester {
        fn request_open_log_folder(&self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct PipeIpcRequester(Arc<Mutex<std::fs::File>>);
    impl open_logs::IpcRequester for PipeIpcRequester {
        fn request_open_log_folder(&self) -> std::io::Result<()> {
            let line = format!("{}\n", ipc::to_line(&TrayToCore::OpenLogFolder));
            self.0.lock().unwrap().write_all(line.as_bytes())
        }
    }

    fn handle_open_logs() {
        let writer = APP.lock().unwrap().as_ref().unwrap().writer.clone();
        let outcome: ConnectOutcome<()> =
            if writer.is_some() { ConnectOutcome::Connected(()) } else { ConnectOutcome::CoreNotRunning };
        let action = open_logs::decide(&outcome);
        match writer {
            Some(w) => {
                let _ = open_logs::perform(&action, &PipeIpcRequester(w), &open_logs::WindowsFolderOpener);
            }
            None => {
                let _ = open_logs::perform(&action, &NoopIpcRequester, &open_logs::WindowsFolderOpener);
            }
        }
    }

    unsafe fn show_popup_menu(hwnd: HWND, items: &[(usize, String)]) -> Option<usize> {
        let menu = CreatePopupMenu().ok()?;
        for (id, label) in items {
            let wide_label = wide(label);
            let _ = AppendMenuW(menu, MF_STRING, *id, PCWSTR(wide_label.as_ptr()));
        }
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let _ = SetForegroundWindow(hwnd);
        let selected =
            TrackPopupMenuEx(menu, (TPM_LEFTALIGN | TPM_RETURNCMD).0, cursor.x, cursor.y, hwnd, None);
        let _ = DestroyMenu(menu);
        if selected.0 == 0 { None } else { Some(selected.0 as usize) }
    }

    fn send_to_core(msg: &TrayToCore) {
        let writer = APP.lock().unwrap().as_ref().unwrap().writer.clone();
        if let Some(writer) = writer {
            let line = format!("{}\n", ipc::to_line(msg));
            let _ = writer.lock().unwrap().write_all(line.as_bytes());
        }
    }

    // --- Applying an incoming CoreToTray message (posted from the reader thread). ---

    fn on_core_message(msg: CoreToTray) {
        let mut app_guard = APP.lock().unwrap();
        let app = app_guard.as_mut().unwrap();
        app.picker.apply(&msg);
        app.first_run.apply(&msg);
        if let Some(new_status) = status::status_for_message(&msg) {
            app.status = Some(new_status);
        }
        if let Some(current) = app.status.clone() {
            let _ = status::windows_impl::render(app.hwnd.get(), &current);
        }
    }

    fn set_writer(hwnd: HWND, writer: Option<Arc<Mutex<std::fs::File>>>) {
        let mut app_guard = APP.lock().unwrap();
        let app = app_guard.as_mut().unwrap();
        app.writer = writer.clone();
        if writer.is_none() {
            let outcome: ConnectOutcome<()> = ConnectOutcome::CoreNotRunning;
            if let Some(s) = status::status_for_connect_failure(&outcome) {
                app.status = Some(s.clone());
                let _ = status::windows_impl::render(hwnd, &s);
            }
        }
    }

    // --- The IPC connect/read loop, on its own thread. ---

    fn post_core_message(hwnd: HWND, msg: CoreToTray) {
        let ptr = Box::into_raw(Box::new(msg));
        unsafe {
            if PostMessageW(hwnd, WM_CORE_MESSAGE, WPARAM(ptr as usize), LPARAM(0)).is_err() {
                // The window is gone (process shutting down); reclaim the
                // box instead of leaking it.
                drop(Box::from_raw(ptr));
            }
        }
    }

    fn spawn_ipc_reader(hwnd: HWND) {
        let hwnd = SendHwnd(hwnd);
        std::thread::spawn(move || {
            let hwnd = hwnd.get();
            loop {
                match ipc_client::connect(&WindowsPipeConnector, PIPE_NAME) {
                    ConnectOutcome::CoreNotRunning => {
                        set_writer(hwnd, None);
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                    ConnectOutcome::Connected(file) => {
                        if let Ok(write_file) = file.try_clone() {
                            set_writer(hwnd, Some(Arc::new(Mutex::new(write_file))));
                        }
                        let mut reader = BufReader::new(file);
                        loop {
                            match ipc_client::read_message(&mut reader) {
                                Some(Ok(msg)) => post_core_message(hwnd, msg),
                                Some(Err(_)) => continue, // malformed line: keep reading
                                None => break,            // pipe closed: core stopped or restarted
                            }
                        }
                        set_writer(hwnd, None);
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                }
            }
        });
    }
}
