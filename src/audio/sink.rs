use anyhow::Result;
use std::thread;
use std::time::{Duration, Instant};

pub const ENGINE_SAMPLE_RATE: u32 = 48_000;
pub const ENGINE_CHANNELS: u8 = 2;

pub trait AudioSink: Send {
    fn backend_name(&self) -> &str;
    fn write_interleaved(&mut self, samples: &[f32]) -> Result<()>;
    fn flush(&mut self);
}

/// Opens the best available low-latency audio sink for the host platform:
/// - **Linux**: PulseAudio/PipeWire (`libpulse-simple.so.0`) -> ALSA (`libasound.so.2`) -> Timed Null Sink
/// - **macOS / Windows**: Native CoreAudio / WASAPI via `cpal` -> Timed Null Sink
pub fn open_best_sink(force_null: bool) -> Box<dyn AudioSink> {
    if force_null {
        return Box::new(NullSink::new("Null Audio (Timed)"));
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(pulse) = linux_sinks::PulseSimpleSink::try_open() {
            return Box::new(pulse);
        }
        if let Ok(alsa) = linux_sinks::AlsaSink::try_open() {
            return Box::new(alsa);
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        if let Ok(cpal_sink) = cpal_sink::CpalSink::try_open() {
            return Box::new(cpal_sink);
        }
    }

    Box::new(NullSink::new("Null Fallback (No Audio Device)"))
}

// ============================================================================
// Linux Dynamic PulseAudio/PipeWire & ALSA Sinks (Zero `-devel` headers needed)
// ============================================================================

#[cfg(target_os = "linux")]
mod linux_sinks {
    use super::{AudioSink, ENGINE_CHANNELS, ENGINE_SAMPLE_RATE};
    use anyhow::{anyhow, Result};
    use libloading::Library;
    use std::ffi::{c_char, c_int, c_long, c_uint, c_void, CString};
    use std::ptr;

    fn load_shared_lib(candidates: &[&str]) -> Result<Library> {
        let mut last_err = None;
        for name in candidates {
            match unsafe { Library::new(*name) } {
                Ok(lib) => return Ok(lib),
                Err(e) => last_err = Some(e),
            }
        }
        Err(anyhow!(
            "Failed to load shared library {:?}: {:?}",
            candidates,
            last_err
        ))
    }

    const PA_STREAM_PLAYBACK: c_int = 1;
    const PA_SAMPLE_FLOAT32LE: c_int = 5;

    #[repr(C)]
    struct PaSampleSpec {
        format: c_int,
        rate: u32,
        channels: u8,
    }

    #[repr(C)]
    struct PaBufferAttr {
        maxlength: u32,
        tlength: u32,
        prebuf: u32,
        minreq: u32,
        fragsize: u32,
    }

    type FnPaSimpleNew = unsafe extern "C" fn(
        server: *const c_char,
        name: *const c_char,
        dir: c_int,
        dev: *const c_char,
        stream_name: *const c_char,
        ss: *const PaSampleSpec,
        map: *const c_void,
        attr: *const PaBufferAttr,
        error: *mut c_int,
    ) -> *mut c_void;

    type FnPaSimpleWrite = unsafe extern "C" fn(
        s: *mut c_void,
        data: *const c_void,
        bytes: usize,
        error: *mut c_int,
    ) -> c_int;

    type FnPaSimpleFlush = unsafe extern "C" fn(s: *mut c_void, error: *mut c_int) -> c_int;
    type FnPaSimpleFree = unsafe extern "C" fn(s: *mut c_void);

    pub struct PulseSimpleSink {
        handle: *mut c_void,
        fn_write: FnPaSimpleWrite,
        fn_flush: FnPaSimpleFlush,
        fn_free: FnPaSimpleFree,
        _lib: Library,
    }

    unsafe impl Send for PulseSimpleSink {}

    impl PulseSimpleSink {
        pub fn try_open() -> Result<Self> {
            let lib = load_shared_lib(&[
                "libpulse-simple.so.0",
                "/usr/lib64/libpulse-simple.so.0",
                "/usr/lib/libpulse-simple.so.0",
                "/usr/lib/x86_64-linux-gnu/libpulse-simple.so.0",
                "/usr/lib/aarch64-linux-gnu/libpulse-simple.so.0",
            ])?;

            let (fn_new, fn_write, fn_flush, fn_free) = unsafe {
                let n = *lib.get::<FnPaSimpleNew>(b"pa_simple_new\0")?;
                let w = *lib.get::<FnPaSimpleWrite>(b"pa_simple_write\0")?;
                let fl = *lib.get::<FnPaSimpleFlush>(b"pa_simple_flush\0")?;
                let fr = *lib.get::<FnPaSimpleFree>(b"pa_simple_free\0")?;
                (n, w, fl, fr)
            };

            let app_name = CString::new("Cadence")?;
            let stream_name = CString::new("Music Playback")?;
            let spec = PaSampleSpec {
                format: PA_SAMPLE_FLOAT32LE,
                rate: ENGINE_SAMPLE_RATE,
                channels: ENGINE_CHANNELS,
            };
            let target_bytes = (ENGINE_SAMPLE_RATE * ENGINE_CHANNELS as u32 * 4) / 22;
            let attr = PaBufferAttr {
                maxlength: target_bytes * 4,
                tlength: target_bytes,
                prebuf: u32::MAX,
                minreq: u32::MAX,
                fragsize: u32::MAX,
            };

            let mut err: c_int = 0;
            let handle = unsafe {
                fn_new(
                    ptr::null(),
                    app_name.as_ptr(),
                    PA_STREAM_PLAYBACK,
                    ptr::null(),
                    stream_name.as_ptr(),
                    &spec,
                    ptr::null(),
                    &attr,
                    &mut err,
                )
            };
            if handle.is_null() {
                return Err(anyhow!("pa_simple_new failed (code {})", err));
            }

            Ok(Self {
                handle,
                fn_write,
                fn_flush,
                fn_free,
                _lib: lib,
            })
        }
    }

    impl AudioSink for PulseSimpleSink {
        fn backend_name(&self) -> &str {
            "PulseAudio / PipeWire (48.0 kHz f32le)"
        }

        fn write_interleaved(&mut self, samples: &[f32]) -> Result<()> {
            if samples.is_empty() {
                return Ok(());
            }
            let mut err: c_int = 0;
            let byte_len = std::mem::size_of_val(samples);
            let rc = unsafe {
                (self.fn_write)(
                    self.handle,
                    samples.as_ptr() as *const c_void,
                    byte_len,
                    &mut err,
                )
            };
            if rc < 0 {
                return Err(anyhow!("pa_simple_write error {}", err));
            }
            Ok(())
        }

        fn flush(&mut self) {
            let mut err: c_int = 0;
            unsafe {
                let _ = (self.fn_flush)(self.handle, &mut err);
            }
        }
    }

    impl Drop for PulseSimpleSink {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                let mut err: c_int = 0;
                unsafe {
                    let _ = (self.fn_flush)(self.handle, &mut err);
                    (self.fn_free)(self.handle);
                }
                self.handle = ptr::null_mut();
            }
        }
    }

    const SND_PCM_STREAM_PLAYBACK: c_int = 0;
    const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;
    const SND_PCM_FORMAT_FLOAT_LE: c_int = 14;

    type FnSndPcmOpen = unsafe extern "C" fn(
        pcm: *mut *mut c_void,
        name: *const c_char,
        stream: c_int,
        mode: c_int,
    ) -> c_int;
    type FnSndPcmSetParams = unsafe extern "C" fn(
        pcm: *mut c_void,
        format: c_int,
        access: c_int,
        channels: c_uint,
        rate: c_uint,
        soft_resample: c_int,
        latency_us: c_uint,
    ) -> c_int;
    type FnSndPcmWritei =
        unsafe extern "C" fn(pcm: *mut c_void, buffer: *const c_void, size: usize) -> c_long;
    type FnSndPcmRecover =
        unsafe extern "C" fn(pcm: *mut c_void, err: c_int, silent: c_int) -> c_int;
    type FnSndPcmDrop = unsafe extern "C" fn(pcm: *mut c_void) -> c_int;
    type FnSndPcmPrepare = unsafe extern "C" fn(pcm: *mut c_void) -> c_int;
    type FnSndPcmClose = unsafe extern "C" fn(pcm: *mut c_void) -> c_int;

    pub struct AlsaSink {
        pcm: *mut c_void,
        fn_writei: FnSndPcmWritei,
        fn_recover: FnSndPcmRecover,
        fn_drop: FnSndPcmDrop,
        fn_prepare: FnSndPcmPrepare,
        fn_close: FnSndPcmClose,
        _lib: Library,
    }

    unsafe impl Send for AlsaSink {}

    impl AlsaSink {
        pub fn try_open() -> Result<Self> {
            let lib = load_shared_lib(&[
                "libasound.so.2",
                "/usr/lib64/libasound.so.2",
                "/usr/lib/libasound.so.2",
                "/usr/lib/x86_64-linux-gnu/libasound.so.2",
                "/usr/lib/aarch64-linux-gnu/libasound.so.2",
            ])?;

            let (fn_open, fn_set_params, fn_writei, fn_recover, fn_drop, fn_prepare, fn_close) = unsafe {
                (
                    *lib.get::<FnSndPcmOpen>(b"snd_pcm_open\0")?,
                    *lib.get::<FnSndPcmSetParams>(b"snd_pcm_set_params\0")?,
                    *lib.get::<FnSndPcmWritei>(b"snd_pcm_writei\0")?,
                    *lib.get::<FnSndPcmRecover>(b"snd_pcm_recover\0")?,
                    *lib.get::<FnSndPcmDrop>(b"snd_pcm_drop\0")?,
                    *lib.get::<FnSndPcmPrepare>(b"snd_pcm_prepare\0")?,
                    *lib.get::<FnSndPcmClose>(b"snd_pcm_close\0")?,
                )
            };

            let dev_name = CString::new("default")?;
            let mut pcm: *mut c_void = ptr::null_mut();
            let rc = unsafe { fn_open(&mut pcm, dev_name.as_ptr(), SND_PCM_STREAM_PLAYBACK, 0) };
            if rc < 0 || pcm.is_null() {
                return Err(anyhow!("snd_pcm_open failed ({})", rc));
            }

            let rc_params = unsafe {
                fn_set_params(
                    pcm,
                    SND_PCM_FORMAT_FLOAT_LE,
                    SND_PCM_ACCESS_RW_INTERLEAVED,
                    ENGINE_CHANNELS as c_uint,
                    ENGINE_SAMPLE_RATE as c_uint,
                    1,
                    50_000,
                )
            };
            if rc_params < 0 {
                unsafe {
                    let _ = fn_close(pcm);
                }
                return Err(anyhow!("snd_pcm_set_params failed ({})", rc_params));
            }

            Ok(Self {
                pcm,
                fn_writei,
                fn_recover,
                fn_drop,
                fn_prepare,
                fn_close,
                _lib: lib,
            })
        }
    }

    impl AudioSink for AlsaSink {
        fn backend_name(&self) -> &str {
            "ALSA (48.0 kHz f32le)"
        }

        fn write_interleaved(&mut self, samples: &[f32]) -> Result<()> {
            let total_frames = samples.len() / ENGINE_CHANNELS as usize;
            let mut written_frames = 0usize;

            while written_frames < total_frames {
                let offset = written_frames * ENGINE_CHANNELS as usize;
                let remaining = total_frames - written_frames;
                let rc = unsafe {
                    (self.fn_writei)(
                        self.pcm,
                        samples[offset..].as_ptr() as *const c_void,
                        remaining,
                    )
                };
                if rc < 0 {
                    let rec = unsafe { (self.fn_recover)(self.pcm, rc as c_int, 1) };
                    if rec < 0 {
                        return Err(anyhow!("ALSA write unrecoverable error ({})", rec));
                    }
                } else {
                    written_frames += rc as usize;
                }
            }
            Ok(())
        }

        fn flush(&mut self) {
            unsafe {
                let _ = (self.fn_drop)(self.pcm);
                let _ = (self.fn_prepare)(self.pcm);
            }
        }
    }

    impl Drop for AlsaSink {
        fn drop(&mut self) {
            if !self.pcm.is_null() {
                unsafe {
                    let _ = (self.fn_drop)(self.pcm);
                    let _ = (self.fn_close)(self.pcm);
                }
                self.pcm = ptr::null_mut();
            }
        }
    }
}

// ============================================================================
// macOS (CoreAudio) & Windows (WASAPI) Native Sink via `cpal`
// ============================================================================

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod cpal_sink {
    use super::{AudioSink, ENGINE_SAMPLE_RATE};
    use crate::audio::resampler::StereoResampler;
    use anyhow::{anyhow, Result};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::SampleFormat;
    use std::collections::VecDeque;
    use std::sync::mpsc;
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::Duration;

    const MAX_BUFFERED_SAMPLES: usize = 4800 * 2; // ~50ms stereo ring capacity

    pub struct CpalSink {
        label: String,
        hw_channels: usize,
        resampler: StereoResampler,
        stereo_scratch: Vec<[f32; 2]>,
        resampled_scratch: Vec<f32>,
        queue: Arc<(Mutex<VecDeque<f32>>, Condvar)>,
        stop_tx: Option<mpsc::Sender<()>>,
    }

    impl CpalSink {
        pub fn try_open() -> Result<Self> {
            let queue = Arc::new((
                Mutex::new(VecDeque::with_capacity(MAX_BUFFERED_SAMPLES * 2)),
                Condvar::new(),
            ));
            let queue_cb = Arc::clone(&queue);
            let (init_tx, init_rx) = mpsc::sync_channel::<Result<(u32, usize, String)>>(1);
            let (stop_tx, stop_rx) = mpsc::channel::<()>();

            thread::Builder::new()
                .name("cadence-cpal".to_string())
                .spawn(move || {
                    let host = cpal::default_host();
                    let Some(device) = host.default_output_device() else {
                        let _ = init_tx.send(Err(anyhow!("No default output device")));
                        return;
                    };
                    let Ok(supported) = device.default_output_config() else {
                        let _ = init_tx.send(Err(anyhow!("No default output config")));
                        return;
                    };
                    let sample_format = supported.sample_format();
                    let config: cpal::StreamConfig = supported.into();
                    let hw_rate = config.sample_rate.0;
                    let hw_channels = config.channels as usize;
                    let err_fn = |err| {
                        let _ = err;
                    };

                    let stream_res = match sample_format {
                        SampleFormat::F32 => device.build_output_stream(
                            &config,
                            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                                drain_into_f32(&queue_cb, data);
                            },
                            err_fn,
                            None,
                        ),
                        SampleFormat::I16 => device.build_output_stream(
                            &config,
                            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                                drain_into_i16(&queue_cb, data);
                            },
                            err_fn,
                            None,
                        ),
                        SampleFormat::U16 => device.build_output_stream(
                            &config,
                            move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                                drain_into_u16(&queue_cb, data);
                            },
                            err_fn,
                            None,
                        ),
                        _ => {
                            let _ = init_tx.send(Err(anyhow!("Unsupported sample format")));
                            return;
                        }
                    };

                    let Ok(stream) = stream_res else {
                        let _ = init_tx.send(Err(anyhow!("Failed to open output stream")));
                        return;
                    };
                    if stream.play().is_err() {
                        let _ = init_tx.send(Err(anyhow!("Failed to start output stream")));
                        return;
                    }

                    let os_label = if cfg!(target_os = "macos") {
                        format!("CoreAudio ({:.1} kHz)", hw_rate as f32 / 1000.0)
                    } else {
                        format!("WASAPI ({:.1} kHz)", hw_rate as f32 / 1000.0)
                    };
                    let _ = init_tx.send(Ok((hw_rate, hw_channels, os_label)));

                    // Keep `stream` alive on its creator thread until dropped
                    let _ = stop_rx.recv();
                    drop(stream);
                })?;

            let (hw_rate, hw_channels, label) = init_rx
                .recv_timeout(Duration::from_secs(3))
                .map_err(|_| anyhow!("Timed out opening audio device"))??;

            Ok(Self {
                label,
                hw_channels: hw_channels.max(1),
                resampler: StereoResampler::new(ENGINE_SAMPLE_RATE, hw_rate),
                stereo_scratch: Vec::with_capacity(1024),
                resampled_scratch: Vec::with_capacity(2048),
                queue,
                stop_tx: Some(stop_tx),
            })
        }
    }

    fn drain_into_f32(q: &(Mutex<VecDeque<f32>>, Condvar), out: &mut [f32]) {
        if let Ok(mut buf) = q.0.lock() {
            for slot in out.iter_mut() {
                *slot = buf.pop_front().unwrap_or(0.0);
            }
            q.1.notify_all();
        } else {
            out.fill(0.0);
        }
    }

    fn drain_into_i16(q: &(Mutex<VecDeque<f32>>, Condvar), out: &mut [i16]) {
        if let Ok(mut buf) = q.0.lock() {
            for slot in out.iter_mut() {
                let s = buf.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0);
                *slot = (s * 32767.0) as i16;
            }
            q.1.notify_all();
        } else {
            out.fill(0);
        }
    }

    fn drain_into_u16(q: &(Mutex<VecDeque<f32>>, Condvar), out: &mut [u16]) {
        if let Ok(mut buf) = q.0.lock() {
            for slot in out.iter_mut() {
                let s = buf.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0);
                *slot = ((s * 0.5 + 0.5) * 65535.0) as u16;
            }
            q.1.notify_all();
        } else {
            out.fill(32768);
        }
    }

    impl AudioSink for CpalSink {
        fn backend_name(&self) -> &str {
            &self.label
        }

        fn write_interleaved(&mut self, samples: &[f32]) -> Result<()> {
            if samples.is_empty() {
                return Ok(());
            }
            self.stereo_scratch.clear();
            for pair in samples.chunks_exact(2) {
                self.stereo_scratch.push([pair[0], pair[1]]);
            }
            self.resampled_scratch.clear();
            self.resampler
                .process(&self.stereo_scratch, &mut self.resampled_scratch);

            let (lock, cvar) = &*self.queue;
            let mut guard = lock.lock().map_err(|_| anyhow!("Audio mutex poisoned"))?;
            let max_cap = MAX_BUFFERED_SAMPLES * self.hw_channels / 2;

            for pair in self.resampled_scratch.chunks_exact(2) {
                while guard.len() >= max_cap {
                    let (g, _) = cvar
                        .wait_timeout(guard, Duration::from_millis(50))
                        .map_err(|_| anyhow!("Audio condvar poisoned"))?;
                    guard = g;
                }
                match self.hw_channels {
                    1 => guard.push_back((pair[0] + pair[1]) * 0.5),
                    2 => {
                        guard.push_back(pair[0]);
                        guard.push_back(pair[1]);
                    }
                    ch => {
                        guard.push_back(pair[0]);
                        guard.push_back(pair[1]);
                        for _ in 2..ch {
                            guard.push_back(0.0);
                        }
                    }
                }
            }
            Ok(())
        }

        fn flush(&mut self) {
            self.resampler.reset();
            if let Ok(mut guard) = self.queue.0.lock() {
                guard.clear();
                self.queue.1.notify_all();
            }
        }
    }

    impl Drop for CpalSink {
        fn drop(&mut self) {
            if let Some(tx) = self.stop_tx.take() {
                let _ = tx.send(());
            }
        }
    }
}

// ============================================================================
// Monotonic Precision-Timed Null Sink (Headless / Testing / Fallback)
// ============================================================================

struct NullSink {
    label: &'static str,
    next_deadline: Option<Instant>,
}

impl NullSink {
    fn new(label: &'static str) -> Self {
        Self {
            label,
            next_deadline: None,
        }
    }
}

impl AudioSink for NullSink {
    fn backend_name(&self) -> &str {
        self.label
    }

    fn write_interleaved(&mut self, samples: &[f32]) -> Result<()> {
        let frames = samples.len() / ENGINE_CHANNELS as usize;
        if frames == 0 {
            return Ok(());
        }
        let chunk_dur = Duration::from_secs_f64(frames as f64 / ENGINE_SAMPLE_RATE as f64);
        let now = Instant::now();
        let target = match self.next_deadline {
            Some(dl) if dl > now => dl,
            _ => now,
        };
        if target > now {
            thread::sleep(target - now);
        }
        self.next_deadline = Some(target + chunk_dur);
        Ok(())
    }

    fn flush(&mut self) {
        self.next_deadline = None;
    }
}
