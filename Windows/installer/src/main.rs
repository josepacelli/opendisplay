//! windows-installer entry point: one-shot elevated first-run setup
//! (driver install, Scheduled Task registration, tray autostart) and the
//! uninstall path, per design.md's `windows-installer` component.
//!
//! This crate is scaffolded here as part of T29 — Phase 7 is the first to
//! touch `Windows/installer/`, so this Cargo.toml/main.rs pair is created
//! now, mirroring how T1 scaffolded `protocol`/`ipc`/`core`/`tray`.

mod autostart;
mod driver_install;
mod scheduled_task;

fn main() {}
