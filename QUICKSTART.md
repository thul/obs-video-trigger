# Quick start

Play a local video over your OBS scene from the native manager or one Stream Deck key.
The video plays with sound, then the overlay becomes transparent again.

## 1. Run the app

If Windows marked the downloaded executable, right-click `obs-video-trigger.exe` →
**Properties** → **Unblock** → **OK**. If SmartScreen is already open, choose
**More info → Run anyway**.

Double-click `obs-video-trigger.exe`. Its blue play icon appears in the notification area
next to the clock; Windows may place it behind the **^** overflow button.

Single-click the tray icon to open **OBS Video Overlay Manager**. You can also right-click
it and choose **Open manager**.

## 2. Add the overlay to OBS

In OBS, choose **Sources → + → Browser**, then use:

| Setting | Value |
|---|---|
| URL | `http://127.0.0.1:4466/overlay` |
| Width / Height | Your canvas size, such as `1920` / `1080` |
| Shutdown source when not visible | unticked |
| Refresh browser when scene becomes active | unticked |
| Control audio via OBS | ticked |

Move the browser source to the top of the source list. It stays transparent while no
video is playing.

The manager's **Overlay** button opens the same page for testing. The tray menu can copy
the overlay URL if you prefer to paste it into OBS.

## 3. Choose and play a video

Drag a video into the manager, or click **Select…** and choose one. The manager displays
the real path and a correctly URL-encoded trigger URL.

Click **Play**. While it is running you can:

- click **Pause** / **Resume**;
- click or drag the scrubber to seek;
- click **Stop** to hide it immediately; or
- drop/select another video to replace it immediately.

When nothing is playing, dropping or selecting a file only prepares it for **Play**.

## 4. Optional: make a Stream Deck key

After choosing a video, click **Copy URL** in the manager. In Stream Deck, add
**System → Website**, paste the copied URL, and enable **Access in background**.

The copied value looks like this, with the Windows path safely encoded:

```text
http://127.0.0.1:4466/play?file=C%3A%5CClips%5Cairhorn.mp4
```

Pressing that key plays the clip. Triggering the same clip while it is active toggles it
off; triggering a different clip replaces it.

A useful emergency-clear key is:

```text
http://127.0.0.1:4466/stop
```

## 5. Optional: continuous folder playback

Drag a folder into the manager, or click **Select…**, browse into the folder, leave
**Select this folder** in the **File name** box, and click **Open**. Click **Play** to
start.

The app shuffles every supported video in that folder, plays each once, then reshuffles
for another cycle. During folder playback the manager shows `Video N/total` and adds a
**Next** button. The following video is preloaded for a seamless swap. **Stop** clears the
folder queue; playing one file also leaves folder mode.

## Tray shortcuts

Right-click the tray icon to:

- open the manager or overlay page;
- copy the overlay URL;
- hide the playing clip;
- open the diagnostics log; or
- exit the app.

The manager's **Exit** button does the same shutdown. Custom notifications are silent;
red indicates an error and blue indicates information.

## If it does not work

- **No tray icon:** check the **^** overflow area. If it is not there, run the executable
  again.
- **Nothing plays:** open `http://127.0.0.1:4466/status`. If it reports
  `"overlays":0`, make sure the OBS browser source is visible and refresh it once.
- **No sound:** keep **Control audio via OBS** enabled and check the source's OBS mixer
  channel.
- **Sound should go to a Windows device:** run
  `obs-video-trigger.exe --list-audio-devices`, then start the daemon with
  `obs-video-trigger.exe --audio-device "Headset"`. In that setup, untick
  **Control audio via OBS**.
- **Playback or seeking behaves strangely:** reproduce it, then choose tray menu →
  **Open diagnostics log**. The file is stored under
  `%LOCALAPPDATA%\obs-video-trigger\diagnostics.log` unless `XDG_STATE_HOME` is set.

Full command-line, HTTP, audio, security, and troubleshooting details are in
[REFERENCE.md](REFERENCE.md).
