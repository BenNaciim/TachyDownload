# Générer l'installateur

[English](BUILD.md) · **Français**

Procédure pour Windows.

## Ce qu'il faut

- Windows 10 ou 11 (64 bits) avec les outils de build C++ de Microsoft (« Développement Desktop en C++ » dans l'installeur des Visual Studio Build Tools).
- [Rust](https://rustup.rs). La version du compilateur est épinglée dans `rust-toolchain.toml` : `rustup` la télécharge tout seul à la première commande `cargo`.
- [Node.js](https://nodejs.org) 20 ou plus récent (`node -v` pour vérifier), qui inclut `npm`. Il sert à la ligne de commande Tauri, aux tests pilotés par navigateur et au script de release.
- Git.
- Une connexion Internet pour le premier build : les crates Rust et les outils de création de l'installateur NSIS sont téléchargés, puis gardés en cache. L'application a besoin du runtime WebView2 (déjà présent sous Windows 11) ; l'installateur le télécharge s'il manque.

## Étapes

1. Récupérer le code :
   ```bash
   git clone https://github.com/BenNaciim/TachyDownload.git
   cd TachyDownload
   ```
2. Installer les dépendances Node :
   ```bash
   npm install
   ```
3. Vérifier que tout fonctionne. D'abord les tests Rust (moteur et coque), puis les tests pilotés par navigateur, qui lancent la vraie application et ont besoin de sa version de débogage :
   ```bash
   cargo test -p tachy-core -p tachy-download
   cargo build -p tachy-download
   npm test
   ```
4. Générer l'installateur :
   ```bash
   npm run dist
   ```
5. Récupérer le résultat : `target\release\bundle\nsis\Tachy Download_<version>_x64-setup.exe`. Le script de release le renomme `Tachy-Download-Setup-<version>.exe` (le nom que cherche la mise à jour automatique).

## Ce que fait `npm run dist`

`tauri build --bundles nsis` compile l'application en mode release (`cargo build --release`, avec optimisation à l'édition de liens : le premier build prend plusieurs minutes), puis enveloppe `tachy-download.exe` dans un installateur NSIS. L'interface de `ui/` est intégrée au programme ; l'extension du navigateur est copiée dans `resources\extension` à côté. La configuration est `src-tauri/tauri.conf.json` ; `src-tauri/windows/hooks.nsi` fait télécharger yt-dlp, Deno et ffmpeg en fin d'installation (`--setup-tools`) et fait que la case « supprimer les données » du désinstalleur efface aussi `%APPDATA%\Tachy Download`.

## Vérifier l'installateur

`powershell -File tools\check-installer.ps1 "target\release\bundle\nsis\Tachy Download_<version>_x64-setup.exe"` l'installe en silencieux dans un dossier jetable, vérifie les fichiers, les raccourcis et l'entrée de désinstallation, lance les tests pilotés par navigateur contre le programme installé, réapplique l'installateur par-dessus un programme en cours comme le fait la mise à jour automatique (`/S /UPDATE /R`), puis désinstalle et vérifie que le dossier de données n'a pas bougé. Il ajoute puis retire les raccourcis de l'utilisateur courant, comme une vraie installation.

## Changer de version

Le script de release s'en charge : `npm run release -- patch|minor|major|x.y.z` met à jour `package.json`, `package-lock.json`, `extension/manifest.json`, `Cargo.toml` et `src-tauri/tauri.conf.json`. À la main, changer ces cinq endroits.

## Changer l'icône

Modifier `tools/make-icon.js`, lancer `npm run icon` (écrit `build/icon.png` et `build/icon.ico`), puis refaire `npm run dist`.

## En cas de problème

- **`link.exe` introuvable** : les outils de build C++ manquent (voir « Ce qu'il faut »).
- **Un build qui semble bloqué** : le build release est lent (optimisation à l'édition de liens) et `rustc` peut paraître inactif un moment. Si rien ne bouge depuis dix minutes, l'arrêter (tuer l'arbre de processus `cargo`) et relancer.
- **« Accès refusé » à l'édition de liens** : Tachy Download ou l'une de ses fenêtres de test tourne encore. La quitter (et fermer les `msedgewebview2.exe` restés) puis recommencer.
- **Téléchargement qui échoue au premier build** : relancer `npm run dist` ; une coupure réseau ou un pare-feu en est souvent la cause.
- **Antivirus qui bloque ou supprime l'installateur** : fréquent avec un programme non signé. Ajouter une exception pour le dossier du projet.
- **Avertissement SmartScreen au lancement** : normal tant que l'installateur n'est pas signé. « Informations complémentaires », puis « Exécuter quand même ».

## Publier

`npm run release -- patch` (ou le workflow « Release » de l'onglet Actions de GitHub) change la version, lance les tests, génère l'installateur, commite, étiquette, pousse et crée la release GitHub avec `Tachy-Download-Setup-<version>.exe` en pièce jointe. La mise à jour automatique de l'application demande une release publique et un fichier qui porte l'empreinte SHA-256 de GitHub (c'est le cas par défaut).
