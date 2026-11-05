# Liste de parité : la 2.0.0 (Rust + Tauri) doit faire tout ce que fait la 1.2.0

Source : lignes de « Fonctions » du `README.fr.md`, menus (`src/menu.js`), zone de notification, notifications et dialogues (`src/main.js`), API locale (`src/server.js`), conception §5. Case cochée = couverte par un test qui passe (Rust : `cargo test --workspace`, 203 tests + 3 contre le vrai réseau ; Node : `npm test`, 40 tests dont la vraie application pilotée) ou vérifiée sur l’installeur final (`tools/check-installer.ps1`). Les cases restées ouvertes demandent un geste humain ou une seconde release : elles sont listées avec ce qui reste à faire. État au 2026-11-05. `auto` = test qui échoue si la fonction casse ; `manuel` = geste décrit, refait sur la version installée.

## 1. Fonctions du README

- [x] **Segments parallèles** : 1 à 32 segments, jamais moins de 1 Mo, écrits à leur position dans le `.part` · auto (`engine`)
- [x] **Redécoupage dynamique** : une connexion libre prend la moitié du segment le plus lent · auto (`steal`)
- [ ] **Panneau de détails** : découpage et vitesse par connexion · manuel (interface inchangée) · **à faire à la main : regarder le panneau sur un vrai téléchargement**
- [x] **Reprise** : offsets enregistrés, pause, coupure réseau, fermeture ; `If-Range` : fichier changé → repart de zéro · auto (`engine`, `network`)
- [x] **Serveur sans `Range`** : flux unique, pas de reprise · auto (`engine`)
- [x] **File d'attente** : téléchargements simultanés réglables · auto (`engine`, `tasklimit`)
- [x] **Limite de vitesse** globale et par téléchargement, partagée entre segments · auto (`engine`)
- [x] **Planification** : départ à une date, plage horaire quotidienne qui passe minuit · auto (`engine`, `features`)
- [x] **Nouvelles tentatives** : 15, 30, 60, 120, 300, 600 s ; connexions adaptatives · auto (`retry`, `network`)
- [x] **Fenêtre de confirmation** : nom, taille, dossier modifiable, « Télécharger maintenant » / « Plus tard » / « Annuler », désactivable · e2e UI + manuel
- [x] **Vidéos YouTube, Vimeo, Dailymotion, Twitch** via yt-dlp, ffmpeg, Deno ; mise à jour de yt-dlp au plus une fois par jour ; qualité maximale ; H.264 + AAC ; qualité, langue audio, audio seul (MP3) dans la fenêtre de confirmation · auto (`ytdlp-run`, `ytopts`, `ytprogress`, `tools`) + manuel (vraie vidéo)
- [x] **Outils téléchargés** à la fin de l'installation (`--setup-tools`), SHA-256 vérifié · auto (`tools`) + manuel (installeur)
- [x] **Playlists YouTube** : 200 vidéos au plus, dossier à leur nom · auto (`ytdlp-run`)
- [x] **Flux HLS** (clair ou AES-128), conversion `.mp4` sans recompression, segment lent sans blocage (jusqu'à 128 Mo en avance), écriture dans l'ordre · auto (`hls`) + manuel (vraie vidéo)
- [x] **Liens expirés** : `.part` gardé, l'extension rouvre la page et la reprise continue ; téléchargement direct : bouton « Ouvrir la page » · auto (`refresh`, `replay`) + manuel
- [x] **Classement par type** : Vidéos, Musique, Images, Documents, Archives, Programmes ; colonne de gauche avec compteurs · auto (`engine`) + e2e UI
- [x] **Re-télécharger** : remplace le fichier, sans « (1) » ; fichiers, HLS, yt-dlp ; ancien fichier ouvert ailleurs : message, rien touché ; lien mort : parcours « Liens expirés » · auto (`redownload`)
- [ ] **Détails en accordéon**, y compris la carte des segments vidéo · manuel (interface inchangée) + e2e UI · **à faire à la main : regarder l’accordéon, dont la carte des segments vidéo**
- [ ] **Mises à jour automatiques** : release GitHub, SHA-256, installation silencieuse, relance ; désactivable · auto (`updater`) + manuel (installeur) · **à faire à la main : chaîne réelle : il faut deux releases publiées (le moteur et l’installation par-dessus sont testés)**
- [ ] **Zone de notification** : croix = réduire, menu clic droit, vitesse en infobulle · manuel · **à faire à la main : clic droit sur l’icône réelle**
- [ ] **Lancement avec Windows** (`--hidden`) et **action de fin** (quitter, veille, arrêt avec 60 s pour annuler) · auto (`afterall`) + manuel · **à faire à la main : arrêt, veille et extinction réels ; entrée de démarrage Windows**
- [ ] **Verre** (acrylique, Windows 11 22H2+) désactivable ; thèmes clair et sombre · manuel (captures comparées à Electron) · **à faire à la main : comparer à l’œil avec la 1.2.0 sur la machine du matin**
- [ ] **Extension navigateur** : détection, bouton, compteur, capture, menu clic droit · auto (`extension`, `sniff`, restent en JS) + manuel · **à faire à la main : essai avec l’extension dans Chrome, Edge ou Firefox**
- [x] **Flux comme le navigateur les a demandés** : `Referer`, `Origin`, cookies rejoués ; playlists au nom étrange ; DASH via yt-dlp · auto (`replay`, `sniff`)
- [x] **Mode diagnostic** : « Copier le diagnostic » · auto (`ytdlp-run`) + manuel

## 2. Menu de la fenêtre (modèle pur testé, branchement testé)

- [x] **Fichier** : Ajouter un lien… (Ctrl+N) · Ouvrir le dossier de téléchargement (Ctrl+Maj+O) · Paramètres (Ctrl+,) · Quitter (Ctrl+Q)
- [x] **Téléchargements** : Tout mettre en pause (Ctrl+Maj+P) · Tout reprendre (Ctrl+Maj+R) · Retirer les téléchargements terminés de la liste
- [x] **Limite de vitesse** : Aucune limite, 256 Ko/s, 512 Ko/s, 1 Mo/s, 2 Mo/s, 5 Mo/s (cases radio cochées selon le réglage) · Autre valeur… (ouvre les paramètres)
- [x] **Quand tout est terminé** : Ne rien faire · Quitter l’application · Mettre l’ordinateur en veille · Éteindre l’ordinateur (cases radio)
- [x] **Affichage** : Rechercher (Ctrl+F) · Thème (Automatique, Clair, Sombre) · Zoom avant, Zoom arrière, Taille normale · Plein écran · Outils de développement (développement seulement)
- [x] **Aide** : Rechercher des mises à jour… · Copier le diagnostic yt-dlp · Page du projet · À propos de Tachy Download
- [x] Le menu se reconstruit quand les réglages changent (limite, action de fin)

## 3. Zone de notification, barre des tâches, notifications

- [ ] Entrées : Ouvrir Tachy Download · Ouvrir le dossier de téléchargement · Tout mettre en pause · Tout reprendre · Quitter ; clic gauche = ouvrir · **à faire à la main : clic droit réel sur l’icône**
- [x] Infobulle : « Tachy Download : N en cours (P %), V Mo/s » ou « Tachy Download » · auto (`shell`)
- [x] Progression globale sur le bouton de la barre des tâches (normale, indéterminée, aucune) · auto (`shell`) + manuel
- [ ] Notification « Téléchargement terminé » / « Téléchargement échoué » seulement si la fenêtre n'est pas visible et active et si les notifications sont activées ; clic : dossier du fichier ou fenêtre (S5 : clic éventuellement perdu, §10.3) · **à faire à la main : toast réel et son clic (S5, §10.3)**
- [x] La croix réduit dans la zone de notification si `closeToTray`, sinon quitte

## 4. Dialogues

- [x] À propos : « Tachy Download <version> / Gestionnaire de téléchargement pour Windows. / <adresse du projet> »
- [x] Mises à jour : « La version X est disponible : utilisez le bandeau en haut de la fenêtre pour l’installer. » / « Tachy Download est à jour (version X). » / « Impossible de vérifier les mises à jour : … »
- [x] Diagnostic copié : « Le diagnostic est copié : collez-le dans un message. »
- [ ] Arrêt de l'ordinateur : « Les téléchargements sont terminés : arrêt de l’ordinateur dans 60 secondes. » avec « Annuler l’arrêt » / « OK » · **à faire à la main : boîte réelle avec ses 60 s**
- [x] Erreur au démarrage : « Le serveur local n’a pas pu démarrer : … » ; l'appli continue sans API
- [ ] Choix d'un dossier (paramètres et fenêtre de confirmation) · **à faire à la main : sélecteur de dossier système**

## 5. API locale (extension)

- [x] Ordre des contrôles : `Host`, `Origin`, `OPTIONS`, `/ping`, jeton, routes, 404 · auto (`server`)
- [x] `GET /ping` · `GET /refresh-requests` · `POST /add` (fichier, `hls`, `ytdlp`, `{"pending": true}`) · `POST /refresh`
- [x] En-têtes CORS et `Cache-Control: no-store` · corps > 64 Kio : 413 et coupure · port 6800 à 6809 · écoute locale seulement
- [x] Jeton `extension-token.txt` conservé s'il existe

## 6. Démarrage, arrêt, données

- [x] Instance unique : relancer montre la fenêtre existante · manuel + e2e
- [x] `--hidden`, `--setup-tools`, `--data-dir` · e2e
- [x] Un téléchargement `downloading` devient `paused` au chargement · auto
- [x] `downloads.json` : forme identique, champs inconnus gardés, écriture atomique, fichier illisible → copie `.illisible` · auto (contrats)
- [ ] Identifiants par site (`logins.json`) : lecture de ceux de la 1.x (DPAPI + `v10`), écriture DPAPI · auto (migration) + manuel (vrai dossier) · **à faire à la main : ouvrir le vrai dossier de données : migration des identifiants de la 1.2.0**
- [x] Reprise des données de l'ancien nom « Download Manager » · auto
- [x] Préférences d'interface (thème, couleur, verre, sections repliées) : remises à zéro une fois (§10.3)

## 7. Installation

- [x] Installeur : raccourcis bureau et menu Démarrer, entrée de désinstallation « Tachy Download », dossier modifiable
- [x] Extension dans `resources\extension` du dossier d'installation
- [x] Mise à jour par-dessus (`/S /UPDATE /R`) : l'appli est relancée
- [ ] Désinstallation : données gardées, ou supprimées si la case est cochée · **à faire à la main : case « supprimer les données » en mode graphique (données gardées : vérifié)**
- [ ] WebView2 absent : amorce téléchargée par l'installeur · **à faire à la main : impossible à tester ici, WebView2 est installé**
