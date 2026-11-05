# Réécriture Rust + Tauri : plan d'exécution (document vivant)

Conception : [`../specs/2026-11-04-rust-tauri-rewrite-design.md`](../specs/2026-11-04-rust-tauri-rewrite-design.md) (validée). La conception dit *quoi* ; ce fichier dit *dans quel ordre* et *où on en est*. Il est mis à jour à chaque tâche terminée, pour que le travail survive à une coupure de session.

## Règles de travail

- Branche `rust-tauri`. `main` reste l'Electron 1.x tant que la 2.0.0 n'est pas à parité. Rien n'est poussé sur `main`, aucune étiquette ni release avant M7.
- TDD pour tout code produit : test rouge (vu échouer pour la bonne raison), code minimal, test vert, suite complète du périmètre verte. Les sondes (S1 à S5) sont jetables et minimales.
- Un commit local par tâche verte, messages en anglais comme le dépôt, fin de message : `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Branche poussée en fin de jalon.
- Tant que le JS existe : `npm test` (152 tests) et `npm run lint` doivent rester verts. Les tests JS ne sont supprimés qu'une fois leur équivalent Rust vert (§7.2 de la conception).
- Le comportement de référence est le code JS de `src/` et ses tests. En cas de doute, le JS gagne (parité, pas d'amélioration).
- Les textes français sont repris mot pour mot (conception §5.5).
- Tout résultat chiffré ou toute surprise va dans la conception §10.4 (preuves) ou dans le journal ci-dessous le jour même.

## Commandes

```bash
cargo build -p tachy-download            # application (fenêtre Tauri)
cargo test --workspace                   # tests Rust
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
npm test                                 # tests JS (Electron actuel, extension, pont)
npm run lint
npm run dist                             # installeur Electron 1.x (référence S0)
```

## Jalons

### M0 Bases et preuves

- [x] Branche `rust-tauri`, interface déplacée dans `ui/`, Electron repointé, 152 tests verts
- [x] Pont `ui/renderer/dm-bridge.js` + `test/bridge.test.js` (9 tests)
- [x] Workspace Cargo (CRT statique, outils épinglés) qui compile
- [x] Fenêtre Tauri qui affiche l'interface avec un faux noyau (capture CDP : liste, graphique, événements `dm:update`)
- [x] S0 référence 1.2.0 : installeur 107,7 Mio, RAM au repos 205,4 Mio, démarrage 623 ms, banc Node (conception §10.4)
- [x] S2 HTTP/1.1 Range : GO (8 connexions : 1 979 Mo/s contre 306 pour Node, 2,6× moins de CPU) · S3 `safeStorage` `v10` : GO · S5 icônes : GO
- [x] S1 verre : GO avec `--disable-gpu-compositing` (défaut WebView2 156 sinon) · S4 installeur : GO (1,83 Mio ; installation, mise à jour `/UPDATE /R` avec relance, désinstallation) · S5 notification : demi-GO (API `on_activated` présente, clic réel à vérifier à la main en M5, repli sans clic)
- [x] Mesure Tauri release (faux noyau, au repos) : 122 Mio, 516 ms, 5,3 % CPU contre Electron 196 Mio, 607 ms, 2,2 % (conception §10.4)
- [x] Document de parité créé (`../specs/2026-11-04-rust-tauri-rewrite-parity.md`) ; résultats consignés en §10.4 pour S0 à S5
- [ ] Pousser la branche (fin de jalon) puis rapport M0

### M1 Noyau (`crates/tachy-core`)

Ordre (chaque module : tests JS portés d'abord, puis code) :

- [x] `util` · `errors` · `fsutil` · `filetypes` (table identique à `ui/shared/filetypes.js`, testée)
- [x] `settings` · `rules` · `limiter` · `store`
- [x] `net` · `probe` · `task` (segments, vol, reprise, tentatives, connexions adaptatives, liens expirés) · `manager`
- [x] `server` (API de l'extension, jeton, CORS, 413, `Connection: close`)
- [x] Tests portés : engine (11), features = tasklimit + cookies + siteauth + refresh + redownload fichiers (15), network = steal + retry + proxy + limites + 429 + isWebPage (14), server (7). Les cas HLS, yt-dlp, afterdownload, afterall viennent avec leurs modules
- [x] Contrats : `tools/make-fixtures.js` ; `tests/contract.rs` (réglages, `downloads.json`, instantanés, API, textes) ; écarts connus nommés dans le test (champs HLS en M2, textes vidéo en M3)
- [x] Banc `examples/bench.rs` : 2,7× à 11× Node, CPU 25-30 %, 49 Mio (conception §10.4)
- [ ] Reporté aux jalons qui les portent : `after_complete` / doublons / extraction (M5), `afterall` (M5)

### M2 HLS (`crates/tachy-core/src/task/hls.rs`)

- [x] Playlists (maître, média, AES-128, `EXT-X-MAP`), refus du DRM et des directs, IV explicite ou numéro de séquence
- [x] Avance hors ordre plafonnée (`hls_ahead_bytes`, 128 Mio), reprise sans retélécharger, progression et `hlsLive` pour la vue détaillée
- [x] Conversion ffmpeg (`Manager::set_ffmpeg`), repli sur le fichier brut
- [x] `tests/hls.rs` (7 tests portés + 4 unitaires) ; contrat d'instantané HLS identique au JS (exemption retirée de `contract.rs`)
- [x] Vidéo réelle de bout en bout : flux Apple bipbop (100 segments, 598 Mo, 18,7 s) ; conversion mp4 avec le vrai ffmpeg sur un flux généré (`tests/hls_real.rs`, `--ignored`)

### M3 Vidéos (`video.rs`, `task/ytdlp.rs`, `tools.rs`)

- [x] `video` : détection des pages et des playlists, `clean_yt_opts`, `format_args`, `header_args`, `ytdlp_args`, `apply_progress`, `explain`, `format_diag`, `find_by_video_id`, `direct_file`, `summarize_probe`, `probe_video`, `list_playlist`
- [x] `task/ytdlp` : processus yt-dlp (sortie UTF-8, arbre tué à l'arrêt), progression, fichier annoncé retrouvé par l'identifiant, diagnostic `ytdlp-last.log`, raccourci du fichier simple par le moteur multi-connexions, « refaire » sur place
- [x] `tools` : chemin (copie propre puis PATH), installation avec SHA-256 vérifié (liste de sommes, zip par `tar.exe`), mise à jour de yt-dlp au plus une fois par 20 h
- [x] `Manager::add_playlist`, `ManagerConfig::{tools, ytdlp_launch}`
- [x] Tests : `tests/ytdlp.rs` (19, avec un faux yt-dlp en Node), `tests/replay.rs` (5), `tests/tools.rs` (8), unitaires `video` (31) ; exemption des textes vidéo retirée de `contract.rs`
- [x] Vraie vidéo de bout en bout (`tests/ytdlp_real.rs`, `--ignored`) : sondage et téléchargement 360p d'une vidéo YouTube publique avec le vrai yt-dlp 2026.10.26, deno et ffmpeg

### M4 Coque Tauri (`src-tauri/src/`)

- [x] Cœur : `progress` (progression de la barre des tâches), `afterall` (inactivité, commandes veille/arrêt), `menu` (barre de menus décrite en données, raccourcis uniques, cases cochées), `Manager::net()`
- [x] Coque : `data` (dossier `%APPDATA%\Tachy Download`, `--data-dir`, jeton, migration de « Download Manager »), `commands` (tous les `dm_*` et `confirm_*`, mêmes messages français), `confirm` (fenêtre « Télécharger ce fichier ? » par demande), `menu`, `tray` (infobulle, barre des tâches), `icons` (SHGetFileInfo → PNG), `notify` (toast WinRT), `events` (pont des événements, notifications, action de fin de file), `autostart`, instance unique, `--setup-tools`, arrêt propre (sauvegarde puis sortie), minuteurs yt-dlp 20 s/6 h et application 45 s/12 h
- [x] `test/ui.test.js` et `test/shell.test.js` pilotent le binaire Tauri (CDP + `test/win.ps1` pour le menu natif) ; l'interface actuelle fonctionne telle quelle avec le vrai noyau
- [ ] À vérifier à la main (M5) : clic réel sur un toast, infobulle et progression de la barre des tâches à l'œil, connexion/identifiants de site (M5)

### M5 Données et mise à jour (`postprocess.rs`, `logins.rs`, `updater.rs`, `src-tauri/src/{secrets,update}.rs`)

- [x] `postprocess` : doublons (taille puis SHA-256), extraction des archives avec le tar du système (dossier « nom (1) » si pris, dossier à moitié fait supprimé), branché sur `Manager::on_settled` ; tests `afterdownload.rs`
- [x] Deux tests de l'ancien `shell.test.js` portés dans `engine.rs` (vider les terminés, pause affichée aussitôt puis reprise)
- [x] `logins` : port de `logins.js` derrière un trait `Crypt` (mêmes messages français), reprise des entrées `v10` d'Electron avec copie `logins.json.electron.bak` ; coque : DPAPI (`secrets.rs`) + lecture de `Local State` ; commandes `dm_logins_*` ; le test d'interface des identifiants n'est plus ignoré
- [x] `updater` : port de `updater.js` (version, release GitHub, `digest` sha256, hôtes autorisés, `.part` supprimé en cas d'échec) ; coque : bandeau (`dm:updateAvail`/`dm:updateProgress`), installeur lancé détaché avec `/S /UPDATE /R` puis sortie, contrôle auto 45 s puis 12 h hors build de développement
- [x] Préférences d'interface (§6.3) : **remises à zéro une fois**, pas de dernière 1.x qui exporte (l'Electron installée est supprimée, aucune 1.x ne sera publiée)
- [ ] À vérifier à la main : clic réel sur un toast, infobulle et progression de la barre des tâches ; la chaîne de mise à jour réelle ne se prouve qu'avec deux releases publiées (essai de l'installeur en M6, sans mise à jour réelle)

### M6 Installeur (`src-tauri/windows/hooks.nsi`, `tools/release.js`, `tools/check-installer.ps1`, CI, docs)

- [x] Installeur NSIS Tauri : 4,37 Mio (objectif ≤ 25 Mio), `installMode: currentUser`, extension dans `resources\extension`, sélecteur de langue retiré (langue du système, enregistrée pour le désinstalleur), hooks : `--setup-tools` en fin d'installation, case « supprimer les données » étendue à `%APPDATA%\Tachy Download`
- [x] `tools/check-installer.ps1` : installe en silencieux dans un dossier jetable, vérifie fichiers, raccourcis, entrée de désinstallation, lance `ui.test.js` et `shell.test.js` contre le programme installé, met à jour par-dessus un programme en cours (`/S /UPDATE /R`), désinstalle, vérifie que les données restent
- [x] `tools/release.js` (versions de `package.json`, `package-lock.json`, `extension/manifest.json`, `Cargo.toml`, `tauri.conf.json` ; tests Rust + Node ; `tauri build` ; copie en `Tachy-Download-Setup-<version>.exe`), `npm run dist`, CI `test.yml` (fmt, clippy -D warnings, tests, lint, tests pilotés) et `release.yml` (non exécutables ici), `docs/BUILD.md` et `BUILD.fr.md`
- [x] README (structure du dépôt) : fait en M7, avec la suppression du code Electron

### M7 Livraison

- [x] Audit des 21 fichiers de tests JS du moteur contre les tests Rust : tout est porté ; seul « classement par type activé ou non » manquait, ajouté (`engine.rs`)
- [x] Code Electron retiré : `src/`, `spikes/`, `build/installer.nsh`, `tools/after-pack.js`, `tools/make-fixtures.js`, `tools/bench/bench-js.js`, les 21 tests JS portés ; `package.json` sans Electron, electron-builder ni undici ; version 2.0.0 partout (`package.json`, extension, Cargo, Tauri)
- [x] `bridge.test.js` : la liste des méthodes exposées est figée dans le test (elle venait de `preload.js`)
- [x] Fixtures de contrat gardées, figées, avec une note (`crates/tachy-core/tests/fixtures/README.md`)
- [x] Tout passe : `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` (203 tests), les 3 tests contre le vrai réseau (vraie vidéo, vrai flux HLS), `npm test` (40), `npm run lint`
- [x] Liste de parité cochée : 45 lignes couvertes, 14 restent à voir par une personne (geste humain ou seconde release)
- [x] Fusion de `rust-tauri` dans `main` (2026-11-05, 6b7760b) (sans étiquette ni release : la publication reste au propriétaire)

## Journal

- 2026-11-04 : M3 : la vidéo « Me at the zoo » (jNQXAC9IJ2w) est devenue indisponible côté YouTube (« This video is unavailable »), le test réel utilise une autre vidéo publique. Un test réel a montré le bon enchaînement complet (arguments, diagnostic, message français). Les tests qui lancent le faux yt-dlp passent l'environnement par `YtdlpLaunch` et non par `set_var` : les tests Rust tournent en parallèle dans un même processus.
- 2026-11-04 : conception validée ; autorisation donnée de tout faire sans s'arrêter (installeur d'essai compris). M0 étapes 1 et 2 faites (`be9ffc1`).
- 2026-11-04 : S1 : le compositeur GPU de WebView2 156 (portable à deux GPU) laisse des couches `backdrop-filter` à moitié peintes après un changement de style en direct ; contournements côté interface sans effet, `--disable-gpu-compositing` règle tout (RAM au repos 122 Mio au lieu de 223, CPU au repos 5,3 % au lieu de 0,8 %). À refaire avec le vrai noyau en M7.
- 2026-11-04 : un `cargo build --release` est resté bloqué 47 min (rustc quasi inactif) : tuer l'arbre de processus et relancer suffit (2 min).
- 2026-11-04 : M1 : le vrai noyau fait aussi bien que le spike S2 (1 974 Mo/s à 8 connexions, 1 813 à 16). Une course trouvée par `queue_respects_max_concurrent` : deux téléchargements du même nom choisissaient le même `.part` ; le nom est maintenant pris et réservé sous un verrou (`Ctx::naming`). Le JS avait la même fenêtre mais s'en sortait parce qu'il est mono-fil.
- 2026-11-04 : `rust-toolchain.toml` épingle `1.99.1` (rustup télécharge la chaîne au premier appel). `.cargo/config.toml` : `+crt-static`. Icônes lues depuis `build/` pour l'instant (déplacées en M7).
- 2026-11-04 : M4 : une instance de test tuée sans son arbre de processus laisse des `msedgewebview2.exe` qui gardent le dossier de données de WebView2 ; l'instance suivante (autre port de débogage) ne répond plus. `test/app.js` met donc un délai aux requêtes CDP, et les tests tuent l'arbre (`taskkill /T /F`). Sous Tauri, `window.close()` n'existe pas côté page : la fermeture d'une fenêtre se teste par WM_CLOSE (`win.ps1`).
- 2026-11-04 : M5 : le test d'ancien `shell.test.js` « pause affichée aussitôt » passait déjà : `Task::stop` positionne l'état avant d'attendre la fin du transfert. Le format des secrets passe à `base64(blob DPAPI)` ; un secret `v10` est reconnu à son préfixe et réécrit au premier démarrage.
- 2026-11-04 : M6 : le verre. `--disable-gpu-compositing` coûtait 8 à 10 % d'un cœur au repos (Electron : 0,1 %) ; retiré avec le matériau système de Windows 11 appliqué directement (§10.4, S1 revu). Les changements de style à chaud ne laissent plus de couches à moitié peintes (24 changements, contraste mesuré identique).
- 2026-11-04 : M6 : McAfee (`mc-fw-host`) garde ouvert l'exécutable installé ; la vérification « appli en cours » du gabarit (Restart Manager) y voit une appli non fermable et abandonne : installation par-dessus, mise à jour silencieuse et désinstallation échouaient. Contournement dans les hooks : arrêt de l'appli puis renommage de l'ancien exécutable avant la vérification, suppression du `.old` à la fin. Le désinstalleur demandait aussi la langue (boîte « Installer Language ») faute de valeur enregistrée : le hook l'écrit.
- 2026-11-04 : M6 : l'icône de type de fichier de la fenêtre de confirmation manquait par intermittence (5 essais sur 8 en release) : `SHGetFileInfo` répond parfois « succès, sans icône » juste après le démarrage. Relances (8 fois, 100 ms) ; 12 essais sur 12 ensuite.
- 2026-11-04 : `Start-Process -Wait` attend aussi les processus lancés par l'installeur (l'appli relancée par `/R`) : ne pas l'utiliser pour l'installeur en mode mise à jour.
- 2026-11-04 : M6 terminé. `check-installer.ps1` passe en entier sur l'installeur final (4,37 Mio) : installation, 12 tests pilotés contre le programme installé, mise à jour par-dessus un programme en cours (ancien arrêté, relancé, pas de `.old`, une seule entrée), désinstallation, données conservées. Mesure du build final (médiane de 3, `measure-app.js`) : démarrage à froid 565 ms, mémoire 217 Mio (7 à 9 processus), CPU au repos 2,1 % ; Electron : environ 990 ms, 207 Mio, 0,1 %. Non vérifiés ici : les workflows CI, la case « supprimer les données » du désinstalleur en mode graphique.
- 2026-11-05 : M7 : suite complète verte après suppression d'Electron ; `npm run lint` lisait `target/` (JS généré par Tauri) : ignoré dans `eslint.config.js`.
