---
title: Everyday Use
description: Library ordering, notes, and local play history.
nav_order: 3
---

## Library order

By default, the sidebar sorts playlists by when you last played them. Drag a
playlist to switch to a custom order. New playlists appear below the pinned
group. Choose **Sort by recently played** from a playlist's context menu to
restore the default order.

## Notes

`N`, or the pencil beside the lyrics button, opens a panel that follows
whatever is playing. Write in it and it saves itself; there is nothing to
press. Move to another song and the panel moves with you, unless you are
mid-sentence, in which case it waits until you stop.

Any time you write (`1:04`, `12:05`) appears as a button above the text.
Press it and the song jumps there.

**All notes** opens the page of everything you have written, newest
first. Search runs over the song, the artists, the album, and the note
itself. Click a row to play that song with its note open again; right
click to delete the note.

Notes are kept per account in the state directory and never leave this
computer. They are the one thing there that Spotify cannot give back, so
they are worth a backup. If you have notes in
[My Song Notes](https://songnotes.codyh.xyz), export them from its
Settings page and run `fastpotify --import-notes song-notes.json` once.

## Recent

The queue panel's second tab combines Spotify's history with tracks played
through Fastpotify, which Spotify does not record.

A song is added after about 30 seconds, or halfway through a shorter song.
Paused time and seeking do not count.

The local list is stored in `history.json` and is never uploaded. Settings →
Storage shows its location and has a **Clear history** button.

On Windows, the main window's minimize, maximize, and close buttons share the
top bar with Fastpotify's controls. Drag an empty part of that bar to move or
snap the window, and drag a window edge or corner to resize it.
