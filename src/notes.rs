//! A note for every song.
//!
//! The notes themselves live in My Song Notes, so they are the same notes
//! on every device. See [`crate::notes_sync`]. The JSON file per account in
//! the state directory is a cache and an outbox: it shows the notes before
//! the first answer comes back, and it holds an edit made with no network
//! until the server has taken it.
//!
//! Keyed by Spotify URI because that is what Fastpotify identifies
//! everything by. Each note carries the track's name, artists, album,
//! cover, and length so the notes page draws from disk without asking
//! Spotify about songs it may no longer be able to reach.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The format of the file. Bumped only if a later version has to be told
/// apart from this one; unknown fields are kept out of the way by serde.
const VERSION: u32 = 1;

/// What a note remembers about the song it belongs to.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackInfo {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub art_url: Option<String>,
    #[serde(default)]
    pub duration_ms: u32,
}

/// One song's note.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Note {
    #[serde(default)]
    pub text: String,
    /// RFC 3339, the same shape as every other time Fastpotify writes down.
    /// This is the server's time for the note, not this computer's, because
    /// it is what the next write offers back as the version it saw.
    #[serde(default)]
    pub updated_at: String,
    /// Written here and not yet taken by the server. An unsent note is
    /// never replaced by what the server has.
    #[serde(default)]
    pub pending: bool,
    #[serde(flatten)]
    pub track: TrackInfo,
}

impl Note {
    /// The first line with anything on it, for the notes page.
    pub fn summary(&self) -> &str {
        self.text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    notes: HashMap<String, Note>,
}

/// Every note this account has written, by Spotify URI.
#[derive(Debug, Default)]
pub struct Notes {
    notes: HashMap<String, Note>,
    /// Set when the notes in memory differ from the file.
    dirty: bool,
}

impl Notes {
    /// Reads the notes, or returns none if the file is missing or unreadable.
    /// An unreadable file is left alone rather than overwritten.
    pub fn load(path: &Path) -> Self {
        let notes = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<File>(&text) {
                Ok(file) => file.notes,
                Err(error) => {
                    log::warn!("could not read the notes in {}: {error}", path.display());
                    HashMap::new()
                }
            },
            Err(_) => HashMap::new(),
        };
        Self {
            notes,
            dirty: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.written().next().is_none()
    }

    pub fn len(&self) -> usize {
        self.written().count()
    }

    /// The notes with words in them. A note emptied here is kept as an
    /// empty one until the server has been told, and that is not a note
    /// anybody wants to look at.
    fn written(&self) -> impl Iterator<Item = (&str, &Note)> {
        self.notes
            .iter()
            .filter(|(_, note)| !note.text.is_empty())
            .map(|(uri, note)| (uri.as_str(), note))
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The note written for `uri`, if there is one.
    pub fn get(&self, uri: &str) -> Option<&Note> {
        self.notes.get(uri)
    }

    /// The text written for `uri`, empty when nothing has been.
    pub fn text(&self, uri: &str) -> &str {
        self.notes
            .get(uri)
            .map(|note| note.text.as_str())
            .unwrap_or_default()
    }

    /// Writes `text` down for `uri`, to be sent on. An empty note is no
    /// note: it is removed, the way clearing the field in the web app
    /// removes it.
    ///
    /// `now` is only the time a note nobody has seen yet is stamped with,
    /// so it sorts sensibly before the server answers. A note the server
    /// already knows keeps the time the server gave it, which is the
    /// version the next write offers back.
    pub fn set(&mut self, uri: &str, text: &str, track: TrackInfo, now: jiff::Timestamp) {
        let text = text.trim();
        if text.is_empty() {
            self.remove(uri);
            return;
        }
        let updated_at = self
            .notes
            .get(uri)
            .map(|note| note.updated_at.clone())
            .filter(|at| !at.is_empty())
            .unwrap_or_else(|| now.to_string());
        let note = Note {
            text: text.to_string(),
            updated_at,
            pending: true,
            track,
        };
        if self.notes.get(uri) == Some(&note) {
            return;
        }
        self.notes.insert(uri.to_string(), note);
        self.dirty = true;
    }

    /// Forgets the note for `uri`. A note that has been to the server is
    /// kept as an empty one until the deletion has been sent, because that
    /// is the only record that it has to be.
    pub fn remove(&mut self, uri: &str) {
        let Some(note) = self.notes.get_mut(uri) else {
            return;
        };
        if note.text.is_empty() {
            return;
        }
        note.text.clear();
        note.pending = true;
        self.dirty = true;
    }

    /// Every note, newest first. Notes with the same time keep a stable
    /// order by URI so the page does not shuffle between frames.
    pub fn newest_first(&self) -> Vec<(&str, &Note)> {
        let mut rows: Vec<(&str, &Note)> = self.written().collect();
        rows.sort_by(|a, b| {
            b.1.updated_at
                .cmp(&a.1.updated_at)
                .then_with(|| a.0.cmp(b.0))
        });
        rows
    }

    /// Writes the notes if they changed since the last save. Written whole
    /// to a temporary file and renamed over the old one, so an interrupted
    /// save cannot leave half a file behind.
    pub fn save(&mut self, path: &Path) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = File {
            version: VERSION,
            notes: self.notes.clone(),
        };
        let text = match serde_json::to_string_pretty(&file) {
            Ok(text) => text,
            Err(error) => {
                log::warn!("unable to encode the notes: {error}");
                return;
            }
        };
        let temporary = path.with_extension("json.tmp");
        let written = std::fs::write(&temporary, text)
            .and_then(|()| crate::settings::replace_file(&temporary, path));
        if let Err(error) = written {
            log::warn!("unable to save the notes to {}: {error}", path.display());
        }
    }

    /// Merges a note in, the newer `updated_at` winning. Returns whether
    /// this note is now the one on file.
    pub fn merge(&mut self, uri: &str, note: Note) -> bool {
        if note.text.trim().is_empty() {
            return false;
        }
        if let Some(held) = self.notes.get(uri)
            && !is_after(&note.updated_at, &held.updated_at)
        {
            return false;
        }
        self.notes.insert(uri.to_string(), note);
        self.dirty = true;
        true
    }

    /// Fills in what a note knows about its song, for a note that knows
    /// nothing about it. A note that already has a title keeps everything
    /// it has: the answer is about the same song, but that note was never
    /// waiting on it. Answers whether the note took it.
    pub fn fill_track(&mut self, uri: &str, track: TrackInfo) -> bool {
        let Some(note) = self.notes.get_mut(uri) else {
            return false;
        };
        if !note.track.title.is_empty() || track.title.is_empty() {
            return false;
        }
        note.track = track;
        self.dirty = true;
        true
    }

    /// Whether anything written here is still waiting for the server.
    pub fn has_pending(&self) -> bool {
        self.notes.values().any(|note| note.pending)
    }

    /// Every note waiting for the server, so it can be offered again.
    pub fn pending(&self) -> Vec<(String, Note)> {
        let mut rows: Vec<(String, Note)> = self
            .notes
            .iter()
            .filter(|(_, note)| note.pending)
            .map(|(uri, note)| (uri.clone(), note.clone()))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// The server took this note. An emptied note is now gone for good;
    /// anything else keeps the time the server gave it back.
    pub fn mark_sent(&mut self, uri: &str, updated_at: &str) {
        let Some(note) = self.notes.get_mut(uri) else {
            return;
        };
        if note.text.is_empty() {
            self.notes.remove(uri);
            self.dirty = true;
            return;
        }
        note.pending = false;
        if !updated_at.is_empty() {
            note.updated_at = updated_at.to_string();
        }
        self.dirty = true;
    }

    /// Puts the server's copy in place whatever is held here. `None` means
    /// the server has no note for this song, so neither do we.
    pub fn take_server(&mut self, uri: &str, incoming: Option<Note>) {
        match incoming {
            Some(mut note) => {
                note.pending = false;
                // The server has no album name and no track length, so a
                // note that has played here keeps what it already knew.
                if let Some(held) = self.notes.get(uri) {
                    if note.track.album.is_empty() {
                        note.track.album = held.track.album.clone();
                    }
                    if note.track.duration_ms == 0 {
                        note.track.duration_ms = held.track.duration_ms;
                    }
                }
                self.notes.insert(uri.to_string(), note);
            }
            None => {
                self.notes.remove(uri);
            }
        }
        self.dirty = true;
    }

    /// The server's copy, unless what is held here has not been sent yet.
    /// Answers whether the server's copy is now the one on file.
    pub fn apply_server(&mut self, uri: &str, incoming: Option<Note>) -> bool {
        if self.notes.get(uri).is_some_and(|note| note.pending) {
            return false;
        }
        self.take_server(uri, incoming);
        true
    }

    /// Replaces everything with the server's list, keeping notes written
    /// here that have not been sent. A note the server does not have and
    /// this computer has already sent was deleted somewhere else, so it
    /// goes.
    pub fn apply_server_list(&mut self, incoming: Vec<(String, Note)>) {
        let mut seen: std::collections::HashSet<String> =
            std::collections::HashSet::with_capacity(incoming.len());
        for (uri, note) in incoming {
            seen.insert(uri.clone());
            self.apply_server(&uri, Some(note));
        }
        let dropped: Vec<String> = self
            .notes
            .iter()
            .filter(|(uri, note)| !note.pending && !seen.contains(uri.as_str()))
            .map(|(uri, _)| uri.clone())
            .collect();
        for uri in dropped {
            self.notes.remove(&uri);
            self.dirty = true;
        }
    }
}

/// The bare track id in a `spotify:track:<id>` URI. Notes are for songs:
/// an episode, an ad, or a local file has nowhere to keep one.
pub fn track_id(uri: &str) -> Option<&str> {
    let id = uri.strip_prefix("spotify:track:")?;
    (!id.is_empty() && !id.contains(':')).then_some(id)
}

/// Whether `candidate` was written after `held`. Compared as instants
/// rather than as text, because the web app's times carry fractional
/// seconds and Fastpotify's do not.
fn is_after(candidate: &str, held: &str) -> bool {
    match (
        candidate.parse::<jiff::Timestamp>(),
        held.parse::<jiff::Timestamp>(),
    ) {
        (Ok(candidate), Ok(held)) => candidate > held,
        _ => candidate > held,
    }
}

/// Every `m:ss` or `mm:ss` in `text`, in the order written, each with the
/// position it points at. Repeats are listed once: the chips are a set of
/// places in the song, not a count of how often each was typed.
///
/// The bounds are the web app's: a colon inside a word is not a timestamp,
/// and neither is `1:99`. Written by hand because the regex crate is not a
/// dependency and this is the whole grammar.
pub fn timestamps(text: &str) -> Vec<(String, u32)> {
    let bytes = text.as_bytes();
    let digit = |index: usize| bytes.get(index).is_some_and(u8::is_ascii_digit);
    // A word character on either side means this is part of something else.
    let word = |index: usize| {
        bytes
            .get(index)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    };
    let mut found: Vec<(String, u32)> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b':' {
            at += 1;
            continue;
        }
        // The seconds: exactly two digits, the first at most five.
        if !(matches!(bytes.get(at + 1), Some(b'0'..=b'5')) && digit(at + 2)) || word(at + 3) {
            at += 1;
            continue;
        }
        // The minutes: one or two digits, and a word boundary before them.
        let mut start = at;
        while start > 0 && at - start < 2 && bytes[start - 1].is_ascii_digit() {
            start -= 1;
        }
        if start == at || (start > 0 && word(start - 1)) {
            at += 1;
            continue;
        }
        let label = &text[start..at + 3];
        let minutes: u32 = text[start..at].parse().unwrap_or(0);
        let seconds: u32 = text[at + 1..at + 3].parse().unwrap_or(0);
        if !found.iter().any(|(held, _)| held == label) {
            found.push((label.to_string(), (minutes * 60 + seconds) * 1000));
        }
        at += 3;
    }
    found
}

/// Turns the web app's stored HTML into the plain text this editor holds.
///
/// [`crate::util::strip_html`] flattens a playlist description to one line;
/// a note keeps its line breaks, so the block tags the browser editor
/// leaves behind become newlines here.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            // An unclosed bracket is just text.
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let inside = &after[..end];
        let closing = inside.starts_with('/');
        let tag = inside
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        // A line ends where a block ends, and wherever a break is asked for.
        // The tag that opens a block is not a line of its own.
        if tag == "br"
            || (closing && matches!(tag.as_str(), "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3"))
        {
            out.push('\n');
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    let out = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        // Ampersands last, so `&amp;lt;` does not become a bracket.
        .replace("&amp;", "&");
    // The editor opens and closes a paragraph around every line, so runs of
    // empty lines are the markup's, not the writer's.
    let mut lines: Vec<&str> = Vec::new();
    for line in out.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() && lines.last().is_some_and(|last: &&str| last.is_empty()) {
            continue;
        }
        lines.push(if line.trim().is_empty() { "" } else { line });
    }
    lines.join("\n").trim().to_string()
}

/// Turns the note held here into the HTML the web app stores.
///
/// The desktop editor is plain text, so a note written here is plain text
/// with line breaks. Bold and italic written in the browser survive being
/// read here, but not being written back: the note becomes what the
/// editor showed.
pub fn text_to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br>"),
            '\r' => {}
            _ => out.push(character),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> TrackInfo {
        TrackInfo {
            title: "Long Way Home".into(),
            artists: vec!["Marconi Union".into()],
            album: "Weightless".into(),
            art_url: Some("https://i.scdn.co/image/abc".into()),
            duration_ms: 214_000,
        }
    }

    fn now() -> jiff::Timestamp {
        "2026-09-03T18:22:10Z".parse().unwrap()
    }

    /// A note that arrived with nothing about its song takes what Spotify
    /// says. A note that already knows its song is left alone.
    #[test]
    fn only_a_note_with_no_song_takes_one() {
        let mut notes = Notes::default();
        notes.set("spotify:track:known", "already knows", info(), now());
        notes.set(
            "spotify:track:blank",
            "knows nothing",
            TrackInfo::default(),
            now(),
        );
        let dir =
            std::env::temp_dir().join(format!("fastpotify-notes-fill-{}", std::process::id()));
        let path = dir.join("notes.json");
        notes.save(&path);
        assert!(!notes.is_dirty(), "everything is on disk to start with");

        let looked_up = TrackInfo {
            title: "Caramelldansen".into(),
            artists: vec!["Caramella Girls".into()],
            album: "Speedy Mixes".into(),
            art_url: Some("https://i.scdn.co/image/xyz".into()),
            duration_ms: 175_000,
        };
        assert!(notes.fill_track("spotify:track:blank", looked_up.clone()));
        assert_eq!(
            notes.get("spotify:track:blank").unwrap().track,
            looked_up,
            "the song it had no idea about is now on the note"
        );
        assert!(notes.is_dirty(), "the file has to be written again");

        assert!(
            !notes.fill_track("spotify:track:known", looked_up.clone()),
            "a note that already has a title keeps what it has"
        );
        assert_eq!(notes.get("spotify:track:known").unwrap().track, info());
        assert!(
            !notes.fill_track("spotify:track:missing", looked_up),
            "there is no note to fill in"
        );
        assert!(
            !notes.fill_track("spotify:track:blank", TrackInfo::default()),
            "an answer with no title fills nothing in"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// What was written comes back, with the track it belongs to.
    #[test]
    fn a_note_survives_the_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("fastpotify-notes-{}", std::process::id()));
        let path = dir.join("notes.json");
        let mut notes = Notes::default();
        notes.set("spotify:track:a", "drop at 1:04", info(), now());
        notes.save(&path);
        let restored = Notes::load(&path);
        let note = restored.get("spotify:track:a").expect("the note is there");
        assert_eq!(note.text, "drop at 1:04");
        assert_eq!(note.track, info());
        assert_eq!(note.updated_at, now().to_string());
        // The temporary file is renamed, never left behind.
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A saved file is only written when something changed.
    #[test]
    fn saving_twice_writes_once() {
        let dir =
            std::env::temp_dir().join(format!("fastpotify-notes-dirty-{}", std::process::id()));
        let path = dir.join("notes.json");
        let mut notes = Notes::default();
        notes.set("spotify:track:a", "something", info(), now());
        assert!(notes.is_dirty());
        notes.save(&path);
        assert!(!notes.is_dirty());
        notes.set("spotify:track:a", "something", info(), now());
        assert!(!notes.is_dirty(), "the same text is not a change");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Emptying a note removes it rather than keeping a blank row.
    #[test]
    fn a_note_emptied_of_words_is_no_note() {
        let mut notes = Notes::default();
        notes.set("spotify:track:a", "words", info(), now());
        assert_eq!(notes.len(), 1);
        notes.set("spotify:track:a", "   \n ", info(), now());
        assert!(notes.is_empty());
    }

    /// An emptied note is kept, empty, until the server has been told.
    #[test]
    fn an_emptied_note_waits_to_be_deleted_everywhere() {
        let mut notes = Notes::default();
        notes.set("spotify:track:a", "words", info(), now());
        notes.mark_sent("spotify:track:a", "2026-09-03T18:22:10.512Z");
        assert!(!notes.has_pending());
        notes.remove("spotify:track:a");
        assert!(notes.is_empty(), "it is gone from the page");
        assert!(notes.has_pending(), "but the deletion is still to be sent");
        assert_eq!(notes.pending().len(), 1);
        notes.mark_sent("spotify:track:a", "");
        assert!(!notes.has_pending());
        assert!(notes.get("spotify:track:a").is_none());
    }

    /// A note keeps the time the server gave it, so the next write can
    /// offer that back as the version it saw.
    #[test]
    fn a_sent_note_keeps_the_servers_time() {
        let mut notes = Notes::default();
        notes.set("spotify:track:a", "first", info(), now());
        assert!(notes.get("spotify:track:a").unwrap().pending);
        notes.mark_sent("spotify:track:a", "2026-09-03T18:22:10.512Z");
        let note = notes.get("spotify:track:a").unwrap();
        assert!(!note.pending);
        assert_eq!(note.updated_at, "2026-09-03T18:22:10.512Z");
        notes.set("spotify:track:a", "second", info(), now());
        let note = notes.get("spotify:track:a").unwrap();
        assert!(note.pending);
        assert_eq!(
            note.updated_at, "2026-09-03T18:22:10.512Z",
            "an edit does not invent a new version"
        );
    }

    fn server_note(text: &str, at: &str) -> Note {
        Note {
            text: text.into(),
            updated_at: at.into(),
            pending: false,
            track: info(),
        }
    }

    /// A note written here and not yet sent survives the server's list; a
    /// note that has been sent takes the server's words.
    #[test]
    fn unsent_notes_win_and_sent_notes_follow_the_server() {
        let mut notes = Notes::default();
        notes.set(
            "spotify:track:unsent",
            "written on the plane",
            info(),
            now(),
        );
        notes.set("spotify:track:sent", "old words", info(), now());
        notes.mark_sent("spotify:track:sent", "2026-09-01T00:00:00Z");

        notes.apply_server_list(vec![
            (
                "spotify:track:unsent".into(),
                server_note("what the server has", "2026-09-02T00:00:00Z"),
            ),
            (
                "spotify:track:sent".into(),
                server_note("new words", "2026-09-04T00:00:00Z"),
            ),
        ]);

        assert_eq!(
            notes.text("spotify:track:unsent"),
            "written on the plane",
            "an unsent note is never overwritten"
        );
        assert!(notes.get("spotify:track:unsent").unwrap().pending);
        assert_eq!(notes.text("spotify:track:sent"), "new words");
        assert_eq!(
            notes.get("spotify:track:sent").unwrap().updated_at,
            "2026-09-04T00:00:00Z"
        );
    }

    /// A note the server no longer has was deleted on another device, so
    /// it goes here too, unless it is the one still waiting to be sent.
    #[test]
    fn a_note_deleted_elsewhere_goes_here_too() {
        let mut notes = Notes::default();
        notes.set("spotify:track:gone", "deleted on the phone", info(), now());
        notes.mark_sent("spotify:track:gone", "2026-09-01T00:00:00Z");
        notes.set("spotify:track:mine", "still going", info(), now());

        notes.apply_server_list(Vec::new());

        assert!(notes.get("spotify:track:gone").is_none());
        assert_eq!(notes.text("spotify:track:mine"), "still going");
    }

    /// Notes are for songs, so only a track URI has an id to key one by.
    #[test]
    fn only_a_song_has_somewhere_to_keep_a_note() {
        assert_eq!(
            track_id("spotify:track:4uLU6hMCjMI75M1A2tKUQC"),
            Some("4uLU6hMCjMI75M1A2tKUQC")
        );
        assert_eq!(track_id("spotify:episode:abc"), None);
        assert_eq!(track_id("spotify:track:"), None);
        assert_eq!(track_id("spotify:local:a:b:c:1"), None);
        assert_eq!(track_id("4uLU6hMCjMI75M1A2tKUQC"), None);
    }

    /// What the editor holds becomes markup the browser can show.
    #[test]
    fn plain_lines_become_the_web_editors_markup() {
        assert_eq!(text_to_html("drop at 1:04"), "drop at 1:04");
        assert_eq!(text_to_html("one\ntwo"), "one<br>two");
        assert_eq!(
            text_to_html("Tom & Jerry \"live\""),
            "Tom &amp; Jerry &quot;live&quot;"
        );
        assert_eq!(text_to_html("2 < 3 > 1"), "2 &lt; 3 &gt; 1");
        assert_eq!(text_to_html("one\r\ntwo"), "one<br>two", "no stray returns");
    }

    /// A note written here, sent, and read back is the same note.
    #[test]
    fn a_note_survives_the_round_trip_through_the_server() {
        for written in [
            "drop at 1:04",
            "one\ntwo\nthree",
            "Tom & Jerry \"live\", 2 < 3",
            "a line\n\na paragraph later",
        ] {
            assert_eq!(strip_html(&text_to_html(written)), written);
        }
    }

    /// A file written by a later version keeps the fields this one knows.
    #[test]
    fn a_file_with_fields_from_the_future_still_loads() {
        let dir =
            std::env::temp_dir().join(format!("fastpotify-notes-future-{}", std::process::id()));
        let path = dir.join("notes.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            r#"{
              "version": 7,
              "mood_rings": true,
              "notes": {
                "spotify:track:a": {
                  "text": "kept",
                  "updated_at": "2026-09-03T18:22:10Z",
                  "title": "Long Way Home",
                  "colour": "teal"
                }
              }
            }"#,
        )
        .unwrap();
        let notes = Notes::load(&path);
        let note = notes.get("spotify:track:a").expect("the note is there");
        assert_eq!(note.text, "kept");
        assert_eq!(note.track.title, "Long Way Home");
        assert!(
            note.track.artists.is_empty(),
            "a missing field is a default"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The newest note leads the list.
    #[test]
    fn the_newest_note_comes_first() {
        let mut notes = Notes::default();
        notes.set(
            "spotify:track:old",
            "older",
            info(),
            "2026-01-01T00:00:00Z".parse().unwrap(),
        );
        notes.set(
            "spotify:track:new",
            "newer",
            info(),
            "2026-06-01T00:00:00Z".parse().unwrap(),
        );
        let uris: Vec<&str> = notes.newest_first().iter().map(|(uri, _)| *uri).collect();
        assert_eq!(uris, vec!["spotify:track:new", "spotify:track:old"]);
    }

    /// Merging only replaces a note that is older than the one merged in.
    #[test]
    fn merging_keeps_whichever_note_is_newer() {
        let mut notes = Notes::default();
        notes.set(
            "spotify:track:a",
            "here",
            info(),
            "2026-06-01T00:00:00Z".parse().unwrap(),
        );
        let older = Note {
            text: "from the web".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            pending: false,
            track: info(),
        };
        assert!(!notes.merge("spotify:track:a", older));
        assert_eq!(notes.text("spotify:track:a"), "here");
        let newer = Note {
            text: "from the web".into(),
            updated_at: "2026-09-01T00:00:00Z".into(),
            pending: false,
            track: info(),
        };
        assert!(notes.merge("spotify:track:a", newer));
        assert_eq!(notes.text("spotify:track:a"), "from the web");
    }

    /// The same bounds the web app's regex has.
    #[test]
    fn only_real_timestamps_become_chips() {
        let labels = |text: &str| -> Vec<String> {
            timestamps(text)
                .into_iter()
                .map(|(label, _)| label)
                .collect()
        };
        assert_eq!(labels("drop at 1:04"), vec!["1:04"]);
        assert_eq!(labels("0:07 is the count-in"), vec!["0:07"]);
        assert_eq!(labels("12:05 in"), vec!["12:05"]);
        assert!(
            labels("nothing at 1:99").is_empty(),
            "sixty seconds is a minute"
        );
        assert!(
            labels("bar:23 is a word").is_empty(),
            "a colon inside a word"
        );
        assert!(labels("1:234").is_empty(), "three digits is not seconds");
        assert!(
            labels("123:45").is_empty(),
            "hours are not written this way"
        );
        assert_eq!(labels("1:23 and again 1:23"), vec!["1:23"], "listed once");
        assert_eq!(labels("1:04 then 3:30"), vec!["1:04", "3:30"], "in order");
    }

    /// A chip points at the position it names.
    #[test]
    fn a_timestamp_points_at_its_position() {
        assert_eq!(timestamps("10:00")[0].1, 600_000);
        assert_eq!(timestamps("0:07")[0].1, 7_000);
        assert_eq!(timestamps("1:04")[0].1, 64_000);
    }

    /// A note from the web app never overwrites a note written here more
    /// recently, even though the web app's times carry fractional seconds
    /// and Fastpotify's do not.
    #[test]
    fn a_web_note_leaves_a_newer_local_note_alone() {
        let mut notes = Notes::default();
        notes.set(
            "spotify:track:a",
            "written here",
            info(),
            "2026-09-03T18:22:11Z".parse().unwrap(),
        );
        let older = Note {
            text: "from the web".into(),
            updated_at: "2026-09-03T18:22:10.999Z".into(),
            pending: false,
            track: info(),
        };
        assert!(!notes.merge("spotify:track:a", older));
        let newer = Note {
            text: "from the web".into(),
            updated_at: "2026-09-03T18:22:11.001Z".into(),
            pending: false,
            track: info(),
        };
        assert!(notes.merge("spotify:track:a", newer));
    }

    /// The web editor's markup comes out as the lines it stood for.
    #[test]
    fn web_html_becomes_plain_lines() {
        assert_eq!(
            strip_html("<p>drop at <b>1:04</b></p><p>mix out after 3:30</p>"),
            "drop at 1:04\nmix out after 3:30"
        );
        assert_eq!(strip_html("one<br>two"), "one\ntwo");
        assert_eq!(
            strip_html("Tom &amp; Jerry &quot;live&quot;"),
            "Tom & Jerry \"live\""
        );
        assert_eq!(strip_html("&#39;s &nbsp;fine"), "'s  fine");
        assert_eq!(
            strip_html("<div>one</div><div><br></div><div><br></div><div>two</div>"),
            "one\n\ntwo",
            "a run of empty lines is one"
        );
        assert_eq!(strip_html("2 < 3"), "2 < 3", "a stray bracket is text");
    }
}
