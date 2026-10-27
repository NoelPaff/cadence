use cadence::audio::decoder::open_decoder;
use cadence::audio::visualizer::SpectrumAnalyzer;
use cadence::audio::{AudioCommand, AudioEngine, AudioEvent};
use cadence::config::{CoverRenderMode, VisualizerStyle};
use cadence::media::cover::CoverArt;
use cadence::media::lyrics::Lyrics;
use cadence::media::palette::ThemePalette;
use cadence::model::{MusicLibrary, Playlist};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Writes a valid 16-bit PCM WAV file containing a sine tone.
fn write_sine_wav(path: &Path, sample_rate: u32, duration_ms: u32, freq_hz: f32) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let channels: u16 = 2;
    let total_frames = (sample_rate as u64 * duration_ms as u64 / 1000) as u32;
    let data_bytes = total_frames * channels as u32 * 2;
    let riff_size = 36 + data_bytes;
    let byte_rate = sample_rate * channels as u32 * 2;
    let block_align = channels * 2;

    let mut buf = Vec::with_capacity((44 + data_bytes) as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&riff_size.to_le_bytes());
    buf.extend_from_slice(b"WAVEfmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_bytes.to_le_bytes());

    for n in 0..total_frames {
        let t = n as f32 / sample_rate as f32;
        let sample = ((2.0 * std::f32::consts::PI * freq_hz * t).sin() * 18_000.0) as i16;
        buf.extend_from_slice(&sample.to_le_bytes());
        buf.extend_from_slice(&sample.to_le_bytes());
    }

    let mut f = fs::File::create(path).unwrap();
    f.write_all(&buf).unwrap();
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cadence_test_{}_{}",
        name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn library_indexing_smart_queries_and_m3u_roundtrip() {
    let root = unique_temp_dir("lib");
    let t1 = root.join("Miles Davis/Kind of Blue/01 - So What.wav");
    let t2 = root.join("Miles Davis/Kind of Blue/02 - Freddie Freeloader.wav");
    let t3 = root.join("Bill Evans/Waltz for Debby/01 - My Foolish Heart.wav");

    write_sine_wav(&t1, 44_100, 120, 440.0);
    write_sine_wav(&t2, 44_100, 120, 523.25);
    write_sine_wav(&t3, 48_000, 120, 329.63);

    let cache_path = root.join("cache.json");
    let lib = MusicLibrary::scan_with_cache(&root, Some(&cache_path));
    assert_eq!(lib.len(), 3);

    // Chained colon query `artist:album:track`
    let chained = lib
        .match_smart_query("miles:kind:freddie", false)
        .expect("chained match");
    assert_eq!(chained.tracks.len(), 1);
    assert!(chained.tracks[0].title.contains("Freddie"));

    // Directory match returns tracks in album track order
    let dir_match = lib
        .match_directory("Kind of Blue", false)
        .expect("dir match");
    assert_eq!(dir_match.tracks.len(), 2);
    assert!(dir_match.tracks[0].title.contains("So What"));
    assert!(dir_match.tracks[1].title.contains("Freddie"));

    // M3U append deduplication & reload
    let m3u = root.join("cadence.m3u8");
    assert!(Playlist::append_track_to_m3u(&dir_match.tracks[0], &m3u).unwrap());
    assert!(!Playlist::append_track_to_m3u(&dir_match.tracks[0], &m3u).unwrap());
    assert!(Playlist::append_track_to_m3u(&dir_match.tracks[1], &m3u).unwrap());
    let loaded = Playlist::load_m3u_paths(&m3u).unwrap();
    assert_eq!(loaded.len(), 2);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn decoder_seek_and_true_gapless_engine_handoff() {
    let root = unique_temp_dir("gapless");
    let wav_44k = root.join("track1_44k.wav");
    let wav_48k = root.join("track2_48k.wav");
    write_sine_wav(&wav_44k, 44_100, 140, 440.0);
    write_sine_wav(&wav_48k, 48_000, 140, 880.0);

    // Direct decoder + seek verification
    let mut dec = open_decoder(&wav_44k).expect("open wav decoder");
    assert!(dec.duration_ms() >= 130 && dec.duration_ms() <= 150);
    let seeked = dec.seek_ms(60).expect("seek wav");
    assert!(seeked <= 75);
    let mut pcm = Vec::new();
    while dec.decode_next(&mut pcm).unwrap() {}
    assert!(!pcm.is_empty());

    // Gapless AudioEngine verification across mixed sample rates (44.1kHz -> 48kHz)
    let engine = AudioEngine::spawn(75, true);
    engine.send(AudioCommand::PlayTrack {
        path: wav_44k.clone(),
        queue_token: 101,
    });
    engine.send(AudioCommand::SetGaplessNext(Some((wav_48k.clone(), 202))));

    let mut saw_started = false;
    let mut saw_gapless = false;
    let mut saw_finished = false;
    let deadline = Instant::now() + Duration::from_secs(4);

    while Instant::now() < deadline && !saw_finished {
        if let Some(ev) = engine.try_recv_event() {
            match ev {
                AudioEvent::TrackStarted { queue_token, .. } if queue_token == 101 => {
                    saw_started = true;
                }
                AudioEvent::GaplessTransitioned { queue_token, .. } if queue_token == 202 => {
                    saw_gapless = true;
                }
                AudioEvent::PlaybackFinished { queue_token } if queue_token == 202 => {
                    saw_finished = true;
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(saw_started, "expected TrackStarted for token 101");
    assert!(saw_gapless, "expected seamless GaplessTransitioned for token 202");
    assert!(saw_finished, "expected PlaybackFinished after token 202");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn cover_palette_visualizer_and_lyrics_render_cleanly() {
    let track = cadence::model::Track::from_path_fallback(PathBuf::from(
        "/music/Jazz Club - Midnight Arana.flac",
    ));
    let mut cover = CoverArt::procedural_for_track(&track);
    let palette = ThemePalette::from_album_image(&cover.image);

    for mode in [
        CoverRenderMode::HalfBlock,
        CoverRenderMode::Ascii,
        CoverRenderMode::Braille,
    ] {
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
        cover.render(Rect::new(0, 0, 40, 20), &mut buf, mode);
    }

    let tap = cadence::audio::visualizer::VisualizerTap::new();
    let samples: Vec<f32> = (0..4096)
        .map(|i| ((i as f32 * 0.09).sin()) * 0.6)
        .collect();
    tap.push_stereo(&samples);

    let mut analyzer = SpectrumAnalyzer::new();
    analyzer.update(&tap, 48, true);

    for style in [
        VisualizerStyle::Bars,
        VisualizerStyle::Mirrored,
        VisualizerStyle::Braille,
        VisualizerStyle::Led,
    ] {
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 12));
        analyzer.render(Rect::new(0, 0, 80, 12), &mut buf, style, &palette);
    }

    let lrc = Lyrics::parse("[00:01.00]Hello terminal\n[00:04.50]Second line");
    assert_eq!(lrc.active_synced_index(2500), Some(0));
    assert_eq!(lrc.active_synced_index(5000), Some(1));
}
