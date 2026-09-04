//! A note for every song.
//!
//! One JSON file per account in the state directory, keyed by Spotify URI
//! because that is what Fastpotify identifies everything by and a track id
//! is optional. Each note carries the track's name, artists, album, cover,
//! and length so the notes page draws from disk without asking Spotify
//! about songs it may no longer be able to reach.

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
    #[serde(default)]
    pub updated_at: String,
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
        self.notes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.notes.len()
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

    /// Writes `text` down for `uri`. An empty note is no note: it is
    /// removed, the way clearing the field in the web app removes it.
    pub fn set(&mut self, uri: &str, text: &str, track: TrackInfo, now: jiff::Timestamp) {
        let text = text.trim();
        if text.is_empty() {
            self.remove(uri);
            return;
        }
        let note = Note {
            text: text.to_string(),
            updated_at: now.to_string(),
            track,
        };
        if self.notes.get(uri) == Some(&note) {
            return;
        }
        self.notes.insert(uri.to_string(), note);
        self.dirty = true;
    }

    pub fn remove(&mut self, uri: &str) {
        if self.notes.remove(uri).is_some() {
            self.dirty = true;
        }
    }

    /// Every note, newest first. Notes with the same time keep a stable
    /// order by URI so the page does not shuffle between frames.
    pub fn newest_first(&self) -> Vec<(&str, &Note)> {
        let mut rows: Vec<(&str, &Note)> = self
            .notes
            .iter()
            .map(|(uri, note)| (uri.as_str(), note))
            .collect();
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

/// One row of the export My Song Notes writes from its Settings page.
///
/// The export carries `track_name`, `note_html` and `note_text`; the
/// aliases accept a raw dump of the app's own list endpoint too, which
/// names the same fields `name` and `note`.
#[derive(serde::Deserialize)]
struct Exported {
    track_id: String,
    #[serde(default, alias = "name")]
    track_name: String,
    #[serde(default)]
    artists: Vec<String>,
    /// Already plain text, and preferred when it is there.
    #[serde(default)]
    note_text: Option<String>,
    #[serde(default, alias = "note")]
    note_html: Option<String>,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    image_url: Option<String>,
}

#[derive(serde::Deserialize)]
struct Export {
    #[serde(default)]
    notes: Vec<Exported>,
}

/// Reads an export from My Song Notes into `notes`, the newer of the two
/// notes for a song winning, and answers how many landed.
///
/// The export has no album and no track length, so those stay empty until
/// the song plays here and the note is written again.
pub fn import(notes: &mut Notes, path: &Path) -> anyhow::Result<usize> {
    let text = std::fs::read_to_string(path)?;
    let export: Export = serde_json::from_str(&text)?;
    let now = jiff::Timestamp::now().to_string();
    let mut landed = 0;
    for row in export.notes {
        if row.track_id.is_empty() {
            continue;
        }
        let written = match row.note_text {
            Some(text) if !text.trim().is_empty() => text.trim().to_string(),
            _ => strip_html(row.note_html.as_deref().unwrap_or_default()),
        };
        if written.is_empty() {
            continue;
        }
        let uri = format!("spotify:track:{}", row.track_id);
        let updated_at = if row.updated_at.trim().is_empty() {
            now.clone()
        } else {
            row.updated_at
        };
        let note = Note {
            text: written,
            updated_at,
            track: TrackInfo {
                title: row.track_name,
                artists: row.artists,
                album: String::new(),
                art_url: row.image_url.filter(|url| !url.is_empty()),
                duration_ms: 0,
            },
        };
        if notes.merge(&uri, note) {
            landed += 1;
        }
    }
    Ok(landed)
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

    /// An import only replaces a note that is older than the one imported.
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
            track: info(),
        };
        assert!(!notes.merge("spotify:track:a", older));
        assert_eq!(notes.text("spotify:track:a"), "here");
        let newer = Note {
            text: "from the web".into(),
            updated_at: "2026-09-01T00:00:00Z".into(),
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

    /// The export from the web app's Settings page lands as notes.
    #[test]
    fn an_export_from_the_web_app_lands_as_notes() {
        let dir = std::env::temp_dir().join(format!("fastpotify-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song-notes.json");
        std::fs::write(
            &path,
            r#"{
              "exported_at": "2026-09-03T18:30:00.000Z",
              "count": 3,
              "notes": [
                {
                  "track_id": "4uLU6hMCjMI75M1A2tKUQC",
                  "track_name": "Long Way Home",
                  "artists": ["Marconi Union"],
                  "spotify_url": "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC",
                  "note_html": "<p>drop at <b>1:04</b></p>",
                  "note_text": "drop at 1:04",
                  "updated_at": "2026-09-03T18:22:10.512Z"
                },
                {
                  "track_id": "onlyhtml",
                  "track_name": "Tides",
                  "artists": [],
                  "note_html": "<p>second verse</p>",
                  "updated_at": "2026-08-01T00:00:00.000Z"
                },
                {
                  "track_id": "empty",
                  "track_name": "Nothing",
                  "note_html": "<p><br></p>",
                  "updated_at": "2026-08-01T00:00:00.000Z"
                }
              ]
            }"#,
        )
        .unwrap();

        let mut notes = Notes::default();
        assert_eq!(
            import(&mut notes, &path).unwrap(),
            2,
            "the blank one is not a note"
        );
        assert_eq!(
            notes.text("spotify:track:4uLU6hMCjMI75M1A2tKUQC"),
            "drop at 1:04"
        );
        assert_eq!(
            notes.text("spotify:track:onlyhtml"),
            "second verse",
            "html is used when there is no plain text"
        );
        let note = notes.get("spotify:track:4uLU6hMCjMI75M1A2tKUQC").unwrap();
        assert_eq!(note.track.title, "Long Way Home");
        assert_eq!(note.track.artists, vec!["Marconi Union".to_string()]);

        // Importing the same file again changes nothing.
        assert_eq!(import(&mut notes, &path).unwrap(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An import never overwrites a note written here more recently, even
    /// though the web app's times carry fractional seconds and ours do not.
    #[test]
    fn an_import_leaves_a_newer_local_note_alone() {
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
            track: info(),
        };
        assert!(!notes.merge("spotify:track:a", older));
        let newer = Note {
            text: "from the web".into(),
            updated_at: "2026-09-03T18:22:11.001Z".into(),
            track: info(),
        };
        assert!(notes.merge("spotify:track:a", newer));
    }

    /// The web editor's markup comes out as the lines it stood for.
    #[test]
    fn imported_html_becomes_plain_lines() {
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
