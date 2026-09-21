# obs-video-trigger — reference

Everything the tool does, and how to drive it from the command line or from a browser.
For the five-minute version, see [QUICKSTART.md](QUICKSTART.md).

---

## Contents

1. [How it works](#how-it-works)
2. [The daemon](#the-daemon)
3. [The tray icon](#the-tray-icon)
4. [The OBS browser source](#the-obs-browser-source)
5. [Triggering from the command line](#triggering-from-the-command-line)
6. [Triggering from a browser or URL](#triggering-from-a-browser-or-url)
7. [Playback behaviour](#playback-behaviour)
8. [Stream Deck](#stream-deck)
9. [Other launchers](#other-launchers)
10. [Clip formats and preparation](#clip-formats-and-preparation)
11. [HTTP API](#http-api)
12. [Security](#security)
13. [Troubleshooting](#troubleshooting)

---

## How it works

`obs-video-trigger.exe` is one file with two jobs, chosen by how it is started:

| Started as | Role |
|---|---|
| `obs-video-trigger.exe` (no arguments) | **Daemon.** Runs in the background, shows a tray icon, serves the overlay page. |
| `obs-video-trigger.exe --play <file>` (or any other option) | **Trigger.** Sends one command to the running daemon, then exits. |
| `obs-video-trigger.exe --play-folder <folder>` | **Folder mode.** Randomly plays supported clips continuously; run it again for the same folder to stop. |

The daemon hosts a small web page at `http://127.0.0.1:4466/overlay`. OBS loads that
page as a browser source. The page uses two fullscreen media slots with no controls and
a transparent background. It creates a fresh `<video>` element for each source assignment
and preloads a replacement before removing the visible clip. It clears itself when
playback ends.

Triggers reach the daemon over HTTP on port 4466. The command line is a thin wrapper
around that HTTP call, so anything that can open a URL can trigger a clip too.

Nothing is installed. There is no config file. Clips can live anywhere on disk.

---

## The daemon

### Starting

Double-click `obs-video-trigger.exe`, or run it from a terminal with no arguments.

- **From Explorer or a shortcut:** no window opens. The only sign of it is the tray icon.
- **From a terminal:** it prints its log to that terminal and stays in the foreground.
  Ctrl+C stops it.

Only one daemon can run per port. Starting a second one prints
`error: port 4466 is already in use` and exits.

### Stopping

- Right-click the tray icon → **Exit**.
- Ctrl+C in the terminal it was started from.
- Task Manager → end `obs-video-trigger.exe`.

### Options

| Option | Default | Meaning |
|---|---|---|
| `--port <number>` | `4466` | Port to listen on. Every trigger and the OBS URL must use the same port. |
| `--host <addr>` | `127.0.0.1` | Address to listen on. `0.0.0.0` accepts triggers from other machines on your network. |
| `--no-tray` | off | Do not show the tray icon. |
| `--audio-device <name>` | none | Play clip audio through this Windows output device instead of the OBS browser source. See [Audio output device](#audio-output-device). |

Example — run on a different port, no tray icon:

```
obs-video-trigger.exe --port 5000 --no-tray
```

### Audio output device

By default the clip's sound comes out of the OBS browser source, where OBS mixes it into
the stream. `--audio-device` sends it to a specific Windows output device instead — a
headset, a second sound card, a virtual cable:

```
obs-video-trigger.exe --list-audio-devices
obs-video-trigger.exe --audio-device "Headset"
```

`--list-audio-devices` asks the running daemon for the output devices the overlay page can
see and prints one per line with its browser id. It needs an overlay connected (OBS or a
browser tab on the overlay page). `<name>` is any part of a listed name, case-insensitive,
or the id in brackets. The first device whose name contains it wins.

Two things to know:

- In OBS, **untick "Control audio via OBS"** on the browser source. Ticked, OBS captures
  the page's audio itself and the device binding has no effect. Unticked, the sound goes
  straight to the chosen Windows device and does not reach the stream unless that device
  is captured by OBS (a virtual cable, or "Desktop Audio" set to it).
- The routing is done by the overlay page with the browser's `setSinkId()`. If the name
  matches nothing, or the browser refuses the device, the clip still plays on the default
  output and the [debug view](#debug-view) says why.

### Environment variables

| Variable | Effect |
|---|---|
| `OBS_VIDEO_TRIGGER_TOKEN` | Pins the shutdown token (see [HTTP API](#http-api)) instead of generating a random one per run. Useful for scripting a shutdown. |

---

## The tray icon

An application icon next to the clock. Windows may tuck it behind
the **^** overflow arrow.

| Action | Result |
|---|---|
| Single-click | Opens the native manager. |
| Right-click → **Open manager** | Opens the native manager. |
| Right-click → **Open overlay page** | Opens `http://127.0.0.1:4466/overlay` in your default browser. Useful to confirm the daemon is up; a normal browser will show the clip but may mute it (see [Playback behaviour](#playback-behaviour)). |
| Right-click → **Copy overlay URL** | Copies the overlay URL to the clipboard, ready to paste into OBS. |
| Right-click → **Hide the playing clip** | Same as `--stop`. Takes the current clip off screen immediately. |
| Right-click → **Open diagnostics log** | Opens the persistent media, queue, WebSocket, and HTTP-range event timeline. |
| Right-click → **Exit** | Shuts the daemon down and removes the icon. |
| Double-click | Opens the native manager. |
| Hover | Shows the **OBS Video Overlay** tooltip. |

The icon, menu and error popup are native Win32 UI owned by the daemon. If the daemon
dies, Windows removes the icon. No helper process is involved.

Bad paths and clips the OBS browser cannot decode appear in a small silent popup near
the taskbar. These errors are never drawn in the browser source.

The dark native manager accepts files and folders by drag-and-drop or through its single
**Select…** dialog. It displays and copies the URL-encoded trigger, and provides a
scrubber, time/duration, **Play/Pause/Resume**, **Stop**, **Next** during folder mode,
**Overlay**, and **Exit**. Selecting a file while playback is active replaces the current
clip immediately; while idle it only prepares the trigger. Folder playback also shows
`Video N/total`.

The diagnostics log is stored at
`$XDG_STATE_HOME/obs-video-trigger/diagnostics.log` when that variable exists, otherwise
at `%LOCALAPPDATA%\obs-video-trigger\diagnostics.log`.

---

## The OBS browser source

Add one browser source per OBS profile. It can live in one scene and be reused in others
by nesting that scene.

**Sources → + → Browser**, then set:

| Property | Value | Why |
|---|---|---|
| URL | `http://127.0.0.1:4466/overlay` | The overlay page. Change the port if the daemon uses a different one. |
| Width, Height | Your canvas size, e.g. `1920` × `1080` | The video scales into this box. |
| Shutdown source when not visible | **unticked** | If OBS unloads the page, it stops listening for triggers. |
| Refresh browser when scene becomes active | **unticked** | A refresh mid-clip would cut it off. |
| Control audio via OBS | ticked | Puts the clip's audio on its own mixer channel so you can set its level once, and so it reaches the stream and the recording. |

Put the source at the **top** of the source list so clips draw over everything else.

### Debug view

Append `?debug=1` to the URL (`http://127.0.0.1:4466/overlay?debug=1`) to show a small
status label in the bottom-left corner of the source:

| Label | Meaning |
|---|---|
| `connected` | The page is listening for triggers. |
| `waiting` | Connected, nothing playing. |
| `playing <name>` | A clip is on screen. |
| `toggled off <name>` | The same clip was triggered again and was stopped. |
| `disconnected, reconnecting` | Lost the daemon; reconnects automatically. |
| `autoplay blocked (...), retrying muted` | The browser refused unmuted autoplay; the clip plays silently. Does not happen inside OBS. |
| `error loading clip` | The file could not be decoded. |
| `audio device not found: <name>, using default` | `--audio-device` matched none of the devices the page can see. Check `--list-audio-devices`. |
| `audio device rejected (...), using default` | The browser refused to route audio to that device. |

Remove `?debug=1` when done.

### Automatic reload after an update

The daemon stamps the page with a version. When a newer daemon starts, a page that is
still open in OBS notices the mismatch on reconnect and reloads itself. If OBS ever seems
to run old behaviour anyway, right-click the source → **Refresh**.

---

## Triggering from the command line

All commands talk to the daemon and exit immediately. Run them from a terminal, a
Stream Deck **System → Open** action, a batch file, or any other launcher.

### Play a file

```
obs-video-trigger.exe --play "C:\Clips\airhorn.mp4"
obs-video-trigger.exe "C:\Clips\airhorn.mp4"
```

Both forms are identical; a bare path is treated as `--play`. Quote the path if it
contains spaces. Relative paths are resolved against the current directory.

### Play options

| Option | Default | Meaning |
|---|---|---|
| `--volume <0..1>` | `1` | Volume for this clip. `0.5` is half. |
| `--fit <mode>` | `contain` | How the video fills the source: `contain` (letterbox, whole video visible), `cover` (crop to fill), `fill` (stretch). |
| `--interrupt` | — | Accepted for compatibility; replacing is now the default. |

```
obs-video-trigger.exe --play "C:\Clips\intro.mp4" --volume 0.6 --fit cover
```

### Stop

```
obs-video-trigger.exe --stop
```

Hides whatever is on screen right now.

### Continuous random folder playback

```
obs-video-trigger.exe --play-folder "C:\Clips\Break"
```

The daemon shuffles the supported video files directly inside the folder, plays each one
once, then creates another shuffled cycle for as long as needed. It avoids an immediate
repeat at a cycle boundary and reserves/preloads one upcoming video for a seamless swap.
Folders are never combined into one queue. Run the same command again to stop. The
manager's **Next** button advances to the reserved item. **Stop** clears the queue so its
next start reshuffles from scratch. `--volume` and `--fit` apply to every clip. A normal
`--play` or `--stop` exits folder mode.

### Status

```
obs-video-trigger.exe --status
```

Prints:

```json
{"ok":true,"overlays":1,"clips":3,"port":4466,"audioDevice":""}
```

| Field | Meaning |
|---|---|
| `overlays` | Number of OBS browser sources (or browser tabs) currently connected. `0` means nothing will play. |
| `clips` | Distinct files triggered since the daemon started. |
| `port` | Port the daemon is listening on. |
| `audioDevice` | The `--audio-device` the daemon was started with; empty when audio stays in the browser source. |

### List audio output devices

```
obs-video-trigger.exe --list-audio-devices
```

Prints the output devices the connected overlay can see, for use with the daemon's
`--audio-device` option (see [Audio output device](#audio-output-device)):

```
Audio output devices seen by the overlay (1 connected):
  Speakers (Realtek(R) Audio)   [a1b2c3...]
  Headphones (USB Audio Device)   [d4e5f6...]
Bind one with: obs-video-trigger --audio-device "<name>"
```

Exits `1` with `no overlay connected` when nothing is showing the overlay page.

### Targeting a daemon on another port or host

Add `--port` and/or `--host` to any command:

```
obs-video-trigger.exe --port 5000 --play "C:\Clips\x.mp4"
obs-video-trigger.exe --host 192.168.1.20 --play "D:\Clips\x.mp4"
```

With `--host`, the path is interpreted **on the daemon's machine**, not the one running
the command.

### Help

```
obs-video-trigger.exe --help
```

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Command delivered; at least one overlay is connected. |
| `1` | Daemon not running, or no overlay connected. |
| `2` | Bad argument, or the file does not exist. |

Messages go to the terminal. From Explorer or the Stream Deck there is no terminal, so
use the exit code (Stream Deck multi-actions can branch on it) or `--status`. The exe is a
window-less program, so a shell prompt returns before it finishes: in a batch file use
`start /wait obs-video-trigger.exe --play "..."` to get the exit code in `%ERRORLEVEL%`.

---

## Triggering from a browser or URL

Every command is also a URL. Paste it into a browser's address bar, put it in a Stream
Deck **System → Website** action, or call it from any HTTP client.

| Command | URL |
|---|---|
| Play | `http://127.0.0.1:4466/play?file=C:\Clips\airhorn.mp4` |
| Play, half volume, cropped | `http://127.0.0.1:4466/play?file=C:\Clips\airhorn.mp4&volume=0.5&fit=cover` |
| Toggle random folder playback | `http://127.0.0.1:4466/play-folder?folder=C:\Clips\Break` |
| Stop | `http://127.0.0.1:4466/stop` |
| Status | `http://127.0.0.1:4466/status` |
| The overlay page itself | `http://127.0.0.1:4466/overlay` |

Backslashes in the path are fine as they are. Percent-encode characters that have a
meaning inside a URL:

| Character | Write as |
|---|---|
| space | `%20` |
| `#` | `%23` |
| `&` | `%26` |
| `%` | `%25` |

`http://127.0.0.1:4466/play?file=C:\Clips\my%20clip.mp4`

Each URL returns a small JSON reply, for example `{"ok":true,"file":"C:\\Clips\\airhorn.mp4","overlays":1}`.
`ok:false` comes with an `error` field explaining why.

`GET` and `POST` both work. A `POST` may carry the same parameters as a JSON body:

```
POST /play
Content-Type: application/json

{"file":"C:\\Clips\\airhorn.mp4","volume":0.5,"fit":"cover"}
```

---

## Playback behaviour

- **Plays once.** No loop. When the file ends, the overlay is transparent again.
- **Fullscreen, no controls.** The video fills the browser source; nothing else is drawn.
- **Unmuted, full volume** unless `--volume` says otherwise. Inside OBS this always
  works. In an ordinary browser tab, autoplay policy may force the clip to play muted —
  that is the browser, not the tool.
- **Same file again while it is playing → stops it.** The key acts as a toggle. Trigger
  it again after it has finished and it plays from the start.
- **Different file while one is playing → replaces it seamlessly.** The new clip buffers
  behind the current one and takes over when ready. Nothing is queued; the newest trigger wins.
- **`--stop` at any time** clears the screen.
- **Trigger with no overlay connected** does nothing visible; the command exits `1`.

---

## Stream Deck

Two built-in actions work. Use one key per clip.

### System → Open

| Field | Value |
|---|---|
| App / File | Browse to `obs-video-trigger.exe` |
| Arguments | `--play "C:\Clips\airhorn.mp4"` plus any options |

No window appears when the key is pressed.

### System → Website

| Field | Value |
|---|---|
| URL | `http://127.0.0.1:4466/play?file=C:\Clips\airhorn.mp4` plus any `&option=value` |
| Access in background | ticked — otherwise a browser tab opens |

### Suggested extra keys

| Key | Arguments / URL |
|---|---|
| Panic — clear the screen | `--stop` / `http://127.0.0.1:4466/stop` |

### Multi-actions

A Stream Deck multi-action can chain a clip with a scene switch, a sound, or a hotkey.
Put the `obs-video-trigger.exe` step where you want the clip to start.

---

## Other launchers

Anything that can run a program or open a URL:

| Tool | How |
|---|---|
| Touch Portal, Loupedeck, Razer Stream Controller | "Run program" with the CLI form, or "Open URL" with the URL form |
| Chat bots, channel-point redeems (Streamer.bot, Mix It Up, SAMMI) | HTTP request / "Open URL" action with the URL form |
| Batch file, PowerShell, AutoHotkey | Run `obs-video-trigger.exe --play "..."` |
| OBS hotkey via obs-websocket | Not needed — bind the Stream Deck key directly |

---

## Clip formats and preparation

The overlay is a Chromium browser, so it plays what Chromium plays:

| Extension | Notes |
|---|---|
| `.mp4` (H.264 / AAC) | Safest choice for fullscreen clips. No transparency. |
| `.webm` (VP9 / VP8) | Safest choice for overlays. Supports an alpha channel. |
| `.mov`, `.m4v`, `.mkv`, `.ogv` | Work if the codec inside is one Chromium decodes (H.264, VP8, VP9, AV1). |
| `.avi` | Not served. The codecs usually found in AVI (DivX, Xvid, MJPEG) do not play in Chromium; convert to MP4 or WebM. |

**Transparency.** Only WebM carries alpha. An MP4 logo sting will show a black
rectangle. Convert with [ffmpeg](https://ffmpeg.org):

```
ffmpeg -i input.mov -c:v libvpx-vp9 -pix_fmt yuva420p -b:v 2M output.webm
```

**Size.** A clip the same resolution as the canvas fills it exactly. Smaller clips are
letterboxed with `contain` (default), cropped with `cover`, or stretched with `fill`.

**Loudness.** Match levels between clips in your editor, or give the loud ones a lower
`--volume`.

**Length.** Anything works; short stingers (2–8 s) suit the replace-on-trigger
behaviour best.

---

## HTTP API

Base URL: `http://127.0.0.1:4466` (or whatever `--host`/`--port` the daemon uses).

| Method | Path | Parameters | Reply |
|---|---|---|---|
| `GET` / `POST` | `/play` | `file` (required) — absolute path on the daemon's machine. `volume` (0–1). `fit` (`contain`, `cover`, `fill`; anything else falls back to `contain`). | `{"ok":true,"file":"<absolute path>","overlays":<n>}`, `400` for a missing or relative `file`, `404 {"ok":false,"error":"no such file: ..."}` |
| `GET` / `POST` | `/play-folder` | `folder` (required) — absolute path. `volume` and `fit` as for `/play`. Calling it again with the active folder toggles it off. | `{"ok":true,"active":true,"folder":"...","clips":<n>,"overlays":<n>}` or `active:false` when toggled off. |
| `GET` / `POST` | `/stop` | — | `{"ok":true,"overlays":<n>}` |
| `GET` | `/status` | — | `{"ok":true,"overlays":<n>,"clips":<n>,"port":<n>,"audioDevice":"<name>"}` |
| `GET` | `/audio-devices` | — | `{"ok":true,"overlays":<n>,"audioDevice":"<name>","devices":[{"id":"...","label":"..."}]}` as last reported by an overlay; `503` when no overlay is connected. |
| `POST` | `/audio-devices` | JSON `{"devices":[{"id","label"}]}` | Used by the overlay page to report its output devices. `{"ok":true}`; `400` on a malformed body. |
| `GET` | `/overlay` | `debug=1` (optional) | The overlay HTML page |
| `GET` | `/events` | — | Server-sent event stream used by the overlay page. Emits `hello`, `play` and `stop` events; `hello` and `play` carry `sink`, the `--audio-device` name. |
| `GET` | `/media/<id>` | Range header supported | Streams a file that has been triggered. Ids are internal; the overlay page uses them. |
| `POST` | `/shutdown` | `token` (required) | `{"ok":true}` then the daemon exits. `403` on a bad token. |

`POST` bodies may be JSON with the same parameter names.

`/play`, `/stop` and `POST /audio-devices` answer `403` to requests that a browser labels as coming from another
website (a `Sec-Fetch-Site` header other than `none` or `same-origin`). Typing the URL
into the address bar, a Stream Deck, curl and the CLI are unaffected. Every endpoint
answers `403` when the `Host` header is not a loopback name while the daemon is bound to
loopback; see [Security](#security).

The shutdown token is random per run and known only to the tray helper. Set the
`OBS_VIDEO_TRIGGER_TOKEN` environment variable before starting the daemon to choose it
yourself:

```
set OBS_VIDEO_TRIGGER_TOKEN=mysecret
obs-video-trigger.exe
...
curl -X POST "http://127.0.0.1:4466/shutdown?token=mysecret"
```

---

## Security

- The daemon listens on `127.0.0.1` by default: only programs on the same PC can reach it.
  `--host 0.0.0.0` opens it to your network; do that only on a network you trust, because
  anyone on it could then play any file the daemon's user can read.
- Web pages open in your own browser can also reach `127.0.0.1`. Two checks keep a page
  from another site out: `/play` and `/stop` refuse requests the browser labels as
  cross-site (`Sec-Fetch-Site`), and on a loopback bind every request must carry a loopback
  `Host` header, which defeats DNS rebinding (a hostname the attacker points at
  `127.0.0.1`). The `Host` check is skipped when you bind to a network address, because
  you then want to be reached by name or LAN IP.
- `/media/<id>` serves only files that have been triggered through `/play`. The overlay
  page cannot be used to browse or fetch arbitrary files.
- `/shutdown` requires the per-run token. The token is passed to the tray helper on its
  command line, so it guards against web pages, not against other programs running as
  the same Windows user (which could simply end the process anyway).
- Nothing leaves the machine. No telemetry, no updates, no accounts.

---

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Key does nothing, no message | Overlay not connected | Check the browser source is in the active scene, both "unticked" boxes are unticked, then right-click the source → **Refresh**. Confirm with `--status` (`overlays` should be ≥ 1). |
| `error: no daemon listening on http://127.0.0.1:4466` | Daemon not running | Start `obs-video-trigger.exe`. Look for the tray icon. |
| `warning: no overlay connected - is the OBS browser source running?` | Daemon up, OBS page not loaded | As the first row. |
| `error: no such file: ...` | Wrong path, or drive not mounted | Paste the path into Explorer. Keep quotes around it. |
| `error: port 4466 is already in use` | A daemon is already running, or another program has the port | Check the tray. Otherwise use `--port` on the daemon **and** update every URL and key. |
| Clip plays silently | Browser source audio not routed, or `--volume 0` | Tick **Control audio via OBS**; check the mixer channel; check the key's `--volume`. |
| `--audio-device` has no effect | **Control audio via OBS** is ticked, or the name matches nothing | Untick it on the browser source. Compare the name with `--list-audio-devices`; open the overlay with `?debug=1` to see the reason. |
| Clip audible on the device but not on the stream | `--audio-device` sends sound past OBS | Expected. Capture that device in OBS (virtual cable or Desktop Audio), or drop `--audio-device`. |
| Black box around the clip | File has no alpha channel | Re-encode to VP9 WebM with `yuva420p`. |
| Clip letterboxed / cropped unexpectedly | Source size ≠ canvas, or `--fit` | Match width/height to the canvas; choose `--fit`. |
| Clip cut short | Same key pressed again (toggle), another clip key (replace), or `--stop` | Expected behaviour. |
| Old behaviour after updating the exe | OBS still has the old page | Right-click the source → **Refresh**. |
| No tray icon, but clips work | Windows Explorer has not refreshed its notification area | Restart the daemon; if needed, restart Explorer. |
| "Windows protected your PC" on first run | Binary is unsigned and carries the downloaded-from-internet mark | **More info → Run anyway** once, or remove the mark for good: right-click the exe → **Properties** → tick **Unblock** → **OK** (`Unblock-File .obs-video-trigger.exe` in PowerShell). |
