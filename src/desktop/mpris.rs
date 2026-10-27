use crate::config::RepeatMode;

/// Commands sent from desktop media keys / MPRIS applets (e.g. `playerctl`, GNOME, KDE) to Cadence.
#[derive(Debug, Clone)]
pub enum MprisCommand {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    SeekMicros(i64),
    SetPositionMicros(i64),
    SetVolume(f64),
    SetShuffle(bool),
    SetLoopStatus(String),
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MprisStateSnapshot {
    pub playback_status: &'static str, // "Playing" | "Paused" | "Stopped"
    pub loop_status: &'static str,     // "None" | "Track" | "Playlist"
    pub shuffle: bool,
    pub volume: f64,
    pub position_us: i64,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_us: i64,
    pub art_url: Option<String>,
}

impl Default for MprisStateSnapshot {
    fn default() -> Self {
        Self {
            playback_status: "Stopped",
            loop_status: "None",
            shuffle: false,
            volume: 0.8,
            position_us: 0,
            track_id: "/org/cadence/track/none".to_string(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration_us: 0,
            art_url: None,
        }
    }
}

impl MprisStateSnapshot {
    pub fn loop_str(mode: RepeatMode) -> &'static str {
        match mode {
            RepeatMode::Off => "None",
            RepeatMode::One => "Track",
            RepeatMode::All => "Playlist",
        }
    }
}

// ============================================================================
// Linux D-Bus MPRIS2 + Desktop Notifications Implementation
// ============================================================================

#[cfg(target_os = "linux")]
mod imp {
    use super::{MprisCommand, MprisStateSnapshot};
    use std::collections::HashMap;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use zbus::blocking::connection::Builder as ConnectionBuilder;
    use zbus::blocking::Connection;
    use zbus::interface;
    use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

    impl MprisStateSnapshot {
        fn build_metadata_map(&self) -> HashMap<String, OwnedValue> {
            let mut map = HashMap::new();
            let obj_path = ObjectPath::try_from(self.track_id.as_str())
                .unwrap_or_else(|_| ObjectPath::from_static_str_unchecked("/org/cadence/track/0"));
            if let Ok(v) = Value::from(obj_path).try_to_owned() {
                map.insert("mpris:trackid".to_string(), v);
            }
            if !self.title.is_empty() {
                if let Ok(v) = Value::from(self.title.as_str()).try_to_owned() {
                    map.insert("xesam:title".to_string(), v);
                }
            }
            if !self.artist.is_empty() {
                if let Ok(v) = Value::from(vec![self.artist.as_str()]).try_to_owned() {
                    map.insert("xesam:artist".to_string(), v);
                }
            }
            if !self.album.is_empty() {
                if let Ok(v) = Value::from(self.album.as_str()).try_to_owned() {
                    map.insert("xesam:album".to_string(), v);
                }
            }
            if self.duration_us > 0 {
                if let Ok(v) = Value::from(self.duration_us).try_to_owned() {
                    map.insert("mpris:length".to_string(), v);
                }
            }
            if let Some(ref url) = self.art_url {
                if let Ok(v) = Value::from(url.as_str()).try_to_owned() {
                    map.insert("mpris:artUrl".to_string(), v);
                }
            }
            map
        }
    }

    struct MprisRoot {
        cmd_tx: Sender<MprisCommand>,
    }

    #[interface(name = "org.mpris.MediaPlayer2")]
    impl MprisRoot {
        fn raise(&self) {}

        fn quit(&self) {
            let _ = self.cmd_tx.send(MprisCommand::Quit);
        }

        #[zbus(property)]
        fn can_quit(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_raise(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn has_track_list(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn identity(&self) -> &str {
            "Cadence"
        }

        #[zbus(property)]
        fn desktop_entry(&self) -> &str {
            "cadence"
        }

        #[zbus(property)]
        fn supported_uri_schemes(&self) -> Vec<&str> {
            vec!["file"]
        }

        #[zbus(property)]
        fn supported_mime_types(&self) -> Vec<&str> {
            vec![
                "audio/mpeg",
                "audio/flac",
                "audio/ogg",
                "audio/opus",
                "audio/mp4",
                "audio/wav",
                "audio/x-wav",
                "audio/aac",
            ]
        }
    }

    struct MprisPlayer {
        cmd_tx: Sender<MprisCommand>,
        state: Arc<Mutex<MprisStateSnapshot>>,
    }

    #[interface(name = "org.mpris.MediaPlayer2.Player")]
    impl MprisPlayer {
        fn next(&self) {
            let _ = self.cmd_tx.send(MprisCommand::Next);
        }

        fn previous(&self) {
            let _ = self.cmd_tx.send(MprisCommand::Previous);
        }

        fn pause(&self) {
            let _ = self.cmd_tx.send(MprisCommand::Pause);
        }

        fn play_pause(&self) {
            let _ = self.cmd_tx.send(MprisCommand::PlayPause);
        }

        fn stop(&self) {
            let _ = self.cmd_tx.send(MprisCommand::Stop);
        }

        fn play(&self) {
            let _ = self.cmd_tx.send(MprisCommand::Play);
        }

        fn seek(&self, offset: i64) {
            let _ = self.cmd_tx.send(MprisCommand::SeekMicros(offset));
        }

        fn set_position(&self, _track_id: OwnedObjectPath, position: i64) {
            let _ = self.cmd_tx.send(MprisCommand::SetPositionMicros(position));
        }

        fn open_uri(&self, _uri: &str) {}

        #[zbus(property)]
        fn playback_status(&self) -> String {
            self.state
                .lock()
                .map(|s| s.playback_status.to_string())
                .unwrap_or_else(|_| "Stopped".to_string())
        }

        #[zbus(property)]
        fn loop_status(&self) -> String {
            self.state
                .lock()
                .map(|s| s.loop_status.to_string())
                .unwrap_or_else(|_| "None".to_string())
        }

        #[zbus(property)]
        fn set_loop_status(&self, status: &str) {
            let _ = self
                .cmd_tx
                .send(MprisCommand::SetLoopStatus(status.to_string()));
        }

        #[zbus(property)]
        fn rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn set_rate(&self, _rate: f64) {}

        #[zbus(property)]
        fn shuffle(&self) -> bool {
            self.state.lock().map(|s| s.shuffle).unwrap_or(false)
        }

        #[zbus(property)]
        fn set_shuffle(&self, shuffle: bool) {
            let _ = self.cmd_tx.send(MprisCommand::SetShuffle(shuffle));
        }

        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, OwnedValue> {
            self.state
                .lock()
                .map(|s| s.build_metadata_map())
                .unwrap_or_default()
        }

        #[zbus(property)]
        fn volume(&self) -> f64 {
            self.state.lock().map(|s| s.volume).unwrap_or(0.8)
        }

        #[zbus(property)]
        fn set_volume(&self, volume: f64) {
            let _ = self.cmd_tx.send(MprisCommand::SetVolume(volume));
        }

        #[zbus(property)]
        fn position(&self) -> i64 {
            self.state.lock().map(|s| s.position_us).unwrap_or(0)
        }

        #[zbus(property)]
        fn minimum_rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn maximum_rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn can_go_next(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_go_previous(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_play(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_pause(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_seek(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_control(&self) -> bool {
            true
        }
    }

    enum OutboundBusEvent {
        SyncProperties(MprisStateSnapshot),
        NotifyTrack {
            title: String,
            body: String,
            icon_url: String,
        },
        Shutdown,
    }

    pub struct MprisBridge {
        cmd_rx: Receiver<MprisCommand>,
        bus_tx: Option<Sender<OutboundBusEvent>>,
        last_published: MprisStateSnapshot,
        active: bool,
    }

    impl MprisBridge {
        pub fn start(enabled: bool) -> Self {
            let (cmd_tx, cmd_rx) = mpsc::channel::<MprisCommand>();
            if !enabled {
                return Self {
                    cmd_rx,
                    bus_tx: None,
                    last_published: MprisStateSnapshot::default(),
                    active: false,
                };
            }

            let (bus_tx, bus_rx) = mpsc::channel::<OutboundBusEvent>();
            let state = Arc::new(Mutex::new(MprisStateSnapshot::default()));
            let state_for_player = Arc::clone(&state);

            let spawned = thread::Builder::new()
                .name("cadence-mpris".to_string())
                .spawn(move || {
                    run_dbus_worker(cmd_tx, bus_rx, state_for_player);
                })
                .is_ok();

            Self {
                cmd_rx,
                bus_tx: if spawned { Some(bus_tx) } else { None },
                last_published: MprisStateSnapshot::default(),
                active: spawned,
            }
        }

        pub fn is_active(&self) -> bool {
            self.active
        }

        pub fn try_recv(&self) -> Option<MprisCommand> {
            self.cmd_rx.try_recv().ok()
        }

        pub fn update_state(&mut self, next: MprisStateSnapshot) {
            let major_changed = self.last_published.playback_status != next.playback_status
                || self.last_published.loop_status != next.loop_status
                || self.last_published.shuffle != next.shuffle
                || (self.last_published.volume - next.volume).abs() > 0.005
                || self.last_published.track_id != next.track_id
                || self.last_published.title != next.title
                || self.last_published.art_url != next.art_url;

            let pos_jumped = (self.last_published.position_us - next.position_us).abs() > 2_000_000;

            if major_changed || pos_jumped {
                self.last_published = next.clone();
                if let Some(ref tx) = self.bus_tx {
                    let _ = tx.send(OutboundBusEvent::SyncProperties(next));
                }
            }
        }

        pub fn send_track_notification(
            &self,
            title: &str,
            artist: &str,
            album: &str,
            icon_url: Option<&str>,
        ) {
            if let Some(ref tx) = self.bus_tx {
                let body = if album.is_empty() || album == "Unknown Album" {
                    artist.to_string()
                } else {
                    format!("{} — {}", artist, album)
                };
                let _ = tx.send(OutboundBusEvent::NotifyTrack {
                    title: title.to_string(),
                    body,
                    icon_url: icon_url.unwrap_or("audio-x-generic").to_string(),
                });
            }
        }
    }

    impl Drop for MprisBridge {
        fn drop(&mut self) {
            if let Some(tx) = self.bus_tx.take() {
                let _ = tx.send(OutboundBusEvent::Shutdown);
            }
        }
    }

    fn run_dbus_worker(
        cmd_tx: Sender<MprisCommand>,
        bus_rx: Receiver<OutboundBusEvent>,
        state: Arc<Mutex<MprisStateSnapshot>>,
    ) {
        let root = MprisRoot {
            cmd_tx: cmd_tx.clone(),
        };
        let player = MprisPlayer {
            cmd_tx,
            state: Arc::clone(&state),
        };

        let primary_name = "org.mpris.MediaPlayer2.cadence";
        let conn_res = ConnectionBuilder::session()
            .and_then(|b| b.name(primary_name))
            .and_then(|b| b.serve_at("/org/mpris/MediaPlayer2", root))
            .and_then(|b| b.serve_at("/org/mpris/MediaPlayer2", player))
            .and_then(|b| b.build());

        let Ok(conn) = conn_res else {
            return;
        };

        let mut notification_id: u32 = 0;

        while let Ok(ev) = bus_rx.recv() {
            match ev {
                OutboundBusEvent::Shutdown => break,
                OutboundBusEvent::SyncProperties(snap) => {
                    let mut changed: HashMap<&str, Value<'_>> = HashMap::new();
                    let metadata = snap.build_metadata_map();
                    if let Ok(mut guard) = state.lock() {
                        *guard = snap.clone();
                    }
                    changed.insert("PlaybackStatus", Value::from(snap.playback_status));
                    changed.insert("LoopStatus", Value::from(snap.loop_status));
                    changed.insert("Shuffle", Value::from(snap.shuffle));
                    changed.insert("Volume", Value::from(snap.volume));
                    changed.insert("Metadata", Value::from(metadata));

                    let _ = conn.emit_signal(
                        None::<&str>,
                        "/org/mpris/MediaPlayer2",
                        "org.freedesktop.DBus.Properties",
                        "PropertiesChanged",
                        &(
                            "org.mpris.MediaPlayer2.Player",
                            changed,
                            Vec::<&str>::new(),
                        ),
                    );
                }
                OutboundBusEvent::NotifyTrack {
                    title,
                    body,
                    icon_url,
                } => {
                    notification_id = dispatch_desktop_notification(
                        &conn,
                        notification_id,
                        &icon_url,
                        &title,
                        &body,
                    );
                }
            }
        }
    }

    fn dispatch_desktop_notification(
        conn: &Connection,
        replaces_id: u32,
        icon: &str,
        summary: &str,
        body: &str,
    ) -> u32 {
        let hints: HashMap<&str, Value<'_>> = HashMap::new();
        let actions: Vec<&str> = Vec::new();
        let reply = conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Cadence",
                replaces_id,
                icon,
                summary,
                body,
                actions,
                hints,
                3500i32,
            ),
        );
        reply
            .ok()
            .and_then(|msg| msg.body().deserialize::<u32>().ok())
            .unwrap_or(replaces_id)
    }
}

// ============================================================================
// Non-Linux Stub Implementation (macOS / Windows / BSD)
// ============================================================================

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;
    use std::sync::mpsc::{self, Receiver};

    pub struct MprisBridge {
        cmd_rx: Receiver<MprisCommand>,
    }

    impl MprisBridge {
        pub fn start(_enabled: bool) -> Self {
            let (_tx, cmd_rx) = mpsc::channel();
            Self { cmd_rx }
        }

        pub fn is_active(&self) -> bool {
            false
        }

        pub fn try_recv(&self) -> Option<MprisCommand> {
            self.cmd_rx.try_recv().ok()
        }

        pub fn update_state(&mut self, _next: MprisStateSnapshot) {}

        pub fn send_track_notification(
            &self,
            _title: &str,
            _artist: &str,
            _album: &str,
            _icon_url: Option<&str>,
        ) {
        }
    }
}

pub use imp::MprisBridge;
