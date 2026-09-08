//! Whatever is playing, over MPRIS.
//!
//! MPRIS is the desktop-independent way to ask "what is playing" on Linux — the
//! same interface KDE's media applet, GNOME's, and `playerctl` all use — so this
//! works for Spotify, VLC, mpv, Firefox and Chromium alike without knowing any of
//! them by name, and without a helper binary on PATH.
//!
//! The ring shows track progress; clicking it is `PlayPause`.

use std::collections::HashMap;
use std::ops::Deref;
use zbus::blocking::{Connection, Proxy, fdo::DBusProxy};
use zbus::zvariant::{OwnedValue, Value};

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PREFIX: &str = "org.mpris.MediaPlayer2.";

#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    /// Bus name, kept so the click acts on the player we drew.
    pub bus: String,
    /// "Spotify", "VLC media player" — falls back to the bus suffix.
    pub identity: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// 0.0–1.0 through the track; 0 when the player publishes no length
    /// (live streams, most browser tabs).
    pub progress: f64,
    /// True when a length was published, so the ring can tell "at the start"
    /// apart from "no idea".
    pub has_progress: bool,
    /// Seconds, for the card's "2:14 / 5:03".
    pub position: f64,
    pub length: f64,
    /// The player's own desktop-entry name, used to draw its application icon.
    pub desktop_entry: String,
}

fn as_str(v: &OwnedValue) -> Option<String> {
    match v.deref() {
        Value::Str(s) => Some(s.to_string()),
        // xesam:artist is a list; the first entry is the one anybody reads.
        Value::Array(a) => a.iter().find_map(|x| match x {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        }),
        _ => None,
    }
}

fn as_i64(v: &OwnedValue) -> Option<i64> {
    match v.deref() {
        Value::I64(n) => Some(*n),
        Value::U64(n) => Some(*n as i64),
        Value::I32(n) => Some(*n as i64),
        Value::U32(n) => Some(*n as i64),
        Value::F64(n) => Some(*n as i64),
        _ => None,
    }
}

fn player_proxy<'a>(conn: &Connection, bus: &str) -> Option<Proxy<'a>> {
    Proxy::new(conn, bus.to_string(), PATH, PLAYER).ok()
}

/// Every MPRIS player currently on the session bus.
fn buses(conn: &Connection) -> Vec<String> {
    let Ok(dbus) = DBusProxy::new(conn) else {
        return Vec::new();
    };
    let Ok(names) = dbus.list_names() else {
        return Vec::new();
    };
    names
        .into_iter()
        .map(|n| n.to_string())
        .filter(|n| n.starts_with(PREFIX))
        .collect()
}

/// The player worth drawing: one that is playing, else one that is paused.
///
/// Deliberately not "the most recently used" — MPRIS publishes no such thing, and
/// guessing it from bus-name order changes the ring under the user for no reason.
pub fn poll(conn: &Connection) -> Option<Media> {
    let mut paused: Option<Media> = None;
    for bus in buses(conn) {
        let Some(p) = player_proxy(conn, &bus) else {
            continue;
        };
        let status: String = p.get_property("PlaybackStatus").unwrap_or_default();
        if status == "Stopped" || status.is_empty() {
            continue;
        }
        let meta: HashMap<String, OwnedValue> = p.get_property("Metadata").unwrap_or_default();

        let length = meta.get("mpris:length").and_then(as_i64).unwrap_or(0);
        let position: i64 = p.get_property("Position").unwrap_or(0);
        let has_progress = length > 0;

        let root = Proxy::new(conn, bus.clone(), PATH, ROOT).ok();
        let m = Media {
            identity: root
                .as_ref()
                .and_then(|r| r.get_property::<String>("Identity").ok())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| bus.trim_start_matches(PREFIX).to_string()),
            desktop_entry: root
                .as_ref()
                .and_then(|r| r.get_property::<String>("DesktopEntry").ok())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| bus.trim_start_matches(PREFIX).to_string()),
            title: meta
                .get("xesam:title")
                .and_then(as_str)
                .unwrap_or_else(|| "Unknown track".into()),
            artist: meta
                .get("xesam:artist")
                .and_then(as_str)
                .unwrap_or_default(),
            playing: status == "Playing",
            progress: if has_progress {
                (position as f64 / length as f64).clamp(0.0, 1.0)
            } else {
                0.0
            },
            has_progress,
            // MPRIS counts in microseconds.
            position: position as f64 / 1e6,
            length: length as f64 / 1e6,
            bus,
        };
        if m.playing {
            return Some(m); // a playing player always wins; stop looking
        }
        paused.get_or_insert(m);
    }
    paused
}

/// Toggle the player the ring was drawn for. Opens its own connection: this runs on
/// the GTK thread from a click, and borrowing the poller's connection across threads
/// would mean a lock held around a blocking D-Bus round trip.
pub fn play_pause(bus: &str) -> Result<(), zbus::Error> {
    let conn = Connection::session()?;
    let p = Proxy::new(&conn, bus.to_string(), PATH, PLAYER)?;
    p.call::<_, _, ()>("PlayPause", &())?;
    Ok(())
}

/// "Artist — Title", or just the title when the player publishes no artist.
pub fn describe(m: &Media) -> String {
    if m.artist.is_empty() {
        m.title.clone()
    } else {
        format!("{} — {}", m.artist, m.title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a live session bus with a player on it, so it is not part of the
    /// default run: `cargo test -- --ignored`. Leaves the player as it found it.
    #[test]
    #[ignore]
    fn play_pause_reaches_a_real_player() {
        let conn = Connection::session().expect("no session bus");
        let before = poll(&conn).expect("no MPRIS player running");
        play_pause(&before.bus).expect("call failed");
        std::thread::sleep(std::time::Duration::from_millis(700));
        let after = poll(&conn).expect("player vanished mid-test");
        play_pause(&before.bus).expect("restore failed");
        assert_ne!(before.playing, after.playing, "PlayPause did not toggle");
    }
}
