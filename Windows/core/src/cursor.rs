//! Tracks the Windows cursor's position and image on the virtual display
//! and emits outbound `cursor`/`cursorImg` control messages on change, per
//! `PROTOCOL.md` §6.2 and spec Touch/cursor AC 3.
//!
//! Per the Test Coverage Matrix, this is OS-bound code (`GetCursorInfo`,
//! icon extraction, PNG encoding) verified manually against real Windows 11
//! hardware, not by an automated gate. Nothing here runs on this macOS host
//! (no Rust toolchain, no cursor to watch) — every call is unverified
//! pending real hardware.

use crate::display_spec::DisplaySpec;

/// One outbound message this module produces, per `PROTOCOL.md` §6.2.
#[derive(Debug, Clone, PartialEq)]
pub enum CursorEvent {
    /// `cursor`: `x`/`y` normalized to the display (`None` when hidden,
    /// per §6.2: "when hidden they MAY be absent"), `visible` is the wire's
    /// `v`.
    Cursor {
        position: Option<(f32, f32)>,
        visible: bool,
    },
    /// `cursorImg`: `png` (already base64-appropriate raw PNG bytes — the
    /// wire layer, not this module, is responsible for base64-encoding
    /// it), plus the sprite's size/hotspot normalized per §6.2/§7.
    CursorImg {
        png: Vec<u8>,
        width_norm: f32,
        height_norm: f32,
        hotspot_x_norm: f32,
        hotspot_y_norm: f32,
    },
}

/// A cursor's identity for change detection — an icon handle's numeric
/// value is stable across `GetCursorInfo` calls while the cursor image is
/// unchanged, and changes when the OS swaps in a different cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CursorIdentity(isize);

#[derive(Debug, Clone, Copy, PartialEq)]
struct CursorSnapshot {
    x_px: i32,
    y_px: i32,
    visible: bool,
    icon: Option<CursorIdentity>,
}

/// Diffs two consecutive snapshots into the events that changed, per spec
/// Touch/cursor AC 3 ("WHILE the Windows cursor's position or image
/// changes... send `cursor`/`cursorImg` control messages").
fn diff_snapshots(
    display: &DisplaySpec,
    previous: Option<CursorSnapshot>,
    current: CursorSnapshot,
    png_for_icon: impl FnOnce(CursorIdentity) -> Option<(Vec<u8>, f32, f32, f32, f32)>,
) -> Vec<CursorEvent> {
    let mut events = Vec::new();

    let position_or_visibility_changed = match previous {
        Some(prev) => {
            prev.x_px != current.x_px || prev.y_px != current.y_px || prev.visible != current.visible
        }
        None => true,
    };
    if position_or_visibility_changed {
        let position = if current.visible {
            Some((
                current.x_px as f32 / display.width_px.max(1) as f32,
                current.y_px as f32 / display.height_px.max(1) as f32,
            ))
        } else {
            None
        };
        events.push(CursorEvent::Cursor {
            position,
            visible: current.visible,
        });
    }

    let icon_changed = previous.and_then(|p| p.icon) != current.icon;
    if icon_changed {
        if let Some(icon) = current.icon {
            if let Some((png, width_norm, height_norm, hotspot_x_norm, hotspot_y_norm)) =
                png_for_icon(icon)
            {
                events.push(CursorEvent::CursorImg {
                    png,
                    width_norm,
                    height_norm,
                    hotspot_x_norm,
                    hotspot_y_norm,
                });
            }
        }
    }

    events
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use windows::Win32::Graphics::Gdi::{DeleteObject, GetDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS};
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
        IWICImagingFactory, WICBitmapEncoderNoCache,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, ICONINFO,
    };

    /// Polls the current cursor position/visibility/identity via
    /// `GetCursorInfo`, per `PROTOCOL.md` §6.2's `cursor` fields.
    fn poll_cursor_info() -> windows::core::Result<CursorSnapshot> {
        let mut info = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetCursorInfo(&mut info)? };

        Ok(CursorSnapshot {
            x_px: info.ptScreenPos.x,
            y_px: info.ptScreenPos.y,
            visible: info.flags == CURSOR_SHOWING,
            icon: if info.hCursor.is_invalid() {
                None
            } else {
                Some(CursorIdentity(info.hCursor.0 as isize))
            },
        })
    }

    /// Extracts `hcursor`'s bitmap into a normalized PNG sprite via WIC
    /// (`GUID_ContainerFormatPng`), per `PROTOCOL.md` §6.2's `cursorImg`
    /// fields (`nw`/`nh` normalized to the display, `ax`/`ay` normalized to
    /// the sprite itself).
    fn encode_cursor_png(
        display: &DisplaySpec,
        hcursor: windows::Win32::UI::WindowsAndMessaging::HICON,
    ) -> windows::core::Result<(Vec<u8>, f32, f32, f32, f32)> {
        unsafe {
            let mut icon_info = ICONINFO::default();
            GetIconInfo(hcursor, &mut icon_info)?;

            let bitmap = if icon_info.hbmColor.is_invalid() {
                icon_info.hbmMask
            } else {
                icon_info.hbmColor
            };

            let mut header = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    ..Default::default()
                },
                ..Default::default()
            };
            let screen_dc = windows::Win32::Graphics::Gdi::GetDC(None);
            // First call (no output buffer) fills in width/height/bpp.
            GetDIBits(screen_dc, bitmap, 0, 0, None, &mut header, DIB_RGB_COLORS);
            let width = header.bmiHeader.biWidth.unsigned_abs();
            let height = header.bmiHeader.biHeight.unsigned_abs();

            header.bmiHeader.biBitCount = 32;
            header.bmiHeader.biCompression = BI_RGB.0;
            header.bmiHeader.biHeight = -(height as i32); // top-down DIB
            let mut pixels = vec![0u8; (width * height * 4) as usize];
            GetDIBits(
                screen_dc,
                bitmap,
                0,
                height,
                Some(pixels.as_mut_ptr() as *mut _),
                &mut header,
                DIB_RGB_COLORS,
            );
            windows::Win32::Graphics::Gdi::ReleaseDC(None, screen_dc);
            let _ = DeleteObject(icon_info.hbmColor);
            let _ = DeleteObject(icon_info.hbmMask);

            let png = encode_bgra_as_png(&pixels, width, height)?;

            let width_norm = width as f32 / display.width_px.max(1) as f32;
            let height_norm = height as f32 / display.height_px.max(1) as f32;
            let hotspot_x_norm = icon_info.xHotspot as f32 / width.max(1) as f32;
            let hotspot_y_norm = icon_info.yHotspot as f32 / height.max(1) as f32;

            Ok((png, width_norm, height_norm, hotspot_x_norm, hotspot_y_norm))
        }
    }

    /// Encodes a top-down 32bpp BGRA buffer as PNG via WIC, so no
    /// third-party PNG-encoding crate is needed (matches design.md's
    /// "no forced external dependency" principle already applied to the
    /// WiFi/USB transport choices).
    fn encode_bgra_as_png(bgra: &[u8], width: u32, height: u32) -> windows::core::Result<Vec<u8>> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let stream = factory.CreateStream()?;
            stream.InitializeFromMemory(&mut Vec::new())?; // grows via IStream::Write below
            let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
            encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
            let frame = {
                let mut frame = None;
                let mut props: Option<
                    windows::Win32::System::Com::StructuredStorage::IPropertyBag2,
                > = None;
                encoder.CreateNewFrame(&mut frame, &mut props)?;
                frame.expect("CreateNewFrame succeeded without a frame")
            };
            frame.Initialize(None)?;
            frame.SetSize(width, height)?;
            let mut format = GUID_WICPixelFormat32bppBGRA;
            frame.SetPixelFormat(&mut format)?;
            let stride = width * 4;
            frame.WritePixels(height, stride, bgra)?;
            frame.Commit()?;
            encoder.Commit()?;

            // Read the encoded bytes back out of the in-memory IStream.
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let mut read = 0u32;
                stream
                    .Read(chunk.as_mut_ptr() as *mut _, chunk.len() as u32, Some(&mut read as *mut u32))
                    .ok()?;
                if read == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..read as usize]);
            }
            Ok(buf)
        }
    }

    /// Polls once and, if anything changed since `previous`, returns the
    /// events to emit plus the new snapshot for the next call.
    pub fn poll(
        display: &DisplaySpec,
        previous: Option<super::CursorSnapshotHandle>,
    ) -> windows::core::Result<(Vec<CursorEvent>, super::CursorSnapshotHandle)> {
        let current = poll_cursor_info()?;
        let prev_snapshot = previous.map(|h| h.0);
        let events = diff_snapshots(display, prev_snapshot, current, |icon| {
            let hcursor = windows::Win32::UI::WindowsAndMessaging::HICON(icon.0 as *mut _);
            encode_cursor_png(display, hcursor).ok()
        });
        Ok((events, super::CursorSnapshotHandle(current)))
    }
}

/// Opaque handle to the previous poll's snapshot, threaded through
/// successive [`windows_impl::poll`] calls by whatever runtime loop drives
/// this module (the polling cadence itself is a runtime concern, same as
/// `session_state`'s retry timer).
#[cfg(windows)]
pub struct CursorSnapshotHandle(CursorSnapshot);

#[cfg(windows)]
pub use windows_impl::poll;

#[cfg(not(windows))]
#[allow(dead_code)]
fn _unused(_: &DisplaySpec) {}
