<p align="center"><img src="docs/logo.svg" alt="Tachy Download" width="420"></p>

[Français](README.fr.md) · **English**

A fast download manager for Windows: parallel segments, resume, queue, YouTube and HLS video, and a Chrome, Edge and Firefox extension. Open source, no ads, no account.

Built from scratch with Electron and Node.js on plain HTTP (`Range`, `Content-Range`, `If-Range`).

## Features

- **Parallel segments**: each file is split into 1 to 32 segments (never smaller than 1 MB), downloaded in parallel and written straight to their position in a `.part` file. **Dynamic re-splitting**: a connection that finishes takes half of the slowest segment still running, so all connections stay busy until the end. The details panel shows the split and the speed of every connection.
- **Resume**: every segment's offset is saved. After a pause, a network drop or closing the app, only the missing bytes are requested again. If the remote file changed (`If-Range`), the download starts over.
- **Servers without `Range`**: automatic fallback to a single stream (no resume possible).
- **Queue**: configurable number of simultaneous downloads.
- **Speed limit**: one global limit (KB/s) shared by all segments.
- **Scheduling**: start at a given date and time, plus a daily time window (may cross midnight) outside which downloads stop.
- **Confirmation window** (like IDM): every download handed over by the browser opens a small window with the file name, size and destination folder (editable) and three choices: "Download now", "Later" (added paused) or "Cancel". It can be turned off in Settings → Sorting and startup.
- **YouTube, Vimeo, Dailymotion, Twitch (replays)** through yt-dlp, ffmpeg and Deno (the JavaScript engine YouTube needs to offer H.264), downloaded at the end of the install (official GitHub releases, SHA-256 verified). yt-dlp updates itself, at most once a day. Maximum quality is configurable, H.264 + AAC preferred.
- **YouTube playlists**: paste a playlist address to add all its videos (200 at most) into a folder named after it.
- **HLS streams** (`.m3u8`, unencrypted or AES-128): downloaded in segments, then converted to `.mp4` by ffmpeg without re-encoding.
- **Expired links**: when a signed link dies mid-download, the partial file is kept. For a video found by the extension, it reopens the page in the background, gets the new address and the download carries on by itself. For a direct download, the "Open the page" button takes you back to it.
- **Automatic sorting** by type: Videos, Music, Images, Documents, Archives, Programs.
- **Tray icon** (the close button minimizes, right-click menu, speed in the tooltip), **launch with Windows** and an end-of-queue action (quit, sleep, shut down).
- **Liquid Glass**: translucent window (acrylic, Windows 11 22H2 or newer), can be turned off in Settings → Appearance. Light and dark themes.
- **Browser extension** (Chrome, Edge, Brave, Firefox): detects videos on pages, shows a "Download this video" button on the player and a counter in the toolbar, captures downloads and adds a "Download with Tachy Download" context-menu entry.

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
2. In the app: Settings → Browser extension → **Copy** the token.
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
- No DRM (Netflix, Disney+, Prime…), no live streams, no DASH (`.mpd`). A site that serves its video in an unsupported format is not detected.
- The extension is tested in Chrome; for Firefox the manifest is validated (web-ext lint) but loading it end to end has not been tested.
- Windows only for now.
