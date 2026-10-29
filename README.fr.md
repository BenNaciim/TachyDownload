<p align="center"><img src="docs/logo.svg" alt="Tachy Download" width="420"></p>

**Français** · [English](README.md)

Gestionnaire de téléchargement rapide pour Windows : segments parallèles, reprise, file d'attente, vidéos YouTube et flux HLS, extension pour Chrome, Edge et Firefox. Open source, sans publicité ni compte.

Écrit de zéro avec Electron et Node.js, à partir de HTTP standard (`Range`, `Content-Range`, `If-Range`).

## Fonctions

- **Segments parallèles** : le fichier est découpé en 1 à 32 segments (jamais moins de 1 Mo) téléchargés en parallèle et écrits directement à leur position dans un fichier `.part`. Le panneau de détails montre le découpage et la vitesse de chaque connexion.
- **Reprise** : les offsets de chaque segment sont enregistrés. Pause, coupure réseau ou fermeture de l'application : la reprise redemande uniquement les octets manquants. Si le fichier distant a changé (`If-Range`), le téléchargement repart de zéro.
- **Serveurs sans `Range`** : repli automatique sur un flux unique (sans reprise possible).
- **File d'attente** : nombre de téléchargements simultanés réglable.
- **Limite de vitesse** globale (Ko/s), partagée entre tous les segments.
- **Planification** : départ à une date précise, et plage horaire quotidienne (peut passer minuit) hors de laquelle les téléchargements s'arrêtent.
- **Fenêtre de confirmation** (comme IDM) : chaque téléchargement venu du navigateur ouvre une petite fenêtre avec le nom, la taille, le dossier de destination (modifiable) et trois choix : « Télécharger maintenant », « Plus tard » (ajouté en pause) ou « Annuler ». Désactivable dans Paramètres → Classement et démarrage.
- **YouTube, Vimeo, Dailymotion, Twitch (rediffusions)** via yt-dlp et ffmpeg, téléchargés à la fin de l'installation (releases officielles GitHub, vérifiées par SHA-256). yt-dlp est remis à jour automatiquement, au plus une fois par jour. Qualité maximale réglable, H.264 + AAC de préférence.
- **Playlists YouTube** : coller l'adresse d'une playlist ajoute toutes ses vidéos (200 maximum) dans un dossier à son nom.
- **Flux HLS** (`.m3u8`, non chiffrés ou AES-128) : téléchargés en segments, puis convertis en `.mp4` par ffmpeg sans recompression.
- **Liens expirés** : si un lien signé meurt en cours de route, le fichier partiel est gardé. Pour une vidéo détectée par l'extension, celle-ci rouvre la page en arrière-plan, récupère la nouvelle adresse et la reprise continue toute seule. Pour un téléchargement direct, le bouton « Ouvrir la page » ramène à la page.
- **Classement automatique** par type : Vidéos, Musique, Images, Documents, Archives, Programmes.
- **Zone de notification** (la croix réduit la fenêtre, menu clic droit, vitesse en infobulle), **lancement avec Windows** et action en fin de file (quitter, veille, arrêt).
- **Liquid Glass** : fenêtre translucide (acrylique Windows 11 22H2 ou plus récent), désactivable dans Paramètres → Apparence. Thèmes clair et sombre.
- **Extension navigateur** (Chrome, Edge, Brave, Firefox) : détecte les vidéos des pages, affiche un bouton « Télécharger cette vidéo » sur le lecteur et un compteur dans la barre d'outils, capture les téléchargements et ajoute un menu clic droit « Télécharger avec Tachy Download ».

## Installer

Téléchargez `Tachy-Download-Setup-<version>.exe` et lancez-le (installation pour l'utilisateur courant, sans droits administrateur). En fin d'installation, yt-dlp et ffmpeg sont téléchargés automatiquement ; hors connexion, ils restent installables depuis Paramètres → Vidéos.

L'installateur n'est pas signé : Windows SmartScreen affichera « Windows a protégé votre ordinateur » au premier lancement (« Informations complémentaires » puis « Exécuter quand même »). Un certificat de signature de code supprime cet avertissement.

## Lancer depuis les sources

```bash
npm install
npm start
npm test
```

Construire l'installateur (voir [docs/BUILD.fr.md](docs/BUILD.fr.md)) : `npm run dist` produit `dist/Tachy-Download-Setup-<version>.exe`. Pour changer l'icône, modifier `tools/make-icon.js` puis `npm run icon`.

Les téléchargements vont par défaut dans le dossier Téléchargements. Les réglages et la liste sont enregistrés dans le dossier de données de l'application.

## Installer l'extension

**Firefox** : ouvrir `about:debugging#/runtime/this-firefox`, « Charger un module complémentaire temporaire », choisir `extension/manifest.json` (temporaire jusqu'au redémarrage ; une installation permanente demande une signature Mozilla). Puis les étapes 2 et 3.

1. **Chrome / Edge** : ouvrir `chrome://extensions`, activer le mode développeur, « Charger l'extension non empaquetée », choisir le dossier `extension/` (copié aussi dans `resources\extension` du dossier d'installation).
2. Dans l'application : Paramètres → Extension navigateur → **Copier** le jeton.
3. Dans les options de l'extension : coller le jeton, vérifier le port, « Enregistrer et tester ».

Si l'application est fermée ou ne répond pas, l'extension laisse le navigateur télécharger normalement.

## Sécurité

L'application écoute uniquement sur `127.0.0.1` (port 6800, puis 6801… si occupé). Trois contrôles protègent l'API : l'en-tête `Host` (contre le DNS rebinding), l'origine (extensions uniquement) et un jeton secret de 192 bits propre à votre installation. Seuls les liens `http` et `https` sont acceptés. L'extension transmet aussi les cookies du site à l'application pour les téléchargements qui exigent une connexion ; ils restent sur votre machine.

## Structure

```
src/engine/   moteur (sans dépendance Electron) : download, hls, ytdlp, manager, tools, probe
src/server.js API locale pour l'extension
src/main.js   processus principal Electron ; preload.js pont IPC ; renderer/ interface
extension/    extension MV3 (Chrome, Edge, Firefox)
test/         tests node:test (serveur HTTP local avec Range)
```

## Limites connues

- Pas de FTP ni de torrent.
- Pas de DRM (Netflix, Disney+, Prime…), pas de directs, pas de DASH (`.mpd`). Un site qui diffuse sa vidéo dans un format non pris en charge n'est pas détecté.
- Les segments ne se redécoupent pas dynamiquement : un segment lent n'est pas partagé avec ceux qui ont fini.
- L'extension est testée dans Chrome ; pour Firefox, le manifeste est validé (web-ext lint) mais le chargement réel n'a pas été testé de bout en bout.
- Windows uniquement pour l'instant.
