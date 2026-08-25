//! Device transports (WiFi, USB) `windows-core` can discover and dial.
//!
//! Split into one module per transport plus a shared `dial` module (T12)
//! that unifies them. See `design.md`'s `windows-core` component for the
//! interface shapes this mirrors.

pub mod wifi;
