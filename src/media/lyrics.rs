use crate::media::metadata::extract_embedded_lyrics;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub enum Lyrics {
    #[default]
    None,
    Synced(Vec<LyricLine>),
    Plain(Vec<String>),
}

impl Lyrics {
    pub fn is_available(&self) -> bool {
        !matches!(self, Lyrics::None)
    }

    /// Loads sidecar `.lrc`, embedded tag lyrics, or sidecar `.txt` lyrics for a track.
    pub fn load_for_track(audio_path: &Path) -> Self {
        let lrc_path = audio_path.with_extension("lrc");
        if let Ok(raw) = fs::read_to_string(&lrc_path) {
            let parsed = Self::parse(&raw);
            if parsed.is_available() {
                return parsed;
            }
        }

        if let Some(embedded) = extract_embedded_lyrics(audio_path) {
            let parsed = Self::parse(&embedded);
            if parsed.is_available() {
                return parsed;
            }
        }

        let txt_path = audio_path.with_extension("txt");
        if let Ok(raw) = fs::read_to_string(&txt_path) {
            let parsed = Self::parse(&raw);
            if parsed.is_available() {
                return parsed;
            }
        }

        Self::None
    }

    /// Parses either timestamped `.lrc` text or unsynced plain lyrics.
    pub fn parse(raw: &str) -> Self {
        let mut offset_ms: i64 = 0;
        let mut synced: Vec<LyricLine> = Vec::new();
        let mut plain: Vec<String> = Vec::new();

        for raw_line in raw.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                if synced.is_empty() && !plain.last().map(|s: &String| s.is_empty()).unwrap_or(true) {
                    plain.push(String::new());
                }
                continue;
            }

            // Metadata tag `[offset:+250]`
            if let Some(rest) = line.strip_prefix("[offset:") {
                if let Some(end) = rest.find(']') {
                    if let Ok(val) = rest[..end].trim().parse::<i64>() {
                        offset_ms = val;
                    }
                }
                continue;
            }

            // Skip ID tags `[ar:...]`, `[ti:...]`, `[al:...]`, `[by:...]`
            if is_lrc_header_tag(line) {
                continue;
            }

            let (timestamps, lyric_text) = extract_lrc_timestamps(line);
            if !timestamps.is_empty() {
                for ts in timestamps {
                    let adjusted = (ts as i64 - offset_ms).max(0) as u64;
                    synced.push(LyricLine {
                        time_ms: adjusted,
                        text: lyric_text.to_string(),
                    });
                }
            } else {
                plain.push(line.to_string());
            }
        }

        if !synced.is_empty() {
            synced.sort_by_key(|l| l.time_ms);
            Self::Synced(synced)
        } else if !plain.is_empty() {
            Self::Plain(plain)
        } else {
            Self::None
        }
    }

    /// Finds the active synced lyric line index for `elapsed_ms`.
    pub fn active_synced_index(&self, elapsed_ms: u64) -> Option<usize> {
        let Self::Synced(lines) = self else {
            return None;
        };
        if lines.is_empty() || elapsed_ms < lines[0].time_ms {
            return None;
        }
        match lines.binary_search_by_key(&elapsed_ms, |l| l.time_ms) {
            Ok(mut idx) => {
                while idx + 1 < lines.len() && lines[idx + 1].time_ms == elapsed_ms {
                    idx += 1;
                }
                Some(idx)
            }
            Err(ins) => ins.checked_sub(1),
        }
    }
}

fn is_lrc_header_tag(line: &str) -> bool {
    for prefix in ["[ar:", "[ti:", "[al:", "[au:", "[by:", "[length:", "[re:", "[ve:"] {
        if line.len() >= prefix.len() && line[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return true;
        }
    }
    false
}

fn extract_lrc_timestamps(mut s: &str) -> (Vec<u64>, &str) {
    let mut times = Vec::new();
    while let Some(after_open) = s.strip_prefix('[') {
        let Some(close_idx) = after_open.find(']') else {
            break;
        };
        let inside = &after_open[..close_idx];
        let Some(ms) = parse_single_timestamp(inside) else {
            break;
        };
        times.push(ms);
        s = &after_open[close_idx + 1..];
    }
    (times, s.trim())
}

fn parse_single_timestamp(s: &str) -> Option<u64> {
    let (mins_str, rest) = s.split_once(':')?;
    let mins: u64 = mins_str.trim().parse().ok()?;
    let (secs_str, frac_str) = if let Some((sec, fr)) = rest.split_once('.') {
        (sec, Some(fr))
    } else if let Some((sec, fr)) = rest.split_once(':') {
        (sec, Some(fr))
    } else {
        (rest, None)
    };
    let secs: u64 = secs_str.trim().parse().ok()?;
    let frac_ms: u64 = match frac_str {
        None => 0,
        Some(f) => {
            let trimmed = f.trim();
            let val: u64 = trimmed.parse().ok()?;
            match trimmed.len() {
                1 => val * 100,
                2 => val * 10,
                3 => val,
                _ => val,
            }
        }
    };
    Some(mins * 60_000 + secs * 1_000 + frac_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synced_lrc_and_finds_active_line() {
        let sample = "\
[ti:Arana]
[offset:0]
[00:05.20]First verse begins
[00:12.50][00:42.00]Repeated chorus
[00:25.00]Second verse";
        let lyr = Lyrics::parse(sample);
        let Lyrics::Synced(ref lines) = lyr else {
            panic!("expected synced lyrics");
        };
        assert_eq!(lines.len(), 4);
        assert_eq!(lyr.active_synced_index(1000), None);
        assert_eq!(lyr.active_synced_index(6000), Some(0));
        assert_eq!(lyr.active_synced_index(15000), Some(1));
        assert_eq!(lyr.active_synced_index(45000), Some(3));
    }
}
