<p align="center"><img src="docs/logo.svg" alt="Tachy Download" width="420"></p>

[Français](README.fr.md) · **English**

A fast download manager for Windows: parallel segments, resume, queue, YouTube and HLS video, and a Chrome, Edge and Firefox extension. Open source, no ads, no account.

Built from scratch with Electron and Node.js on plain HTTP (`Range`, `Content-Range`, `If-Range`).

## Features

- **Parallel segments**: each file is split into 1 to 32 segments (never smaller than 1 MB), downloaded in parallel and written straight to their position in a `.part` file. **Dynamic re-splitting**: a connection that finishes takes half of the slowest segment still running, so all connections stay busy until the end. The details panel shows the split and the speed of every connection.
- **Resume**: every segment's offset is saved. After a pause, a network drop or closing the app, only the missing bytes are requested again. If the remote file changed (`If-Range`), the download starts over.
- **Servers without `Range`**: automatic fallback to a single stream (no resume possible).
- **Queue**: configurable number of simultaneous downloads.
- **Speed limit**: one global limit (KB/s) shared by all segments, an own limit per download (details panel), and a stricter limit during a chosen time of day (Settings → Network). The tightest one applies.
- **Network**: an HTTP/HTTPS proxy (also used by yt-dlp), automatic retry after a network cut, and adaptive connections: when a server answers 429 or 503, the download uses fewer connections and takes them back once things calm down.
- **After the download**: filing rules (`*.pdf = Documents`, `example.com = D:\Sites`), a warning when a file is identical to one already received, and optional unpacking of archives (`.zip`, `.7z`, `.tar*`) next to the archive with the `tar` that ships with Windows.
- **Site logins**: user and password, bearer token or cookie kept per site (and its subdomains), encrypted with the Windows account and sent only to that site. Not used for videos through yt-dlp.
- **Menu bar**: File, Downloads (pause/resume all, clear finished, speed limit, end-of-queue action), View (search, theme, zoom) and Help, with keyboard shortcuts. The taskbar button shows the overall progress.
- **Scheduling**: start at a given date and time, plus a daily time window (may cross midnight) outside which downloads stop.
- **Confirmation window** (like IDM): every download handed over by the browser opens a small window with the file name, size and destination folder (editable) and three choices: "Download now", "Later" (added paused) or "Cancel". It can be turned off in Settings → Sorting and startup.
- **YouTube, Vimeo, Dailymotion, Twitch (replays)** through yt-dlp, ffmpeg and Deno (the JavaScript engine YouTube needs to offer H.264), downloaded at the end of the install (official GitHub releases, SHA-256 verified). yt-dlp updates itself, at most once a day. Maximum quality is configurable, H.264 + AAC preferred. The confirmation window probes the video and lets you pick the **quality** (2160p, 1080p, 720p…), the **audio language** when the video offers several, or **audio only** (MP3 with ffmpeg).
- **YouTube playlists**: paste a playlist address to add all its videos (200 at most) into a folder named after it.
- **HLS streams** (`.m3u8`, unencrypted or AES-128): downloaded in segments over several connections, then converted to `.mp4` by ffmpeg without re-encoding. A slow or retried segment does not hold the other connections back: they keep fetching ahead (up to 128 MB waiting in memory) while the file is still written in order.
- **Expired links**: when a signed link dies mid-download, the partial file is kept. For a video found by the extension, it reopens the page in the background, gets the new address and the download carries on by itself. For a direct download, the "Open the page" button takes you back to it.
- **Re-download**: the details panel of a finished download has a "Re-download" button that fetches the file again and **replaces** the old one (same name, no "(1)"). It works for files, HLS streams and videos through yt-dlp. If the old file is open in another program, the app says so and leaves it untouched. A link that has died since goes through the "Expired links" path above.
- **Automatic sorting** by type: Videos, Music, Images, Documents, Archives, Programs. The left column lists the same types (Videos, Music, Documents, Compressed, Executables, Images, Other) to filter the list, with a counter for each.
- **Accordion details**: clicking a download unfolds its split map, per-connection speed and information right underneath; a second click folds it away. For a video stream the map shows how far the file is written and outlines the segments being fetched, and the information gives the number of connections at work.
- **Automatic updates**: the app checks the project's GitHub releases, downloads the new installer, verifies its SHA-256, installs it silently and restarts (can be turned off in Settings → Updates). This requires the releases to be public.
- **Tray icon** (the close button minimizes, right-click menu, speed in the tooltip), **launch with Windows** and an end-of-queue action (quit, sleep, shut down).
- **Liquid Glass**: translucent window (acrylic, Windows 11 22H2 or newer), can be turned off in Settings → Appearance. Light and dark themes.
- **Browser extension** (Chrome, Edge, Brave, Firefox): detects videos on pages, shows a "Download this video" button on the player and a counter in the toolbar, captures downloads and adds a "Download with Tachy Download" context-menu entry.
- **Streams like the browser asked for them**: the extension keeps the `Referer` and `Origin` the browser sent with each stream (the player's frame, not only the page) and the app replays them, which video hosts that check them require. It also recognises playlists served under odd names (`text/plain`, no extension, `.m3u8` in the query) by reading their first bytes, and **DASH** manifests (`.mpd`, unprotected), which yt-dlp downloads and merges.
- **Diagnostic mode** (extension popup): records what the browser saw on a page and why each response was or was not offered as a video; "Copy the diagnostic" puts the report in the clipboard.

## Install

Download `Tachy-Download-Setup-<version>.exe` and run it (per-user install, no administrator rights). At the end of the install, yt-dlp, Deno and ffmpeg are downloaded automatically; offline, they can still be installed from Settings → Videos.

The installer is not code-signed: Windows SmartScreen shows "Windows protected your PC" on first launch ("More info", then "Run anyway"). A code-signing certificate removes that warning.

## Run from source

```bash
npm install
npm start
npm test
```

To build the installer (see [docs/BUILD.md](docs/BUILD.md)), `npm run dist` produces `dist/Tachy-Download-Setup-<version>.exe`. To change the icon, edit `tools/make-icon.js` and run `npm run icon`.

Downloads go to your Downloads folder by default. Settings and the download list are stored in the app's data folder.

## Install the extension

**Firefox**: open `about:debugging#/runtime/this-firefox`, "Load Temporary Add-on", and pick `extension/manifest.json` (temporary until Firefox restarts; a permanent install needs a Mozilla signature). Then steps 2 and 3.

1. **Chrome / Edge**: open `chrome://extensions`, turn on developer mode, "Load unpacked", and pick the `extension/` folder (also copied to `resources\extension` in the install folder).
2. In the app: Settings → Browser extension → **Copy** the token (the button turns into a "Copied" with a check for two seconds).
3. In the extension's options: paste the token, check the port, "Save and test".

If the app is closed or not answering, the extension lets the browser download normally.

## Security

The app listens on `127.0.0.1` only (port 6800, then 6801… if busy). Three checks protect the API: the `Host` header (against DNS rebinding), the origin (extensions only) and a secret 192-bit token unique to your install. Only `http` and `https` links are accepted. The extension also sends the site's cookies to the app for downloads that need a login; they stay on your machine.

## Layout

```
src/engine/   engine (no Electron dependency): download, hls, ytdlp, manager, tools, probe
src/server.js local API for the extension
src/main.js   Electron main process; preload.js IPC bridge; renderer/ user interface
extension/    MV3 extension (Chrome, Edge, Firefox)
test/         node:test suite (local HTTP server with Range)
```

## Known limits

- No FTP, no torrent.
- No DRM (Netflix, Disney+, Prime…; a protected DASH manifest is not offered) and no live streams. The extension sees what the browser loads in its tabs: a video served in a way that gives it no recognisable playlist, manifest or file is not detected, and nothing outside the browser is captured.
- A host that checks more than `Referer`, `Origin` and cookies (a TLS fingerprint, a one-time token bound to the browser) can still refuse the app.
- The extension is tested in Chrome. In Firefox 159 it was loaded as a temporary add-on and driven end to end (detection in a cross-origin player frame, `Referer`/`Origin` capture, button, sending to the app, error messages, popup); the permanent install still needs a Mozilla signature.
- Windows only for now.
