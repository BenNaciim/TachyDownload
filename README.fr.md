<p align="center"><img src="docs/logo.svg" alt="Tachy Download" width="420"></p>

**Français** · [English](README.md)

Gestionnaire de téléchargement rapide pour Windows : segments parallèles, reprise, file d'attente, vidéos YouTube et flux HLS, extension pour Chrome, Edge et Firefox. Open source, sans publicité ni compte.

Écrit de zéro en Rust (moteur) et Tauri 2 (fenêtre, menu, zone de notification), à partir de HTTP standard (`Range`, `Content-Range`, `If-Range`).

## Fonctions

- **Segments parallèles** : le fichier est découpé en 1 à 32 segments (jamais moins de 1 Mo) téléchargés en parallèle et écrits directement à leur position dans un fichier `.part`. **Redécoupage dynamique** : une connexion qui a fini prend la moitié du segment le plus lent encore en cours, pour que toutes restent occupées jusqu'à la fin. Le panneau de détails montre le découpage et la vitesse de chaque connexion.
- **Reprise** : les offsets de chaque segment sont enregistrés. Pause, coupure réseau ou fermeture de l'application : la reprise redemande uniquement les octets manquants. Si le fichier distant a changé (`If-Range`), le téléchargement repart de zéro.
- **Serveurs sans `Range`** : repli automatique sur un flux unique (sans reprise possible).
- **File d'attente** : nombre de téléchargements simultanés réglable.
- **Limite de vitesse** globale (Ko/s), partagée entre tous les segments.
- **Planification** : départ à une date précise, et plage horaire quotidienne (peut passer minuit) hors de laquelle les téléchargements s'arrêtent.
- **Fenêtre de confirmation** (comme IDM) : chaque téléchargement venu du navigateur ouvre une petite fenêtre avec le nom, la taille, le dossier de destination (modifiable) et trois choix : « Télécharger maintenant », « Plus tard » (ajouté en pause) ou « Annuler ». Désactivable dans Paramètres → Classement et démarrage.
- **YouTube, Vimeo, Dailymotion, Twitch (rediffusions)** via yt-dlp, ffmpeg et Deno (le moteur JavaScript dont YouTube a besoin pour proposer le H.264), téléchargés à la fin de l'installation (releases officielles GitHub, vérifiées par SHA-256). yt-dlp est remis à jour automatiquement, au plus une fois par jour. Qualité maximale réglable, H.264 + AAC de préférence. La fenêtre de confirmation analyse la vidéo et propose de choisir la **qualité** (2160p, 1080p, 720p…), la **langue audio** quand la vidéo en offre plusieurs, ou **audio seul** (MP3 avec ffmpeg).
- **Playlists YouTube** : coller l'adresse d'une playlist ajoute toutes ses vidéos (200 maximum) dans un dossier à son nom.
- **Flux HLS** (`.m3u8`, non chiffrés ou AES-128) : téléchargés en segments sur plusieurs connexions, puis convertis en `.mp4` par ffmpeg sans recompression. Un segment lent ou à réessayer ne retient pas les autres connexions : elles continuent d'avancer (jusqu'à 128 Mo en attente en mémoire) pendant que le fichier s'écrit toujours dans l'ordre.
- **Liens expirés** : si un lien signé meurt en cours de route, le fichier partiel est gardé. Pour une vidéo détectée par l'extension, celle-ci rouvre la page en arrière-plan, récupère la nouvelle adresse et la reprise continue toute seule. Pour un téléchargement direct, le bouton « Ouvrir la page » ramène à la page.
- **Classement automatique** par type : Vidéos, Musique, Images, Documents, Archives, Programmes. La colonne de gauche reprend ces types (Vidéos, Musique, Documents, Compressés, Exécutables, Images, Autres) pour filtrer la liste, avec un compteur par type.
- **Re-télécharger** : le panneau de détails d'un téléchargement terminé a un bouton « Re-télécharger » qui récupère à nouveau le fichier et **remplace** l'ancien (même nom, pas de « (1) »). Il fonctionne pour les fichiers, les flux HLS et les vidéos passant par yt-dlp. Si l'ancien fichier est ouvert dans un autre programme, l'application le dit et n'y touche pas. Un lien mort entre-temps suit le parcours « Liens expirés » ci-dessus.
- **Détails en accordéon** : un clic sur un téléchargement déroule, juste en dessous, le découpage, la vitesse de chaque connexion et les informations ; un second clic le referme. Pour un flux vidéo, la carte montre jusqu'où le fichier est écrit et entoure les segments en cours de réception, et les informations donnent le nombre de connexions au travail.
- **Mises à jour automatiques** : l’application consulte les releases GitHub du projet, télécharge le nouvel installateur, vérifie son SHA-256 puis l’installe en silence et se relance (désactivable dans Paramètres → Mises à jour). Cela suppose que les releases soient publiques.
- **Zone de notification** (la croix réduit la fenêtre, menu clic droit, vitesse en infobulle), **lancement avec Windows** et action en fin de file (quitter, veille, arrêt).
- **Liquid Glass** : fenêtre translucide (acrylique Windows 11 22H2 ou plus récent), désactivable dans Paramètres → Apparence. Thèmes clair et sombre.
- **Extension navigateur** (Chrome, Edge, Brave, Firefox) : détecte les vidéos des pages, affiche un bouton « Télécharger cette vidéo » sur le lecteur et un compteur dans la barre d'outils, capture les téléchargements et ajoute un menu clic droit « Télécharger avec Tachy Download ».
- **Flux comme le navigateur les a demandés** : l'extension garde le `Referer` et l'`Origin` que le navigateur a envoyés avec chaque flux (le cadre du lecteur, pas seulement la page) et l'application les rejoue, ce qu'exigent les hébergeurs vidéo qui les contrôlent. Elle reconnaît aussi les playlists servies sous un nom étrange (`text/plain`, sans extension, `.m3u8` dans l'adresse) en lisant leurs premiers octets, et les manifestes **DASH** (`.mpd`, non protégés), que yt-dlp télécharge et fusionne.
- **Mode diagnostic** (popup de l'extension) : enregistre ce que le navigateur a vu sur une page et pourquoi chaque réponse a été proposée ou non comme vidéo ; « Copier le diagnostic » met le rapport dans le presse-papiers.

## Installer

Téléchargez `Tachy-Download-Setup-<version>.exe` et lancez-le (installation pour l'utilisateur courant, sans droits administrateur). En fin d'installation, yt-dlp, Deno et ffmpeg sont téléchargés automatiquement ; hors connexion, ils restent installables depuis Paramètres → Vidéos.

L'installateur n'est pas signé : Windows SmartScreen affichera « Windows a protégé votre ordinateur » au premier lancement (« Informations complémentaires » puis « Exécuter quand même »). Un certificat de signature de code supprime cet avertissement.

## Lancer depuis les sources

```bash
npm install
cargo run -p tachy-download
cargo test -p tachy-core -p tachy-download
```

Les tests pilotés par navigateur (`npm test`) lancent la version de débogage : faire d'abord `cargo build -p tachy-download`. Construire l'installateur (voir [docs/BUILD.fr.md](docs/BUILD.fr.md)) : `npm run dist` le produit dans `target/release/bundle/nsis/`. Pour changer l'icône, modifier `tools/make-icon.js` puis `npm run icon`.

Les téléchargements vont par défaut dans le dossier Téléchargements. Les réglages et la liste sont enregistrés dans le dossier de données de l'application.

## Installer l'extension

**Firefox** : ouvrir `about:debugging#/runtime/this-firefox`, « Charger un module complémentaire temporaire », choisir `extension/manifest.json` (temporaire jusqu'au redémarrage ; une installation permanente demande une signature Mozilla). Puis les étapes 2 et 3.

1. **Chrome / Edge** : ouvrir `chrome://extensions`, activer le mode développeur, « Charger l'extension non empaquetée », choisir le dossier `extension/` (copié aussi dans `resources\extension` du dossier d'installation).
2. Dans l'application : Paramètres → Extension navigateur → **Copier** le jeton (le bouton devient « Copié » avec une coche pendant deux secondes).
3. Dans les options de l'extension : coller le jeton, vérifier le port, « Enregistrer et tester ».

Si l'application est fermée ou ne répond pas, l'extension laisse le navigateur télécharger normalement.

## Sécurité

L'application écoute uniquement sur `127.0.0.1` (port 6800, puis 6801… si occupé). Trois contrôles protègent l'API : l'en-tête `Host` (contre le DNS rebinding), l'origine (extensions uniquement) et un jeton secret de 192 bits propre à votre installation. Seuls les liens `http` et `https` sont acceptés. L'extension transmet aussi les cookies du site à l'application pour les téléchargements qui exigent une connexion ; ils restent sur votre machine.

## Structure

```
crates/tachy-core/  moteur (sans dépendance à l'interface) : téléchargement, HLS, vidéo (yt-dlp), gestionnaire, outils, mise à jour, API locale
src-tauri/          coque Tauri : fenêtre, menu, zone de notification, notifications, hooks de l'installateur (windows/hooks.nsi)
ui/           interface (renderer/ pages et scripts, shared/ table des types de fichiers)
extension/    extension MV3 (Chrome, Edge, Firefox)
crates/*/tests/     tests Rust (serveur HTTP local avec Range, faux yt-dlp, contrats enregistrés)
test/               tests node:test : la vraie application pilotée par sa fenêtre, l'extension
```

## Limites connues

- Pas de FTP ni de torrent.
- Pas de DRM (Netflix, Disney+, Prime… ; un manifeste DASH protégé n'est pas proposé) et pas de directs. L'extension voit ce que le navigateur charge dans ses onglets : une vidéo servie sans playlist, manifeste ou fichier reconnaissable n'est pas détectée, et rien n'est capturé hors du navigateur.
- Un hébergeur qui contrôle autre chose que `Referer`, `Origin` et les cookies (empreinte TLS, jeton à usage unique lié au navigateur) peut encore refuser l'application.
- L'extension est testée dans Chrome. Dans Firefox 159, elle a été chargée en module temporaire et pilotée de bout en bout (détection dans le cadre d'un lecteur d'un autre domaine, capture de `Referer`/`Origin`, bouton, envoi à l'application, messages d'erreur, popup) ; l'installation permanente demande toujours une signature Mozilla.
- Windows uniquement pour l'instant.
