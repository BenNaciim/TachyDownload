These files were recorded from the JavaScript (Electron, v1.2.0) engine with `tools/make-fixtures.js`, which was
removed with the Electron code in 2.0.0 (it stays in the git history, on `main` before the Rust merge). They freeze
what the Rust engine must keep producing: settings, `downloads.json`, window snapshots, API answers and the French
texts. Do not regenerate them; change one only when a behaviour is changed on purpose.

`fake-ytdlp.js` is a stand-in for yt-dlp that the video tests start through Node.
