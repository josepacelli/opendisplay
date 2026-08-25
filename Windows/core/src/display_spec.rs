//! Derives a [`DisplaySpec`] from a completed handshake's [`PanelReport`],
//! per `design.md`'s Data Models section.
//!
//! Pure derivation only — no device-interface/IOCTL call here (that's T16's
//! `display::create/resize/destroy`). This module exists so the sizing math
//! (native pixel density, orientation) is plain, unit-testable Rust
//! independent of `opendisplay-idd`.

use crate::protocol_session::PanelReport;

/// The virtual display's orientation, derived from the panel's reported
/// pixel dimensions (`PROTOCOL.md` §7: `hello.pixelsWide/High` are in the
/// panel's *current* orientation — portrait swaps them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
}

/// Session-scoped display parameters derived once per session at handshake
/// time (design.md: "Created once per session at handshake time, owned
/// entirely by `windows-core`. Never crosses the IPC boundary").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplaySpec {
    pub width_px: u32,
    pub height_px: u32,
    /// e.g. 2.0 / 3.0 for `@2x` / `@3x` panels — taken directly from
    /// `hello.scale` (`PROTOCOL.md` §6.1: "the device's UI scale factor").
    pub scale_factor: f32,
    pub orientation: Orientation,
}

/// Derives a [`DisplaySpec`] from the panel report a handshake yields
/// (spec WiFi AC 2: size the virtual display "from the device's reported
/// panel dimensions at native pixel density"; spec WiFi AC 4: rotation
/// yields the new native resolution).
///
/// `pixelsWide`/`pixelsHigh` already reflect the panel's current
/// orientation (`PROTOCOL.md` §7), so `width_px`/`height_px` are carried
/// through unchanged; `orientation` is derived by comparing them (a taller
/// panel is portrait, a wider one landscape).
pub fn derive_display_spec(panel: &PanelReport) -> DisplaySpec {
    let orientation = if panel.pixels_high >= panel.pixels_wide {
        Orientation::Portrait
    } else {
        Orientation::Landscape
    };

    DisplaySpec {
        width_px: panel.pixels_wide,
        height_px: panel.pixels_high,
        scale_factor: panel.scale,
        orientation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(pixels_wide: u32, pixels_high: u32, scale: f32) -> PanelReport {
        PanelReport {
            pixels_wide,
            pixels_high,
            scale,
            device: None,
            id: None,
        }
    }

    #[test]
    fn a_2x_panel_report_yields_scale_factor_2() {
        let spec = derive_display_spec(&panel(1170, 2532, 2.0));
        assert_eq!(spec.scale_factor, 2.0);
    }

    #[test]
    fn a_3x_panel_report_yields_scale_factor_3() {
        let spec = derive_display_spec(&panel(1284, 2778, 3.0));
        assert_eq!(spec.scale_factor, 3.0);
    }

    #[test]
    fn a_portrait_hello_yields_portrait_orientation_with_dimensions_unswapped() {
        // Taller than wide: portrait, per PROTOCOL.md §7's "current
        // orientation" convention.
        let spec = derive_display_spec(&panel(750, 1334, 2.0));

        assert_eq!(spec.orientation, Orientation::Portrait);
        assert_eq!(spec.width_px, 750);
        assert_eq!(spec.height_px, 1334);
    }

    #[test]
    fn a_landscape_hello_yields_landscape_orientation_with_dimensions_swapped() {
        // Same device, rotated: PROTOCOL.md §7 says the panel re-reports
        // pixelsWide/High swapped for the new orientation.
        let spec = derive_display_spec(&panel(1334, 750, 2.0));

        assert_eq!(spec.orientation, Orientation::Landscape);
        assert_eq!(spec.width_px, 1334);
        assert_eq!(spec.height_px, 750);
    }
}
