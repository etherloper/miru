# Miru

A local media player built with Rust, GPUI and libmpv.

## How video is composited (Windows)

mpv renders into a native child window (`wid`), using its own D3D11 swap chain and hardware decoding.
GPUI draws the UI with a transparent window background. A one-line patch to the vendored
`gpui-pre-windows` crate makes GPUI's DirectComposition visual composite *above* child windows, so
the UI overlays the video. See [vendor/gpui-pre-windows/MIRU_PATCH.md](vendor/gpui-pre-windows/MIRU_PATCH.md).
The child window is disabled, so all mouse input over the video goes to GPUI.

## Setup (Windows)

Requirements: Rust stable (MSVC) and Visual Studio Build Tools.

Stage libmpv. This builds `mpv.lib` from a `libmpv-2.dll` (e.g. from the shinchiro `mpv-dev` builds):

```powershell
./scripts/setup-mpv-windows.ps1 -Dll "C:\path\to\libmpv-2.dll"
```

The build copies the DLL next to the executable. Then run the app and add your media folders from
**Settings** (the gear in the top right), which also holds playback and display preferences:

```powershell
cargo run -p miru
```

Pass a file path (`cargo run -p miru -- movie.mkv`) to start straight in the player.

### Artwork and metadata

- **Local artwork** works offline: put `poster.jpg` (or `folder.jpg` / `cover.jpg`) and `fanart.jpg`
  (or `backdrop.jpg`) in a movie's folder, or in a show's folder above its `Season N` folders.
- **TMDB**: set `TMDB_API_KEY` to a v3 API key or a v4 read access token to download posters,
  backdrops, titles and overviews. Local artwork takes precedence.

### Data location

The library database and artwork cache live in `%LOCALAPPDATA%\Miru`. Set `MIRU_DATA_DIR`
to use a different folder (useful for testing).

### Packaging

`scripts/package-windows.ps1` builds a release and writes an installer
(`dist/miru_<version>_x64-setup.exe`, per-user, no admin prompt) and a portable zip to `dist/`,
both with libmpv included. It needs `cargo install cargo-packager --locked`. Release builds have no
console window and log to `miru.log` beside the library database.

No TMDB key is built into the app. Each person adds their own in **Settings → Metadata** (keys are
free at themoviedb.org); for development, `TMDB_API_KEY` in `.env` works too and is git-ignored.

One value from `.env` is built into the release:

- `MIRU_UPDATE_REPO=owner/name`: a **public** GitHub repo whose releases installed copies
  update from (Settings → About, or automatically once a day). It can be this repo; the app reads
  its releases without signing in.

To ship an update: bump `version` in `Cargo.toml`, run `scripts/package-windows.ps1`, then
`scripts/publish-release.ps1 -Notes "What changed"` (needs the GitHub CLI, signed in).

## Controls

Library:
- Ctrl+K (or Ctrl+F) opens the search panel. Results appear as you type; ↑/↓ to choose, Enter to open,
  Ctrl+Enter for the full results page.
- Tab / Shift+Tab or the arrow keys move focus (↑/↓ jump a row in grids); Enter or Space opens.
- Back: Alt+←, Backspace, Esc or the mouse back button. Forward: Alt+→ or the mouse forward button.
- Ctrl+1 / 2 / 3: Home, Movies, TV Shows. Ctrl+B collapses the sidebar.

Player: Space/K play-pause, ←/J and →/L seek ±10 s, Home restarts, Esc goes back. Right-click the
picture to play/pause. Mouse wheel over the picture changes the volume in 5% steps; the speaker button
mutes. Hover the seek bar for a scrub handle and time preview; chapter markers and the
Chapters button jump between chapters. When the credits start (a credits chapter, or the last 30 s), a **Next episode** button appears and
starts the next episode after a 10 s countdown that pauses with the video (Settings → Playback → Autoplay
next episode). Media keys and the Windows volume flyout control playback too. Click the bar to seek. **Audio** and **Subtitles** menus switch tracks; a show remembers your choice for its other episodes. Move the mouse to show the controls.

Playback position is saved every few seconds. Titles past 30 s can be resumed; past 92% they count as
watched, and the next episode appears in **Continue watching**. The **⋯** button on a detail page marks a
movie, show or season watched or unwatched (or click the check on an episode), and **Fix match** picks
the right TMDB entry for a mismatched title. Library folders are watched, so new files appear on
their own.

## Layout

- `crates/core`: library engine: filename parser (`parse`), SQLite + FTS5 storage (`db`),
  incremental folder scanner (`scan`), browse queries (`library`). Try it with
  `cargo run -p miru-core --bin miru-scan -- library.db D:\Media` (add `--search <query>`).
- `crates/metadata`: TMDB client, artwork cache, local artwork discovery
- `crates/player`: libmpv wrapper (`Player`, `PlaybackState`)
- `crates/app`: GPUI app: `library_view.rs` (home, grids, show page), `player_view.rs`,
  `library_sync.rs` (background scan and metadata), `video_host.rs` (native child window)
- `vendor/gpui-pre-windows`: patched GPUI Windows backend
