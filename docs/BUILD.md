# Building the installer

**English** · [Français](BUILD.fr.md)

Steps for Windows.

## Requirements

- Windows 10 or 11 (64-bit) with the Microsoft C++ build tools ("Desktop development with C++" in the Visual Studio Build Tools installer).
- [Rust](https://rustup.rs). The compiler version is pinned in `rust-toolchain.toml`: `rustup` downloads it by itself on the first `cargo` command.
- [Node.js](https://nodejs.org) 20 or newer (check with `node -v`), which includes `npm`. It runs the Tauri command line, the browser-driven tests and the release script.
- Git.
- An Internet connection for the first build: the Rust crates and the NSIS installer tooling are downloaded, then cached. The compiled app needs the WebView2 runtime (already on Windows 11); the installer downloads it when it is missing.

## Steps

1. Get the code:
   ```bash
   git clone https://github.com/BenNaciim/TachyDownload.git
   cd TachyDownload
   ```
2. Install the Node dependencies:
   ```bash
   npm install
   ```
3. Check that everything works. The Rust tests (engine and shell) first, then the browser-driven tests, which launch the real app and need its debug build:
   ```bash
   cargo test -p tachy-core -p tachy-download
   cargo build -p tachy-download
   npm test
   ```
4. Build the installer:
   ```bash
   npm run dist
   ```
5. Pick up the result: `target\release\bundle\nsis\Tachy Download_<version>_x64-setup.exe`. The release script renames it `Tachy-Download-Setup-<version>.exe` (the name the app's updater looks for).

## What `npm run dist` does

`tauri build --bundles nsis` compiles the app in release mode (`cargo build --release`, with link-time optimisation: the first build takes several minutes), then wraps `tachy-download.exe` in an NSIS installer. The user interface in `ui/` is embedded in the program; the browser extension is copied to `resources\extension` next to it. The configuration is `src-tauri/tauri.conf.json`; `src-tauri/windows/hooks.nsi` makes the installer download yt-dlp, Deno and ffmpeg at the end of the install (`--setup-tools`) and lets the "delete the data" box of the uninstaller also remove `%APPDATA%\Tachy Download`.

## Checking the installer

`powershell -File tools\check-installer.ps1 "target\release\bundle\nsis\Tachy Download_<version>_x64-setup.exe"` installs it silently into a throwaway folder, checks the files, the shortcuts and the uninstall entry, runs the browser-driven tests against the installed program, applies the installer again over a running program the way the app's updater does (`/S /UPDATE /R`), then uninstalls and checks that the data folder is untouched. It adds and removes the current user's shortcuts like a real install.

## Changing the version

The release script does it: `npm run release -- patch|minor|major|x.y.z` updates `package.json`, `package-lock.json`, `extension/manifest.json`, `Cargo.toml` and `src-tauri/tauri.conf.json`. By hand, change the same five places.

## Changing the icon

Edit `tools/make-icon.js`, run `npm run icon` (writes `build/icon.png` and `build/icon.ico`), then run `npm run dist` again.

## Troubleshooting

- **`link.exe` not found**: the C++ build tools are missing (see Requirements).
- **A build that seems stuck**: the release build is slow (link-time optimisation) and `rustc` can look idle for a while. If nothing moves for ten minutes, stop it (kill the `cargo` process tree) and run it again.
- **"Access denied" while linking**: Tachy Download or one of its test windows is still running. Quit it (and any `msedgewebview2.exe` left behind) and retry.
- **Download failing on the first build**: run `npm run dist` again; a network drop or a firewall is the usual cause.
- **Antivirus blocking or deleting the installer**: common with an unsigned program. Add an exception for the project folder.
- **SmartScreen warning on launch**: expected while the installer is unsigned. "More info", then "Run anyway".

## Publishing

`npm run release -- patch` (or the "Release" workflow in the GitHub Actions tab) bumps the version, runs the tests, builds the installer, commits, tags, pushes and creates the GitHub release with `Tachy-Download-Setup-<version>.exe` attached. The app's updater needs the release to be public and the file to carry GitHub's SHA-256 digest (it does by default).
