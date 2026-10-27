use crate::media::metadata::extract_track_metadata;
use crate::model::track::{is_playlist_file, is_supported_audio_file, natural_cmp, Track};
use rand::seq::SliceRandom;
use rand::thread_rng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

/// Library hierarchy browsing mode in F3 Library View.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryTreeMode {
    #[default]
    Directories,
    Artists,
}

impl LibraryTreeMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Directories => Self::Artists,
            Self::Artists => Self::Directories,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Directories => "Folder Tree",
            Self::Artists => "Artist / Album Tree",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryNodeKind {
    Directory(PathBuf),
    Artist(String),
    Album { artist: String, album: String },
    Track(Track),
    PlaylistFile(PathBuf),
}

#[derive(Debug, Clone)]
pub struct FlatLibraryItem {
    pub key: String,
    pub depth: usize,
    pub display_name: String,
    pub sub_info: String,
    pub is_container: bool,
    pub is_expanded: bool,
    pub kind: LibraryNodeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartMatchKind {
    Album,
    Directory,
    Artist,
    Track,
    ChainedFilter,
}

#[derive(Debug, Clone)]
pub struct SmartMatch {
    pub kind: SmartMatchKind,
    pub label: String,
    pub tracks: Vec<Track>,
    pub suggest_shuffle: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LibraryCacheDisk {
    entries: HashMap<String, Track>,
}

/// Indexed local music library with incremental disk cache, tree projection, and smart search.
#[derive(Debug, Clone)]
pub struct MusicLibrary {
    pub root: PathBuf,
    tracks: Vec<Track>,
    playlists: Vec<PathBuf>,
    pub expanded_keys: HashSet<String>,
    pub tree_mode: LibraryTreeMode,
}

impl MusicLibrary {
    pub fn empty(root: PathBuf) -> Self {
        let mut expanded_keys = HashSet::new();
        expanded_keys.insert("dir:".to_string());
        Self {
            root,
            tracks: Vec::new(),
            playlists: Vec::new(),
            expanded_keys,
            tree_mode: LibraryTreeMode::Directories,
        }
    }

    /// Scans `root` recursively, reusing cached metadata for unchanged `(mtime, size)` files.
    pub fn scan_with_cache(root: &Path, cache_file: Option<&Path>) -> Self {
        let old_cache: HashMap<String, Track> = cache_file
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<LibraryCacheDisk>(&s).ok())
            .map(|c| c.entries)
            .unwrap_or_default();

        let mut new_cache = HashMap::new();
        let mut tracks = Vec::new();
        let mut playlists = Vec::new();

        if root.exists() {
            for entry in WalkDir::new(root)
                .follow_links(true)
                .max_depth(16)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                if is_playlist_file(path) {
                    playlists.push(path.to_path_buf());
                    continue;
                }
                if !is_supported_audio_file(path) {
                    continue;
                }

                let (mtime_secs, file_size) = fs::metadata(path)
                    .map(|m| {
                        let mt = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        (mt, m.len())
                    })
                    .unwrap_or((0, 0));

                let key = path.to_string_lossy().to_string();
                let track = if let Some(cached) = old_cache.get(&key) {
                    if cached.mtime_secs == mtime_secs && cached.file_size == file_size && mtime_secs > 0 {
                        cached.clone()
                    } else {
                        extract_track_metadata(path, mtime_secs, file_size)
                    }
                } else {
                    extract_track_metadata(path, mtime_secs, file_size)
                };

                new_cache.insert(key, track.clone());
                tracks.push(track);
            }
        }

        tracks.sort_by(|a, b| {
            natural_cmp(a.display_artist(), b.display_artist())
                .then_with(|| natural_cmp(a.display_album(), b.display_album()))
                .then_with(|| a.album_order_cmp(b))
        });
        playlists.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));

        if let Some(cf) = cache_file {
            if let Some(parent) = cf.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let disk = LibraryCacheDisk { entries: new_cache };
            if let Ok(json) = serde_json::to_string(&disk) {
                let _ = fs::write(cf, json);
            }
        }

        let mut lib = Self {
            root: root.to_path_buf(),
            tracks,
            playlists,
            expanded_keys: HashSet::new(),
            tree_mode: LibraryTreeMode::Directories,
        };
        // Expand top-level directories if library is compact, otherwise keep clean
        lib.seed_default_expansion();
        lib
    }

    fn seed_default_expansion(&mut self) {
        // If all tracks live directly in root, root items are depth 0 automatically.
        // If there are <= 5 top-level folders, leave collapsed so user sees crisp folder tree,
        // or expand if there is only 1 top-level folder.
        let mut top_dirs = HashSet::new();
        for t in &self.tracks {
            if let Ok(rel) = t.path.strip_prefix(&self.root) {
                if let Some(first) = rel.components().next() {
                    if rel.components().count() > 1 {
                        top_dirs.insert(first.as_os_str().to_string_lossy().to_string());
                    }
                }
            }
        }
        if top_dirs.len() == 1 {
            if let Some(only) = top_dirs.into_iter().next() {
                self.expanded_keys.insert(format!("dir:{}", only));
            }
        }
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn playlists(&self) -> &[PathBuf] {
        &self.playlists
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Collects all tracks matching a `LibraryNodeKind` in proper playback order.
    pub fn tracks_for_node(&self, kind: &LibraryNodeKind) -> Vec<Track> {
        match kind {
            LibraryNodeKind::Track(t) => vec![t.clone()],
            LibraryNodeKind::Directory(dir) => {
                let mut list: Vec<Track> = self
                    .tracks
                    .iter()
                    .filter(|t| t.path.starts_with(dir))
                    .cloned()
                    .collect();
                list.sort_by(|a, b| {
                    let pa = a.path.parent().map(|p| p.to_string_lossy()).unwrap_or_default();
                    let pb = b.path.parent().map(|p| p.to_string_lossy()).unwrap_or_default();
                    natural_cmp(&pa, &pb).then_with(|| a.album_order_cmp(b))
                });
                list
            }
            LibraryNodeKind::Artist(artist) => {
                let mut list: Vec<Track> = self
                    .tracks
                    .iter()
                    .filter(|t| t.display_artist().eq_ignore_ascii_case(artist))
                    .cloned()
                    .collect();
                list.sort_by(|a, b| {
                    a.year
                        .unwrap_or(9999)
                        .cmp(&b.year.unwrap_or(9999))
                        .then_with(|| natural_cmp(a.display_album(), b.display_album()))
                        .then_with(|| a.album_order_cmp(b))
                });
                list
            }
            LibraryNodeKind::Album { artist, album } => {
                let mut list: Vec<Track> = self
                    .tracks
                    .iter()
                    .filter(|t| {
                        t.display_artist().eq_ignore_ascii_case(artist)
                            && t.display_album().eq_ignore_ascii_case(album)
                    })
                    .cloned()
                    .collect();
                list.sort_by(|a, b| a.album_order_cmp(b));
                list
            }
            LibraryNodeKind::PlaylistFile(m3u) => {
                if let Ok(paths) = crate::model::Playlist::load_m3u_paths(m3u) {
                    paths
                        .into_iter()
                        .filter(|p| p.exists() && is_supported_audio_file(p))
                        .map(|p| self.resolve_or_extract(&p))
                        .collect()
                } else {
                    Vec::new()
                }
            }
        }
    }

    pub fn resolve_or_extract(&self, path: &Path) -> Track {
        if let Some(found) = self.tracks.iter().find(|t| t.path == path) {
            return found.clone();
        }
        if let Ok(canon) = path.canonicalize() {
            if let Some(found) = self
                .tracks
                .iter()
                .find(|t| t.path.canonicalize().ok().as_ref() == Some(&canon))
            {
                return found.clone();
            }
        }
        extract_track_metadata(path, 0, 0)
    }

    /// Builds the flattened visible tree rows for the Library View (`F3`).
    pub fn visible_tree_items(&self, filter: &str) -> Vec<FlatLibraryItem> {
        let filter_trimmed = filter.trim().to_lowercase();
        match self.tree_mode {
            LibraryTreeMode::Directories => self.build_dir_tree(&filter_trimmed),
            LibraryTreeMode::Artists => self.build_artist_tree(&filter_trimmed),
        }
    }

    fn build_dir_tree(&self, filter: &str) -> Vec<FlatLibraryItem> {
        #[derive(Default)]
        struct DirNode {
            subdirs: BTreeMap<String, DirNode>,
            tracks: Vec<Track>,
            playlists: Vec<PathBuf>,
            full_path: PathBuf,
        }

        let mut root_node = DirNode {
            full_path: self.root.clone(),
            ..Default::default()
        };

        let filtering = !filter.is_empty();

        for track in &self.tracks {
            if filtering && !track_matches_filter(track, filter) {
                continue;
            }
            let rel = track.path.strip_prefix(&self.root).unwrap_or(&track.path);
            let comps: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect();

            if comps.len() <= 1 {
                root_node.tracks.push(track.clone());
            } else {
                let mut cur = &mut root_node;
                let mut acc_path = self.root.clone();
                for dir_name in &comps[..comps.len() - 1] {
                    acc_path = acc_path.join(dir_name);
                    cur = cur.subdirs.entry(dir_name.clone()).or_insert_with(|| DirNode {
                        full_path: acc_path.clone(),
                        ..Default::default()
                    });
                }
                cur.tracks.push(track.clone());
            }
        }

        for pl in &self.playlists {
            let name = pl
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if filtering && !name.contains(filter) {
                continue;
            }
            root_node.playlists.push(pl.clone());
        }

        fn count_tracks(n: &DirNode) -> usize {
            n.tracks.len() + n.subdirs.values().map(count_tracks).sum::<usize>()
        }

        fn flatten_dir(
            node: &DirNode,
            rel_prefix: &str,
            depth: usize,
            expanded: &HashSet<String>,
            force_expand: bool,
            out: &mut Vec<FlatLibraryItem>,
        ) {
            let mut dir_keys: Vec<&String> = node.subdirs.keys().collect();
            dir_keys.sort_by(|a, b| natural_cmp(a, b));

            for dir_name in dir_keys {
                let child = &node.subdirs[dir_name];
                let rel_key = if rel_prefix.is_empty() {
                    dir_name.clone()
                } else {
                    format!("{}/{}", rel_prefix, dir_name)
                };
                let key = format!("dir:{}", rel_key);
                let is_exp = force_expand || expanded.contains(&key);
                let total = count_tracks(child);

                out.push(FlatLibraryItem {
                    key: key.clone(),
                    depth,
                    display_name: dir_name.clone(),
                    sub_info: format!("{} tracks", total),
                    is_container: true,
                    is_expanded: is_exp,
                    kind: LibraryNodeKind::Directory(child.full_path.clone()),
                });

                if is_exp {
                    flatten_dir(child, &rel_key, depth + 1, expanded, force_expand, out);
                }
            }

            let mut sorted_tracks = node.tracks.clone();
            sorted_tracks.sort_by(|a, b| a.album_order_cmp(b));

            for t in sorted_tracks {
                let prefix = t
                    .track_number
                    .map(|n| format!("{:02}. ", n))
                    .unwrap_or_default();
                out.push(FlatLibraryItem {
                    key: format!("trk:{}", t.path.display()),
                    depth,
                    display_name: format!("{}{}", prefix, t.title),
                    sub_info: format!("{} • {}", t.display_artist(), t.duration_formatted()),
                    is_container: false,
                    is_expanded: false,
                    kind: LibraryNodeKind::Track(t),
                });
            }

            for pl in &node.playlists {
                let name = pl
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("playlist.m3u")
                    .to_string();
                out.push(FlatLibraryItem {
                    key: format!("pl:{}", pl.display()),
                    depth,
                    display_name: name,
                    sub_info: "Playlist (.m3u)".to_string(),
                    is_container: false,
                    is_expanded: false,
                    kind: LibraryNodeKind::PlaylistFile(pl.clone()),
                });
            }
        }

        let mut items = Vec::new();
        flatten_dir(
            &root_node,
            "",
            0,
            &self.expanded_keys,
            filtering,
            &mut items,
        );
        items
    }

    fn build_artist_tree(&self, filter: &str) -> Vec<FlatLibraryItem> {
        let filtering = !filter.is_empty();
        let mut tree: BTreeMap<String, BTreeMap<String, Vec<Track>>> = BTreeMap::new();

        for track in &self.tracks {
            if filtering && !track_matches_filter(track, filter) {
                continue;
            }
            tree.entry(track.display_artist().to_string())
                .or_default()
                .entry(track.display_album().to_string())
                .or_default()
                .push(track.clone());
        }

        let mut artist_names: Vec<String> = tree.keys().cloned().collect();
        artist_names.sort_by(|a, b| natural_cmp(a, b));

        let mut out = Vec::new();
        for artist in artist_names {
            let albums = &tree[&artist];
            let artist_key = format!("art:{}", artist);
            let art_expanded = filtering || self.expanded_keys.contains(&artist_key);
            let total_tracks: usize = albums.values().map(|v| v.len()).sum();

            out.push(FlatLibraryItem {
                key: artist_key,
                depth: 0,
                display_name: artist.clone(),
                sub_info: format!("{} albums • {} tracks", albums.len(), total_tracks),
                is_container: true,
                is_expanded: art_expanded,
                kind: LibraryNodeKind::Artist(artist.clone()),
            });

            if art_expanded {
                let mut album_names: Vec<String> = albums.keys().cloned().collect();
                album_names.sort_by(|a, b| natural_cmp(a, b));

                for album in album_names {
                    let mut trks = albums[&album].clone();
                    trks.sort_by(|a, b| a.album_order_cmp(b));
                    let alb_key = format!("alb:{}::{}", artist, album);
                    let alb_expanded = filtering || self.expanded_keys.contains(&alb_key);
                    let year_str = trks
                        .first()
                        .and_then(|t| t.year)
                        .map(|y| format!("{} • ", y))
                        .unwrap_or_default();

                    out.push(FlatLibraryItem {
                        key: alb_key,
                        depth: 1,
                        display_name: album.clone(),
                        sub_info: format!("{}{} tracks", year_str, trks.len()),
                        is_container: true,
                        is_expanded: alb_expanded,
                        kind: LibraryNodeKind::Album {
                            artist: artist.clone(),
                            album: album.clone(),
                        },
                    });

                    if alb_expanded {
                        for t in trks {
                            let prefix = t
                                .track_number
                                .map(|n| format!("{:02}. ", n))
                                .unwrap_or_default();
                            out.push(FlatLibraryItem {
                                key: format!("trk:{}", t.path.display()),
                                depth: 2,
                                display_name: format!("{}{}", prefix, t.title),
                                sub_info: t.duration_formatted(),
                                is_container: false,
                                is_expanded: false,
                                kind: LibraryNodeKind::Track(t),
                            });
                        }
                    }
                }
            }
        }
        out
    }

    /// Returns all albums in random album order while keeping tracks inside each album ordered.
    pub fn all_albums_shuffled(&self) -> Vec<Track> {
        let mut by_album: HashMap<(String, String), Vec<Track>> = HashMap::new();
        for t in &self.tracks {
            by_album
                .entry((
                    t.display_artist().to_lowercase(),
                    t.display_album().to_lowercase(),
                ))
                .or_default()
                .push(t.clone());
        }
        let mut album_groups: Vec<Vec<Track>> = by_album
            .into_values()
            .map(|mut v| {
                v.sort_by(|a, b| a.album_order_cmp(b));
                v
            })
            .collect();
        album_groups.shuffle(&mut thread_rng());
        album_groups.into_iter().flatten().collect()
    }

    /// Resolves a named `.m3u` / `.m3u8` playlist from library or custom playlists directory.
    pub fn find_playlist(&self, name: &str, extra_dir: Option<&Path>, exact: bool) -> Option<PathBuf> {
        let q = name.trim().to_lowercase();
        let mut candidates = self.playlists.clone();
        if let Some(dir) = extra_dir {
            if let Ok(rd) = fs::read_dir(dir) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.is_file() && is_playlist_file(&p) {
                        candidates.push(p);
                    }
                }
            }
        }
        candidates
            .into_iter()
            .filter_map(|p| {
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_lowercase();
                let fname = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_lowercase();
                let sc = score_text(&stem, &q, exact).max(score_text(&fname, &q, exact));
                sc.map(|s| (s, p))
            })
            .max_by_key(|(s, _)| *s)
            .map(|(_, p)| p)
    }

    /// Matches directories only (`cadence dir <query>`).
    pub fn match_directory(&self, query: &str, exact: bool) -> Option<SmartMatch> {
        let q = query.trim().to_lowercase();
        let mut dirs: HashMap<PathBuf, String> = HashMap::new();
        for t in &self.tracks {
            let mut cur = t.path.parent();
            while let Some(p) = cur {
                if p == self.root.as_path() || !p.starts_with(&self.root) {
                    break;
                }
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    dirs.entry(p.to_path_buf())
                        .or_insert_with(|| name.to_string());
                }
                cur = p.parent();
            }
        }

        let best = dirs
            .into_iter()
            .filter_map(|(path, name)| score_text(&name, &q, exact).map(|s| (s, path, name)))
            .max_by_key(|(s, _, _)| *s)?;

        let tracks = self.tracks_for_node(&LibraryNodeKind::Directory(best.1));
        if tracks.is_empty() {
            None
        } else {
            Some(SmartMatch {
                kind: SmartMatchKind::Directory,
                label: best.2,
                tracks,
                suggest_shuffle: false,
            })
        }
    }

    /// Matches songs only (`cadence song <query>`).
    pub fn match_song(&self, query: &str, exact: bool) -> Option<SmartMatch> {
        let q = query.trim().to_lowercase();
        let mut scored: Vec<(i64, Track)> = self
            .tracks
            .iter()
            .filter_map(|t| {
                let stem = t
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                let s1 = score_text(&t.title, &q, exact);
                let s2 = score_text(stem, &q, exact);
                let combined = format!("{} {}", t.display_artist(), t.title);
                let s3 = score_text(&combined, &q, exact).map(|v| v.saturating_sub(10));
                s1.into_iter()
                    .chain(s2)
                    .chain(s3)
                    .max()
                    .map(|score| (score, t.clone()))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.album_order_cmp(&b.1)));
        let top = scored.first()?.1.clone();
        Some(SmartMatch {
            kind: SmartMatchKind::Track,
            label: format!("{} — {}", top.display_artist(), top.title),
            tracks: vec![top],
            suggest_shuffle: false,
        })
    }

    /// Smart multi-entity query (`cadence <query>` or `cadence artist:album:track`).
    pub fn match_smart_query(&self, query: &str, exact: bool) -> Option<SmartMatch> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return None;
        }

        // 1. Colon-chained query: `artist:album` or `artist:album:track`
        if trimmed.contains(':') {
            let parts: Vec<String> = trimmed
                .split(':')
                .map(|s| s.trim().to_lowercase())
                .collect();
            let mut matched: Vec<Track> = self
                .tracks
                .iter()
                .filter(|t| matches_colon_chain(t, &parts, exact, &self.root))
                .cloned()
                .collect();
            if !matched.is_empty() {
                matched.sort_by(|a, b| {
                    natural_cmp(a.display_album(), b.display_album())
                        .then_with(|| a.album_order_cmp(b))
                });
                return Some(SmartMatch {
                    kind: SmartMatchKind::ChainedFilter,
                    label: trimmed.to_string(),
                    tracks: matched,
                    suggest_shuffle: false,
                });
            }
        }

        let q = trimmed.to_lowercase();

        // Score Albums, Artists, Directories, and Tracks and pick the highest-confidence entity
        let mut best_album: Option<(i64, String, String)> = None;
        let mut seen_albums = HashSet::new();
        let mut best_artist: Option<(i64, String)> = None;
        let mut seen_artists = HashSet::new();

        for t in &self.tracks {
            let art = t.display_artist();
            let alb = t.display_album();
            if seen_artists.insert(art.to_lowercase()) {
                if let Some(sc) = score_text(art, &q, exact) {
                    if best_artist.as_ref().map(|b| sc > b.0).unwrap_or(true) {
                        best_artist = Some((sc + 8, art.to_string()));
                    }
                }
            }
            if alb != "Unknown Album" && seen_albums.insert((art.to_lowercase(), alb.to_lowercase())) {
                if let Some(sc) = score_text(alb, &q, exact) {
                    if best_album.as_ref().map(|b| sc > b.0).unwrap_or(true) {
                        best_album = Some((sc + 12, art.to_string(), alb.to_string()));
                    }
                }
            }
        }

        let dir_candidate = self.match_directory(trimmed, exact).and_then(|dm| {
            score_text(&dm.label, &q, exact).map(|sc| (sc + 10, dm))
        });

        let song_candidate = self.match_song(trimmed, exact).and_then(|sm| {
            let top = &sm.tracks[0];
            let stem = top.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let combined = format!("{} {}", top.display_artist(), top.title);
            let sc = score_text(&top.title, &q, exact)
                .into_iter()
                .chain(score_text(stem, &q, exact))
                .chain(score_text(&combined, &q, exact))
                .max()?;
            Some((sc, sm))
        });

        let mut candidates: Vec<(i64, SmartMatch)> = Vec::new();
        if let Some((sc, art, alb)) = best_album {
            let tracks = self.tracks_for_node(&LibraryNodeKind::Album {
                artist: art.clone(),
                album: alb.clone(),
            });
            if !tracks.is_empty() {
                candidates.push((
                    sc,
                    SmartMatch {
                        kind: SmartMatchKind::Album,
                        label: format!("{} — {}", art, alb),
                        tracks,
                        suggest_shuffle: false,
                    },
                ));
            }
        }
        if let Some((sc, art)) = best_artist {
            let tracks = self.tracks_for_node(&LibraryNodeKind::Artist(art.clone()));
            if !tracks.is_empty() {
                let shuffle = tracks.len() > 1;
                candidates.push((
                    sc,
                    SmartMatch {
                        kind: SmartMatchKind::Artist,
                        label: art,
                        tracks,
                        suggest_shuffle: shuffle,
                    },
                ));
            }
        }
        if let Some(dc) = dir_candidate {
            candidates.push(dc);
        }
        if let Some(sc) = song_candidate {
            candidates.push(sc);
        }

        candidates
            .into_iter()
            .max_by_key(|(score, _)| *score)
            .map(|(_, m)| m)
    }

    /// Searches all tracks for interactive Search View (`F5`) returning ranked tracks.
    pub fn search_tracks(&self, query: &str, exact: bool) -> Vec<Track> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.tracks.iter().take(250).cloned().collect();
        }
        if q.contains(':') {
            let parts: Vec<String> = q.split(':').map(|s| s.trim().to_string()).collect();
            let mut list: Vec<Track> = self
                .tracks
                .iter()
                .filter(|t| matches_colon_chain(t, &parts, exact, &self.root))
                .cloned()
                .collect();
            list.sort_by(|a, b| a.album_order_cmp(b));
            return list;
        }

        let mut scored: Vec<(i64, &Track)> = self
            .tracks
            .iter()
            .filter_map(|t| {
                let hay_title = &t.title;
                let hay_artist = t.display_artist();
                let hay_album = t.display_album();
                let full = format!("{} {} {}", hay_artist, hay_album, hay_title);

                let best = score_text(hay_title, &q, exact)
                    .map(|s| s + 25)
                    .into_iter()
                    .chain(score_text(hay_artist, &q, exact).map(|s| s + 15))
                    .chain(score_text(hay_album, &q, exact).map(|s| s + 15))
                    .chain(score_text(&full, &q, exact))
                    .max()?;
                Some((best, t))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.album_order_cmp(b.1)));
        scored.into_iter().take(300).map(|(_, t)| t.clone()).collect()
    }
}

fn track_matches_filter(track: &Track, filter_lower: &str) -> bool {
    track.title.to_lowercase().contains(filter_lower)
        || track.display_artist().to_lowercase().contains(filter_lower)
        || track.display_album().to_lowercase().contains(filter_lower)
        || track
            .path
            .to_string_lossy()
            .to_lowercase()
            .contains(filter_lower)
}

fn matches_colon_chain(track: &Track, parts: &[String], exact: bool, root: &Path) -> bool {
    let rel_path = track
        .path
        .strip_prefix(root)
        .unwrap_or(&track.path)
        .to_string_lossy()
        .to_lowercase();
    match parts.len() {
        2 => {
            let p0 = &parts[0];
            let p1 = &parts[1];
            let m0 = p0.is_empty()
                || score_text(track.display_artist(), p0, exact).is_some()
                || score_text(track.display_album(), p0, exact).is_some()
                || score_text(&rel_path, p0, exact).is_some();
            let m1 = p1.is_empty()
                || score_text(track.display_album(), p1, exact).is_some()
                || score_text(&track.title, p1, exact).is_some()
                || score_text(&rel_path, p1, exact).is_some();
            m0 && m1
        }
        n if n >= 3 => {
            let p0 = &parts[0];
            let p1 = &parts[1];
            let p2 = &parts[2];
            let m0 = p0.is_empty()
                || score_text(track.display_artist(), p0, exact).is_some()
                || score_text(&rel_path, p0, exact).is_some();
            let m1 = p1.is_empty()
                || score_text(track.display_album(), p1, exact).is_some()
                || score_text(&rel_path, p1, exact).is_some();
            let m2 = p2.is_empty()
                || score_text(&track.title, p2, exact).is_some()
                || score_text(&rel_path, p2, exact).is_some();
            m0 && m1 && m2
        }
        _ => true,
    }
}

/// Scores how well `target` matches lowercase `query`. Higher is better.
pub fn score_text(target: &str, query_lower: &str, exact_only: bool) -> Option<i64> {
    if query_lower.is_empty() {
        return Some(0);
    }
    let t = target.to_lowercase();
    if t == query_lower {
        return Some(1000);
    }
    if t.starts_with(query_lower) {
        return Some(850 - (t.len() as i64 - query_lower.len() as i64).min(150));
    }
    if let Some(pos) = t.find(query_lower) {
        // Bonus if match starts on a word boundary
        let boundary = pos == 0
            || t.as_bytes()
                .get(pos - 1)
                .map(|b| !b.is_ascii_alphanumeric())
                .unwrap_or(true);
        let base = if boundary { 720 } else { 580 };
        return Some(base - (pos as i64).min(120));
    }
    if exact_only {
        return None;
    }

    // Multi-token all-words match (e.g. "mladost arana" matching "Jazz at Mladost Club - Arana")
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();
    if tokens.len() > 1 && tokens.iter().all(|tok| t.contains(tok)) {
        return Some(520);
    }

    // Fuzzy subsequence score
    let mut score: i64 = 0;
    let mut t_chars = t.chars().enumerate().peekable();
    let mut prev_idx: Option<usize> = None;

    for qc in query_lower.chars() {
        if qc.is_whitespace() {
            continue;
        }
        let mut found = None;
        while let Some((idx, tc)) = t_chars.next() {
            if tc == qc {
                found = Some(idx);
                break;
            }
        }
        let idx = found?;
        if let Some(p) = prev_idx {
            if idx == p + 1 {
                score += 18;
            } else {
                score -= ((idx - p) as i64).min(8);
            }
        } else if idx == 0 {
            score += 25;
        }
        score += 10;
        prev_idx = Some(idx);
    }

    if score >= (query_lower.len() as i64 * 8) {
        Some(200 + score)
    } else {
        None
    }
}
