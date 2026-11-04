# Réécriture de Tachy Download en Rust (Tauri) : conception

**Statut :** brouillon à valider (relu en « constructeur »), aucun code écrit. **Date :** 2026-11-04.
**Tes décisions de départ :** « réécrire toute l'application en Rust » ; « Oui, Tauri » (moteur et coque en Rust, interface HTML/CSS/JS actuelle gardée) ; « l'appli déjà installée en Electron va être supprimée » (§6.4).
**Références vérifiées le 2026-11-04 :** Rust 1.99.1, Tauri 2.12.5, `tauri-bundler` 2.10.4, `window-vibrancy` 0.8.1, `tauri-plugin-single-instance` 2.5.5, `reqwest` 0.13.5, `axum` 0.8.9, `tokio` 1.54, crate `windows` 0.62 (à revérifier au démarrage du chantier).

## En bref (ce que je te demande de valider)

1. **Quoi.** L'application devient une appli Tauri 2. Tout ce que font `src/*.js` et `src/engine/` (≈ 3 550 lignes de JS) est réécrit en Rust. L'interface (2 370 lignes de HTML/CSS/JS) reste telle quelle derrière un fin pont `window.dm`. L'extension reste en JS, inchangée, et parle à la même API locale.
2. **Pourquoi.** Installeur de ≈ 107 Mo ramené à quelques dizaines de Mo au plus, démarrage plus rapide, moteur plus économe en processeur, plus d'Electron/Chromium à tenir à jour. Pas de gain de vitesse sur une ligne ≤ 1 Gbit/s : le réseau reste le goulot.
3. **Parité exacte avec la 1.2.0.** Aucune nouveauté pendant la réécriture. Mêmes données, mêmes fichiers, mêmes messages en français, même API.
4. **L'Electron installée est supprimée, tes données restent.** D'après ton message, aucune montée de version « en place » n'est à garantir : tu désinstalles l'Electron, tu installes la 2.0. `%APPDATA%\Tachy Download` (liste, réglages, jeton, outils) n'est pas touché par la désinstallation et sert tel quel ; les identifiants chiffrés sont relus (§6.2). Si ton message voulait dire autre chose, c'est la première chose à corriger (§6.4).
5. **Méthode.** Branche `rust-tauri` (`main` reste l'Electron 1.x livrable jusqu'à la bascule), jalons M0 à M7 (§8), tests portés d'abord, contrats vérifiés contre des fixtures produites par l'appli JS actuelle, barrière de performance sur un banc local.
6. **Cinq preuves à faire en premier (M0)** : verre acrylique, HTTP/1.1 en parallèle et débit, lecture des identifiants chiffrés par Electron, installeur Tauri, icônes de fichiers et clic sur notification (§9).
7. **À relire :** le périmètre (§2), tes décisions et mes choix (§10), les différences visibles acceptées (§10.3).

---

## 1. Pourquoi, et ce que ça doit rapporter

**Intention.** Remplacer Electron + Node par Tauri + Rust sans que l'utilisateur voie autre chose qu'une appli plus légère.

**Mesuré le 2026-11-04** (moteur Node actuel, serveur local, 1 Gio, HTTP sans TLS, cette machine) :

| Mesure | Valeur |
|---|---|
| Débit du moteur actuel, 1 ou 8 connexions | 150 à 200 Mo/s (≈ 1,2 à 1,6 Gbit/s) |
| CPU du moteur pendant ce transfert | ≈ 1,2 cœur (3,3 à 4,2 s en code applicatif + 4,4 à 5,8 s en noyau par Gio) |
| Même serveur, `curl` (C) vers un fichier / sans disque | 430 Mo/s / 650 Mo/s |
| Installeur 1.2.0 | ≈ 107 Mo (`docs/BUILD.md`) ; Electron 45 seul pèse 357 Mo dépaqueté (`node_modules\electron\dist`, mesuré) |
| RAM au repos, démarrage à froid | **non mesurés** (M0) |

**Ce qu'on ne promet pas :** une vitesse supérieure sur une ligne ≤ 1,5 Gbit/s ; moins de bugs (ceux des dernières semaines étaient de la logique, pas des erreurs de mémoire ou de type) ; un gain de RAM important avec Tauri, car WebView2 est encore un Chromium (le gain de RAM n'est grand qu'avec une interface 100 % Rust, écartée).

**Critères de réussite**, mesurés en M0 sur la 1.2.0 puis en M7 sur la 2.0.0, même machine :

| Type | Critère |
|---|---|
| **Barrière** (bloque la livraison) | Débit du moteur sur le banc ≥ celui du moteur Node. CPU par Gio ≤ celui du moteur Node. Parité fonctionnelle (§7.4) complète. Installation propre, mise à jour 2.x → 2.y par l'updater intégré et désinstallation réussies (§6.4). |
| **Mesure publiée** (pas de seuil, on l'écrit dans les notes de version) | Taille de l'installeur (attendu ≤ 25 Mo), RAM au repos tous processus confondus (WebView2 compris), démarrage à froid jusqu'à la fenêtre visible, débit et CPU du banc. Protocole en §7.3. |

Les objectifs chiffrés du moteur (débit, CPU) sont fixés à l'issue de S2 (§9), à partir d'un client Rust minimal sur le banc, pas avant.

## 2. Périmètre

**Objectifs.**
1. Parité fonctionnelle avec la 1.2.0 ; la liste de référence est le README (§7.4).
2. Même interface : HTML/CSS/JS inchangés, `window.dm` identique.
3. Même API locale : l'extension ne change pas.
4. Reprise des données existantes (même dossier, mêmes fichiers).
5. Moteur au moins aussi rapide et aussi sobre que l'actuel.
6. Mises à jour automatiques d'une 2.x vers la suivante (fonction existante, portée).

**Non-objectifs.** Nouvelles fonctions ; macOS et Linux ; réécrire l'extension (un navigateur n'exécute que du JS) ou yt-dlp, ffmpeg, Deno ; changer le look ; changer le protocole ou les formats de fichiers (sauf ajouts rétro-compatibles) ; signature de code ; chiffrer les cookies de la liste de téléchargements en cours (pas chiffrés aujourd'hui non plus) ; télémétrie ; **montée de version « en place » d'une 1.x installée vers la 2.0** (l'Electron installée sera supprimée, §6.4).

**Anti-objectifs (un échec même si « ça marche »).**
- Perdre liste, réglages, jeton, outils ou identifiants en passant à la 2.0.
- L'Electron et la Rust actives en même temps sur le même dossier de données (elles se réécriraient `downloads.json` l'une l'autre et se disputeraient le port 6800).
- L'extension qui ne se connecte plus (le jeton doit rester le même).
- Un téléchargement qui marchait en 1.2.0 et plus en 2.0 (Referer/Origin, `If-Range`, liens expirés, HLS…).
- Un moteur plus lent ou plus gourmand que l'actuel.
- Un prérequis de plus pour l'utilisateur (redistribuable Visual C++, WebView2 à installer à la main).

**Contraintes.** Windows 10 et 11, x64 seulement (comme l'installeur actuel). Licence MIT. Installation par utilisateur, sans élévation, sans service. Fonctionne hors connexion (sauf téléchargements, outils, mises à jour). Rust stable épinglé (`rust-toolchain.toml`). CRT liée statiquement (`-C target-feature=+crt-static`) pour ne pas exiger le redistribuable Visual C++. `main` reste livrable pendant toute la réécriture.

## 3. Ce qui existe aujourd'hui et où ça va

| Aujourd'hui | Lignes | Devient | Jalon |
|---|---|---|---|
| `engine/manager.js` | 605 | `tachy-core` : `manager.rs`, `settings.rs`, `store.rs` | M1 |
| `engine/download.js` | 486 | `task/http.rs` | M1 |
| `engine/tokenBucket.js`, `net.js`, `probe.js`, `util.js`, `rules.js` | 39 + 27 + 71 + 165 + 44 | `limiter.rs`, `net.rs`, `probe.rs`, `fsutil.rs` + `errors.rs` + `filetypes.rs`, `rules.rs` | M1 |
| `server.js` | 148 | `api.rs` | M1 |
| `engine/hls.js` | 272 | `task/hls.rs` | M2 |
| `engine/ytdlp.js`, `engine/tools.js` | 425 + 175 | `task/ytdlp.rs`, `tools.rs` | M3 |
| `engine/postprocess.js`, `afterall.js` | 79 + 19 | `postprocess.rs`, `afterall.rs` (fonctions pures, dans le noyau) | M5 |
| `logins.js` | 94 | `logins.rs` + `platform/dpapi.rs` | M5 |
| `updater.js` | 86 | `updater.rs` | M5 |
| `main.js`, `preload.js`, `confirm-preload.js`, `menu.js`, `progress.js` | 645 + 42 + 11 + 93 + 24 | `src-tauri` : fenêtres, pont, menu, zone de notification, barre des tâches, notifications, dialogues, icônes, instance unique, démarrage auto | M4 |
| `renderer/*`, `shared/*` | 2 370 | `ui/` inchangé + `dm-bridge.js` | M0 / M4 |
| `extension/*` | 1 049 | inchangé | — |
| `tools/release.js`, `build/installer.nsh`, workflows CI | | scripts de release, hooks NSIS, workflows | M6 |
| 24 fichiers de tests, 152 tests | 3 421 | portés en Rust, ou gardés en JS (extension, UI e2e), voir §7.2 | M1 à M7 |

**Règle d'or :** jusqu'à la bascule, **le code JS fait foi** pour chaque algorithme (constantes en annexe B ; en cas de divergence entre l'annexe et le JS, le JS gagne et l'annexe est corrigée). Ce document ne recopie pas les algorithmes ; le constructeur lit le fichier JS et ses tests, puis porte le comportement.

## 4. Architecture cible

### 4.1 Arborescence du dépôt

```
Cargo.toml  rust-toolchain.toml  .cargo/config.toml (crt-static)     workspace
crates/tachy-core/              bibliothèque sans Tauri, testable partout
  src/  lib.rs  manager.rs  settings.rs  store.rs  api.rs  limiter.rs  net.rs  probe.rs
        fsutil.rs  errors.rs  filetypes.rs  rules.rs  postprocess.rs  afterall.rs
        logins.rs  tools.rs  updater.rs
        task/  mod.rs  http.rs  hls.rs  ytdlp.rs
  tests/                        tests d'intégration portés (serveurs HTTP locaux, faux yt-dlp)
  examples/bench.rs             banc de débit (§7.3)
src-tauri/                      l'application (binaire `tachy-download`)
  src/  main.rs  bridge.rs  windows.rs  menu.rs  tray.rs  platform/{dpapi,icons,power}.rs
  tauri.conf.json  capabilities/  icons/  installer-hooks.nsh
ui/                             ex-`src/renderer` + `src/shared` + `renderer/dm-bridge.js`
extension/                      inchangé
test/                           tests JS restants (extension, e2e UI, génération des fixtures)
tools/                          release, icône, banc
```

Pendant la transition, **Electron et Tauri chargent la même interface** depuis `ui/` : `main.js` pointe sur `../ui/renderer/…` et `src/engine/util.js` lit `../../ui/shared/filetypes` (seule dépendance du moteur JS vers l'interface, vérifiée) ; la liste `files` d'electron-builder gagne `ui/**/*`. Après la bascule, `src/` (JS du moteur et d'Electron) et les dépendances Electron disparaissent ; `package.json` reste pour ESLint, les tests JS et le script de release.

### 4.2 Coupure noyau / application

`tachy-core` ne connaît ni Tauri ni les fenêtres. Il expose un `ManagerHandle` (clonable) et un flux d'événements. L'application Tauri est une coque : elle traduit commandes et événements, et possède tout ce qui est propre à Windows. Le noyau doit pouvoir tourner sans fenêtre (tests, banc, `--setup-tools`).

La configuration du gestionnaire reprend les **réglages réservés aux tests** du JS, dont les tests portés ont besoin : intervalle de cycle (`tickMs`), délais de nouvelle tentative (`retryDelays`), avance HLS (`hlsAheadBytes`), commande `tar` (`tar`), préfixe de lancement de yt-dlp (`ytdlpPrefix`, qui permet de lancer un faux script à la place de `yt-dlp.exe`).

### 4.3 Concurrence

- Runtime `tokio` multi-thread.
- Le **gestionnaire est un acteur** : une tâche possède la table des téléchargements ; le `ManagerHandle` lui envoie des commandes (`add`, `pause`, `resume`, `remove`, `redownload`, `refresh`, `set_settings`…) avec réponse par `oneshot`. C'est l'équivalent du modèle mono-thread de Node : pas de course sur l'état d'une tâche.
- Un téléchargement actif = une tâche `tokio` + un `CancellationToken` (équivalent d'`AbortController`).
- La **table des segments** d'un fichier est un `Arc<Mutex<…>>` (mutex standard, sections critiques courtes, jamais de `.await` dedans) : le vol de segment doit couper la victime de façon atomique pendant que son propriétaire avance `pos`, comme `_steal` le fait de façon synchrone aujourd'hui.
- Événements : `Update(Snapshot)` au plus une fois par tâche tous les 250 ms (comme le ticker JS), immédiat sur changement d'état ; `Removed(id)`, `Settings`, `Tools`.
- Sauvegarde de `downloads.json` différée d'1 s, écriture atomique (fichier temporaire puis renommage) ; sauvegarde immédiate à l'arrêt.
- Arrêt propre : les tâches en cours passent en `paused`, sauvegarde, fermeture de l'API, puis sortie.
- Une panique dans une tâche de téléchargement ne doit pas tuer l'application : la tâche passe en `error`, les autres continuent (pas de `panic = "abort"`).

### 4.4 Pile HTTP (`net.rs`)

- `reqwest` avec `rustls` et **le vérificateur de la plateforme** (magasin de certificats Windows ; le moteur Node actuel utilise le magasin embarqué de Node).
- **HTTP/1.1 uniquement.** Une connexion TCP par segment : HTTP/2 multiplexerait tous les segments sur un seul tuyau et annulerait le découpage. Un test compte les connexions TCP côté serveur.
- Pas de décompression automatique (`Accept-Encoding: identity`) ni de magasin de cookies : `Cookie` est envoyé à la main.
- Redirections suivies, URL finale lisible ; `Authorization` et `Cookie` ne suivent pas une redirection vers un autre site.
- Proxy `http://` ou `https://` (`normalize_proxy` porté tel quel), client reconstruit quand le réglage change, transmis aussi à yt-dlp (`--proxy`).
- Délais : connexion 15 s ; **aucune donnée pendant 60 s = connexion perdue**, nouvelle tentative (Node attend 5 minutes ; mon choix, §10.2).
- Ports interdits : la liste « bad ports » du standard Fetch, comme `fetch` de Node aujourd'hui ; erreur « Ce port est bloqué par mesure de sécurité. »
- En-têtes rejoués : ceux de l'annexe C (`headers`), plus `Authorization` ou `Cookie` du login du site (`siteAuth`), `Range`, `If-Range`.

### 4.5 Disque

- Fichier `.part` pré-dimensionné, écriture positionnelle (`FileExt::seek_write`), `fsync` à la fin ; mêmes règles de reprise (taille du `.part` = taille attendue).
- **Écritures par blocs d'au moins 256 Kio** : sur le banc du 2026-11-04, le temps noyau représente plus de la moitié du CPU du moteur Node (≈ 57 %) ; moins d'appels système est le levier principal.
- HLS : écriture séquentielle dans l'ordre, avec avance limitée à 128 Mio en mémoire.
- Limiteur de débit : seau de jetons partagé plus un seau par téléchargement, même arithmétique (solde négatif toléré, l'appelant attend d'être remboursé).

### 4.6 Processus externes

- yt-dlp, ffmpeg et `tar` par `tokio::process`, avec `CREATE_NO_WINDOW`, sortie lue en UTF-8, `PYTHONUTF8=1` et `PYTHONIOENCODING=utf-8` comme aujourd'hui.
- Arrêt d'un téléchargement = tout l'arbre de processus : un **Job Object Windows** (`KILL_ON_JOB_CLOSE`) remplace `taskkill /T /F` (mon choix) et tue aussi les enfants si l'appli plante.
- `tar` toujours par chemin complet `System32\tar.exe` (un `tar` GNU de Git lit `C:\…` comme un nom d'hôte). Les archives des outils (zip) peuvent passer par la crate `zip` ou par `tar.exe`, au choix du constructeur.
- Outils : mêmes sources, mêmes sommes SHA-256, dossier `tools\` du dossier de données ; la recherche d'un outil se calque sur `Tools.find()`.

### 4.7 Interface et pont

- `ui/` est servi par Tauri (`frontendDist: ../ui`, pas de serveur de développement) : fenêtre principale `renderer/index.html`, confirmation `renderer/confirm.html`. `app.withGlobalTauri: true`, car le pont s'appuie sur `window.__TAURI__` (`invoke`, `listen`).
- **`ui/renderer/dm-bridge.js`**, chargé avant les scripts de l'interface : ne fait rien si `window.dm` existe (Electron). Sinon il définit `window.dm` et `window.confirmApi` au-dessus de `invoke` et `listen`. Les arguments positionnels deviennent un objet nommé ; les abonnements renvoient une fonction de désabonnement **synchrone** ; une erreur de commande devient `Error(message)` avec le **message français brut, sans préfixe** (l'interface retire déjà celui d'Electron, `Error invoking remote method '…': `, par une expression régulière qui ne doit rien trouver à retirer).
- Les deux pages HTML portent une CSP `default-src 'none'` en balise `<meta>` : il faut y **ajouter `connect-src ipc: http://ipc.localhost`**, sinon l'IPC de Tauri est bloquée. Pas de CSP dans `tauri.conf.json` : les balises restent l'autorité.
- Fenêtre principale : 1200 × 720 (minimum 760 × 460), titre « Tachy Download », icône, transparente quand le verre est pris en charge, masquée au démarrage avec `--hidden`. Fermer la fenêtre la masque quand `closeToTray` est actif et que la zone de notification existe ; sinon la fermeture de la dernière fenêtre quitte l'appli.
- Fenêtre de confirmation : 520 × 470 (640 de haut pour une vidéo yt-dlp), non redimensionnable, ni réductible ni agrandissable, toujours au-dessus, montrée une fois prête, décalage en cascade de 28 px sur 6 positions à partir de la position centrée, plusieurs possibles en même temps, **fermée sans réponse = annulation**. La commande appelante retrouve son entrée par l'étiquette de sa fenêtre (équivalent de `e.sender.id`).
- Navigation et ouverture de nouvelles fenêtres refusées, comme aujourd'hui (`setWindowOpenHandler` refuse, `will-navigate` est bloqué) : les liens externes passent par des commandes, jamais par un lien dans la page.
- Réflexes de navigateur à neutraliser pour garder le comportement d'aujourd'hui (l'appli Electron n'a ni menu contextuel, ni rechargement, ni impression) : menu contextuel natif de WebView2, rechargement (F5, Ctrl+R), impression (Ctrl+P), zoom à la molette ou au pincement, outils de développement hors des builds de développement. Moyen au choix du constructeur (options de fenêtre ou écouteurs dans `dm-bridge.js`). L'interface n'utilise ni glisser-déposer HTML, ni `navigator.clipboard`, ni `window.open` (vérifié) : rien à prévoir de ce côté.

### 4.8 Intégrations Windows

| Fonction | Moyen | Risque |
|---|---|---|
| Verre acrylique (Windows 11 ≥ build 22621, comme `glassSupported()` ; sinon fond opaque `#f1f5fd`) | `window-vibrancy` 0.8 `apply_acrylic`, qui sur Windows 11 22H2+ appelle `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW)`, ce que fait déjà Electron (lu dans le code source de la crate) ; fenêtre transparente | faible, S1 |
| Zone de notification (icône 16 px, infobulle rafraîchie chaque seconde « Tachy Download : N en cours (P %), X,Y Mo/s », menu clic droit, clic = ouvrir) | API tray de Tauri 2 | faible |
| Progression sur le bouton de la barre des tâches (`none`, `indeterminate`, `normal`) | `set_progress_bar` | faible |
| Notifications de fin ou d'échec (clic : dossier ou fenêtre), seulement fenêtre masquée ou sans focus, et réglage actif | plugin notification ou `tauri-winrt-notification` (le second expose un rappel de clic) ; nom et icône de l'appli à l'écran | moyen : clic non garanti, S5 |
| Icônes de fichier 32 px (par extension, ou vrai fichier pour `exe`, `msi`, `scr`, `ico` terminés), en cache | `SHGetFileInfoW` (avec `SHGFI_USEFILEATTRIBUTES` une extension suffit, sans fichier factice) puis PNG en `data:` URL | moyen, S5 ; repli : `null`, l'interface sait l'afficher |
| Barre de menus (Fichier, Téléchargements, Affichage, Aide ; accélérateurs ; cases radio) | API menu de Tauri 2 ; le **modèle de menu est une fonction pure** testée, comme `menuTemplate` ; la barre reste visible | faible |
| Zoom avant, arrière, normal et plein écran | éléments de menu personnalisés (pas de rôle prêt à l'emploi) avec les mêmes raccourcis qu'Electron (Ctrl++, Ctrl+-, Ctrl+0, F11) : zoom du webview et plein écran de la fenêtre | faible ; les pas du zoom peuvent différer |
| Instance unique : une seconde instance réveille la première puis quitte ; `--setup-tools` n'en prend pas | **verrou maison dépendant du dossier de données**, comme celui d'Electron : le plugin `tauri-plugin-single-instance` 2.5.5 nomme son mutex d'après l'identifiant Tauri (lu dans son code), donc un test ou une copie de développement serait renvoyé vers l'appli installée. Plus simple : n'activer le plugin que si `--data-dir` est absent | moyen |
| Profil WebView2 | par défaut celui de Tauri (`%LOCALAPPDATA%\<identifiant>\EBWebView`). Avec `--data-dir`, `<dossier>\webview` : WebView2 refuse de démarrer si deux processus partagent un profil avec des options de navigateur différentes (cas du test, qui ajoute `--remote-debugging-port`) | faible |
| Démarrage avec Windows | plugin autostart, valeur Run « Tachy Download » avec `--hidden`, réécrite à chaque lancement, jamais dans un build de développement | faible |
| Dialogues natifs | plugin dialog : choix de dossier (fenêtre principale et confirmation), message d'information à bouton « OK », boîte d'erreur, avertissement d'arrêt à deux boutons « Annuler l’arrêt » (par défaut) et « OK » ; textes repris du JS | faible ; le bouton par défaut peut demander `MessageBoxW` direct |
| Action de fin de file (quitter, veille, extinction avec annulation) | mêmes commandes : `rundll32 powrprof.dll,SetSuspendState 0 1 0`, `shutdown /s /t 60 /c …`, `shutdown /a` ; l'action est remise à `none` avant de s'exécuter | faible |
| Presse-papiers, ouvrir ou afficher un fichier ou un dossier, lien externe | plugins officiels (clipboard-manager, opener) | faible |
| Chiffrement des identifiants | DPAPI (`CryptProtectData`) par la crate `windows` | voir §6.2 |
| Dossiers | données : `%APPDATA%\Tachy Download` calculé **explicitement** (`FOLDERID_RoamingAppData`), pas le dossier que Tauri dérive de l'identifiant ; téléchargements : `FOLDERID_Downloads` ; mises à jour : `%TEMP%\tachy-update` | faible |
| Reprise des données de l'ancien nom « Download Manager » (une fois, si `extension-token.txt` y existe et pas dans le dossier courant ; supprime la valeur Run « Download Manager » dans une appli installée) | port tel quel de `migrateOldData`, pour la parité | faible |

### 4.9 Sécurité

- **API locale :** les trois mêmes contrôles (en-tête `Host`, `Origin` d'extension seulement, jeton comparé en temps constant), 127.0.0.1 seulement, ports 6800 à 6809.
- **Tauri :** capacités minimales par fenêtre ; les commandes de confirmation sont réservées aux fenêtres `confirm-*` ; aucune permission shell générique ; CSP stricte.
- Les arguments des commandes sont validés comme `str()` aujourd'hui : texte, longueur ≤ 8192 sauf mention contraire (100 pour un identifiant, 300 pour un hôte), sinon l'erreur « Paramètre invalide ».

## 5. Contrats à conserver à l'identique

### 5.1 Données

Dossier `%APPDATA%\Tachy Download` (remplaçable par `--data-dir=<dossier>` ou `TACHY_DATA_DIR`, pour les tests).

| Fichier | Contenu | Règle |
|---|---|---|
| `downloads.json` | `{ "settings": {…}, "tasks": [état…] }` | forme JSON identique ; les tâches **gardent leurs champs inconnus** ; les réglages sont normalisés (clés inconnues perdues, comme `_normalize` aujourd'hui) ; écriture atomique (`.tmp` puis renommage). Fichier illisible : démarrage à vide comme aujourd'hui, avec en plus une **copie de sécurité** `downloads.json.illisible` avant d'écraser (mon choix) |
| `logins.json` | tableau `{host, type, user, secret}`, `secret` en base64 | voir §6.2 |
| `extension-token.txt` | 48 caractères hexadécimaux | gardé tel quel s'il existe (≥ 32 caractères) ; sinon 24 octets aléatoires en hexadécimal. Le `0o600` d'aujourd'hui n'a aucun effet sous Windows : rien de plus à faire |
| `tools\` | `yt-dlp.exe`, `ffmpeg.exe`, `deno.exe` | gardé |
| `ytdlp-last.log` | dernier diagnostic yt-dlp | écrit comme aujourd'hui |
| `Local State` | fichier Chromium d'Electron | **lu une fois** pour la migration des identifiants (§6.2), jamais écrit |
| `Cache`, `Local Storage`, `icon-probe`… | restes d'Electron | ignorés |

Au chargement, un téléchargement `downloading` devient `paused` ; un `completed` perd son cookie. Les réglages sont normalisés (annexe A).

### 5.2 Instantané envoyé à l'interface

C'est l'état de la tâche (annexe C) plus des champs calculés : `downloaded`, `speed` (octets/s, fenêtre de 3 s, 0 si pas en cours), `eta` (secondes ou `null`), `headers.cookie` vidé, `segments` copiés. Pour HLS : une seule voie fictive (`segments = [{start: 0, end: total - 1, pos: downloaded}]`) et `hlsProgress`, `hlsTotal`, `hlsDone`, `hlsActive`, `hlsLive`. Noms en camelCase, dates en millisecondes depuis l'époque, `kind ∈ {file, hls, ytdlp}`, `status ∈ {queued, scheduled, downloading, paused, completed, error}`.

### 5.3 API locale pour l'extension

Contrôles, **dans l'ordre exact de `src/server.js`** : (1) `Host` égal à `127.0.0.1:<port>` ou `localhost:<port>`, sinon 403 `{"error":"host"}` ; (2) `Origin` présent et ne correspondant pas à `^(chrome|moz|edge)-extension://[a-z0-9-]+$` (insensible à la casse), sinon 403 `{"error":"origin"}` ; (3) `OPTIONS` : 204 ; (4) `GET /ping` : sans jeton ; (5) jeton `X-DM-Token` comparé en temps constant, sinon 401 `{"error":"token"}` ; (6) les routes ; (7) sinon 404 `{"error":"not found"}` (un chemin inconnu sans jeton répond donc 401, pas 404).

Toutes les réponses : JSON, `Cache-Control: no-store` ; si l'`Origin` est autorisée, `Access-Control-Allow-Origin: <origine>` et `Vary: Origin`. Le 204 d'`OPTIONS` ajoute `Access-Control-Allow-Methods: GET, POST, OPTIONS`, `Access-Control-Allow-Headers: Content-Type, X-DM-Token`, `Access-Control-Max-Age: 600`.

| Route | Jeton | Entrée → réponse |
|---|---|---|
| `GET /ping` | non | → `{"ok": true, "app": "tachy-download"}` |
| `GET /refresh-requests` | oui | → `{"requests": [{id, filename, size, kind, pageUrl, auto}]}` (les tâches `error` marquées `needsRefresh`) |
| `POST /add` | oui | corps JSON `url`, `referer`, `origin`, `cookie`, `userAgent`, `filename`, `page`, `kind` (`hls` ou `ytdlp`, sinon fichier), `sniffed` (booléen), `size` (nombre, pour la fenêtre de confirmation) → `{"id"}`, ou `{"pending": true}` quand la fenêtre de confirmation s'ouvre (réglage `confirmDownloads` ; l'adresse doit alors commencer par `http://` ou `https://`, sinon 400 `{"error":"Adresse invalide"}`) |
| `POST /refresh` | oui | corps `id`, `url`, `referer`, `origin`, `cookie`, `userAgent` → `{"id"}` |

Corps > 64 Kio : 413 `{"error":"too large"}` et la connexion est coupée. JSON invalide ou refus du gestionnaire : 400 `{"error": <message>}` (le texte d'un JSON invalide peut différer de celui de Node). Chaînes tronquées à 8192 caractères. Écoute sur `127.0.0.1` seulement, de 6800 à 6809 (10 essais quand le port est pris) ; échec final : boîte d'erreur « Le serveur local n’a pas pu démarrer : … » et l'appli continue sans API.

### 5.4 Pont `window.dm` et `confirmApi`

La liste exacte des méthodes est celle de `src/preload.js` et `src/confirm-preload.js`. Les **noms d'événements `dm:*` doivent rester identiques** ; les noms de commandes Tauri sont libres. Le tableau donne le sens :

| `window.dm.` | Commande Tauri | Arguments → résultat |
|---|---|---|
| `list()` | `dm_list` | → instantanés |
| `state()` | `dm_state` | → `{settings, port, version, update, token, glass}` |
| `add(urls, opts)` | `dm_add` | 1 à 200 liens, `opts{startAt?, kind?}` → `{added, errors[{url, error}]}` |
| `openPage(id)` | `dm_open_page` | `pageUrl` http(s), sinon erreur « Page d’origine inconnue » |
| `fileIcon(id)` | `dm_file_icon` | → `data:` URL ou `null` |
| `toolsStatus()`, `installTool(name)` | `dm_tools_status`, `dm_tools_install` | `ytdlp`, `ffmpeg`, `deno` |
| `pause`, `resume`, `redownload`, `remove(id, deleteFile)`, `setLimit(id, kbps)` | `dm_pause`… | comme `manager.js` |
| `reveal(id)`, `open(id)` | `dm_reveal`, `dm_open` | le fichier terminé, sinon le `.part` ; `open` refuse un `.part` ; erreur « Téléchargement introuvable » |
| `setSettings(patch)` | `dm_settings_set` | → réglages normalisés ; réapplique le démarrage auto |
| `pickDir()`, `copyToken()`, `copyDiagnostic()` | `dm_pick_dir`, `dm_copy_token`, `dm_copy_diagnostic` | |
| `checkUpdate()`, `installUpdate()` | `dm_update_check`, `dm_update_install` | |
| `logins()`, `saveLogin(entry)`, `removeLogin(host)` | `dm_logins_*` | |
| `onUpdate`, `onRemoved`, `onSettings`, `onTools`, `onUpdateAvail`, `onUpdateProgress`, `onMenu` | événements `dm:update`, `dm:removed`, `dm:settings`, `dm:tools`, `dm:updateAvail`, `dm:updateProgress`, `dm:menu` | `dm:menu` porte `{action, arg}` ; un élément de menu ou de la zone de notification qui agit sur l'interface montre d'abord la fenêtre |
| `confirmApi.init`, `category`, `pickDir`, `probe`, `answer` | `confirm_init`, `confirm_category`, `confirm_pick_dir`, `confirm_probe`, `confirm_answer` | réservées aux fenêtres de confirmation |

### 5.5 Messages en français

Tous les textes visibles (erreurs, boîtes de dialogue, menus, infobulles, notifications) sont repris **mot pour mot**. Sources : `friendlyError` et `httpError`, `explain` de `ytdlp.js`, messages de `manager.js`, `updater.js`, `logins.js`, `tools.js`, `main.js`, `menu.js`. Deux garde-fous : les tests portés vérifient les messages qu'ils déclenchent ; une liste de chaînes établie une fois à partir de ces modules (fixture, §7.1) est cherchée dans les sources Rust (garde-fou contre l'oubli, pas preuve d'usage).

### 5.6 Ligne de commande

`--hidden` (démarrer masqué), `--setup-tools` (reprise de l'ancien nom, puis installer yt-dlp, deno, ffmpeg dans cet ordre en sautant ceux déjà présents et en ignorant les échecs, puis quitter, sans fenêtre ni verrou, plafonné à 8 minutes), `--data-dir=<dossier>` (nouveau, pour les tests). `--setup-tools` est traité avant de créer l'application Tauri.

### 5.7 Démarrage et arrêt

**Démarrage normal**, dans l'ordre de `main.js` : (1) verrou d'instance unique ; (2) reprise de l'ancien nom (§4.8) ; (3) jeton ; (4) outils (événement `dm:tools`), vérification de yt-dlp 20 s après le lancement puis toutes les 6 h, vérification de l'appli 45 s après puis toutes les 12 h seulement si l'appli est installée (pas en développement) et `autoUpdate` actif ; (5) identifiants ; (6) gestionnaire, avec pour réglages de départ `dir` = dossier Téléchargements et `categories: true` (premier lancement), puis chargement de `downloads.json` ; (7) API locale ; (8) événements du gestionnaire vers l'interface (`dm:update`, `dm:removed`, `dm:settings` qui reconstruit aussi le menu), démarrage du gestionnaire ; (9) commandes, menu, fenêtre, zone de notification, démarrage automatique.

**Arrêt :** `shutdown()` du gestionnaire (tâches en cours → `paused`, sauvegarde immédiate) et fermeture de l'API, puis sortie. « Quitter » (menu, zone de notification, action de fin) quitte pour de bon ; la croix de la fenêtre obéit à `closeToTray` (§4.7).

## 6. Reprise des données et installation

### 6.1 Données

Même dossier, mêmes fichiers : rien à convertir. L'extension déjà configurée continue de marcher parce que `extension-token.txt` est conservé.

### 6.2 Identifiants (`logins.json`)

Format conservé. Un secret se lit de deux façons, reconnues à ses premiers octets une fois décodé :
- **bloc DPAPI** (`CryptProtectData`) : le format de la 2.0 ;
- **`v10`** : le format d'Electron `safeStorage` sous Windows (Chromium OSCrypt) : préfixe `v10`, nonce de 12 octets, chiffré AES-256-GCM avec étiquette de 16 octets ; la clé de 32 octets est dans `Local State`, `os_crypt.encrypted_key` (base64, préfixe `DPAPI`, enveloppée par DPAPI).

Au premier lancement : copie de sécurité `logins.json.electron.bak`, lecture des `v10`, réécriture en DPAPI. **À prouver par S3** ; repli : l'interface demande de ressaisir les identifiants. La migration a besoin de `Local State`, que la désinstallation de l'Electron laisse en place ; si tu effaces aussi le dossier de données, les identifiants (et tout le reste) disparaissent.

### 6.3 Préférences d'interface

Thème, couleur, force du verre, sections repliées sont dans `localStorage` (`dm-theme`, `dm-look-*`, clés du verre et des types), que Tauri ne peut pas reprendre d'Electron : **remises à zéro une fois** (§10.3). Option décidée en M5 : une dernière 1.x qui exporte ces valeurs dans un petit fichier que le pont réinjecte.

### 6.4 Installation de la 2.0 et sort de l'Electron installée

**Ce que tu as dit :** l'appli installée en Electron va être supprimée. **Ma lecture :** pas de montée de version « en place » à garantir ; la 2.0 s'installe comme une nouvelle appli. (À confirmer à la relecture : c'est la seule ligne de ce document qui repose sur une interprétation.)

**Ce que ça implique.**
1. Tu désinstalles l'Electron avant d'installer la 2.0 (« Applications installées »). Son désinstalleur garde `%APPDATA%\Tachy Download` (`deleteAppDataOnUninstall: false`) : les données ne bougent pas.
2. **L'extension.** Si tu l'as chargée « non empaquetée » depuis `…\resources\extension` de l'ancienne installation, ce dossier disparaît avec elle : il faut la recharger depuis le nouveau `<dossier d'installation>\resources\extension` (ou depuis `extension/` du dépôt). Le jeton ne change pas. L'installeur 2.0 doit donc placer l'extension à ce même endroit relatif.
3. Ne jamais lancer l'Electron et la Rust en même temps sur le même dossier de données. Pendant la réécriture, les essais de la Rust se font avec `--data-dir` sur une copie.
4. La valeur Run « Tachy Download » (démarrage avec Windows) est réécrite par la 2.0 à chaque lancement quand « Lancer avec Windows » est actif ; une valeur laissée par l'Electron est donc simplement écrasée.
5. **Installeur :** bundler NSIS de Tauri, `installMode: currentUser`, dossier modifiable, raccourcis bureau et menu Démarrer, WebView2 en `downloadBootstrapper`, extension copiée à `resources\extension`, nom final `Tachy-Download-Setup-<version>.exe` (renommé par le script de release). Un hook `NSIS_HOOK_POSTINSTALL` lance `--setup-tools` (idempotent, n'échoue jamais l'installeur). Un hook de désinstallation fait que la case « supprimer les données » du gabarit (qui ne vise que `%APPDATA%\<identifiant>` et `%LOCALAPPDATA%\<identifiant>`) supprime aussi `%APPDATA%\Tachy Download` quand elle est cochée (mon choix).
6. **Mises à jour 2.x → 2.y :** même principe qu'aujourd'hui (releases GitHub, fichier `Tachy-Download-Setup-*.exe` avec `digest` sha256, téléchargement dans `%TEMP%\tachy-update`, vérification), puis lancement détaché de `<installeur> /S /UPDATE /R` et sortie immédiate. Ce sont les indicateurs natifs du gabarit Tauri (lus dans son code : `/S` silencieux, `/UPDATE` mode mise à jour, `/R` relance l'appli une fois installé, `/ARGS` pour ses arguments ; l'installeur gère lui-même l'appli en cours d'exécution). **À prouver par S4.**
7. Une 1.x encore installée ailleurs se mettra à jour toute seule vers la 2.0 si la release 2.0 est « latest » (son updater lit `releases/latest` et lance `/S --updated --force-run`). L'installeur 2.0 s'installera alors tel quel, à côté : deux entrées « Tachy Download » dans « Applications installées » jusqu'à la désinstallation de l'ancienne. Aucun traitement spécial prévu ; acceptable si ton installation est la seule.

**Option écartée (si tu changes d'avis) : montée en place depuis la 1.x.** Faits déjà relevés : l'installeur electron-builder est par utilisateur (`perMachine: false`), son entrée de désinstallation (clé propre à electron-builder, à relever sur une vraie installation) porte `DisplayName` = « Tachy Download » (`uninstallDisplayName`) et son désinstalleur s'appelle `Uninstall Tachy Download.exe` (dossier par défaut `%LOCALAPPDATA%\Programs\Tachy Download`) ; la clé de désinstallation du gabarit Tauri est le nom du produit, donc une autre : il ne remplace pas l'ancienne ; il faudrait un hook `NSIS_HOOK_PREINSTALL` qui retrouve l'ancien désinstalleur, le lance en silencieux avec `/S _?=<dossier>` (sans ce paramètre il rend la main aussitôt) et garde les données, plus la lecture de `--updated --force-run` que le gabarit ne connaît pas ; repli : une dernière 1.x qui annonce la 2.0 et ouvre la page de la release. Cela ajouterait un jalon et ferait de S4 une preuve lourde.

### 6.5 Retour arrière

Les données restent lisibles par une 1.x, sauf les identifiants migrés (copie `.bak`).

## 7. Tests et critères d'acceptation

### 7.1 Niveaux

1. **Rust (`cargo test`)** : portage des tests JS avec les mêmes serveurs HTTP locaux (`test/helpers.js` devient un module de tests) et un faux yt-dlp (le `test/fake-ytdlp.js` actuel peut servir tel quel tant que Node est installé). On écrit le test avant le code, comme aujourd'hui (TDD).
2. **Contrats** : `tools/make-fixtures.js` pilote le gestionnaire JS actuel (serveurs locaux) pour produire : (a) un `downloads.json` avec une tâche par type (`file`, `hls`, `ytdlp`) et par statut ; (b) les instantanés correspondants (`list()`) ; (c) les réglages par défaut et normalisés ; (d) des réponses d'API (chaque route, chaque erreur) ; (e) la table des types de fichiers (`ui/shared/filetypes.js` est gardé par l'interface, `filetypes.rs` doit lui rester identique) ; (f) la liste des chaînes françaises (§5.5). Un test Rust charge (a), vérifie qu'aucun champ de tâche ne se perd à la réécriture, et compare (b) à (f). Tant que le JS existe, un test JS vérifie que les fixtures committées sont à jour. Elles sont produites avant la suppression du JS, puis figées.
3. **Interface** : `test/ui.test.js` adapté. Il lance l'exécutable avec `--data-dir=<tmp>` et `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<p>`, et parle au même protocole CDP qu'aujourd'hui. Trois différences à régler : (a) le test actuel lit le menu et la fenêtre **dans le processus principal** (`--inspect` de Node, `Menu.getApplicationMenu()`), ce qui n'existe pas en Rust : le menu est vérifié sur son modèle pur (étiquettes, cases cochées selon les réglages, raccourcis, action de chaque entrée) par des tests Rust, et son branchement à l'interface par un point d'entrée de test compilé seulement avec une fonction Cargo `e2e` ; (b) le verrou d'instance unique et le profil WebView2 doivent dépendre de `--data-dir` (§4.8), sinon le test se heurte à l'appli lancée à côté ; (c) l'API prend le premier port libre à partir de 6800, comme aujourd'hui.
4. **Manuel** : la liste de parité (§7.4) sur sites réels.

### 7.2 Correspondance des fichiers de tests

Règle : un fichier qui importe `src/` se porte en Rust ; un fichier qui n'importe que `extension/` reste en JS.

| Fichier (tests) | Importe | Devient |
|---|---|---|
| `engine` (13), `steal` (2), `retry` (4), `network` (8), `tasklimit` (3), `cookies` (2), `siteauth` (3), `refresh` (2), `redownload` (7), `server` (4), `afterdownload` (6), `tools` (4), `updater` (6), `hls` (7), `features` (7), `ytdlp-run` (18), `ytopts` (6), `ytprogress` (6), `replay` (7) | `src/` | Rust, `tachy-core/tests/` |
| `afterall` (3), `shell` (11) | `src/` | Rust : logique pure dans le noyau, menu et progression dans l'application |
| `extension` (10), `sniff` (9) | `extension/` | restent en JS |
| `ui` (4) | rien (lance l'appli) | JS adapté (§7.1.3) |

### 7.3 Banc de performance et protocole de mesure

`tools/bench/` reprend le banc écrit le 2026-11-04 : un serveur local qui répond à `Range` en continu, et un client qui pilote le moteur actuel (débit moyen, débit stable entre 10 et 90 %, CPU applicatif et noyau, pic de mémoire), avec `curl` comme plafond de référence. `crates/tachy-core/examples/bench.rs` fait la même mesure sur le moteur Rust, contre **le même serveur**. L'API locale n'a pas de route de liste : le banc Rust pilote le gestionnaire directement. Le résultat de M0 sur la 1.2.0 est la référence ; c'est la barrière de §1.

Protocole : 1 Gio, boucle locale, 1, 8 et 16 connexions, trois passes, médiane ; build release avec LTO ; même machine, mêmes dossiers, même antivirus. **RAM au repos :** mémoire privée de tout l'arbre de processus de l'appli (Electron : principal, rendu, GPU, utilitaires ; Tauri : l'exécutable et ses `msedgewebview2.exe`), relevée 60 s après le lancement, fenêtre visible, liste vide, médiane de trois lancements. **Démarrage à froid :** du lancement du processus au premier `dm.list()` résolu côté interface (mesuré par CDP).

### 7.4 Liste de parité

Chaque ligne de « Fonctions » du README, plus les menus, les entrées de la zone de notification et les dialogues de `main.js`, devient une case à cocher dans `docs/superpowers/specs/2026-11-04-rust-tauri-rewrite-parity.md` (créé en M0), avec le moyen de la vérifier (test automatique ou geste manuel). Aucune case ne reste ouverte à la livraison.

## 8. Plan par jalons

| Jalon | Contenu | Terminé quand | Dépend de |
|---|---|---|---|
| **M0 Bases et preuves** | Branche `rust-tauri`. `ui/` déplacé (§4.1), Electron pointe dessus. Workspace qui compile (CRT statique), fenêtre Tauri qui affiche l'interface avec un faux noyau. Référence 1.2.0 mesurée (S0). Document de parité créé. Preuves S1 à S5 consignées en §10.4. | les cinq preuves sont tranchées (go, ou repli décidé) ; les mesures de référence sont écrites | — |
| **M1 Noyau** | `settings`, `store`, `limiter`, `net`, `probe`, `fsutil`, `errors`, `filetypes`, `rules`, `task/http` (segments, vol, reprise, tentatives, connexions adaptatives, en-têtes, liens expirés), `manager` (file, planification, nouvelles tentatives, `redownload`, `refresh`), `api` | tests portés verts (§7.2, lignes du noyau hors HLS, yt-dlp, outils, mise à jour) ; contrats verts ; banc ≥ référence | M0 |
| **M2 HLS** | `task/hls` : playlists, AES-128, avance de 128 Mio, conversion ffmpeg | `hls` (7) vert ; vidéo réelle de bout en bout | M1 |
| **M3 Vidéos** | `task/ytdlp`, `tools`, sondage, playlists, chemin « fichier direct » | `ytdlp-run`, `ytopts`, `ytprogress`, `features`, `tools`, `replay` verts | M1 |
| **M4 Coque** | Fenêtres, pont complet, menu (zoom, plein écran), zone de notification, barre des tâches, notifications, dialogues, icônes, confirmation, instance unique, démarrage auto, action de fin, verre, réflexes de navigateur neutralisés | e2e UI vert ; interface identique en comparant des captures avec Electron | M0 (faux noyau), puis M1 |
| **M5 Données et mise à jour** | `logins` + migration DPAPI/`v10`, `postprocess` (doublons, désarchivage), `updater` (`/S /UPDATE /R`), préférences d'interface | `shell`, `afterdownload`, `updater`, `afterall` verts ; migration vérifiée sur un vrai dossier de données | M1 |
| **M6 Installeur et outillage** | Hooks NSIS (`--setup-tools`, case « supprimer les données »), installeur renommé, script de release (`tools/release.js` : versions Cargo, Tauri et `package.json`, étiquette, fichier publié), workflows CI (Rust : `fmt`, `clippy -D warnings`, `test` ; Node : tests JS restants ; release), README et `BUILD` (EN et FR), `npm run dist` conservé comme point d'entrée | installation propre, mise à jour par-dessus et désinstallation vérifiées sur un vrai installeur ; CI verte | M4, M5 |
| **M7 Livraison** | Liste de parité complète, banc et mesures finales, dogfooding, fusion dans `main`, suppression du JS d'Electron, version 2.0.0 | barrières de §1 franchies | tout |

**Parallélisme.** Après M1, M2, M3, M4 et M5 touchent des fichiers disjoints (`task/hls.rs` ; `task/ytdlp.rs` et `tools.rs` ; `src-tauri/` ; `logins.rs`, `postprocess.rs`, `updater.rs`) et peuvent avancer en même temps. M0 et M1 sont séquentiels.

## 9. Preuves à faire en premier (M0) et risques

| Preuve | Question | Si non |
|---|---|---|
| **S0 Référence** | Construire la 1.2.0 (`npm run dist`), noter taille de l'installeur, RAM au repos, démarrage à froid, banc (§7.3) | — (c'est la base de toutes les comparaisons) |
| **S1 Verre** | Fenêtre transparente + `apply_acrylic` avec la vraie interface : lisible, sans saccade au redimensionnement, thèmes clair et sombre | repli : fond opaque (le réglage existe déjà) |
| **S2 HTTP** | `reqwest` en HTTP/1.1 : N requêtes `Range` en parallèle ouvrent N connexions TCP ; débit et CPU sur le banc face à Node et à `curl` | fixer les objectifs réalistes ; si le moteur n'égale pas Node, on cherche (tailles de blocs, écriture) avant d'aller plus loin |
| **S3 Identifiants** | Produire un secret avec `safeStorage` (script Electron sans fenêtre, dossier de données temporaire), le déchiffrer en Rust | repli : ressaisie des identifiants, copie `.bak` |
| **S4 Installeur** | L'installeur Tauri se construit ; installation silencieuse dans un dossier temporaire (`/D=`) avec raccourcis, entrée de désinstallation et extension à `resources\extension` ; `--setup-tools` en fin d'installation ; mise à jour par-dessus avec `/S /UPDATE /R` (l'appli est relancée) ; désinstallation propre, données gardées. **L'essai installe un logiciel sur ta machine (dossier temporaire, désinstallé ensuite) : il faudra ton accord au moment de le faire.** | repli : hooks NSIS écrits à la main pour les flags manquants |
| **S5 Icônes et notifications** | `SHGetFileInfoW` → PNG ; clic sur une notification ; nom et icône affichés par la notification d'une appli installée | repli : icône `null` ; notification sans clic |

| Risque | Parade |
|---|---|
| Régressions silencieuses sur des sites réels (en-têtes, `If-Range`, liens expirés, HLS) | tests portés d'abord, fixtures de contrat, liste de parité, essai sur tes sites |
| L'interface diverge pendant la réécriture (corrections sur `main`) | `ui/` partagée, fusion régulière de `main` dans la branche |
| HTTP/2 négocié par erreur | `http1_only`, test qui compte les connexions |
| Binaire Rust non signé : SmartScreen (déjà le cas) et faux positifs d'antivirus possibles | vérifier à chaque build sur Windows Defender ; pas de signature prévue |
| Dépendances Rust | `cargo audit` en CI (optionnel) |
| Temps de compilation et taille de l'exécutable | profil release avec LTO et `strip` |
| WebView2 absent sur un Windows 10 ancien | installeur en mode `downloadBootstrapper` |
| Encodage des sorties yt-dlp | variables UTF-8 comme aujourd'hui |
| Deux apps sur le même dossier de données pendant les essais | `--data-dir` sur une copie (§6.4) |

## 10. Décisions

### 10.1 Ce que tu as décidé

1. Réécrire toute l'application en Rust.
2. Tauri : moteur et coque en Rust, interface actuelle gardée.
3. Parité avec la 1.2.0, branche séparée, tests d'abord, trois preuves avant de s'engager (ton « Ok » à mon cadrage).
4. L'appli installée en Electron va être supprimée (ma lecture en §6.4).

### 10.2 Mes choix (à relire)

1. Branche `rust-tauri` ; `main` reste l'Electron 1.x (avec ses corrections) jusqu'à la bascule ; la 2.0.0 sort à parité.
2. Noyau sans Tauri + application en coque.
3. HTTP/1.1 seulement ; magasin de certificats de Windows ; **plus de 60 s sans donnée = connexion perdue** (Node : 5 minutes).
4. Job Object pour tuer l'arbre de processus de yt-dlp.
5. Écritures disque par blocs ≥ 256 Kio.
6. Textes français repris mot pour mot, contrôlés par des tests.
7. Fixtures de contrat produites par l'appli JS avant sa suppression.
8. WebView2 en `downloadBootstrapper` ; CRT statique.
9. Identifiant Tauri `local.downloadmanager.app` (celui d'aujourd'hui, utilisé pour l'identité des notifications) à changer si l'outil Tauri refuse le suffixe `.app` ; exécutable `tachy-download.exe`.
10. Barrière de performance = ne pas être pire que le moteur actuel ; les objectifs chiffrés sont fixés après S2.
11. Pas de chaîne de montée de version depuis la 1.x (lecture de ton message, §6.4).
12. Verrou d'instance unique maison, ou plugin désactivé avec `--data-dir` ; profil WebView2 dans `<--data-dir>\webview` pour les tests.
13. La case « supprimer les données » du désinstalleur supprime aussi `%APPDATA%\Tachy Download`.
14. Copie de sécurité `downloads.json.illisible` avant d'écraser un fichier illisible.
15. Reprise des données de l'ancien nom « Download Manager » portée telle quelle (parité, quelques lignes).
16. Réflexes de navigateur de WebView2 neutralisés (menu contextuel, rechargement, impression, zoom à la molette).

### 10.3 Différences visibles acceptées

- **Thème, couleur, force du verre et sections repliées sont remis à zéro une fois** (§6.3).
- La 1.x installée ne se met pas à jour toute seule vers la 2.0 : elle est supprimée puis la 2.0 installée. L'extension chargée depuis l'ancien dossier d'installation est à recharger (§6.4).
- Le clic sur une notification peut ne pas ouvrir le dossier (S5).
- Les pas du zoom peuvent différer légèrement.
- Un téléchargement sans aucune donnée pendant 60 s est relancé plus tôt qu'avant.
- Les raccourcis et le dossier d'installation sont ceux du nouvel installeur (par défaut `%LOCALAPPDATA%\Tachy Download` au lieu de `%LOCALAPPDATA%\Programs\Tachy Download`).
- Le texte d'erreur d'un JSON invalide envoyé à l'API peut différer.

### 10.4 Résultats des preuves

Mesures du 2026-11-04 sur la machine de développement (Windows 11, 10.0.26300), un seul utilisateur, antivirus actif. Les scripts sont dans `tools/bench/` et `spikes/` (sondes jetables, supprimées en M7).

**S0 Référence 1.2.0 (Electron 45).**

| Mesure | Résultat |
|---|---|
| Installeur `Tachy-Download-Setup-1.2.0.exe` | 107,7 Mio (dossier déballé : 305,6 Mio) |
| RAM au repos (mémoire privée des 4 processus, 60 s après le lancement, 3 lancements) | 240,5 · 205,4 · 201,8 → **médiane 205,4 Mio** |
| Démarrage à froid jusqu'à `dm.state()` résolu (`dm.list()` suit aussitôt) | 12 326 · 623 · 605 ms → **médiane 623 ms** ; le premier lancement après la construction paie l'analyse antivirus des 305 Mio |
| Moteur Node, 1 Gio en boucle locale, débit moyen (6 passes) | 1 connexion ≈ 380 Mo/s · 8 ≈ 250 · 16 ≈ 170 (très variable : 109 à 282 à 16 connexions) |
| Moteur Node, CPU (utilisateur + noyau) par Gio | 3,5 s · 4,2 s · 4,7 s ; le noyau pèse 55 à 60 % ; RSS du processus du banc 155 à 164 Mio |
| `curl` sur le même serveur (plafond d'un flux) | 640 à 940 Mo/s |

**S2 HTTP : GO, largement.** `reqwest` 0.13 (`default-features = false`, HTTP/1.1, sans proxy système, redirections laissées au moteur), segments `Range` en parallèle, un descripteur par segment, tampon de 1 Mio, `seek_write` hors du fil asynchrone, fichier `.part` préalloué. Même séance, mêmes serveur et disque, 3 passes :

| Connexions | Débit moyen Rust | Débit moyen Node | CPU par Gio Rust | CPU par Gio Node | Pic mémoire Rust |
|---|---|---|---|---|---|
| 1 | 787 Mo/s | 417 Mo/s | 1,28 s | 3,47 s | 10 Mio |
| 8 | 1 979 Mo/s | 306 Mo/s | 1,55 s | 4,07 s | 28 Mio |
| 16 | 1 946 Mo/s | 181 Mo/s | 2,12 s | 4,64 s | 46 Mio |

Le serveur a vu exactement N connexions TCP pour N plages (plus une pour le `HEAD`) : le HTTP/1.1 force bien une connexion par plage (à revérifier en HTTPS avec ALPN en M1). Le contenu écrit est contrôlé (taille et 2 000 octets répartis). Variantes à 8 connexions : tampon de 256 Kio, 1 Mio et 4 Mio équivalents (mémoire 20, 27, 56 Mio) → **1 Mio retenu** ; sans préallocation : équivalent ; **fichier clairsemé** (`FSCTL_SET_SPARSE`) : 0,27 s au lieu de 0,5 s (le noyau ne remplit plus de zéros les trous entre segments) mais autant de CPU et un fichier qui reste « clairsemé » : **non retenu**, sans objet sur un vrai réseau. L'écart avec Node vient surtout des fils d'exécution : Node traite tout dans un fil JavaScript (≈ 100 % d'un cœur), Rust répartit réseau et disque sur plusieurs cœurs.

**Objectifs chiffrés fixés (barrière de §1).** Sur le banc de 1 Gio en boucle locale, médiane de 3 passes alternées avec le moteur Node dans la même séance : débit à 8 et 16 connexions ≥ 2 × celui de Node ; CPU par Gio ≤ 60 % de celui de Node ; pic mémoire du moteur à 16 connexions ≤ 100 Mio. Installeur publié ≤ 25 Mio. RAM au repos et démarrage à froid de l'application Tauri en build release : à comparer à 205,4 Mio et 623 ms une fois mesurés (objectif : pas pire).

**S3 Identifiants : GO.** Un script Electron 45 sans fenêtre, avec un dossier de données temporaire, produit trois secrets synthétiques (ASCII ; accents, CJK et émoji ; 4 096 octets) avec `safeStorage.encryptString(…).toString('base64')`, comme `src/main.js`. La sonde Rust lit `Local State` (`os_crypt.encrypted_key` = `DPAPI` + blob), le déchiffre avec `CryptUnprotectData` (32 octets), puis ouvre chaque secret (`v10` + nonce de 12 octets + AES-256-GCM) : les trois sont identiques à l'original. Aller-retour `CryptProtectData` / `CryptUnprotectData` direct également bon. La migration (§6.2) est donc faisable telle que décrite.

**S5 Icônes (première moitié) : GO.** `SHGetFileInfoW` (`SHGFI_USEFILEATTRIBUTES` pour une extension, sans fichier) puis `GetIconInfo` et `GetDIBits` donnent un PNG RGBA de 32 × 32 px (comme `getFileIcon({size: 'large'})` d'Electron) de 1 à 2,6 Kio : `.zip` (WinRAR), `.pdf`, un type inconnu (page blanche), et le vrai fichier `notepad.exe` (son icône propre, transparence correcte). Le repli par le masque est en place pour les vieilles icônes sans canal alpha. Le clic sur une notification reste à voir (S5, seconde moitié).

**S1 Verre, S4 Installeur, S5 notification :** en cours.

### 10.5 Questions ouvertes

Aucune ne bloque le démarrage. À confirmer : si d'autres personnes que toi ont la 1.x installée, elles ne seront pas mises à jour en place (§6.4, point 7).

## 11. Laissé au constructeur

Choix précis des crates (`axum` ou `hyper` pour l'API, `thiserror`/`anyhow`, `tracing`), découpage fin des modules, tailles de tampons (à mesurer), forme interne des erreurs, configuration de `clippy`, pas exacts du zoom, langue de l'installeur (le gabarit Tauri propose le français et l'anglais), pas de journal d'activité (l'appli n'en a pas aujourd'hui).

---

## Annexe A. Réglages (`settings` dans `downloads.json`)

Heures `HH:MM` en **heure locale**. Les valeurs hors bornes ou de mauvais type retombent sur le défaut (voir `_normalize` dans `manager.js`).

| Clé | Défaut | Normalisation |
|---|---|---|
| `dir` | `""` | texte ; au premier lancement et si vide : dossier Téléchargements de l'utilisateur |
| `connections` | 8 | entier 1 à 32 |
| `maxConcurrent` | 3 | entier 1 à 10 |
| `speedLimitKBps` | 0 | entier ≥ 0 |
| `videoMaxHeight` | 1080 | parmi 360, 480, 720, 1080, 1440, 2160 |
| `convertToMp4` | true | faux seulement si `false` |
| `categories` | false | booléen ; **true au premier lancement** (valeur passée par l'appli au gestionnaire) |
| `autoRetry`, `adaptive` | true | faux seulement si `false` |
| `proxy` | `""` | `normalize_proxy` ; au chargement une adresse invalide devient `""`, à l'enregistrement elle lève une erreur affichée par l'interface |
| `speedSchedule` | `{enabled: false, from: "08:00", to: "18:00", limitKBps: 500}` | heures valides, sinon défaut |
| `rules` | `""` | texte ≤ 4000 caractères |
| `duplicates` | true | faux seulement si `false` |
| `autoExtract`, `autoStart` | false | booléen |
| `closeToTray`, `notifications`, `confirmDownloads`, `autoUpdate` | true | faux seulement si `false` |
| `ytCookies` | `""` | parmi `firefox`, `edge`, `chrome`, `brave` |
| `afterAll` | `none` | parmi `none`, `quit`, `sleep`, `shutdown` |
| `schedule` | `{enabled: false, from: "01:00", to: "07:00"}` | fenêtre pouvant passer minuit |

## Annexe B. Constantes à reprendre

**Fichiers (`download.js`).** Segment minimal 1 Mio (`MIN_SEGMENT`) ; nombre de segments = min(connexions, ⌈taille / 1 Mio⌉), pas égaux, le dernier prend le reste. Vol : seulement un segment avec plus de 2 Mio restants (`MIN_STEAL`), la victime garde la première moitié. Essais par segment : 5 (`MAX_RETRIES`), remis à zéro dès qu'il y a du progrès ; attente = max(`Retry-After` plafonné à 30 s, min(8 s, 500 ms × 2^essai)). 429 et 503 : le jeton de connexion est rendu pendant l'attente et `allowed = max(1, min(allowed, workers) − 1)` si `adaptive` ; une connexion revient après 8 s calmes. Un 200 à une requête `Range` = fichier distant changé, on repart de zéro. 401, 403 et 410 = lien expiré (`needsRefresh`) ; les autres 4xx sont fatales ; 5xx et 429 se rejouent. Serveur sans `Range` : un seul flux, repart de zéro, 5 essais. Vitesse = octets des 3 dernières secondes (`SPEED_WINDOW_MS`) ÷ max(1 s, durée) ; ETA = ⌈(taille − reçu) ÷ vitesse⌉ ; mise à jour toutes les 250 ms. `If-Range` : l'ETag fort, sinon `Last-Modified`. Sondage : `Range: bytes=0-0`.

**HLS (`hls.js`).** Essais par segment 4 (`MAX_RETRIES`), attente min(6 s, 500 ms × 2^essai). Fenêtre = nombre de connexions, avance maximale 128 Mio (`MAX_AHEAD_BYTES`). Seulement les VOD (`#EXT-X-ENDLIST`). Liste maître : variante de plus grande `BANDWIDTH`. AES-128-CBC, IV de l'attribut ou numéro de séquence sur 16 octets ; toute autre méthode = erreur « Vidéo protégée (DRM) » ; clé de 16 octets exactement. fMP4 par `#EXT-X-MAP`. Extension `.ts` ou `.mp4`. Conversion `ffmpeg -c copy -f mp4 -movflags +faststart` si ffmpeg est là et `convertToMp4`. Taille estimée = octets ÷ segments écrits × total.

**Gestionnaire (`manager.js`).** Cycle 1000 ms ; sauvegarde différée 1000 ms ; nouvelles tentatives après 15, 30, 60, 120, 300, 600 s (`RETRY_DELAYS`), compteur remis à zéro si des octets ont été reçus depuis ; seulement si l'erreur est `retryable`, sans `needsRefresh`, et que `autoRetry` est actif ; fenêtre horaire (`schedule`) avec passage de minuit ; statut `scheduled` avec `startAt`.

**Mises à jour.** Appli : 45 s après le lancement puis toutes les 12 h (si installée et `autoUpdate`) ; yt-dlp : 20 s après le lancement puis toutes les 6 h, au plus une vérification réelle toutes les 20 h (fichier `ytdlp.checked`), et jamais pendant qu'une vidéo yt-dlp est en cours.

**API.** Corps ≤ 64 Kio ; champs texte ≤ 8192 caractères.

## Annexe C. Champs d'état d'un téléchargement

`id` (UUID), `kind`, `ownFolder`, `pageUrl`, `autoRefresh`, `needsRefresh`, `url`, `finalUrl`, `dir`, `filename`, `filenameLocked`, `size`, `rangeOk`, `segments[{start, end, pos}]`, `connections`, `etag`, `lastModified`, `headers{referer, origin, cookie, userAgent}`, `status`, `error`, `createdAt`, `completedAt`, `startAt`, et selon le cas : `ytOpts{audioOnly, height, lang}`, `outName`, `outStem`, `speedLimitKBps`, `retryAt`, `retryCount`, `retryBytes`, `retryable`, `hls{next, bytes, total}`, `ytd{base, cur, total}`, `frag{done, total}`, `phase`, `direct`, `replaces`, `duplicateOf`, `extracting`, `extractedTo`, `extractError`, `postDone`.

Tout champ absent prend une valeur par défaut ; tout champ inconnu est conservé. L'union exacte est celle des champs écrits par `src/engine/*.js` ; le test de contrat (§7.1.2) la vérifie.
