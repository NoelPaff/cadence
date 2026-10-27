use crate::model::Track;
use image::DynamicImage;
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::tag::ItemKey;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Extracts rich audio metadata & stream properties from `path`, falling back gracefully.
pub fn extract_track_metadata(path: &Path, mtime_secs: u64, file_size: u64) -> Track {
    let mut track = Track::from_path_fallback(path.to_path_buf());
    track.mtime_secs = mtime_secs;
    track.file_size = file_size;

    if let Ok(tagged) = lofty::read_from_path(path) {
        let props = tagged.properties();
        let dur_ms = props.duration().as_millis() as u64;
        if dur_ms > 0 {
            track.duration_ms = dur_ms;
        }
        if let Some(sr) = props.sample_rate() {
            if sr > 0 {
                track.sample_rate = sr;
            }
        }
        if let Some(br) = props.audio_bitrate().or_else(|| props.overall_bitrate()) {
            track.bitrate_kbps = br;
        }
        if let Some(ch) = props.channels() {
            if ch > 0 {
                track.channels = ch;
            }
        }
        track.codec = file_type_label(tagged.file_type(), path);

        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(t) = tag.title() {
                let s = t.trim();
                if !s.is_empty() {
                    track.title = s.to_string();
                }
            }
            if let Some(a) = tag.artist() {
                let s = a.trim();
                if !s.is_empty() {
                    track.artist = s.to_string();
                }
            }
            if let Some(alb) = tag.album() {
                let s = alb.trim();
                if !s.is_empty() {
                    track.album = s.to_string();
                }
            }
            if let Some(aa) = tag.get_string(&ItemKey::AlbumArtist) {
                let s = aa.trim();
                if !s.is_empty() {
                    track.album_artist = s.to_string();
                }
            } else if !track.artist.is_empty() {
                track.album_artist = track.artist.clone();
            }
            track.track_number = tag.track();
            track.disc_number = tag.disk();
            track.year = tag.year();
            track.genre = tag.genre().map(|g| g.trim().to_string()).filter(|s| !s.is_empty());
        }
    }

    // If duration is still unknown, probe container header via Symphonia
    if track.duration_ms == 0 {
        probe_symphonia_properties(path, &mut track);
    }

    track
}

fn file_type_label(ft: lofty::file::FileType, path: &Path) -> String {
    use lofty::file::FileType;
    match ft {
        FileType::Flac => "FLAC".to_string(),
        FileType::Mpeg => "MP3".to_string(),
        FileType::Opus => "Opus".to_string(),
        FileType::Vorbis => "Vorbis".to_string(),
        FileType::Mp4 => "M4A".to_string(),
        FileType::Aac => "AAC".to_string(),
        FileType::Wav => "WAV".to_string(),
        FileType::Aiff => "AIFF".to_string(),
        FileType::Ape => "APE".to_string(),
        FileType::WavPack => "WavPack".to_string(),
        FileType::Mpc => "Musepack".to_string(),
        FileType::Speex => "Speex".to_string(),
        _ => path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_uppercase())
            .unwrap_or_else(|| "AUDIO".to_string()),
    }
}

fn probe_symphonia_properties(path: &Path, track: &mut Track) {
    let Ok(file) = File::open(path) else {
        return;
    };
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let Ok(probed) = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) else {
        return;
    };
    if let Some(default_track) = probed.format.default_track() {
        let params = &default_track.codec_params;
        if let Some(sr) = params.sample_rate {
            track.sample_rate = sr;
            if let Some(n_frames) = params.n_frames {
                track.duration_ms = (n_frames.saturating_mul(1000)) / (sr as u64);
            }
        }
        if let Some(ch) = params.channels {
            track.channels = ch.count() as u8;
        }
    }
}

/// Loads embedded album cover art or searches the track's parent directory for external cover files.
pub fn load_cover_image_for_track(path: &Path) -> Option<DynamicImage> {
    // 1. Try embedded artwork via Lofty
    if let Ok(tagged) = lofty::read_from_path(path) {
        for tag in tagged.tags() {
            let pics = tag.pictures();
            if let Some(front) = pics
                .iter()
                .find(|p| p.pic_type() == PictureType::CoverFront)
                .or_else(|| pics.first())
            {
                if let Ok(img) = image::load_from_memory(front.data()) {
                    return Some(img);
                }
            }
        }
    }

    // 2. Search sibling image files in the track's directory
    let parent = path.parent()?;
    find_external_cover_in_dir(parent)
}

fn find_external_cover_in_dir(dir: &Path) -> Option<DynamicImage> {
    const PRIORITY_STEMS: &[&str] = &[
        "cover", "folder", "front", "album", "albumart", "art", "artwork", "thumb",
    ];
    const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "gif"];

    let entries: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|ext| IMAGE_EXTS.contains(&ext.to_ascii_lowercase().as_str()))
                    .unwrap_or(false)
        })
        .collect();

    if entries.is_empty() {
        return None;
    }

    for prio in PRIORITY_STEMS {
        for candidate in &entries {
            let stem = candidate
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if stem == *prio || stem.starts_with(prio) {
                if let Ok(img) = image::open(candidate) {
                    return Some(img);
                }
            }
        }
    }

    // Fallback to first image in directory if not home/root
    let is_home = dirs::home_dir().as_deref() == Some(dir);
    if !is_home && dir.parent().is_some() {
        if let Ok(img) = image::open(&entries[0]) {
            return Some(img);
        }
    }
    None
}

/// Extracts embedded lyrics text from audio tags if present.
pub fn extract_embedded_lyrics(path: &Path) -> Option<String> {
    let tagged = lofty::read_from_path(path).ok()?;
    for tag in tagged.tags() {
        if let Some(lyr) = tag.get_string(&ItemKey::Lyrics) {
            let trimmed = lyr.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Writes a 256x256 PNG thumbnail to `~/.cache/cadence/covers/` for MPRIS/desktop notifications.
pub fn write_mpris_cover_cache(
    img: &DynamicImage,
    cache_dir: &Path,
    track: &Track,
) -> Option<String> {
    let _ = fs::create_dir_all(cache_dir);
    let mut hash: u64 = 14695981039346656037;
    for b in format!("{}::{}", track.display_artist(), track.display_album()).bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    let out_path = cache_dir.join(format!("cover_{:016x}.png", hash));
    if !out_path.exists() {
        let thumb = img.thumbnail(256, 256);
        thumb.save(&out_path).ok()?;
    }
    Some(format!("file://{}", out_path.display()))
}
