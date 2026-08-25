//! windows-tray's UI modules. Each renders a slice of `windows-core`'s
//! `CoreToTray` stream (via `ipc_client`, T24) as a native tray/menu
//! surface — OS-bound rendering the Test Coverage Matrix marks `none`
//! (manual verification only), with the underlying state/decision logic
//! kept plain and separable from the OS calls.

pub mod first_run;
pub mod picker;
pub mod status;
