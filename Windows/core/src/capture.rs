//! DXGI Desktop Duplication capture targeting the virtual display created by
//! `display::create` (T16), per `design.md`'s Tech Decisions ("purpose-built
//! for capture one output into a streaming pipeline... hands back GPU
//! textures... excludes the cursor from the frame by default").
//!
//! Per the Test Coverage Matrix, this is OS/hardware-bound code (DXGI is a
//! real GPU/display API) verified manually against real Windows 11
//! hardware, not by an automated gate. Nothing here runs on this macOS host
//! (no Rust toolchain, no DXGI, no virtual display to target) — every call
//! is unverified pending real hardware.
//!
//! **Output targeting.** `opendisplay-idd`'s device-interface/IOCTL contract
//! (`Windows/driver/Ioctl.h`) does not hand back a DXGI adapter LUID for the
//! monitor it just created, so this module resolves the target output by
//! matching `DisplaySpec`'s dimensions against each enumerated
//! `DXGI_OUTPUT_DESC.DesktopCoordinates`, skipping the output whose desktop
//! origin is `(0, 0)` (the conventional primary-monitor origin) so a
//! same-resolution physical monitor is not mistaken for the just-created
//! virtual one. This is a heuristic, not a guarantee — it is the best this
//! module can do without a driver-side adapter-identity handoff, and is
//! exactly the kind of assumption the spec's Independent Test (T17 Manual
//! verification) exists to catch on real hardware.

use crate::display_spec::DisplaySpec;

#[cfg(windows)]
mod windows_impl {
    use super::DisplaySpec;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::Dxgi::{
        IDXGIAdapter, IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
        DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTPUT_DESC,
    };
    use windows::core::Interface;

    /// One GPU-resident captured frame, ready to hand to `encode::start`
    /// (T18). The cursor is not composited into `texture` — DXGI Desktop
    /// Duplication excludes it by design (design.md's Tech Decisions); it is
    /// forwarded separately by `cursor::watch` (T21).
    pub struct CapturedFrame {
        pub texture: ID3D11Texture2D,
        pub captured_at: std::time::Instant,
    }

    /// An open DXGI Desktop Duplication session against the virtual
    /// display's output.
    pub struct FrameStream {
        device: ID3D11Device,
        _context: ID3D11DeviceContext,
        duplication: IDXGIOutputDuplication,
    }

    /// Finds the `IDXGIOutput1` whose `DesktopCoordinates` size matches
    /// `spec`'s pixel dimensions, preferring one that is not at the
    /// conventional primary-monitor origin `(0, 0)` (see the module-level
    /// doc comment on why this is a heuristic).
    fn find_target_output(
        adapter: &IDXGIAdapter,
        spec: &DisplaySpec,
    ) -> windows::core::Result<IDXGIOutput1> {
        let mut candidate: Option<IDXGIOutput1> = None;

        for i in 0.. {
            let output = unsafe { adapter.EnumOutputs(i) };
            let output = match output {
                Ok(o) => o,
                Err(_) => break, // DXGI_ERROR_NOT_FOUND: no more outputs
            };
            let output1: IDXGIOutput1 = output.cast()?;
            let desc = unsafe { output1.GetDesc()? };

            let width = (desc.DesktopCoordinates.right - desc.DesktopCoordinates.left) as u32;
            let height = (desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top) as u32;
            if width != spec.width_px || height != spec.height_px {
                continue;
            }

            let is_primary_origin =
                desc.DesktopCoordinates.left == 0 && desc.DesktopCoordinates.top == 0;
            if !is_primary_origin {
                // A non-primary-origin match is the best signal available;
                // take it immediately.
                return Ok(output1);
            }
            candidate.get_or_insert(output1);
        }

        candidate.ok_or_else(|| {
            windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "no DXGI output matches the virtual display's DisplaySpec dimensions",
            )
        })
    }

    /// Starts DXGI Desktop Duplication against the output matching `spec`
    /// (the just-created virtual display), per spec WSEND-03.
    pub fn start(spec: &DisplaySpec) -> windows::core::Result<FrameStream> {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        let device = device.expect("D3D11CreateDevice succeeded without a device");
        let context = context.expect("D3D11CreateDevice succeeded without a context");

        let dxgi_device: IDXGIDevice = device.cast()?;
        let adapter: IDXGIAdapter = unsafe { dxgi_device.GetAdapter()? };
        let output = find_target_output(&adapter, spec)?;
        let duplication = unsafe { output.DuplicateOutput(&device)? };

        Ok(FrameStream {
            device,
            _context: context,
            duplication,
        })
    }

    impl FrameStream {
        /// Blocks up to `timeout_ms` for the next desktop frame. Releases
        /// the previously acquired frame first (Desktop Duplication allows
        /// only one outstanding frame at a time per session).
        pub fn next_frame(&mut self, timeout_ms: u32) -> windows::core::Result<CapturedFrame> {
            unsafe {
                let _ = self.duplication.ReleaseFrame();

                let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut resource: Option<IDXGIResource> = None;
                self.duplication
                    .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)?;
                let resource = resource.expect("AcquireNextFrame succeeded without a resource");
                let texture: ID3D11Texture2D = resource.cast()?;

                let _ = &self.device; // kept alive for the duplication's lifetime

                Ok(CapturedFrame {
                    texture,
                    captured_at: std::time::Instant::now(),
                })
            }
        }
    }

    impl Drop for FrameStream {
        fn drop(&mut self) {
            unsafe {
                let _ = self.duplication.ReleaseFrame();
            }
        }
    }
}

#[cfg(windows)]
pub use windows_impl::{start, CapturedFrame, FrameStream};

// Kept for doc-linking from the module comment even on non-Windows builds
// (e.g. `cargo doc` on this macOS host), without pulling in windows-rs.
#[cfg(not(windows))]
#[allow(dead_code)]
fn _unused(_: &DisplaySpec) {}
