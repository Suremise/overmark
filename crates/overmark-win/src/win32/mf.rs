//! Media Foundation: H.264 into an MP4 through `IMFSinkWriter`, and reading an MP4 back.
//!
//! **Loaded at run time.** Windows N and KN editions have no Media Foundation unless the Media
//! Feature Pack is installed. The `windows` crate links each function into the exe's import
//! table, so an exe that called `MFStartup` directly wouldn't start at all there. The few free
//! functions used are found with `GetProcAddress` instead ([`available`]); everything else is
//! COM methods, which need no import. Overmark starts on every edition, and only recording
//! says it can't.
//!
//! **The encoder** is Windows' own H.264 MFT: the GPU's (Intel Quick Sync, NVIDIA NVENC, AMD
//! VCE) when the sink writer may use hardware transforms and one takes the format, else
//! Microsoft's software encoder ([`Mp4Writer::create`] falls back by itself).

use std::ffi::c_void;
use std::path::Path;
use std::sync::OnceLock;

use windows::Win32::Foundation::{E_FAIL, HMODULE};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::core::{GUID, HRESULT, HSTRING, Interface, PCSTR, PCWSTR, s, w};

/// What `GetProcAddress` gives.
type Proc = unsafe extern "system" fn() -> isize;
type Startup = unsafe extern "system" fn(u32, u32) -> HRESULT;
type Shutdown = unsafe extern "system" fn() -> HRESULT;
type CreateAttributes = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
type CreateObject = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type CreateMemoryBuffer = unsafe extern "system" fn(u32, *mut *mut c_void) -> HRESULT;
type CreateSinkWriterFromUrl =
    unsafe extern "system" fn(PCWSTR, *mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;
type CreateSourceReaderFromUrl =
    unsafe extern "system" fn(PCWSTR, *mut c_void, *mut *mut c_void) -> HRESULT;

/// The Media Foundation functions Overmark calls, from `mfplat.dll` and `mfreadwrite.dll`.
struct Api {
    startup: Startup,
    shutdown: Shutdown,
    create_attributes: CreateAttributes,
    create_media_type: CreateObject,
    create_sample: CreateObject,
    create_memory_buffer: CreateMemoryBuffer,
    create_sink_writer: CreateSinkWriterFromUrl,
    create_source_reader: CreateSourceReaderFromUrl,
}

fn load() -> Option<Api> {
    // SAFETY: loads two system DLLs from System32 only (never the exe's folder or the current
    // one), and takes functions from them by their documented names and signatures. The
    // libraries stay loaded for the process's life, so the pointers stay valid.
    unsafe {
        let plat = LoadLibraryExW(w!("mfplat.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
        let rw = LoadLibraryExW(w!("mfreadwrite.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
        fn get(module: HMODULE, name: PCSTR) -> Option<Proc> {
            // SAFETY: `name` is a NUL-terminated literal.
            unsafe { GetProcAddress(module, name) }
        }
        Some(Api {
            startup: std::mem::transmute::<Proc, Startup>(get(plat, s!("MFStartup"))?),
            shutdown: std::mem::transmute::<Proc, Shutdown>(get(plat, s!("MFShutdown"))?),
            create_attributes: std::mem::transmute::<Proc, CreateAttributes>(get(
                plat,
                s!("MFCreateAttributes"),
            )?),
            create_media_type: std::mem::transmute::<Proc, CreateObject>(get(
                plat,
                s!("MFCreateMediaType"),
            )?),
            create_sample: std::mem::transmute::<Proc, CreateObject>(get(
                plat,
                s!("MFCreateSample"),
            )?),
            create_memory_buffer: std::mem::transmute::<Proc, CreateMemoryBuffer>(get(
                plat,
                s!("MFCreateMemoryBuffer"),
            )?),
            create_sink_writer: std::mem::transmute::<Proc, CreateSinkWriterFromUrl>(get(
                rw,
                s!("MFCreateSinkWriterFromURL"),
            )?),
            create_source_reader: std::mem::transmute::<Proc, CreateSourceReaderFromUrl>(get(
                rw,
                s!("MFCreateSourceReaderFromURL"),
            )?),
        })
    }
}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(load).as_ref()
}

/// Whether Media Foundation is installed, with the functions Overmark needs.
pub fn available() -> bool {
    api().is_some()
}

/// Why Media Foundation can't be used.
#[derive(Debug, Clone)]
pub enum MfError {
    /// It isn't installed (see [`available`]).
    Missing,
    /// The encoder won't take the format: the size, usually.
    Refused(windows::core::Error),
    /// Anything else.
    Failed(windows::core::Error),
}

impl From<windows::core::Error> for MfError {
    fn from(e: windows::core::Error) -> Self {
        MfError::Failed(e)
    }
}

/// Media Foundation started on this thread's behalf; shut down when dropped. Starts nest, so
/// each user holds its own.
pub struct Session {
    api: &'static Api,
    /// Whether this session initialised COM on its thread, to undo in Drop.
    com: bool,
}

impl Session {
    /// Starts Media Foundation, and COM on this thread if it isn't yet (multithreaded; a
    /// thread already in an apartment keeps its own).
    pub fn start() -> Result<Session, MfError> {
        let api = api().ok_or(MfError::Missing)?;
        // SAFETY: documented calls with constants; balanced in Drop (CoUninitialize only if
        // this call initialised COM).
        unsafe {
            let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            if let Err(e) = (api.startup)(MF_VERSION, MFSTARTUP_LITE).ok() {
                if com {
                    CoUninitialize();
                }
                return Err(e.into());
            }
            Ok(Session { api, com })
        }
    }

    fn attributes(&self, size: u32) -> windows::core::Result<IMFAttributes> {
        let mut raw = std::ptr::null_mut();
        // SAFETY: MFCreateAttributes writes one owned interface pointer, taken over here.
        unsafe {
            (self.api.create_attributes)(&mut raw, size).ok()?;
            Ok(IMFAttributes::from_raw(raw))
        }
    }

    fn media_type(&self) -> windows::core::Result<IMFMediaType> {
        let mut raw = std::ptr::null_mut();
        // SAFETY: as `attributes`.
        unsafe {
            (self.api.create_media_type)(&mut raw).ok()?;
            Ok(IMFMediaType::from_raw(raw))
        }
    }

    fn sample(&self) -> windows::core::Result<IMFSample> {
        let mut raw = std::ptr::null_mut();
        // SAFETY: as `attributes`.
        unsafe {
            (self.api.create_sample)(&mut raw).ok()?;
            Ok(IMFSample::from_raw(raw))
        }
    }

    fn memory_buffer(&self, len: u32) -> windows::core::Result<IMFMediaBuffer> {
        let mut raw = std::ptr::null_mut();
        // SAFETY: as `attributes`.
        unsafe {
            (self.api.create_memory_buffer)(len, &mut raw).ok()?;
            Ok(IMFMediaBuffer::from_raw(raw))
        }
    }

    fn source_reader(
        &self,
        path: &Path,
        attributes: Option<&IMFAttributes>,
    ) -> windows::core::Result<IMFSourceReader> {
        let url = HSTRING::from(path.as_os_str());
        let mut raw = std::ptr::null_mut();
        // SAFETY: `url` outlives the call; the attributes are borrowed for it; the reader
        // written is owned and taken over.
        unsafe {
            (self.api.create_source_reader)(
                PCWSTR(url.as_ptr()),
                attributes.map_or(std::ptr::null_mut(), |a| a.as_raw()),
                &mut raw,
            )
            .ok()?;
            Ok(IMFSourceReader::from_raw(raw))
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: balances the MFStartup (and any CoInitializeEx) in `start`.
        unsafe {
            let _ = (self.api.shutdown)();
            if self.com {
                CoUninitialize();
            }
        }
    }
}

/// Two 32-bit values packed as Media Foundation's sizes and ratios are (`MFSetAttributeSize`).
const fn pack(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

/// An MP4's video, as [`Mp4Writer::create`] makes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFormat {
    /// Even, in pixels.
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Bits a second: the average, and the peak the encoder may reach for a moment.
    pub bitrate: u32,
    pub peak_bitrate: u32,
    /// Frames from one key frame to the next.
    pub gop: u32,
    pub rate_control: RateControl,
}

/// How the encoder spends its bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RateControl {
    /// A capped variable bitrate: about [`VideoFormat::bitrate`] on average, never over
    /// [`VideoFormat::peak_bitrate`]. What recordings use: a still screen costs next to
    /// nothing.
    #[default]
    PeakConstrainedVbr,
    /// A constant bitrate, for comparison: it spends the full rate on a still screen too.
    Cbr,
    /// Quality-based variable bitrate, 0 to 100 (`CODECAPI_AVEncCommonQuality`).
    Quality(u32),
}

/// The colour description both media types carry: BT.709 in limited range, as
/// `overmark_core::record::nv12` converts, so players show the colours as they were.
fn set_colour(t: &IMFMediaType) -> windows::core::Result<()> {
    // SAFETY: plain attribute writes on a media type we own.
    unsafe {
        t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
    }
    Ok(())
}

fn video_type(
    session: &Session,
    subtype: &GUID,
    f: &VideoFormat,
) -> windows::core::Result<IMFMediaType> {
    let t = session.media_type()?;
    // SAFETY: plain attribute writes on a media type we own.
    unsafe {
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, pack(f.width, f.height))?;
        t.SetUINT64(&MF_MT_FRAME_RATE, pack(f.fps, 1))?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
    }
    set_colour(&t)?;
    Ok(t)
}

/// Which encoder a writer got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderInfo {
    pub hardware: bool,
    /// Its name as Windows gives it ("NVIDIA H.264 Encoder MFT"), if it gives one.
    pub name: String,
}

/// An MP4 being written: H.264 video, no audio.
pub struct Mp4Writer {
    writer: IMFSinkWriter,
    stream: u32,
    frame_len: u32,
    pub encoder: EncoderInfo,
    /// Last, so Media Foundation shuts down after the writer is released (fields drop in
    /// order).
    session: Session,
}

/// One NV12 frame handed to the encoder. A frame that repeats is written again from the same
/// buffer: no copy, no conversion.
#[derive(Clone)]
pub struct FrameBuffer(IMFMediaBuffer);

impl Mp4Writer {
    /// Starts writing `path` (replacing it). With `hardware`, the GPU's encoder is tried first;
    /// if it can't be set up the software encoder is used. An encoder that won't take the
    /// size gives [`MfError::Refused`].
    pub fn create(path: &Path, format: &VideoFormat, hardware: bool) -> Result<Mp4Writer, MfError> {
        if hardware {
            match Self::create_with(path, format, true) {
                Ok(w) => return Ok(w),
                Err(MfError::Missing) => return Err(MfError::Missing),
                Err(e) => {
                    tracing::info!(
                        ?e,
                        "the hardware H.264 encoder can't be used; trying software"
                    );
                }
            }
        }
        Self::create_with(path, format, false)
    }

    fn create_with(path: &Path, f: &VideoFormat, hardware: bool) -> Result<Mp4Writer, MfError> {
        let session = Session::start()?;
        let attrs = session.attributes(3)?;
        // SAFETY: attribute writes on an attribute store we own.
        unsafe {
            attrs.SetUINT32(
                &MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
                u32::from(hardware),
            )?;
            attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
        }
        let url = HSTRING::from(path.as_os_str());
        let mut raw = std::ptr::null_mut();
        // SAFETY: `url` and `attrs` outlive the call; the writer written is owned.
        let writer = unsafe {
            (session.api.create_sink_writer)(
                PCWSTR(url.as_ptr()),
                std::ptr::null_mut(),
                attrs.as_raw(),
                &mut raw,
            )
            .ok()?;
            IMFSinkWriter::from_raw(raw)
        };

        let out = video_type(&session, &MFVideoFormat_H264, f)?;
        let input = video_type(&session, &MFVideoFormat_NV12, f)?;
        // The encoder's rate control: a capped variable bitrate. A constant bitrate would pad a
        // still screen to the full rate; this spends almost nothing on frames that repeat and
        // keeps busy moments under the peak (docs/ARCHITECTURE.md, Recording).
        let params = session.attributes(5)?;
        // SAFETY: attribute writes on types and stores we own; the writer's calls take them
        // by reference for the call.
        unsafe {
            out.SetUINT32(&MF_MT_AVG_BITRATE, f.bitrate)?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, f.width)?;
            let mode = match f.rate_control {
                RateControl::PeakConstrainedVbr => eAVEncCommonRateControlMode_PeakConstrainedVBR,
                RateControl::Cbr => eAVEncCommonRateControlMode_CBR,
                RateControl::Quality(q) => {
                    params.SetUINT32(&CODECAPI_AVEncCommonQuality, q)?;
                    eAVEncCommonRateControlMode_Quality
                }
            };
            params.SetUINT32(&CODECAPI_AVEncCommonRateControlMode, mode.0 as u32)?;
            params.SetUINT32(&CODECAPI_AVEncCommonMeanBitRate, f.bitrate)?;
            params.SetUINT32(&CODECAPI_AVEncCommonMaxBitRate, f.peak_bitrate)?;
            params.SetUINT32(&CODECAPI_AVEncMPVGOPSize, f.gop)?;
            params.SetUINT32(&CODECAPI_AVEncMPVDefaultBPictureCount, 0)?;

            let stream = writer.AddStream(&out).map_err(MfError::Refused)?;
            writer
                .SetInputMediaType(stream, &input, &params)
                .map_err(MfError::Refused)?;
            writer.BeginWriting().map_err(MfError::Refused)?;
            let encoder = encoder_info(&writer, stream, hardware);
            tracing::info!(
                width = f.width,
                height = f.height,
                fps = f.fps,
                bitrate = f.bitrate,
                hardware = encoder.hardware,
                name = %encoder.name,
                "recording: the encoder is ready"
            );
            Ok(Mp4Writer {
                session,
                writer,
                stream,
                frame_len: f.width * f.height * 3 / 2,
                encoder,
            })
        }
    }

    /// A buffer holding `nv12`, one frame's worth (`width * height * 3 / 2` bytes).
    pub fn frame(&self, nv12: &[u8]) -> windows::core::Result<FrameBuffer> {
        if nv12.len() != self.frame_len as usize {
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_INVALIDARG,
                "an NV12 frame of the wrong size",
            ));
        }
        let buffer = self.session.memory_buffer(self.frame_len)?;
        // SAFETY: the buffer is `frame_len` bytes; it is locked, filled with exactly that many
        // and unlocked before anything else sees it.
        unsafe {
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            std::ptr::copy_nonoverlapping(nv12.as_ptr(), data, nv12.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(self.frame_len)?;
        }
        Ok(FrameBuffer(buffer))
    }

    /// Writes `frame` at `time` for `duration` (100 ns units from the start).
    pub fn write(
        &mut self,
        frame: &FrameBuffer,
        time: i64,
        duration: i64,
    ) -> windows::core::Result<()> {
        let sample = self.session.sample()?;
        // SAFETY: a sample we own, given a buffer that is never written again, then handed to
        // the writer, which keeps its own reference.
        unsafe {
            sample.AddBuffer(&frame.0)?;
            sample.SetSampleTime(time)?;
            sample.SetSampleDuration(duration)?;
            self.writer.WriteSample(self.stream, &sample)
        }
    }

    /// Finishes the file: the encoder's last frames and the MP4's index. A file not finished
    /// doesn't play.
    pub fn finish(self) -> windows::core::Result<()> {
        // SAFETY: the writer's documented last call.
        unsafe { self.writer.Finalize() }
        // `self` drops here: the writer, then the session.
    }
}

/// The encoder the writer loaded for `stream`.
fn encoder_info(writer: &IMFSinkWriter, stream: u32, hardware_allowed: bool) -> EncoderInfo {
    let unknown = EncoderInfo {
        hardware: false,
        name: String::new(),
    };
    let Ok(ex) = writer.cast::<IMFSinkWriterEx>() else {
        return unknown;
    };
    for index in 0.. {
        let mut category = GUID::zeroed();
        let mut transform: Option<IMFTransform> = None;
        // SAFETY: out-parameters we own; the loop ends at the first index with no transform.
        let got =
            unsafe { ex.GetTransformForStream(stream, index, Some(&mut category), &mut transform) };
        let Some(transform) = got.ok().and(transform) else {
            break;
        };
        if category != MFT_CATEGORY_VIDEO_ENCODER {
            continue;
        }
        // SAFETY: plain attribute reads on the transform's own store.
        let attrs = unsafe { transform.GetAttributes() }.ok();
        let hardware = hardware_allowed
            && attrs.as_ref().is_some_and(|a| {
                // SAFETY: as above.
                unsafe { a.GetStringLength(&MFT_ENUM_HARDWARE_URL_Attribute) }.is_ok()
            });
        let name = attrs
            .as_ref()
            .and_then(|a| string(a, &MFT_FRIENDLY_NAME_Attribute))
            .unwrap_or_default();
        return EncoderInfo { hardware, name };
    }
    unknown
}

fn string(attrs: &IMFAttributes, key: &GUID) -> Option<String> {
    // SAFETY: the buffer is the length the store reports plus its NUL.
    unsafe {
        let len = attrs.GetStringLength(key).ok()?;
        let mut buf = vec![0u16; len as usize + 1];
        attrs.GetString(key, &mut buf, None).ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// What an MP4's video is, read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mp4Info {
    pub width: u32,
    pub height: u32,
    /// Whether the video is H.264.
    pub h264: bool,
    pub frames: u64,
    /// From the first frame's start to the last one's end, in 100 ns units.
    pub duration_100ns: i64,
    /// The key frames among them.
    pub key_frames: u64,
}

/// Reads `path`'s video stream sample by sample, without decoding it.
pub fn probe(path: &Path) -> Result<Mp4Info, MfError> {
    let session = Session::start()?;
    let reader = session.source_reader(path, None)?;
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    // SAFETY: plain calls on the reader we own; every out-parameter is ours.
    unsafe {
        let native = reader.GetNativeMediaType(stream, 0)?;
        let size = native.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let h264 = native.GetGUID(&MF_MT_SUBTYPE)? == MFVideoFormat_H264;
        let (mut frames, mut key_frames, mut end) = (0u64, 0u64, 0i64);
        let mut first: Option<i64> = None;
        loop {
            let mut flags = 0u32;
            let mut time = 0i64;
            let mut sample: Option<IMFSample> = None;
            reader.ReadSample(
                stream,
                0,
                None,
                Some(&mut flags),
                Some(&mut time),
                Some(&mut sample),
            )?;
            if let Some(sample) = sample {
                frames += 1;
                let duration = sample.GetSampleDuration().unwrap_or(0);
                first.get_or_insert(time);
                end = end.max(time + duration);
                if sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0 {
                    key_frames += 1;
                }
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
        }
        Ok(Mp4Info {
            width: (size >> 32) as u32,
            height: size as u32,
            h264,
            frames,
            duration_100ns: end - first.unwrap_or(0),
            key_frames,
        })
    }
}

/// The frame shown at `at` (100 ns from the start), decoded to BGRA: `(width, height,
/// pixels)`, rows top first. For checks and tests.
pub fn decode_frame_at(path: &Path, at: i64) -> Result<(u32, u32, Vec<u8>), MfError> {
    let session = Session::start()?;
    let attrs = session.attributes(1)?;
    // SAFETY: an attribute write on a store we own.
    unsafe { attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)? };
    let reader = session.source_reader(path, Some(&attrs))?;
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let want = session.media_type()?;
    // SAFETY: plain calls on the reader and types we own; the buffer is locked only while its
    // rows are copied, within its length.
    unsafe {
        want.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        want.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        reader.SetCurrentMediaType(stream, None, &want)?;
        let current = reader.GetCurrentMediaType(stream)?;
        let size = current.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (w, h) = ((size >> 32) as u32, size as u32);
        let stride = current
            .GetUINT32(&MF_MT_DEFAULT_STRIDE)
            .map(|s| s as i32)
            .unwrap_or(w as i32 * 4);
        let mut last: Option<IMFSample> = None;
        loop {
            let mut flags = 0u32;
            let mut time = 0i64;
            let mut sample: Option<IMFSample> = None;
            reader.ReadSample(
                stream,
                0,
                None,
                Some(&mut flags),
                Some(&mut time),
                Some(&mut sample),
            )?;
            if let Some(s) = sample {
                if time > at && last.is_some() {
                    break;
                }
                last = Some(s);
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
        }
        let sample = last.ok_or_else(|| windows::core::Error::new(E_FAIL, "no frames"))?;
        // The sample's own buffer is a 2-D one that knows its pitch; a contiguous copy isn't.
        let buffer = match sample.GetBufferCount() {
            Ok(1) => sample.GetBufferByIndex(0)?,
            _ => sample.ConvertToContiguousBuffer()?,
        };
        let row = w as usize * 4;
        let mut out = Vec::with_capacity(row * h as usize);
        // The decoder pads its rows (a width that isn't a multiple of 16): a 2-D buffer says
        // its real pitch, negative for a bottom-up image; otherwise the media type's stride.
        if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
            let mut scan0 = std::ptr::null_mut();
            let mut pitch = 0i32;
            two_d.Lock2D(&mut scan0, &mut pitch)?;
            for y in 0..h as isize {
                let start = scan0.offset(y * pitch as isize);
                out.extend_from_slice(std::slice::from_raw_parts(start, row));
            }
            two_d.Unlock2D()?;
        } else {
            let mut data = std::ptr::null_mut();
            let mut len = 0u32;
            buffer.Lock(&mut data, None, Some(&mut len))?;
            // Without a pitch: the decoder pads its frame to whole 16-pixel blocks (680 x 162
            // comes as 688 x 176), so a row is the width rounded up to 16 when the buffer is
            // that big (measured on Windows 11's H.264 decoder and video processor).
            let padded = (w as usize).div_ceil(16) * 16 * 4;
            let stride =
                if stride.unsigned_abs() as usize == row && len as usize >= padded * h as usize {
                    if stride < 0 {
                        -(padded as i32)
                    } else {
                        padded as i32
                    }
                } else {
                    stride
                };
            for y in 0..h as usize {
                // A negative stride is a bottom-up image: its first row in memory is the last.
                let src_row = if stride < 0 { h as usize - 1 - y } else { y };
                let start = src_row * stride.unsigned_abs() as usize;
                if start + row > len as usize {
                    break;
                }
                out.extend_from_slice(std::slice::from_raw_parts(data.add(start), row));
            }
            buffer.Unlock()?;
        }
        Ok((w, h, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_puts_the_first_value_high() {
        assert_eq!(pack(1920, 1080), (1920u64 << 32) | 1080);
        assert_eq!(pack(15, 1), 0x0000_000F_0000_0001);
    }

    #[test]
    fn media_foundation_loads_on_this_pc() {
        // Every edition CI and contributors use has it; N editions are the exception the
        // runtime loading is for.
        assert!(available());
    }
}
