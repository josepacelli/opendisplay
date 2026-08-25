//! Maps incoming `touch`/`scroll` control messages (`PROTOCOL.md` §6.1) to
//! `SendInput` mouse-down/move/up and scroll-wheel calls at the
//! corresponding point on the virtual display.
//!
//! The actual `SendInput` call is isolated behind the [`InputInjector`]
//! trait, per this task's own text, so the coordinate/delta mapping logic
//! is plain, unit-testable Rust independent of the OS call — same seam
//! pattern as `transport::{wifi,usb}`'s source traits.

use crate::display_spec::DisplaySpec;

/// `touch.phase` values, per `PROTOCOL.md` §6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Began,
    Moved,
    Ended,
    Cancelled,
}

/// A mouse button transition [`next_touch_action`] decides on, for
/// [`InputInjector`] to carry out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickAction {
    Down,
    Up,
}

/// Abstraction over the actual `SendInput` call, so the mapping logic below
/// is testable without a live OS call. `x`/`y` are absolute virtual-desktop
/// pixel coordinates (top-left origin), per `PROTOCOL.md` §7's convention
/// for `touch.x/y`.
pub trait InputInjector {
    fn move_cursor(&mut self, x: i32, y: i32);
    fn mouse_down(&mut self);
    fn mouse_up(&mut self);
    /// `horizontal`/`vertical` are signed wheel deltas in Windows'
    /// `WHEEL_DELTA` (120) units, per [`map_scroll_to_wheel_delta`].
    fn scroll(&mut self, horizontal: i32, vertical: i32);
}

/// Maps a `touch.x/y` normalized position (`PROTOCOL.md` §7: `0..1`,
/// top-left origin, video space) to a pixel point on the virtual display —
/// spec Touch/cursor AC 1's "corresponding point on the virtual display".
/// The video's encoded dimensions equal `DisplaySpec`'s (P1 ships no
/// quality/scale presets, per the spec's Out of Scope table), so mapping
/// against `width_px`/`height_px` directly is exact, not an approximation.
pub fn map_touch_to_point(display: &DisplaySpec, x_norm: f64, y_norm: f64) -> (i32, i32) {
    let x = (x_norm * display.width_px as f64).round() as i32;
    let y = (y_norm * display.height_px as f64).round() as i32;
    (x, y)
}

/// Decides the click-button transition for one `touch` message, mirroring
/// `Mac/InputInjector.swift`'s `handleTouch` state machine (finger down =
/// left button down, finger move = drag, finger up = button up): `began`
/// presses, `moved` never changes button state, `ended`/`cancelled` release
/// only if a press is outstanding (a spurious up with no matching down is
/// ignored, same guard Mac's injector uses).
///
/// Returns the new `is_down` state and the action to carry out, if any.
pub fn next_touch_action(is_down: bool, phase: TouchPhase) -> (bool, Option<ClickAction>) {
    match phase {
        TouchPhase::Began => (true, Some(ClickAction::Down)),
        TouchPhase::Moved => (is_down, None),
        TouchPhase::Ended | TouchPhase::Cancelled => {
            if is_down {
                (false, Some(ClickAction::Up))
            } else {
                (false, None)
            }
        }
    }
}

/// Applies one `touch` message to `injector`: always moves the cursor to
/// the mapped point, then carries out whatever click transition
/// [`next_touch_action`] decides. `is_down` is the caller's persisted touch
/// state across calls (one per active session, same role as
/// `Mac/InputInjector.swift`'s `isDown` field).
pub fn inject_touch(
    display: &DisplaySpec,
    injector: &mut dyn InputInjector,
    is_down: &mut bool,
    phase: TouchPhase,
    x_norm: f64,
    y_norm: f64,
) {
    let (x, y) = map_touch_to_point(display, x_norm, y_norm);
    injector.move_cursor(x, y);

    let (next_is_down, action) = next_touch_action(*is_down, phase);
    *is_down = next_is_down;
    match action {
        Some(ClickAction::Down) => injector.mouse_down(),
        Some(ClickAction::Up) => injector.mouse_up(),
        None => {}
    }
}

/// Windows' `WHEEL_DELTA`: the magnitude of one standard wheel "click" for
/// `MOUSEEVENTF_WHEEL`/`MOUSEEVENTF_HWHEEL`.
pub const WHEEL_DELTA: i32 = 120;

/// How many `scroll.dx/dy` video pixels (`PROTOCOL.md` §7: pixels, not
/// normalized) equal one wheel "click". `PROTOCOL.md` does not mandate a
/// pixel-to-wheel-notch ratio — this is an implementation choice, not a
/// spec-defined value (flagged as a spec-precision gap in this task's Test
/// Adequacy Review), chosen to feel roughly comparable to a physical wheel
/// notch at typical phone-panel pixel density.
pub const PIXELS_PER_WHEEL_NOTCH: f64 = 40.0;

/// Maps `scroll.dx/dy` (video pixels, natural-scrolling sign per
/// `PROTOCOL.md` §7: "content follows the fingers") to `(horizontal,
/// vertical)` `WHEEL_DELTA`-scaled wheel deltas. Vertical is negated:
/// fingers moving down (`dy > 0`) should scroll content down, which in
/// Windows' wheel convention (positive = rotated away from the user,
/// conventionally scrolling content up) means a negative `vertical` value.
pub fn map_scroll_to_wheel_delta(dx: f64, dy: f64) -> (i32, i32) {
    let horizontal = ((dx / PIXELS_PER_WHEEL_NOTCH) * WHEEL_DELTA as f64).round() as i32;
    let vertical = -((dy / PIXELS_PER_WHEEL_NOTCH) * WHEEL_DELTA as f64).round() as i32;
    (horizontal, vertical)
}

/// Applies one `scroll` message to `injector`.
pub fn inject_scroll(injector: &mut dyn InputInjector, dx: f64, dy: f64) {
    let (horizontal, vertical) = map_scroll_to_wheel_delta(dx, dy);
    injector.scroll(horizontal, vertical);
}

/// The real `InputInjector`, backed by `SendInput`. Not exercised by any
/// automated gate on this host (no Rust toolchain, no OS to inject into) —
/// see the Test Coverage Matrix's manual-verification note. Every
/// non-trivial branch this module needs (coordinate/delta mapping, click
/// state transitions) lives above this line, behind the trait, and is
/// unit-tested without it.
///
/// **Scope note**: `move_cursor`'s `x`/`y` are expected to already be
/// absolute virtual-desktop pixel coordinates (this display's origin plus
/// the mapped point) — resolving that origin is `display::create`'s (T16)
/// concern, wired in by whichever runtime component owns both the
/// `VirtualDisplayHandle` and this injector; it is not resolved here.
#[cfg(windows)]
pub struct RealInputInjector;

#[cfg(windows)]
mod windows_impl {
    use super::RealInputInjector;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_WHEEL, MOUSEINPUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    fn send_mouse_input(mi: MOUSEINPUT) {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 { mi },
        };
        unsafe {
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        }
    }

    impl super::InputInjector for RealInputInjector {
        fn move_cursor(&mut self, x: i32, y: i32) {
            // SendInput's absolute mode is normalized 0..65535 across the
            // virtual screen (all monitors), per the MOUSEINPUT docs.
            unsafe {
                let vs_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
                let vs_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
                let vs_w = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
                let vs_h = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);
                let normalized_x = ((x - vs_x) * 65535) / vs_w;
                let normalized_y = ((y - vs_y) * 65535) / vs_h;
                send_mouse_input(MOUSEINPUT {
                    dx: normalized_x,
                    dy: normalized_y,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                    time: 0,
                    dwExtraInfo: 0,
                });
            }
        }

        fn mouse_down(&mut self) {
            send_mouse_input(MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_LEFTDOWN,
                time: 0,
                dwExtraInfo: 0,
            });
        }

        fn mouse_up(&mut self) {
            send_mouse_input(MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_LEFTUP,
                time: 0,
                dwExtraInfo: 0,
            });
        }

        fn scroll(&mut self, horizontal: i32, vertical: i32) {
            if vertical != 0 {
                send_mouse_input(MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: vertical as u32,
                    dwFlags: MOUSEEVENTF_WHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                });
            }
            if horizontal != 0 {
                send_mouse_input(MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: horizontal as u32,
                    dwFlags: MOUSEEVENTF_HWHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(width_px: u32, height_px: u32) -> DisplaySpec {
        DisplaySpec {
            width_px,
            height_px,
            scale_factor: 2.0,
            orientation: crate::display_spec::Orientation::Portrait,
        }
    }

    #[derive(Default)]
    struct FakeInjector {
        moves: Vec<(i32, i32)>,
        downs: u32,
        ups: u32,
        scrolls: Vec<(i32, i32)>,
    }

    impl InputInjector for FakeInjector {
        fn move_cursor(&mut self, x: i32, y: i32) {
            self.moves.push((x, y));
        }
        fn mouse_down(&mut self) {
            self.downs += 1;
        }
        fn mouse_up(&mut self) {
            self.ups += 1;
        }
        fn scroll(&mut self, horizontal: i32, vertical: i32) {
            self.scrolls.push((horizontal, vertical));
        }
    }

    // --- Touch coordinate mapping (Done-when: "correct absolute point"). ---

    #[test]
    fn top_left_normalized_origin_maps_to_pixel_origin() {
        assert_eq!(map_touch_to_point(&spec(750, 1334), 0.0, 0.0), (0, 0));
    }

    #[test]
    fn bottom_right_normalized_corner_maps_to_the_display_pixel_bounds() {
        assert_eq!(map_touch_to_point(&spec(750, 1334), 1.0, 1.0), (750, 1334));
    }

    #[test]
    fn center_normalized_point_maps_to_the_display_center() {
        assert_eq!(map_touch_to_point(&spec(1000, 2000), 0.5, 0.5), (500, 1000));
    }

    // --- Touch phase -> click action state machine. ---

    #[test]
    fn began_presses_the_button_and_sets_is_down() {
        assert_eq!(next_touch_action(false, TouchPhase::Began), (true, Some(ClickAction::Down)));
    }

    #[test]
    fn moved_never_changes_button_state() {
        assert_eq!(next_touch_action(true, TouchPhase::Moved), (true, None));
        assert_eq!(next_touch_action(false, TouchPhase::Moved), (false, None));
    }

    #[test]
    fn ended_while_down_releases_the_button() {
        assert_eq!(next_touch_action(true, TouchPhase::Ended), (false, Some(ClickAction::Up)));
    }

    #[test]
    fn a_spurious_up_with_no_matching_down_is_ignored() {
        assert_eq!(next_touch_action(false, TouchPhase::Ended), (false, None));
        assert_eq!(next_touch_action(false, TouchPhase::Cancelled), (false, None));
    }

    // --- inject_touch: mapping + trait wiring together. ---

    #[test]
    fn inject_touch_began_moves_to_the_mapped_point_then_presses_down() {
        let display = spec(1000, 2000);
        let mut injector = FakeInjector::default();
        let mut is_down = false;

        inject_touch(&display, &mut injector, &mut is_down, TouchPhase::Began, 0.5, 0.5);

        assert_eq!(injector.moves, vec![(500, 1000)]);
        assert_eq!(injector.downs, 1);
        assert_eq!(injector.ups, 0);
        assert!(is_down);
    }

    #[test]
    fn inject_touch_ended_releases_only_when_previously_down() {
        let display = spec(1000, 2000);
        let mut injector = FakeInjector::default();
        let mut is_down = true;

        inject_touch(&display, &mut injector, &mut is_down, TouchPhase::Ended, 0.0, 0.0);

        assert_eq!(injector.ups, 1);
        assert!(!is_down);
    }

    // --- Scroll delta mapping (Done-when: "correct wheel magnitude/direction"). ---

    #[test]
    fn positive_dy_natural_scrolling_down_yields_negative_vertical_wheel_delta() {
        let (_, vertical) = map_scroll_to_wheel_delta(0.0, PIXELS_PER_WHEEL_NOTCH);
        assert_eq!(vertical, -WHEEL_DELTA);
    }

    #[test]
    fn doubling_the_scroll_delta_doubles_the_wheel_magnitude() {
        let (_, single) = map_scroll_to_wheel_delta(0.0, PIXELS_PER_WHEEL_NOTCH);
        let (_, double) = map_scroll_to_wheel_delta(0.0, 2.0 * PIXELS_PER_WHEEL_NOTCH);
        assert_eq!(double, 2 * single);
    }

    #[test]
    fn horizontal_and_vertical_scroll_deltas_are_independent() {
        let (horizontal, vertical) = map_scroll_to_wheel_delta(PIXELS_PER_WHEEL_NOTCH, 0.0);
        assert_eq!(vertical, 0);
        assert_eq!(horizontal, WHEEL_DELTA);
    }

    #[test]
    fn inject_scroll_forwards_the_mapped_deltas_to_the_injector() {
        let mut injector = FakeInjector::default();

        inject_scroll(&mut injector, PIXELS_PER_WHEEL_NOTCH, PIXELS_PER_WHEEL_NOTCH);

        assert_eq!(injector.scrolls, vec![(WHEEL_DELTA, -WHEEL_DELTA)]);
    }
}
