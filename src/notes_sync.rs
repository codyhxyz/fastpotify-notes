//! Notes, kept in My Song Notes.
//!
//! <https://songnotes.codyh.xyz> holds the notes; this computer holds a
//! cache of them. Its API takes the same Spotify access token the rest of
//! Fastpotify uses, as a bearer token, and answers with the account that
//! token belongs to. Anyone holding that token can read and write these
//! notes, which is the trust Fastpotify already places in it.
//!
//! Every request here runs on the runtime in [`crate::backend`]. Nothing in
//! this module touches application state.

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::api::client::{ApiClient, ApiError};

/// Where the notes live.
pub const BASE_URL: &str = "https://songnotes.codyh.xyz";

/// How many notes to ask for at a time. The list endpoint caps this at 200.
const PAGE_SIZE: u32 = 200;
/// A stop against a cursor that never ends.
const MAX_PAGES: usize = 50;
/// How many songs' details go in one `PATCH`. The server caps this at 200.
const PATCH_CHUNK: usize = 200;

/// Why a request did not answer.
#[derive(Clone, Debug)]
pub enum SyncError {
    /// The Spotify sign-in is not good enough for the notes API, even after
    /// a fresh token. Nothing to retry until the person signs in again.
    Unauthorized,
    /// Anything else: no network, a server error, an unreadable answer.
    Failed(String),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => write!(formatter, "the Spotify sign-in was refused"),
            Self::Failed(detail) => write!(formatter, "{detail}"),
        }
    }
}

type Result<T> = std::result::Result<T, SyncError>;

/// One note as the server keeps it, with the song it belongs to.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Row {
    #[serde(default)]
    pub track_id: String,
    /// The note as HTML. `None` when the server has no note for this song.
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub artists: Option<Vec<String>>,
    #[serde(default)]
    pub image_url: Option<String>,
}

impl Row {
    /// The note as the editor here holds it: plain text with line breaks.
    pub fn text(&self) -> String {
        crate::notes::strip_html(self.note.as_deref().unwrap_or_default())
    }

    /// The note as this computer caches it. `None` when the server has
    /// nothing written for the song.
    pub fn note(&self) -> Option<crate::notes::Note> {
        let text = self.text();
        if text.is_empty() {
            return None;
        }
        Some(crate::notes::Note {
            text,
            updated_at: self.updated_at.clone().unwrap_or_default(),
            pending: false,
            track: crate::notes::TrackInfo {
                title: self.name.clone().unwrap_or_default(),
                artists: self.artists.clone().unwrap_or_default(),
                // The server keeps neither, so a note that has played here
                // keeps whatever it already knew. See `Notes::take_server`.
                album: String::new(),
                art_url: self.image_url.clone().filter(|url| !url.is_empty()),
                duration_ms: 0,
            },
        })
    }

    pub fn uri(&self) -> String {
        format!("spotify:track:{}", self.track_id)
    }
}

/// What a note says about its song, so a note written here shows its
/// artwork in the web app's library.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Meta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artists: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist_urls: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_url: Option<String>,
}

/// One song's details on their way to the server, with nothing about the
/// note itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MetaPatch {
    pub track_id: String,
    #[serde(flatten)]
    pub meta: Meta,
}

impl MetaPatch {
    /// What the server should be told about a song, from Spotify's answer.
    /// `None` for a track with no id, which no note can be filed under.
    pub fn for_track(track: &crate::api::models::Track) -> Option<Self> {
        let track_id = track.id.clone().filter(|id| !id.is_empty())?;
        let names: Vec<String> = track
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect();
        let urls: Vec<String> = track
            .artists
            .iter()
            .filter_map(|artist| artist.id.as_deref())
            .map(|id| format!("https://open.spotify.com/artist/{id}"))
            .collect();
        let album = track.album.as_ref();
        Some(Self {
            meta: Meta {
                name: (!track.name.is_empty()).then(|| track.name.clone()),
                artists: (!names.is_empty()).then_some(names),
                artist_urls: (!urls.is_empty()).then_some(urls),
                image_url: track.image(u32::MAX).map(str::to_string),
                track_url: Some(format!("https://open.spotify.com/track/{track_id}")),
                album_url: album
                    .filter(|album| !album.id.is_empty())
                    .map(|album| format!("https://open.spotify.com/album/{}", album.id)),
            },
            track_id,
        })
    }
}

/// A note on its way to the server.
#[derive(Clone, Debug)]
pub struct PutRequest {
    pub track_id: String,
    /// The note as HTML. Empty deletes it, the way emptying the field in
    /// the web app does.
    pub html: String,
    /// The version this computer last saw, so the server can refuse a
    /// write over somebody else's newer one.
    pub expected_updated_at: Option<String>,
    pub meta: Meta,
}

/// How a write ended.
#[derive(Clone, Debug)]
pub enum PutOutcome {
    /// Written. The server's new time for the note.
    Written { updated_at: String },
    /// Somebody else wrote first. The server's copy, already fetched.
    Stale { row: Option<Row> },
}

#[derive(Serialize)]
struct PutBody<'a> {
    track_id: &'a str,
    note: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_updated_at: Option<&'a str>,
    #[serde(flatten)]
    meta: &'a Meta,
}

#[derive(Serialize)]
struct PatchBody<'a> {
    notes: &'a [MetaPatch],
}

#[derive(Deserialize)]
struct PatchAnswer {
    #[serde(default)]
    updated: usize,
}

#[derive(Deserialize)]
struct ListPage {
    #[serde(default)]
    notes: Vec<Row>,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct PutAnswer {
    #[serde(default)]
    updated_at: Option<String>,
}

/// Sends a request with the Spotify access token on it, and once more with
/// a freshly minted one if the notes API refuses the first.
///
/// The token the client holds can be a minute past useful without anything
/// here knowing, and the server can only answer 401 to that. One forced
/// refresh tells the two cases apart: a stale token, or a sign-in that
/// really is finished.
async fn authorized<F>(client: &ApiClient, build: F) -> Result<reqwest::Response>
where
    F: Fn(&str) -> reqwest::RequestBuilder,
{
    let mut forced = false;
    loop {
        let token = client
            .notes_access_token(forced)
            .await
            .map_err(|error| match error {
                ApiError::SignInExpired { .. } | ApiError::NotSignedIn => SyncError::Unauthorized,
                other => SyncError::Failed(other.to_string()),
            })?;
        let response = build(&token)
            .send()
            .await
            .map_err(|error| SyncError::Failed(error.to_string()))?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        if forced {
            return Err(SyncError::Unauthorized);
        }
        forced = true;
    }
}

/// Reads the answer, or names the status that came instead. Never logs or
/// carries the token or the note itself.
async fn decode<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    if !status.is_success() {
        return Err(SyncError::Failed(format!("songnotes answered {status}")));
    }
    response
        .json::<T>()
        .await
        .map_err(|error| SyncError::Failed(format!("songnotes sent something unreadable: {error}")))
}

/// Every note this account has, paging until the server runs out.
pub async fn list(http: &reqwest::Client, client: &ApiClient) -> Result<Vec<Row>> {
    let mut rows: Vec<Row> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let url = format!("{BASE_URL}/api/notes/list");
        let query = cursor.clone();
        let page: ListPage = decode(
            authorized(client, |token| {
                let mut request = http
                    .get(&url)
                    .bearer_auth(token)
                    .query(&[("limit", PAGE_SIZE.to_string())]);
                if let Some(cursor) = &query {
                    request = request.query(&[("cursor", cursor)]);
                }
                request
            })
            .await?,
        )
        .await?;
        rows.extend(page.notes);
        match page.next_cursor {
            Some(next) if !next.is_empty() => cursor = Some(next),
            _ => return Ok(rows),
        }
    }
    log::warn!("notes: songnotes kept paging past {MAX_PAGES} pages, stopping");
    Ok(rows)
}

/// One song's note, or `None` when nothing is written for it.
pub async fn get(
    http: &reqwest::Client,
    client: &ApiClient,
    track_id: &str,
) -> Result<Option<Row>> {
    let url = format!("{BASE_URL}/api/notes");
    let mut row: Row = decode(
        authorized(client, |token| {
            http.get(&url)
                .bearer_auth(token)
                .query(&[("track_id", track_id)])
        })
        .await?,
    )
    .await?;
    if row.note.is_none() {
        return Ok(None);
    }
    // The single-note answer does not repeat the id it was asked about.
    row.track_id = track_id.to_string();
    Ok(Some(row))
}

/// Writes a note. An empty note deletes it.
pub async fn put(
    http: &reqwest::Client,
    client: &ApiClient,
    request: &PutRequest,
) -> Result<PutOutcome> {
    let url = format!("{BASE_URL}/api/notes");
    let body = PutBody {
        track_id: &request.track_id,
        note: &request.html,
        expected_updated_at: request
            .expected_updated_at
            .as_deref()
            .filter(|at| !at.is_empty()),
        meta: &request.meta,
    };
    let response = authorized(client, |token| {
        http.put(&url).bearer_auth(token).json(&body)
    })
    .await?;
    if response.status() == StatusCode::CONFLICT {
        // Somebody wrote from another device between the read and this
        // write. Fetch what they wrote so the editor can show it.
        let row = get(http, client, &request.track_id).await?;
        return Ok(PutOutcome::Stale { row });
    }
    let answer: PutAnswer = decode(response).await?;
    Ok(PutOutcome::Written {
        updated_at: answer.updated_at.unwrap_or_default(),
    })
}

/// Fills in what the server does not know about the songs its notes are
/// for. Notes bulk imported before the web app kept a song's details have
/// no name and no cover, and this is the only write that fixes that: it
/// never carries a note, and the server leaves the note and the time it
/// was written alone.
///
/// Answers how many notes the server actually had. Songs it has no note
/// for are its business to skip, not ours to know about.
pub async fn patch_meta(
    http: &reqwest::Client,
    client: &ApiClient,
    entries: &[MetaPatch],
) -> Result<usize> {
    let url = format!("{BASE_URL}/api/notes");
    let mut updated = 0;
    for chunk in patch_batches(entries) {
        let body = PatchBody { notes: chunk };
        let answer: PatchAnswer = decode(
            authorized(client, |token| {
                http.patch(&url).bearer_auth(token).json(&body)
            })
            .await?,
        )
        .await?;
        updated += answer.updated;
    }
    Ok(updated)
}

/// The songs split into the batches `PATCH /api/notes` takes.
fn patch_batches(entries: &[MetaPatch]) -> Vec<&[MetaPatch]> {
    entries.chunks(PATCH_CHUNK).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list endpoint's rows become the notes this computer caches.
    #[test]
    fn a_server_row_becomes_a_note() {
        let row: Row = serde_json::from_str(
            r#"{
              "track_id": "4uLU6hMCjMI75M1A2tKUQC",
              "note": "<p>drop at <b>1:04</b></p><p>mix out after 3:30</p>",
              "updated_at": "2026-09-03T18:22:10.512Z",
              "name": "Long Way Home",
              "artists": ["Marconi Union"],
              "artist_urls": ["https://open.spotify.com/artist/abc"],
              "image_url": "https://i.scdn.co/image/abc",
              "track_url": "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC",
              "album_url": "https://open.spotify.com/album/abc"
            }"#,
        )
        .expect("the row reads");
        assert_eq!(row.uri(), "spotify:track:4uLU6hMCjMI75M1A2tKUQC");
        let note = row.note().expect("there is a note");
        assert_eq!(note.text, "drop at 1:04\nmix out after 3:30");
        assert_eq!(note.updated_at, "2026-09-03T18:22:10.512Z");
        assert!(!note.pending);
        assert_eq!(note.track.title, "Long Way Home");
        assert_eq!(note.track.artists, vec!["Marconi Union".to_string()]);
        assert_eq!(
            note.track.art_url.as_deref(),
            Some("https://i.scdn.co/image/abc")
        );
    }

    /// A song with nothing written for it answers with a null note.
    #[test]
    fn a_song_with_nothing_written_has_no_note() {
        let row: Row = serde_json::from_str(r#"{"note": null}"#).expect("the row reads");
        assert!(row.note.is_none());
        assert!(row.note().is_none());
        let blank: Row =
            serde_json::from_str(r#"{"track_id": "a", "note": "<p><br></p>"}"#).expect("reads");
        assert!(blank.note().is_none(), "markup with no words is no note");
    }

    /// Every page the cursor points at is read, and the rows all land.
    #[test]
    fn a_list_is_read_across_its_pages() {
        let first: ListPage = serde_json::from_str(
            r#"{
              "notes": [
                {"track_id": "a", "note": "<p>one</p>", "updated_at": "2026-09-03T00:00:00.000Z"},
                {"track_id": "b", "note": "<p>two</p>", "updated_at": "2026-09-02T00:00:00.000Z"}
              ],
              "next_cursor": "2026-09-02T00:00:00.000Z"
            }"#,
        )
        .expect("the first page reads");
        let second: ListPage = serde_json::from_str(
            r#"{
              "notes": [
                {"track_id": "c", "note": "<p>three</p>", "updated_at": "2026-09-01T00:00:00.000Z"}
              ],
              "next_cursor": null
            }"#,
        )
        .expect("the second page reads");

        assert_eq!(
            first.next_cursor.as_deref(),
            Some("2026-09-02T00:00:00.000Z")
        );
        assert!(
            second.next_cursor.is_none(),
            "the last page ends the paging"
        );

        let mut rows = first.notes;
        rows.extend(second.notes);
        let ids: Vec<&str> = rows.iter().map(|row| row.track_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        let texts: Vec<String> = rows.iter().map(Row::text).collect();
        assert_eq!(texts, vec!["one", "two", "three"]);
    }

    /// A backfill carries only the songs, never a note, and goes two
    /// hundred at a time because that is all the server takes.
    #[test]
    fn a_backfill_carries_the_songs_two_hundred_at_a_time() {
        let entries: Vec<MetaPatch> = (0..450)
            .map(|n| MetaPatch {
                track_id: format!("id{n:03}"),
                meta: Meta {
                    name: Some(format!("Song {n}")),
                    ..Default::default()
                },
            })
            .collect();
        let batches = patch_batches(&entries);
        assert_eq!(batches.len(), 3, "450 songs is three requests");
        assert_eq!(batches[0].len(), PATCH_CHUNK);
        assert_eq!(batches[1].len(), PATCH_CHUNK);
        assert_eq!(batches[2].len(), 50);
        assert_eq!(batches[0][0].track_id, "id000");
        assert_eq!(batches[2][49].track_id, "id449");
        assert!(
            patch_batches(&[]).is_empty(),
            "nothing to fill is no request"
        );

        let body = PatchBody {
            notes: &entries[..2],
        };
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&body).unwrap()).unwrap();
        assert_eq!(json["notes"].as_array().unwrap().len(), 2);
        assert_eq!(json["notes"][0]["track_id"], "id000");
        assert_eq!(json["notes"][0]["name"], "Song 0");
        assert!(
            json["notes"][0].get("note").is_none(),
            "a backfill never carries the note"
        );
        assert!(
            json["notes"][0].get("expected_updated_at").is_none(),
            "a backfill never carries a version to write over"
        );
        assert!(
            json["notes"][0].get("image_url").is_none(),
            "what is unknown is not sent, so the server keeps what it has"
        );
    }

    /// Spotify's answer becomes the details the server is missing.
    #[test]
    fn a_song_from_spotify_becomes_the_details_to_send() {
        let track: crate::api::models::Track = serde_json::from_str(
            r#"{
              "id": "4uLU6hMCjMI75M1A2tKUQC",
              "name": "Long Way Home",
              "duration_ms": 214000,
              "artists": [
                {"id": "art1", "name": "Marconi Union"},
                {"id": "art2", "name": "Guest"}
              ],
              "album": {
                "id": "alb1",
                "name": "Distance",
                "images": [
                  {"url": "https://i.scdn.co/image/small", "width": 64, "height": 64},
                  {"url": "https://i.scdn.co/image/large", "width": 640, "height": 640}
                ]
              }
            }"#,
        )
        .expect("the song reads");
        let patch = MetaPatch::for_track(&track).expect("the song has an id");
        assert_eq!(patch.track_id, "4uLU6hMCjMI75M1A2tKUQC");
        assert_eq!(patch.meta.name.as_deref(), Some("Long Way Home"));
        assert_eq!(
            patch.meta.artists,
            Some(vec!["Marconi Union".to_string(), "Guest".to_string()])
        );
        assert_eq!(
            patch.meta.artist_urls,
            Some(vec![
                "https://open.spotify.com/artist/art1".to_string(),
                "https://open.spotify.com/artist/art2".to_string(),
            ])
        );
        assert_eq!(
            patch.meta.image_url.as_deref(),
            Some("https://i.scdn.co/image/large"),
            "the biggest cover, so the web app has one worth showing"
        );
        assert_eq!(
            patch.meta.track_url.as_deref(),
            Some("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC")
        );
        assert_eq!(
            patch.meta.album_url.as_deref(),
            Some("https://open.spotify.com/album/alb1")
        );

        let nameless: crate::api::models::Track =
            serde_json::from_str(r#"{"name": "No id"}"#).expect("reads");
        assert!(
            MetaPatch::for_track(&nameless).is_none(),
            "a song with no id has no note to fill in"
        );

        let bare: crate::api::models::Track =
            serde_json::from_str(r#"{"id": "trk", "name": "Bare"}"#).expect("reads");
        let patch = MetaPatch::for_track(&bare).expect("the song has an id");
        assert!(patch.meta.artists.is_none());
        assert!(patch.meta.artist_urls.is_none());
        assert!(patch.meta.image_url.is_none());
        assert!(patch.meta.album_url.is_none());
    }

    /// The body carries the note, the version it was written over, and the
    /// song's details, and leaves out what it does not know.
    #[test]
    fn a_write_carries_the_note_and_the_song() {
        let meta = Meta {
            name: Some("Long Way Home".into()),
            artists: Some(vec!["Marconi Union".into()]),
            artist_urls: Some(vec!["https://open.spotify.com/artist/abc".into()]),
            image_url: Some("https://i.scdn.co/image/abc".into()),
            track_url: Some("https://open.spotify.com/track/trk".into()),
            album_url: None,
        };
        let body = PutBody {
            track_id: "trk",
            note: "drop at 1:04",
            expected_updated_at: Some("2026-09-03T18:22:10.512Z"),
            meta: &meta,
        };
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&body).unwrap()).unwrap();
        assert_eq!(json["track_id"], "trk");
        assert_eq!(json["note"], "drop at 1:04");
        assert_eq!(json["expected_updated_at"], "2026-09-03T18:22:10.512Z");
        assert_eq!(json["name"], "Long Way Home");
        assert_eq!(json["artists"][0], "Marconi Union");
        assert_eq!(json["track_url"], "https://open.spotify.com/track/trk");
        assert!(
            json.get("album_url").is_none(),
            "what is unknown is not sent, so the server keeps what it has"
        );

        let fresh = PutBody {
            track_id: "trk",
            note: "",
            expected_updated_at: None,
            meta: &Meta::default(),
        };
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&fresh).unwrap()).unwrap();
        assert_eq!(json["note"], "");
        assert!(json.get("expected_updated_at").is_none());
        assert!(json.get("name").is_none());
    }
}
