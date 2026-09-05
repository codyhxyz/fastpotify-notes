---
title: How It Connects
description: Fastpotify's independent Spotify grants, what is stored, and how API traffic is routed.
nav_order: 1
---

## Independent grants, once each

Fastpotify uses separate credentials for Web API access, a personal app, and
local playback:

1. **The shared Web API app** keeps full catalogue and playlist coverage.
2. **Your optional personal Web API app** handles supported playback, library,
   catalog, playlist creation, and owned or collaborative playlist requests
   without using the shared app's quota. Complete playlist-library views and
   playlist-bearing search stay on the shared app so Spotify-owned results are
   not filtered out. Both Web API grants must verify as the same Spotify
   account.
3. **Local playback** uses
   [librespot](https://github.com/librespot-org/librespot). It needs one more
   browser approval and stores its own reusable credential. Spotify Premium
   is required.

Local playback authorization stays separate from both Web API grants.

By default, Fastpotify uses the public app shared with spotify-player, ncspot,
and Omarchy Spotify. Spotify divides its quota among all users. A personal app
adds a separate Development Mode quota. See
[Use a Personal Spotify App](/make-it-even-faster/).

## What the client stores

- Shared and personal Web API refresh tokens, plus librespot's credential, in
  the state directory with owner-only permissions
  ([file locations](/settings-and-files/)).
- Downloaded audio and artwork, in the cache directory, within the budget
  you set.
- The first time MilkDrop opens with an empty preset folder, the two projectM
  preset packs are downloaded from GitHub (about 26 MB) and stored in the
  config directory.
- On Windows and macOS, desktop media controls receive artwork from that
  cache instead of downloading the Spotify image a second time. Linux MPRIS
  carries the Spotify artwork URL for the desktop to resolve.
- Lyrics, in the cache directory, for a month.
- A copy of your notes, in the state directory. The notes themselves live in
  My Song Notes; see [Notes](#notes) below.
- Fastpotify has no telemetry and no analytics. When the lyrics panel is open
  and Spotify has no lyrics, it sends the track's artist, title, album, and
  length to [lrclib.net](https://lrclib.net). It also checks api.github.com
  once a day for updates. You can turn off update checks in Settings.

## When Spotify pushes back

Each Web API session has separate concurrency and rate limits. A `Retry-After`
response pauses only that session. Fastpotify routes each request once and
does not retry it through the other app.

Spotify can also explicitly refuse the key needed to decrypt a track. When
that happens, Fastpotify stops local playback and leaves the rest of the queue
alone instead of treating every following track as unavailable. This refusal
comes from Spotify; trying again later may work.

Before adding songs to an existing playlist, Fastpotify checks the rows it
already holds. A known duplicate produces an immediate confirmation naming the
song. Only a playlist that has not been fully loaded needs a background scan to
rule out duplicates. Once confirmed, the new rows appear locally at once. A
successful write advances the cached playlist to Spotify's returned snapshot
instead of downloading the playlist again. If Spotify cannot answer the scan,
Fastpotify preserves the requested edit and lets the write report its result.

## Notes

Notes are kept in [My Song Notes](https://songnotes.codyh.xyz) at
songnotes.codyh.xyz, so the same notes are in Fastpotify, in the browser,
and on a phone. Fastpotify is a client of it and holds only a copy.

Four requests go there, each carrying your Spotify Web API access token as
a bearer token. The server asks Spotify who that token belongs to and answers
with that account's notes; nothing else identifies you, and no separate
sign-in is needed.

| Request | When | What it carries |
| --- | --- | --- |
| `GET /api/notes/list` | Signing in, opening the notes panel, opening the notes page | The token |
| `GET /api/notes?track_id=` | Every song change while the panel is open | The token and the track id |
| `PUT /api/notes` | A second after you stop typing, and when the panel closes, the song changes, or the app quits | The token, the note as HTML, the version last seen, and the song's name, artists, artist and album and track links, and cover URL |
| `PATCH /api/notes` | Right after a list that contained notes with no song details, up to 200 songs at a time | The token and the songs' names, artists, artist and album and track links, and cover URLs, and never a note |

Some notes were written before My Song Notes kept a song's name and cover,
so a list can arrive with nothing to draw a row with: Fastpotify asks Spotify
`GET /v1/tracks` about those songs, fifty ids at a time, and shows a song it
no longer knows as Unknown song. Those details then go back to the server as
a `PATCH /api/notes`, which carries the songs and nothing else, so the note
and the time it was last written stay exactly as they were and every device
lists them filled in from then on.

An empty note is a deleted note, so a deletion is the same `PUT`. If the
server holds a newer version of that note, it refuses the write, Fastpotify
takes the newer one, and says so. A write that cannot reach the server is
kept and offered again every thirty seconds until it lands.

Notes are for songs. An episode, an advert, or a local file has no track id,
so the panel says so instead of offering an editor.

Anyone holding your Spotify access token can read and write your notes. That
is the same trust Fastpotify already places in that token.

## Receivers on the local network

Spotify's device list only shows signed-in receivers. A new librespot or
spotifyd receiver is therefore invisible to the Web API.

Receivers announce themselves over mDNS as `_spotify-connect._tcp` and answer
a small HTTP interface. Fastpotify encrypts the stored librespot credential
with a receiver-specific key and a key from a Diffie-Hellman exchange. The
encrypted value only works for that receiver and exchange. Fastpotify does not
save another copy of the credential.

The receiver then signs in and appears in Spotify's device list. Fastpotify
uses the Web API for subsequent control requests.

## The engine

Playback runs on a separate runtime. Librespot maintains the Spotify Connect
session, exposes this computer as a device, receives transfers, and reports
playback state. If the session drops, it reconnects with the stored credential.

The engine discovers access points through `apresolve.spotify.com` and
connects over TCP in the resolver's preference order: port 4070 first,
falling back to 443 and 80. Only outbound connections are needed; no
inbound ports have to be open.
