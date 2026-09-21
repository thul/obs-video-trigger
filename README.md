# obs-video-trigger

A small native Windows application for playing local video clips over an OBS scene. A
clip appears fullscreen in an OBS browser source, plays with sound, and leaves the source
transparent when it ends. Trigger it from the native manager, a URL, the command line, a
Stream Deck, or any other launcher that can make an HTTP request.

- One native Rust `.exe`; no runtime, installer, or configuration file.
- Native dark-mode manager and Windows tray icon.
- Play files from anywhere on disk without copying them into a special directory.
- Seamlessly replace the visible clip using a preloaded second player.
- Run a folder as continuously reshuffled playback, with one-video lookahead.
- Scrub, pause/resume, stop, skip to the next folder video, and inspect time/duration.
- Generate and copy correctly URL-encoded file and folder triggers from the GUI.
- Silent native success/error notifications with copy and dismiss controls.
- Persistent diagnostics for media, queue, HTTP-range, and WebSocket problems.
- Route audio through OBS or to a selected Windows output device.

## Quick start

1. Run `obs-video-trigger.exe`. The blue play icon appears in the notification area.
2. Single-click the tray icon (or right-click → **Open manager**).
3. Click **Select…** and choose a video, or drag a video into the manager.
4. Add `http://127.0.0.1:4466/overlay` as an OBS **Browser** source. Set it to your
   canvas size and keep **Shutdown source when not visible** and
   **Refresh browser when scene becomes active** unticked.
5. Click **Play**, or click **Copy URL** and assign the copied trigger URL to a Stream Deck
   **System → Website** action.

See [QUICKSTART.md](QUICKSTART.md) for the complete first-run walkthrough and
[REFERENCE.md](REFERENCE.md) for command-line and HTTP details.

## Native manager

Open the manager with a single click on the tray icon, from **Open manager** in its
context menu, or by double-clicking the icon.

### Choose media

- Drag a supported video file or folder anywhere into the window.
- Or click **Select…**. Choose a video normally; to choose a folder, browse into it,
  leave **Select this folder** in the **File name** box, and click **Open**.
- The manager shows the real Windows path and creates a reusable, URL-encoded trigger.
- **Copy URL** copies that trigger for Stream Deck, a browser, or another launcher.
- Dropping or selecting a video while anything is active immediately leaves folder mode
  and replaces the playing clip. While idle, it only prepares the selection.

### Playback controls

- **Play / Pause / Resume** starts the selected trigger or controls the active video.
- **Stop** hides the video and clears an active folder queue.
- **Next** appears during folder playback and advances to the next shuffled video.
- **Overlay** opens the OBS overlay page in the default browser.
- **Exit** shuts down the daemon.
- The scrubber supports click-to-seek and drag-to-preview; the seek is committed on mouse
  release. Time and duration appear on the right. Folder mode also shows `Video N/total`
  on the left.

Buttons have hover, pressed, and status feedback. The manager follows the Windows dark
theme, carries the application icon, opens on the monitor containing the pointer, and
repaints only when its state changes.

## Folder playback

Selecting or dropping a directory creates a `/play-folder` trigger. Starting it collects
the supported videos directly inside that folder, shuffles them, and plays each file once
per cycle. At the end of a cycle the same folder is shuffled again; different folders are
never merged, and the shuffle avoids repeating the boundary video immediately.

The next video is reserved from the queue and preloaded in a second, fresh media element
so a normal transition can swap without exposing the OBS scene underneath. **Next**
consumes that reserved video. **Stop** clears the queue, so the next folder start creates
a fresh shuffle.

## Tray, notifications, and diagnostics

The tray tooltip and context menu identify the app as **OBS Video Overlay**. The menu can
open the manager or overlay, copy the overlay URL, hide playback, open the diagnostics
log, or exit.

Notifications are custom native popups and do not play the Windows notification sound.
Errors have a red left edge; informational messages use blue. A popup appears on the
monitor containing the pointer and includes **Copy** and **Dismiss** buttons.

Diagnostics are appended and flushed immediately to:

- `$XDG_STATE_HOME/obs-video-trigger/diagnostics.log`, when `XDG_STATE_HOME` is set.
- `%LOCALAPPDATA%\obs-video-trigger\diagnostics.log` otherwise.

Use tray menu → **Open diagnostics log**. It records queue/token transitions, media source
and readiness events, preload/swap decisions, seeks and recovery, WebSocket reconnects,
and HTTP byte-range requests. It does not contain video data, but it does contain local
file paths.

## Command line

```powershell
obs-video-trigger.exe                              # start daemon + tray
obs-video-trigger.exe --play "C:\Clips\intro.mp4"
obs-video-trigger.exe "C:\Clips\intro.mp4"       # same as --play
obs-video-trigger.exe --play-folder "C:\Clips\Break"
obs-video-trigger.exe --stop
obs-video-trigger.exe --status
obs-video-trigger.exe --list-audio-devices
```

Daemon options are `--host`, `--port`, `--audio-device`, and `--no-tray`. Play and folder
triggers accept `--volume 0..1` and `--fit contain|cover|fill`. Run with `--help` or see
[REFERENCE.md](REFERENCE.md) for all behavior and HTTP endpoints.

## How it works

```text
Manager / Stream Deck / CLI
          │ HTTP
          ▼
daemon on 127.0.0.1:4466 ─── WebSocket ───▶ overlay page in OBS
          ▲                                      │
          └──────── GET /media/<id> ─────────────┘  byte-range media
```

The daemon validates and registers local paths, broadcasts tokenized commands over one
bidirectional WebSocket, and serves only registered files from `/media/<id>`. The overlay
reports visible playback state, time, errors, and completion over the same connection.
Tokens keep delayed events from an old player from changing the current queue.

The overlay uses two media slots, but creates a fresh `<video>` element for every new
source assignment so Chromium cannot leak stale decoder events into the next clip. It
preloads the reserved folder item, swaps only after the new media says it can play, and
has bounded recovery for browser seek stalls.

Loopback host validation and same-origin checks prevent unrelated web pages from firing
clips or reading local files. The overlay HTML carries a version hash and reloads itself
after a daemon update.

## Supported media

The folder scanner recognizes `.mp4`, `.webm`, `.mov`, `.m4v`, `.mkv`, and `.ogv`.
Actual playback depends on whether the codec inside the file is supported by OBS's
Chromium browser; H.264/AAC MP4 and VP8/VP9 WebM are the safest choices. WebM is also the
usual choice when the video needs an alpha channel. See
[REFERENCE.md](REFERENCE.md#clip-formats-and-preparation) for preparation guidance.

## Build and develop

Requires Rust 1.95 or newer on the build machine only.

```powershell
cargo build --release
Copy-Item target/release/obs-video-trigger.exe dist/obs-video-trigger.exe

cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

The optimized Windows GUI-subsystem binary is currently under 1 MB. Releases are built
by the Rust CI workflow and include `SHA256SUMS.txt`. The version in a release tag must
match `Cargo.toml`.

Prepare and publish a release from a clean `main` branch with:

```powershell
.\scripts\prepare-release.ps1 1.2.0
git add Cargo.toml Cargo.lock
git commit -m "Release v1.2.0"
.\scripts\tag-release.ps1 -Push
```

The preparation script updates both Cargo files and runs formatting, linting, and tests.
The tagging script derives the tag from Cargo metadata, pushes the commit first, and only
then pushes the matching tag that starts the release workflow.

The unsigned download may initially trigger SmartScreen. Right-click the executable →
**Properties** → **Unblock**, or choose **More info → Run anyway** once. PowerShell users
can run `Unblock-File .\obs-video-trigger.exe`.

Source layout:

- `src/main.rs` — process entry and daemon setup.
- `src/server.rs` — HTTP, WebSocket, media serving, and folder queue.
- `src/overlay.html` / `src/overlay.rs` — OBS player and embedded page versioning.
- `src/tray.rs` — native tray, manager, popup, picker, and playback UI.
- `src/client.rs` / `src/args.rs` — command client and argument parser.
- `src/diagnostics.rs` — persistent event timeline.

## License

MIT
