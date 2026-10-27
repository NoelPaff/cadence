use crate::audio::resampler::{mix_to_stereo_frames, StereoResampler};
use crate::audio::sink::ENGINE_SAMPLE_RATE;
use anyhow::{anyhow, Result};
#[cfg(unix)]
use libloading::Library;
#[cfg(unix)]
use std::ffi::{c_char, c_int, c_longlong, c_void, CString};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Unified streaming audio decoder outputting interleaved stereo `f32` at `ENGINE_SAMPLE_RATE` (48 kHz).
pub trait TrackDecoder: Send {
    fn duration_ms(&self) -> u64;
    /// Appends decoded 48 kHz stereo `f32` samples to `out`.
    /// Returns `Ok(true)` if stream is still active, `Ok(false)` on EOF.
    fn decode_next(&mut self, out: &mut Vec<f32>) -> Result<bool>;
    /// Seeks to `target_ms` and returns the new position in milliseconds.
    fn seek_ms(&mut self, target_ms: u64) -> Result<u64>;
}

/// Opens a 3-tier audio decoder for `path`:
/// 1. Native Opus via `libopusfile.so.0` (for `.opus` files)
/// 2. Pure-Rust `Symphonia` (MP3, FLAC, Ogg Vorbis, M4A/AAC, ALAC, WAV, AIFF, CAF, MKV)
/// 3. `ffmpeg` raw PCM pipe fallback (for WMA, APE, WavPack, DSD, or exotic containers)
pub fn open_decoder(path: &Path) -> Result<Box<dyn TrackDecoder>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if ext == "opus" {
        #[cfg(unix)]
        if let Ok(opus) = OpusFileDecoder::try_open(path) {
            return Ok(Box::new(opus));
        }
    }

    match SymphoniaDecoder::try_open(path) {
        Ok(dec) => Ok(Box::new(dec)),
        Err(sym_err) => {
            #[cfg(unix)]
            if let Ok(opus) = OpusFileDecoder::try_open(path) {
                return Ok(Box::new(opus));
            }
            if let Ok(ff) = FfmpegDecoder::try_open(path) {
                return Ok(Box::new(ff));
            }
            Err(sym_err)
        }
    }
}

// ============================================================================
// Tier 1: Pure-Rust Symphonia Decoder (with Gapless Trim & Resampler)
// ============================================================================

struct SymphoniaDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    duration_ms: u64,
    resampler: StereoResampler,
    sample_buf: Option<SampleBuffer<f32>>,
    stereo_scratch: Vec<[f32; 2]>,
}

impl SymphoniaDecoder {
    fn try_open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &format_opts,
            &MetadataOptions::default(),
        )?;

        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| anyhow!("No supported audio stream found"))?;

        let track_id = track.id;
        let sample_rate = track.codec_params.sample_rate.unwrap_or(ENGINE_SAMPLE_RATE);
        let duration_ms = track
            .codec_params
            .n_frames
            .map(|nf| (nf.saturating_mul(1000)) / (sample_rate.max(1) as u64))
            .unwrap_or(0);

        let decoder =
            symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

        Ok(Self {
            format,
            decoder,
            track_id,
            duration_ms,
            resampler: StereoResampler::new(sample_rate, ENGINE_SAMPLE_RATE),
            sample_buf: None,
            stereo_scratch: Vec::with_capacity(2048),
        })
    }
}

impl TrackDecoder for SymphoniaDecoder {
    fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    fn decode_next(&mut self, out: &mut Vec<f32>) -> Result<bool> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(pkt) => pkt,
                Err(SymphoniaError::IoError(ref io_err))
                    if io_err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    self.resampler.flush_eof(out);
                    return Ok(false);
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(anyhow!("Container read error: {}", e)),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    let capacity = decoded.capacity() as u64;
                    let channels = spec.channels.count().max(1);
                    self.resampler.set_input_rate(spec.rate);

                    let needs_realloc = self
                        .sample_buf
                        .as_ref()
                        .map(|sb| sb.capacity() < decoded.capacity())
                        .unwrap_or(true);
                    if needs_realloc {
                        self.sample_buf = Some(SampleBuffer::<f32>::new(capacity, spec));
                    }

                    let sb = self.sample_buf.as_mut().unwrap();
                    sb.copy_interleaved_ref(decoded);
                    let samples = sb.samples();
                    if samples.is_empty() {
                        continue;
                    }
                    mix_to_stereo_frames(samples, channels, &mut self.stereo_scratch);
                    self.resampler.process(&self.stereo_scratch, out);
                    return Ok(true);
                }
                Err(SymphoniaError::DecodeError(_)) => {
                    // Skip single corrupted frame cleanly
                    continue;
                }
                Err(SymphoniaError::IoError(ref io_err))
                    if io_err.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    self.resampler.flush_eof(out);
                    return Ok(false);
                }
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(anyhow!("Decode error: {}", e)),
            }
        }
    }

    fn seek_ms(&mut self, target_ms: u64) -> Result<u64> {
        let clamped_ms = if self.duration_ms > 150 {
            target_ms.min(self.duration_ms - 100)
        } else {
            target_ms
        };

        let secs = clamped_ms / 1000;
        let frac = (clamped_ms % 1000) as f64 / 1000.0;
        let make_seek = || SeekTo::Time {
            time: Time::new(secs, frac),
            track_id: Some(self.track_id),
        };

        match self.format.seek(SeekMode::Accurate, make_seek()) {
            Ok(seeked) => {
                self.decoder.reset();
                self.resampler.reset();
                let rate = self.resampler.in_rate().max(1) as u64;
                let actual_ms = (seeked.actual_ts.saturating_mul(1000)) / rate;
                Ok(actual_ms)
            }
            Err(_) => {
                // Retry with Coarse seek for headerless VBR streams
                let seeked = self.format.seek(SeekMode::Coarse, make_seek())?;
                self.decoder.reset();
                self.resampler.reset();
                let rate = self.resampler.in_rate().max(1) as u64;
                Ok((seeked.actual_ts.saturating_mul(1000)) / rate)
            }
        }
    }
}

// ============================================================================
// Tier 2: Dynamic Native Opus Decoder (`libopusfile` on Unix/Linux/macOS)
// ============================================================================

#[cfg(unix)]
type FnOpOpenFile = unsafe extern "C" fn(path: *const c_char, error: *mut c_int) -> *mut c_void;
#[cfg(unix)]
type FnOpPcmTotal = unsafe extern "C" fn(of: *const c_void, li: c_int) -> c_longlong;
#[cfg(unix)]
type FnOpReadFloatStereo =
    unsafe extern "C" fn(of: *mut c_void, pcm: *mut f32, buf_size: c_int) -> c_int;
#[cfg(unix)]
type FnOpPcmSeek = unsafe extern "C" fn(of: *mut c_void, offset: c_longlong) -> c_int;
#[cfg(unix)]
type FnOpFree = unsafe extern "C" fn(of: *mut c_void);

#[cfg(unix)]
struct OpusFileDecoder {
    of: *mut c_void,
    duration_ms: u64,
    scratch: Vec<f32>,
    fn_read_stereo: FnOpReadFloatStereo,
    fn_seek: FnOpPcmSeek,
    fn_free: FnOpFree,
    _lib: Library,
}

#[cfg(unix)]
unsafe impl Send for OpusFileDecoder {}

#[cfg(unix)]
impl OpusFileDecoder {
    fn try_open(path: &Path) -> Result<Self> {
        let candidates = [
            "libopusfile.so.0",
            "/usr/lib64/libopusfile.so.0",
            "/usr/lib/libopusfile.so.0",
            "/usr/lib/x86_64-linux-gnu/libopusfile.so.0",
            "/usr/lib/aarch64-linux-gnu/libopusfile.so.0",
            "libopusfile.0.dylib",
            "/opt/homebrew/lib/libopusfile.0.dylib",
            "/usr/local/lib/libopusfile.0.dylib",
        ];
        let mut loaded = None;
        for c in candidates {
            if let Ok(l) = unsafe { Library::new(c) } {
                loaded = Some(l);
                break;
            }
        }
        let lib = loaded.ok_or_else(|| anyhow!("libopusfile not found"))?;

        let (fn_open, fn_total, fn_read_stereo, fn_seek, fn_free) = unsafe {
            (
                *lib.get::<FnOpOpenFile>(b"op_open_file\0")?,
                *lib.get::<FnOpPcmTotal>(b"op_pcm_total\0")?,
                *lib.get::<FnOpReadFloatStereo>(b"op_read_float_stereo\0")?,
                *lib.get::<FnOpPcmSeek>(b"op_pcm_seek\0")?,
                *lib.get::<FnOpFree>(b"op_free\0")?,
            )
        };

        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow!("Non-UTF8 path for opusfile"))?;
        let c_path = CString::new(path_str)?;
        let mut err: c_int = 0;
        let of = unsafe { fn_open(c_path.as_ptr(), &mut err) };
        if of.is_null() {
            return Err(anyhow!("op_open_file failed ({})", err));
        }

        let total_samples_48k = unsafe { fn_total(of, -1) };
        let duration_ms = if total_samples_48k > 0 {
            ((total_samples_48k as u64).saturating_mul(1000)) / 48_000
        } else {
            0
        };

        Ok(Self {
            of,
            duration_ms,
            scratch: vec![0.0f32; 4096],
            fn_read_stereo,
            fn_seek,
            fn_free,
            _lib: lib,
        })
    }
}

#[cfg(unix)]
impl TrackDecoder for OpusFileDecoder {
    fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    fn decode_next(&mut self, out: &mut Vec<f32>) -> Result<bool> {
        loop {
            let frames = unsafe {
                (self.fn_read_stereo)(
                    self.of,
                    self.scratch.as_mut_ptr(),
                    self.scratch.len() as c_int,
                )
            };
            if frames == 0 {
                return Ok(false);
            }
            if frames < 0 {
                // OP_HOLE (-3) is recoverable stream hole
                if frames == -3 {
                    continue;
                }
                return Err(anyhow!("op_read_float_stereo error {}", frames));
            }
            let sample_count = (frames as usize) * 2;
            out.extend_from_slice(&self.scratch[..sample_count]);
            return Ok(true);
        }
    }

    fn seek_ms(&mut self, target_ms: u64) -> Result<u64> {
        let clamped = if self.duration_ms > 50 {
            target_ms.min(self.duration_ms - 25)
        } else {
            target_ms
        };
        let pcm_offset = ((clamped * 48_000) / 1000) as c_longlong;
        let rc = unsafe { (self.fn_seek)(self.of, pcm_offset) };
        if rc != 0 {
            return Err(anyhow!("op_pcm_seek failed ({})", rc));
        }
        Ok(clamped)
    }
}

#[cfg(unix)]
impl Drop for OpusFileDecoder {
    fn drop(&mut self) {
        if !self.of.is_null() {
            unsafe {
                (self.fn_free)(self.of);
            }
            self.of = std::ptr::null_mut();
        }
    }
}

// ============================================================================
// Tier 3: Universal FFmpeg Pipe Decoder Fallback
// ============================================================================

struct FfmpegDecoder {
    path: PathBuf,
    child: Option<Child>,
    byte_buf: Vec<u8>,
    duration_ms: u64,
}

impl FfmpegDecoder {
    fn try_open(path: &Path) -> Result<Self> {
        let mut dec = Self {
            path: path.to_path_buf(),
            child: None,
            byte_buf: vec![0u8; 8192],
            duration_ms: 0,
        };
        dec.spawn_at_ms(0)?;
        Ok(dec)
    }

    fn spawn_at_ms(&mut self, pos_ms: u64) -> Result<()> {
        self.kill_child();
        let secs_arg = format!("{:.3}", pos_ms as f64 / 1000.0);
        let child = Command::new("ffmpeg")
            .args([
                "-v",
                "quiet",
                "-ss",
                &secs_arg,
                "-i",
            ])
            .arg(&self.path)
            .args([
                "-vn",
                "-f",
                "f32le",
                "-ac",
                "2",
                "-ar",
                "48000",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        self.child = Some(child);
        Ok(())
    }

    fn kill_child(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl TrackDecoder for FfmpegDecoder {
    fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    fn decode_next(&mut self, out: &mut Vec<f32>) -> Result<bool> {
        let Some(child) = self.child.as_mut() else {
            return Ok(false);
        };
        let Some(stdout) = child.stdout.as_mut() else {
            return Ok(false);
        };

        let mut filled = 0usize;
        while filled < self.byte_buf.len() {
            match stdout.read(&mut self.byte_buf[filled..]) {
                Ok(0) => break,
                Ok(n) => {
                    filled += n;
                    if filled % 8 == 0 && filled >= 2048 {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(anyhow!("ffmpeg pipe read error: {}", e)),
            }
        }

        let aligned = filled - (filled % 4);
        if aligned == 0 {
            return Ok(false);
        }
        out.reserve(aligned / 4);
        for chunk in self.byte_buf[..aligned].chunks_exact(4) {
            out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        Ok(true)
    }

    fn seek_ms(&mut self, target_ms: u64) -> Result<u64> {
        self.spawn_at_ms(target_ms)?;
        Ok(target_ms)
    }
}

impl Drop for FfmpegDecoder {
    fn drop(&mut self) {
        self.kill_child();
    }
}
