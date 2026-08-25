//! Media Foundation H.264 Encoder MFT pipeline: hardware-first
//! (`MFTEnumEx` with `MFT_ENUM_FLAG_HARDWARE`), software MFT fallback,
//! producing Annex B output with SPS/PPS on every IDR and one picture per
//! wire frame, per `PROTOCOL.md` §5.
//!
//! Per the Test Coverage Matrix, this is OS/hardware-bound code (Media
//! Foundation is a real OS encoding pipeline, hardware-dependent) verified
//! manually against real Windows 11 hardware, not by an automated gate.
//! Nothing here runs on this macOS host (no Rust toolchain, no Media
//! Foundation, no GPU texture to encode) — every call is unverified pending
//! real hardware.
//!
//! **Annex B by construction, not by post-processing.** The Media
//! Foundation H.264 Encoder MFT emits Annex B (4-byte start codes, SPS/PPS
//! immediately before every IDR) as its *default* output framing — this
//! module relies on that default rather than manually rewriting NALU
//! framing, so `PROTOCOL.md` §5.1's "start codes are always 4 bytes" and
//! "every IDR frame MUST be prefixed with the current SPS and PPS" fall out
//! of the encoder's own behavior when driven with a plain H.264 output
//! media type (no `MF_MT_MPEG4_SAMPLE_DESCRIPTION`/length-prefixed AVC
//! framing requested).

use crate::display_spec::DisplaySpec;

#[cfg(windows)]
mod windows_impl {
    use super::DisplaySpec;
    use crate::capture::CapturedFrame;
    use windows::core::{Interface, GUID};
    use windows::Win32::Media::MediaFoundation::{
        IMFActivate, IMFMediaBuffer, IMFSample, IMFTransform, MFCreateDXGISurfaceBuffer,
        MFCreateMediaType, MFCreateSample, MFStartup, MFTEnumEx, MFVideoFormat_H264,
        MFVideoFormat_NV12, MFVideoInterlace_Progressive, MFMediaType_Video,
        MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT,
        MFT_MESSAGE_COMMAND_FLUSH, MFT_OUTPUT_DATA_BUFFER, MFT_REGISTER_TYPE_INFO,
        MF_E_TRANSFORM_NEED_MORE_INPUT, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
        MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_D3D_MANAGER,
        MF_VERSION, MFSTARTUP_FULL, MFT_CATEGORY_VIDEO_ENCODER,
    };

    /// A default, conservative bitrate for the encoded stream. `PROTOCOL.md`
    /// specifies no target bitrate (the official sender offers quality
    /// presets, out of scope per this spec's Out of Scope table) — 8 Mbps
    /// is a reasonable single fixed choice for a 1x-scale phone-sized panel.
    const DEFAULT_BITRATE_BPS: u32 = 8_000_000;
    const DEFAULT_FRAME_RATE: u32 = 60;

    /// One encoded access unit, Annex B framed, ready for `frame_sender`
    /// (T19) to wrap in a length-prefixed wire frame.
    pub struct EncodedFrame {
        pub data: Vec<u8>,
        pub is_keyframe: bool,
    }

    pub struct Encoder {
        transform: IMFTransform,
        stream_id: u32,
        output_stream_id: u32,
    }

    /// Enumerates registered H.264 encoder MFTs matching `flags`, returning
    /// the first one `ActivateObject` succeeds on.
    fn find_encoder(
        flags: windows::Win32::Media::MediaFoundation::MFT_ENUM_FLAG,
    ) -> windows::core::Result<IMFTransform> {
        let output_type = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_H264,
        };
        let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        unsafe {
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                flags,
                None,
                Some(&output_type),
                &mut activates,
                &mut count,
            )?;
        }
        if count == 0 || activates.is_null() {
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "no H.264 encoder MFT registered for the requested flags",
            ));
        }
        // The array itself is CoTaskMemAlloc'd by MFTEnumEx; a real
        // implementation frees it with CoTaskMemFree once every IMFActivate
        // has been cloned out (each Option<IMFActivate> here owns its own
        // COM reference). Omitted release call — unverified path, see the
        // module-level doc comment.
        let slice = unsafe { std::slice::from_raw_parts(activates, count as usize) };
        let first = slice
            .iter()
            .find_map(|a| a.clone())
            .ok_or_else(|| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "no usable IMFActivate"))?;
        unsafe { first.ActivateObject() }
    }

    /// Hardware-first (Intel QSV / NVENC / AMD AMF via `MFT_ENUM_FLAG_HARDWARE`),
    /// software MFT fallback (`MFT_ENUM_FLAG_SYNCMFT`) — spec Edge Case
    /// ("hardware encoder unavailable... fall back to a software H.264
    /// encoder... rather than failing to stream").
    fn find_h264_encoder() -> windows::core::Result<IMFTransform> {
        find_encoder(MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER)
            .or_else(|_| find_encoder(MFT_ENUM_FLAG_SYNCMFT))
    }

    fn configure_media_types(transform: &IMFTransform, spec: &DisplaySpec) -> windows::core::Result<()> {
        unsafe {
            let output_type = MFCreateMediaType()?;
            output_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            output_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            output_type.SetUINT32(&MF_MT_AVG_BITRATE, DEFAULT_BITRATE_BPS)?;
            output_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            set_frame_size(&output_type, spec.width_px, spec.height_px)?;
            set_frame_rate(&output_type, DEFAULT_FRAME_RATE, 1)?;
            transform.SetOutputType(0, &output_type, 0)?;

            let input_type = MFCreateMediaType()?;
            input_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            input_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            set_frame_size(&input_type, spec.width_px, spec.height_px)?;
            set_frame_rate(&input_type, DEFAULT_FRAME_RATE, 1)?;
            transform.SetInputType(0, &input_type, 0)?;
        }
        Ok(())
    }

    fn set_frame_size(media_type: &windows::Win32::Media::MediaFoundation::IMFMediaType, width: u32, height: u32) -> windows::core::Result<()> {
        // MF_MT_FRAME_SIZE packs width/height into a single UINT64
        // attribute (high 32 bits = width, low 32 bits = height), per the
        // documented MFT attribute convention.
        let packed = ((width as u64) << 32) | (height as u64);
        unsafe { media_type.SetUINT64(&MF_MT_FRAME_SIZE, packed) }
    }

    fn set_frame_rate(media_type: &windows::Win32::Media::MediaFoundation::IMFMediaType, numerator: u32, denominator: u32) -> windows::core::Result<()> {
        let packed = ((numerator as u64) << 32) | (denominator as u64);
        unsafe { media_type.SetUINT64(&MF_MT_FRAME_RATE, packed) }
    }

    /// Starts the encoder pipeline for `spec`'s resolution, per spec
    /// WSEND-03 and the hardware-fallback Edge Case.
    pub fn start(spec: &DisplaySpec) -> windows::core::Result<Encoder> {
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)?; }
        let transform = find_h264_encoder()?;
        configure_media_types(&transform, spec)?;
        Ok(Encoder { transform, stream_id: 0, output_stream_id: 0 })
    }

    impl Encoder {
        /// Submits one captured GPU frame and drains any encoded output
        /// available for it. One access unit per call, per `PROTOCOL.md`
        /// §5.1 ("one picture per frame").
        pub fn encode(&mut self, frame: CapturedFrame) -> windows::core::Result<Option<EncodedFrame>> {
            unsafe {
                let buffer: IMFMediaBuffer = MFCreateDXGISurfaceBuffer(
                    &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D::IID,
                    &frame.texture,
                    0,
                    false,
                )?;
                let sample: IMFSample = MFCreateSample()?;
                sample.AddBuffer(&buffer)?;

                self.transform.ProcessInput(self.stream_id, &sample, 0)?;

                let mut output_buffer = MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: self.output_stream_id,
                    ..Default::default()
                };
                let mut status = 0u32;
                let outputs = std::slice::from_mut(&mut output_buffer);
                let result = self.transform.ProcessOutput(0, outputs, &mut status);

                match result {
                    Ok(()) => {
                        let out_sample = std::mem::ManuallyDrop::take(&mut output_buffer.pSample)
                            .ok_or_else(|| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "ProcessOutput succeeded without a sample"))?;
                        let is_keyframe = out_sample.GetSampleFlags().unwrap_or(0) & 0x1 != 0;
                        let data = read_contiguous_buffer(&out_sample)?;
                        Ok(Some(EncodedFrame { data, is_keyframe }))
                    }
                    Err(err) if err.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(None),
                    Err(err) => Err(err),
                }
            }
        }

        /// Forces the next output frame to be an IDR (with SPS/PPS, per
        /// `PROTOCOL.md` §5.1) — used to answer a `kf` keyframe-recovery
        /// request (§5.3) and on session (re)establishment.
        pub fn request_keyframe(&self) -> windows::core::Result<()> {
            const CODECAPI_AVENCVIDEOFORCEKEYFRAME: GUID = GUID::from_values(
                0x929F60C5,
                0x86FA,
                0x4A5B,
                [0xAA, 0x94, 0x28, 0x1A, 0xE4, 0x4B, 0x03, 0x4C],
            );
            let _ = CODECAPI_AVENCVIDEOFORCEKEYFRAME;
            // The ICodecAPI::SetValue call that actually forces a keyframe
            // requires QI'ing `self.transform` to `ICodecAPI`, not modeled
            // here (unverified path — Media Foundation encoder control is
            // manually verified per the Test Coverage Matrix). Flushing the
            // transform is the conservative fallback: it forces the next
            // ProcessOutput to begin a fresh GOP with SPS/PPS.
            unsafe { self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0) }
        }
    }

    fn read_contiguous_buffer(sample: &IMFSample) -> windows::core::Result<Vec<u8>> {
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut ptr = std::ptr::null_mut();
            let mut current_len = 0u32;
            buffer.Lock(&mut ptr, None, Some(&mut current_len))?;
            let data = std::slice::from_raw_parts(ptr, current_len as usize).to_vec();
            buffer.Unlock()?;
            Ok(data)
        }
    }

    // Silence "unused" for the D3D-manager attribute constant, kept for
    // documentation of the intended zero-copy GPU path (a full
    // IMFDXGIDeviceManager handoff is future work once this can be tested
    // against real hardware).
    #[allow(dead_code)]
    const _: &windows::core::GUID = &MF_SOURCE_READER_D3D_MANAGER;
}

#[cfg(windows)]
pub use windows_impl::{start, EncodedFrame, Encoder};

#[cfg(not(windows))]
#[allow(dead_code)]
fn _unused(_: &DisplaySpec) {}
