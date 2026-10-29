# Générer l'installateur

[English](BUILD.md) · **Français**

Procédure pour Windows.

## Ce qu'il faut

- Windows 10 ou 11.
- [Node.js](https://nodejs.org) 20 ou plus récent (`node -v` pour vérifier), qui inclut `npm`.
- Git.
- Une connexion Internet pour le premier build : Electron et les outils de création d'installateur sont téléchargés (quelques centaines de Mo), puis gardés en cache.

## Étapes

1. Récupérer le code :
   ```bash
   git clone https://github.com/BenNaciim/TachyDownload.git
   cd TachyDownload
   ```
2. Installer les dépendances :
   ```bash
   npm install
   ```
3. Vérifier que tout fonctionne (52 tests, une dizaine de secondes) :
   ```bash
   npm test
   ```
4. Générer l'installateur :
   ```bash
   npm run dist
   ```
5. Récupérer le résultat : `dist\Tachy-Download-Setup-<version>.exe` (environ 107 Mo). Le dossier `dist\win-unpacked\` contient l'application non installée, pratique pour un essai rapide.

## Ce que fait `npm run dist`

[electron-builder](https://www.electron.build) copie le moteur Electron, y place le code de `src/` dans une archive `app.asar`, supprime le fichier de licences Chromium (`tools/after-pack.js`), puis compile un installateur NSIS. Le script `build/installer.nsh` y ajoute, en fin d'installation, le téléchargement de yt-dlp, de Deno et de ffmpeg. L'extension du navigateur est copiée dans `resources\extension`. La configuration est dans la section `build` de `package.json`.

## Changer de version

Modifier `"version"` dans `package.json` **et** dans `extension/manifest.json`. Le nom du fichier produit suit automatiquement.

## Changer l'icône

Modifier `tools/make-icon.js`, lancer `npm run icon` (écrit `build/icon.png` et `build/icon.ico`), puis refaire `npm run dist`.

## En cas de problème

- **`EBUSY` ou « resource busy or locked »** : quelque chose utilise le dossier `dist`. Quitter Tachy Download, fermer les terminaux et explorateurs ouverts dans `dist\win-unpacked`, ou supprimer `dist` avant de recommencer.
- **Téléchargement qui échoue au premier build** : relancer `npm run dist` ; une coupure réseau ou un pare-feu en est souvent la cause.
- **Antivirus qui bloque ou supprime l'installateur** : fréquent avec un programme non signé. Ajouter une exception pour le dossier du projet.
- **Avertissement SmartScreen au lancement** : normal tant que l'installateur n'est pas signé. « Informations complémentaires », puis « Exécuter quand même ».

## Publier

Sur GitHub, créer une release (par exemple `v1.0.0`) et y déposer le fichier `.exe`. Le dossier `dist/` n'est pas versionné.
