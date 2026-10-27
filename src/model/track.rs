use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "m4a", "mp4", "aac", "alac", "wav", "wave", "aif",
    "aiff", "caf", "mkv", "webm", "wma", "wv", "ape", "mpc", "dsf",
];

pub fn is_supported_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn is_playlist_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "m3u" | "m3u8"))
        .unwrap_or(false)
}

/// Core metadata representation of a local audio track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub duration_ms: u64,
    pub codec: String,
    pub sample_rate: u32,
    pub bitrate_kbps: u32,
    pub channels: u8,
    pub mtime_secs: u64,
    pub file_size: u64,
}

impl Track {
    /// Creates a minimal fallback Track from a file path if tag reading fails.
    pub fn from_path_fallback(path: PathBuf) -> Self {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown Track")
            .to_string();

        // Parse "Artist - Title" from filename if present
        let (inferred_artist, inferred_title) = if let Some((a, t)) = stem.split_once(" - ") {
            (a.trim().to_string(), t.trim().to_string())
        } else {
            ("Unknown Artist".to_string(), stem)
        };

        let inferred_album = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown Album")
            .to_string();

        let ext_codec = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_uppercase())
            .unwrap_or_else(|| "AUDIO".to_string());

        Self {
            path,
            title: inferred_title,
            artist: inferred_artist.clone(),
            album: inferred_album,
            album_artist: inferred_artist,
            track_number: None,
            disc_number: None,
            year: None,
            genre: None,
            duration_ms: 0,
            codec: ext_codec,
            sample_rate: 48_000,
            bitrate_kbps: 0,
            channels: 2,
            mtime_secs: 0,
            file_size: 0,
        }
    }

    pub fn display_artist(&self) -> &str {
        if !self.artist.trim().is_empty() {
            &self.artist
        } else if !self.album_artist.trim().is_empty() {
            &self.album_artist
        } else {
            "Unknown Artist"
        }
    }

    pub fn display_album(&self) -> &str {
        if !self.album.trim().is_empty() {
            &self.album
        } else {
            "Unknown Album"
        }
    }

    pub fn duration_formatted(&self) -> String {
        format_duration_ms(self.duration_ms)
    }

    pub fn tech_summary(&self) -> String {
        let mut parts = Vec::with_capacity(4);
        if !self.codec.is_empty() {
            parts.push(self.codec.clone());
        }
        if self.sample_rate > 0 {
            let khz = self.sample_rate as f64 / 1000.0;
            if (khz.fract()).abs() < 0.05 {
                parts.push(format!("{:.0} kHz", khz));
            } else {
                parts.push(format!("{:.1} kHz", khz));
            }
        }
        if self.bitrate_kbps > 0 {
            parts.push(format!("{} kbps", self.bitrate_kbps));
        }
        match self.channels {
            1 => parts.push("Mono".to_string()),
            2 => parts.push("Stereo".to_string()),
            n if n > 2 => parts.push(format!("{} ch", n)),
            _ => {}
        }
        parts.join(" • ")
    }

    /// Compares two tracks in natural disc -> track_number -> filename order.
    pub fn album_order_cmp(&self, other: &Self) -> Ordering {
        let d1 = self.disc_number.unwrap_or(1);
        let d2 = other.disc_number.unwrap_or(1);
        d1.cmp(&d2)
            .then_with(|| match (self.track_number, other.track_number) {
                (Some(t1), Some(t2)) => t1.cmp(&t2),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
            .then_with(|| {
                let f1 = self
                    .path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&self.title);
                let f2 = other
                    .path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&other.title);
                natural_cmp(f1, f2)
            })
    }
}

pub fn format_duration_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, mins, secs)
    } else {
        format!("{:02}:{:02}", mins, secs)
    }
}

/// Case-insensitive natural string comparison ("Track 2" < "Track 10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ia = a.chars().peekable();
    let mut ib = b.chars().peekable();

    loop {
        match (ia.peek(), ib.peek()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(&ca), Some(&cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let mut na: u64 = 0;
                    while let Some(&d) = ia.peek() {
                        if d.is_ascii_digit() {
                            na = na.saturating_mul(10).saturating_add((d as u8 - b'0') as u64);
                            ia.next();
                        } else {
                            break;
                        }
                    }
                    let mut nb: u64 = 0;
                    while let Some(&d) = ib.peek() {
                        if d.is_ascii_digit() {
                            nb = nb.saturating_mul(10).saturating_add((d as u8 - b'0') as u64);
                            ib.next();
                        } else {
                            break;
                        }
                    }
                    let ord = na.cmp(&nb);
                    if ord != Ordering::Equal {
                        return ord;
                    }
                } else {
                    let la = ca.to_lowercase().next().unwrap_or(ca);
                    let lb = cb.to_lowercase().next().unwrap_or(cb);
                    let ord = la.cmp(&lb);
                    if ord != Ordering::Equal {
                        return ord;
                    }
                    ia.next();
                    ib.next();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_sorting_orders_numbers_correctly() {
        assert_eq!(natural_cmp("Track 2.mp3", "Track 10.mp3"), Ordering::Less);
        assert_eq!(natural_cmp("01 Intro", "02 Main"), Ordering::Less);
        assert_eq!(natural_cmp("album", "Album"), Ordering::Equal);
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration_ms(0), "00:00");
        assert_eq!(format_duration_ms(65_400), "01:05");
        assert_eq!(format_duration_ms(3_661_000), "1:01:01");
    }
}
