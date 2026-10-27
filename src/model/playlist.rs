use crate::config::RepeatMode;
use crate::model::Track;
use anyhow::Result;
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Active playback queue with reversible shuffle, repeat modes, and M3U I/O.
#[derive(Debug, Clone, Default)]
pub struct Playlist {
    tracks: Vec<Track>,
    original_order: Vec<Track>,
    current_index: Option<usize>,
    pub shuffled: bool,
    pub repeat: RepeatMode,
}

impl Playlist {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.current_index.and_then(|idx| self.tracks.get(idx))
    }

    pub fn total_duration_ms(&self) -> u64 {
        self.tracks.iter().map(|t| t.duration_ms).sum()
    }

    /// Replaces the entire queue and optionally shuffles while anchoring `start_index` first.
    pub fn set_tracks(&mut self, tracks: Vec<Track>, start_index: Option<usize>, shuffle: bool) {
        self.original_order = tracks.clone();
        self.tracks = tracks;
        self.shuffled = false;

        if self.tracks.is_empty() {
            self.current_index = None;
            self.shuffled = shuffle;
            return;
        }

        let valid_idx = start_index
            .map(|i| i.min(self.tracks.len() - 1))
            .or(Some(0));
        self.current_index = valid_idx;

        if shuffle {
            self.enable_shuffle_anchoring_current();
        }
    }

    /// Selects a specific index in the queue for playback.
    pub fn select_index(&mut self, index: usize) -> Option<&Track> {
        if index < self.tracks.len() {
            self.current_index = Some(index);
            self.tracks.get(index)
        } else {
            None
        }
    }

    /// Appends tracks to the end of the queue.
    pub fn enqueue(&mut self, mut new_tracks: Vec<Track>) {
        if new_tracks.is_empty() {
            return;
        }
        self.original_order.extend(new_tracks.iter().cloned());
        if self.shuffled {
            new_tracks.shuffle(&mut thread_rng());
        }
        let was_empty = self.tracks.is_empty();
        self.tracks.extend(new_tracks);
        if was_empty && !self.tracks.is_empty() {
            self.current_index = Some(0);
        }
    }

    /// Inserts a track immediately after the currently playing track.
    pub fn enqueue_next(&mut self, track: Track) {
        self.original_order.push(track.clone());
        match self.current_index {
            Some(idx) if idx + 1 <= self.tracks.len() => {
                self.tracks.insert(idx + 1, track);
            }
            _ => {
                let was_empty = self.tracks.is_empty();
                self.tracks.push(track);
                if was_empty {
                    self.current_index = Some(0);
                }
            }
        }
    }

    /// Toggles reversible shuffle, preserving the currently playing song seamlessly.
    pub fn toggle_shuffle(&mut self) {
        if self.shuffled {
            let current_path = self.current_track().map(|t| t.path.clone());
            // Reconcile original_order so only tracks still in `self.tracks` remain,
            // plus any newly added tracks.
            let mut restored: Vec<Track> = self
                .original_order
                .iter()
                .filter(|orig| self.tracks.iter().any(|t| t.path == orig.path))
                .cloned()
                .collect();
            for t in &self.tracks {
                if !restored.iter().any(|r| r.path == t.path) {
                    restored.push(t.clone());
                }
            }
            self.tracks = restored;
            self.original_order.clear();
            self.shuffled = false;
            if let Some(p) = current_path {
                self.current_index = self.tracks.iter().position(|t| t.path == p).or_else(|| {
                    if self.tracks.is_empty() {
                        None
                    } else {
                        Some(0)
                    }
                });
            }
        } else {
            self.original_order = self.tracks.clone();
            self.enable_shuffle_anchoring_current();
        }
    }

    fn enable_shuffle_anchoring_current(&mut self) {
        self.shuffled = true;
        if self.tracks.len() <= 1 {
            return;
        }
        let mut rng = thread_rng();
        if let Some(idx) = self.current_index {
            if idx < self.tracks.len() {
                self.tracks.swap(0, idx);
                self.tracks[1..].shuffle(&mut rng);
                self.current_index = Some(0);
                return;
            }
        }
        self.tracks.shuffle(&mut rng);
        self.current_index = Some(0);
    }

    /// Computes the index that will naturally play next when the current track ends.
    pub fn peek_natural_next_index(&self) -> Option<usize> {
        if self.tracks.is_empty() {
            return None;
        }
        let cur = self.current_index?;
        match self.repeat {
            RepeatMode::One => Some(cur),
            RepeatMode::All => Some((cur + 1) % self.tracks.len()),
            RepeatMode::Off => {
                if cur + 1 < self.tracks.len() {
                    Some(cur + 1)
                } else {
                    None
                }
            }
        }
    }

    /// Advances to the next track (`manual_skip = true` skips past `RepeatMode::One`).
    pub fn advance_next(&mut self, manual_skip: bool) -> Option<&Track> {
        if self.tracks.is_empty() {
            self.current_index = None;
            return None;
        }
        let next_idx = match self.current_index {
            None => Some(0),
            Some(cur) => {
                if !manual_skip && self.repeat == RepeatMode::One {
                    Some(cur)
                } else if cur + 1 < self.tracks.len() {
                    Some(cur + 1)
                } else if self.repeat == RepeatMode::All || (manual_skip && self.repeat == RepeatMode::One) {
                    Some(0)
                } else {
                    None
                }
            }
        };

        if let Some(idx) = next_idx {
            self.current_index = Some(idx);
            self.tracks.get(idx)
        } else {
            None
        }
    }

    /// Steps backward to the previous track in the queue.
    pub fn advance_prev(&mut self) -> Option<&Track> {
        if self.tracks.is_empty() {
            self.current_index = None;
            return None;
        }
        let prev_idx = match self.current_index {
            None => 0,
            Some(0) => {
                if self.repeat == RepeatMode::All {
                    self.tracks.len() - 1
                } else {
                    0
                }
            }
            Some(cur) => cur - 1,
        };
        self.current_index = Some(prev_idx);
        self.tracks.get(prev_idx)
    }

    /// Moves a track from `from` to `to`, keeping `current_index` pointing to the active song.
    pub fn move_item(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tracks.len() || to >= self.tracks.len() || from == to {
            return false;
        }
        let item = self.tracks.remove(from);
        self.tracks.insert(to, item);

        if let Some(cur) = self.current_index {
            self.current_index = Some(if cur == from {
                to
            } else if from < cur && cur <= to {
                cur - 1
            } else if to <= cur && cur < from {
                cur + 1
            } else {
                cur
            });
        }
        true
    }

    /// Removes the track at `index`. Returns `true` if the currently playing track was removed.
    pub fn remove_at(&mut self, index: usize) -> bool {
        if index >= self.tracks.len() {
            return false;
        }
        let removed = self.tracks.remove(index);
        self.original_order.retain(|t| t.path != removed.path);

        if self.tracks.is_empty() {
            let was_playing = self.current_index.is_some();
            self.current_index = None;
            return was_playing;
        }

        if let Some(cur) = self.current_index {
            if index == cur {
                let clamped = cur.min(self.tracks.len() - 1);
                self.current_index = Some(clamped);
                return true;
            } else if index < cur {
                self.current_index = Some(cur - 1);
            }
        }
        false
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.original_order.clear();
        self.current_index = None;
    }

    /// Exports the queue as an Extended M3U8 file.
    pub fn save_m3u(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = String::from("#EXTM3U\n");
        for track in &self.tracks {
            let secs = (track.duration_ms / 1000) as i64;
            out.push_str(&format!(
                "#EXTINF:{},{} - {}\n{}\n",
                secs,
                track.display_artist(),
                track.title,
                track.path.display()
            ));
        }
        fs::write(path, out)?;
        Ok(())
    }

    /// Appends a single track to a persistent `.m3u` / `.m3u8` playlist if not already present.
    /// Returns `Ok(true)` if added, `Ok(false)` if already present.
    pub fn append_track_to_m3u(track: &Track, m3u_path: &Path) -> Result<bool> {
        if let Some(parent) = m3u_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let existing_paths = if m3u_path.exists() {
            Self::load_m3u_paths(m3u_path).unwrap_or_default()
        } else {
            Vec::new()
        };

        let target_canon = track
            .path
            .canonicalize()
            .unwrap_or_else(|_| track.path.clone());
        for p in &existing_paths {
            let p_canon = p.canonicalize().unwrap_or_else(|_| p.clone());
            if p_canon == target_canon {
                return Ok(false);
            }
        }

        let needs_header = !m3u_path.exists()
            || fs::metadata(m3u_path).map(|m| m.len() == 0).unwrap_or(true);

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(m3u_path)?;

        if needs_header {
            writeln!(file, "#EXTM3U")?;
        }
        let secs = (track.duration_ms / 1000) as i64;
        writeln!(
            file,
            "#EXTINF:{},{} - {}",
            secs,
            track.display_artist(),
            track.title
        )?;
        writeln!(file, "{}", track.path.display())?;
        Ok(true)
    }

    /// Parses an `.m3u` or `.m3u8` file into resolved local `PathBuf`s.
    pub fn load_m3u_paths(m3u_path: &Path) -> Result<Vec<PathBuf>> {
        let content = fs::read_to_string(m3u_path)?;
        let base_dir = m3u_path.parent().unwrap_or_else(|| Path::new("."));
        let mut paths = Vec::new();

        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cleaned = line.strip_prefix("file://").unwrap_or(line);
            let candidate = PathBuf::from(cleaned);
            let resolved = if candidate.is_absolute() {
                candidate
            } else {
                base_dir.join(candidate)
            };
            paths.push(resolved);
        }
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_track(name: &str) -> Track {
        Track::from_path_fallback(PathBuf::from(format!("/music/{}.mp3", name)))
    }

    #[test]
    fn reversible_shuffle_preserves_current_and_restores_order() {
        let mut pl = Playlist::new();
        let items: Vec<Track> = (1..=8).map(|i| dummy_track(&format!("song_{}", i))).collect();
        pl.set_tracks(items.clone(), Some(3), false);
        assert_eq!(pl.current_track().unwrap().title, "song_4");

        pl.toggle_shuffle();
        assert!(pl.shuffled);
        assert_eq!(pl.current_index(), Some(0));
        assert_eq!(pl.current_track().unwrap().title, "song_4");

        pl.toggle_shuffle();
        assert!(!pl.shuffled);
        assert_eq!(pl.tracks(), items.as_slice());
        assert_eq!(pl.current_index(), Some(3));
    }

    #[test]
    fn move_and_remove_keep_active_index_accurate() {
        let mut pl = Playlist::new();
        let items = vec![dummy_track("a"), dummy_track("b"), dummy_track("c")];
        pl.set_tracks(items, Some(1), false); // playing "b"

        pl.move_item(0, 2); // ["b", "c", "a"] -> "b" moved to 0
        assert_eq!(pl.current_index(), Some(0));
        assert_eq!(pl.current_track().unwrap().title, "b");

        let removed_active = pl.remove_at(2); // remove "a"
        assert!(!removed_active);
        assert_eq!(pl.current_index(), Some(0));
    }
}
