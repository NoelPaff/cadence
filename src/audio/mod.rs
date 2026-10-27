pub mod decoder;
pub mod resampler;
pub mod sink;
pub mod visualizer;

use crate::audio::decoder::{open_decoder, TrackDecoder};
use crate::audio::sink::{open_best_sink, ENGINE_SAMPLE_RATE};
use crate::audio::visualizer::VisualizerTap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const CHUNK_FRAMES: usize = 1024;
const CHUNK_SAMPLES: usize = CHUNK_FRAMES * 2;

#[derive(Debug, Clone)]
pub enum AudioCommand {
    PlayTrack {
        path: PathBuf,
        queue_token: u64,
    },
    /// Informs the engine which track should immediately follow on EOF for true gapless playback.
    SetGaplessNext(Option<(PathBuf, u64)>),
    TogglePause,
    SetPaused(bool),
    Stop,
    SeekRelativeMs(i64),
    SeekAbsoluteMs(u64),
    SetVolume(u8),
    SetMuted(bool),
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum AudioEvent {
    TrackStarted {
        queue_token: u64,
        duration_ms: u64,
    },
    /// Fired when the audio thread seamlessly crosses a track boundary with zero gap.
    GaplessTransitioned {
        queue_token: u64,
        duration_ms: u64,
    },
    PlaybackFinished {
        queue_token: u64,
    },
    DecodeError {
        path: PathBuf,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct AudioTelemetry {
    pub is_playing: bool,
    pub is_paused: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: u8,
    pub muted: bool,
    pub backend_name: String,
    pub queue_token: u64,
}

impl Default for AudioTelemetry {
    fn default() -> Self {
        Self {
            is_playing: false,
            is_paused: false,
            position_ms: 0,
            duration_ms: 0,
            volume: 80,
            muted: false,
            backend_name: String::from("Initializing..."),
            queue_token: 0,
        }
    }
}

/// Handle owned by the UI / CLI controller to command and inspect the real-time audio engine.
pub struct AudioEngine {
    cmd_tx: Sender<AudioCommand>,
    event_rx: Receiver<AudioEvent>,
    telemetry: Arc<Mutex<AudioTelemetry>>,
    pub visualizer_tap: VisualizerTap,
    worker: Option<JoinHandle<()>>,
}

impl AudioEngine {
    pub fn spawn(initial_volume: u8, force_null_sink: bool) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<AudioCommand>();
        let (event_tx, event_rx) = mpsc::channel::<AudioEvent>();
        let tap = VisualizerTap::new();
        let tap_clone = tap.clone();

        let telemetry = Arc::new(Mutex::new(AudioTelemetry {
            volume: initial_volume.min(100),
            ..Default::default()
        }));
        let telemetry_clone = Arc::clone(&telemetry);

        let worker = thread::Builder::new()
            .name("cadence-audio".to_string())
            .spawn(move || {
                run_audio_thread(
                    cmd_rx,
                    event_tx,
                    telemetry_clone,
                    tap_clone,
                    initial_volume,
                    force_null_sink,
                );
            })
            .expect("failed to spawn cadence-audio thread");

        Self {
            cmd_tx,
            event_rx,
            telemetry,
            visualizer_tap: tap,
            worker: Some(worker),
        }
    }

    pub fn send(&self, cmd: AudioCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn try_recv_event(&self) -> Option<AudioEvent> {
        self.event_rx.try_recv().ok()
    }

    pub fn snapshot(&self) -> AudioTelemetry {
        self.telemetry
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(AudioCommand::Shutdown);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

struct ActiveTrackState {
    decoder: Box<dyn TrackDecoder>,
    path: PathBuf,
    queue_token: u64,
    frames_played: u64,
    base_seek_ms: u64,
}

impl ActiveTrackState {
    fn current_position_ms(&self) -> u64 {
        self.base_seek_ms
            + (self.frames_played.saturating_mul(1000)) / (ENGINE_SAMPLE_RATE as u64)
    }
}

struct PrebufferedNext {
    path: PathBuf,
    queue_token: u64,
    decoder: Option<Box<dyn TrackDecoder>>,
    open_failed: bool,
}

fn run_audio_thread(
    cmd_rx: Receiver<AudioCommand>,
    event_tx: Sender<AudioEvent>,
    telemetry: Arc<Mutex<AudioTelemetry>>,
    tap: VisualizerTap,
    initial_volume: u8,
    force_null_sink: bool,
) {
    let mut sink = open_best_sink(force_null_sink);
    let backend_name = sink.backend_name().to_string();

    let mut active: Option<ActiveTrackState> = None;
    let mut next_spec: Option<PrebufferedNext> = None;
    let mut paused = false;
    let mut volume = initial_volume.min(100);
    let mut muted = false;

    if let Ok(mut t) = telemetry.lock() {
        t.backend_name = backend_name.clone();
        t.volume = volume;
    }

    let mut pcm_fifo: Vec<f32> = Vec::with_capacity(CHUNK_SAMPLES * 4);
    let mut out_chunk: Vec<f32> = vec![0.0; CHUNK_SAMPLES];

    loop {
        // Process all pending commands immediately; block gently when idle or paused
        let wait_idle = active.is_none() || paused;
        let first_cmd = if wait_idle {
            cmd_rx.recv_timeout(Duration::from_millis(25)).ok()
        } else {
            cmd_rx.try_recv().ok()
        };

        let mut cmds = Vec::new();
        if let Some(c) = first_cmd {
            cmds.push(c);
            while let Ok(more) = cmd_rx.try_recv() {
                cmds.push(more);
            }
        }

        let mut shutdown = false;
        for cmd in cmds {
            match cmd {
                AudioCommand::Shutdown => {
                    shutdown = true;
                    break;
                }
                AudioCommand::PlayTrack { path, queue_token } => {
                    sink.flush();
                    pcm_fifo.clear();
                    tap.clear();
                    paused = false;

                    // Reuse prebuffered decoder if it already matches!
                    let pre_dec = next_spec
                        .take()
                        .and_then(|ns| {
                            if ns.path == path {
                                ns.decoder
                            } else {
                                None
                            }
                        });

                    let opened = match pre_dec {
                        Some(d) => Ok(d),
                        None => open_decoder(&path),
                    };

                    match opened {
                        Ok(decoder) => {
                            let dur = decoder.duration_ms();
                            active = Some(ActiveTrackState {
                                decoder,
                                path,
                                queue_token,
                                frames_played: 0,
                                base_seek_ms: 0,
                            });
                            let _ = event_tx.send(AudioEvent::TrackStarted {
                                queue_token,
                                duration_ms: dur,
                            });
                        }
                        Err(err) => {
                            active = None;
                            let _ = event_tx.send(AudioEvent::DecodeError {
                                path,
                                message: err.to_string(),
                            });
                        }
                    }
                }
                AudioCommand::SetGaplessNext(spec) => {
                    match spec {
                        None => next_spec = None,
                        Some((p, tok)) => {
                            let keep_existing = next_spec
                                .as_ref()
                                .map(|ns| ns.path == p && ns.queue_token == tok)
                                .unwrap_or(false);
                            if !keep_existing {
                                next_spec = Some(PrebufferedNext {
                                    path: p,
                                    queue_token: tok,
                                    decoder: None,
                                    open_failed: false,
                                });
                            }
                        }
                    }
                }
                AudioCommand::TogglePause => {
                    if active.is_some() {
                        paused = !paused;
                        if paused {
                            sink.flush();
                        }
                    }
                }
                AudioCommand::SetPaused(p) => {
                    if active.is_some() && paused != p {
                        paused = p;
                        if paused {
                            sink.flush();
                        }
                    }
                }
                AudioCommand::Stop => {
                    active = None;
                    paused = false;
                    pcm_fifo.clear();
                    tap.clear();
                    sink.flush();
                }
                AudioCommand::SeekRelativeMs(delta) => {
                    if let Some(st) = active.as_mut() {
                        let cur = st.current_position_ms() as i64;
                        let target = (cur + delta).max(0) as u64;
                        if let Ok(actual) = st.decoder.seek_ms(target) {
                            st.base_seek_ms = actual;
                            st.frames_played = 0;
                            pcm_fifo.clear();
                            sink.flush();
                        }
                    }
                }
                AudioCommand::SeekAbsoluteMs(target) => {
                    if let Some(st) = active.as_mut() {
                        if let Ok(actual) = st.decoder.seek_ms(target) {
                            st.base_seek_ms = actual;
                            st.frames_played = 0;
                            pcm_fifo.clear();
                            sink.flush();
                        }
                    }
                }
                AudioCommand::SetVolume(v) => {
                    volume = v.min(100);
                }
                AudioCommand::SetMuted(m) => {
                    muted = m;
                }
            }
        }

        if shutdown {
            sink.flush();
            break;
        }

        // Lazily pre-open the next track when within 6 seconds of track end for zero-gap handoff
        if let (Some(st), Some(ns)) = (active.as_ref(), next_spec.as_mut()) {
            if ns.decoder.is_none() && !ns.open_failed {
                let dur = st.decoder.duration_ms();
                let pos = st.current_position_ms();
                if dur == 0 || pos + 6_000 >= dur {
                    // Note: if Repeat-One points to the exact same file, opening a second file descriptor
                    // works cleanly on Linux!
                    match open_decoder(&ns.path) {
                        Ok(dec) => ns.decoder = Some(dec),
                        Err(_) => ns.open_failed = true,
                    }
                }
            }
        }

        // Decode & stream audio when active and unpaused
        if !paused && active.is_some() {
            while pcm_fifo.len() < CHUNK_SAMPLES {
                let decode_res = active
                    .as_mut()
                    .unwrap()
                    .decoder
                    .decode_next(&mut pcm_fifo);

                match decode_res {
                    Ok(true) => {}
                    Ok(false) => {
                        // EOF reached on current track! Attempt seamless gapless transition right inside pcm_fifo!
                        let finished_token = active.as_ref().unwrap().queue_token;
                        let transitioned = if let Some(mut ns) = next_spec.take() {
                            let next_dec = match ns.decoder.take() {
                                Some(d) => Ok(d),
                                None if !ns.open_failed => open_decoder(&ns.path),
                                None => Err(anyhow::anyhow!("next track failed")),
                            };
                            if let Ok(dec) = next_dec {
                                let dur = dec.duration_ms();
                                let tok = ns.queue_token;
                                active = Some(ActiveTrackState {
                                    decoder: dec,
                                    path: ns.path,
                                    queue_token: tok,
                                    frames_played: 0,
                                    base_seek_ms: 0,
                                });
                                let _ = event_tx.send(AudioEvent::GaplessTransitioned {
                                    queue_token: tok,
                                    duration_ms: dur,
                                });
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if !transitioned {
                            active = None;
                            let _ = event_tx.send(AudioEvent::PlaybackFinished {
                                queue_token: finished_token,
                            });
                            break;
                        }
                    }
                    Err(err) => {
                        let failed = active.take().unwrap();
                        let _ = event_tx.send(AudioEvent::DecodeError {
                            path: failed.path,
                            message: err.to_string(),
                        });
                        break;
                    }
                }
            }

            if !pcm_fifo.is_empty() {
                let take_samples = pcm_fifo.len().min(CHUNK_SAMPLES);
                let aligned_samples = take_samples - (take_samples % 2);
                if aligned_samples > 0 {
                    out_chunk.resize(aligned_samples, 0.0);
                    out_chunk.copy_from_slice(&pcm_fifo[..aligned_samples]);
                    pcm_fifo.drain(0..aligned_samples);

                    // Feed pre-gain or post-gain samples to visualizer tap (pre-volume so visualizer dances even at low listening volume, unless muted!)
                    if !muted {
                        tap.push_stereo(&out_chunk);
                    } else {
                        tap.clear();
                    }

                    // Perceptual cubic gain curve (`(vol / 100)^2.4`) + soft limiter
                    let gain = if muted || volume == 0 {
                        0.0f32
                    } else {
                        (volume as f32 / 100.0).powf(2.2)
                    };

                    for s in &mut out_chunk {
                        *s = (*s * gain).clamp(-1.0, 1.0);
                    }

                    let _ = sink.write_interleaved(&out_chunk);

                    if let Some(st) = active.as_mut() {
                        st.frames_played += (aligned_samples / 2) as u64;
                    }
                }
            }
        }

        // Publish telemetry snapshot
        if let Ok(mut t) = telemetry.lock() {
            t.is_playing = active.is_some() && !paused;
            t.is_paused = active.is_some() && paused;
            t.volume = volume;
            t.muted = muted;
            if let Some(st) = active.as_ref() {
                t.position_ms = st.current_position_ms();
                t.duration_ms = st.decoder.duration_ms();
                t.queue_token = st.queue_token;
            } else {
                t.position_ms = 0;
                t.duration_ms = 0;
            }
        }
    }
}
