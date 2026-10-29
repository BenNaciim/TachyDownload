# Building the installer

**English** · [Français](BUILD.fr.md)

Steps for Windows.

## Requirements

- Windows 10 or 11.
- [Node.js](https://nodejs.org) 20 or newer (check with `node -v`), which includes `npm`.
- Git.
- An Internet connection for the first build: Electron and the installer tooling are downloaded (a few hundred MB) and then cached.

## Steps

1. Get the code:
   ```bash
   git clone https://github.com/BenNaciim/TachyDownload.git
   cd TachyDownload
   ```
2. Install the dependencies:
   ```bash
   npm install
   ```
3. Check that everything works (52 tests, about ten seconds):
   ```bash
   npm test
   ```
4. Build the installer:
   ```bash
   npm run dist
   ```
5. Pick up the result: `dist\Tachy-Download-Setup-<version>.exe` (about 107 MB). The `dist\win-unpacked\` folder holds the app without installing it, handy for a quick try.

## What `npm run dist` does

[electron-builder](https://www.electron.build) copies the Electron runtime, puts the code from `src/` into an `app.asar` archive, removes the Chromium licences file (`tools/after-pack.js`), then compiles an NSIS installer. The `build/installer.nsh` script makes the installer download yt-dlp, Deno and ffmpeg at the end of the install. The browser extension is copied to `resources\extension`. The configuration is the `build` section of `package.json`.

## Changing the version

Edit `"version"` in `package.json` **and** in `extension/manifest.json`. The output file name follows automatically.

## Changing the icon

Edit `tools/make-icon.js`, run `npm run icon` (writes `build/icon.png` and `build/icon.ico`), then run `npm run dist` again.

## Troubleshooting

- **`EBUSY` or "resource busy or locked"**: something is using the `dist` folder. Quit Tachy Download, close any terminal or Explorer window open in `dist\win-unpacked`, or delete `dist` and retry.
- **Download failing on the first build**: run `npm run dist` again; a network drop or a firewall is the usual cause.
- **Antivirus blocking or deleting the installer**: common with an unsigned program. Add an exception for the project folder.
- **SmartScreen warning on launch**: expected while the installer is unsigned. "More info", then "Run anyway".

## Publishing

On GitHub, create a release (for example `v1.0.0`) and attach the `.exe`. The `dist/` folder is not versioned.
