'use strict';

const LIMITS = [
  [0, 'Aucune limite'],
  [256, '256 Ko/s'],
  [512, '512 Ko/s'],
  [1024, '1 Mo/s'],
  [2048, '2 Mo/s'],
  [5120, '5 Mo/s'],
];

const AFTER = [
  ['none', 'Ne rien faire'],
  ['quit', 'Quitter l’application'],
  ['sleep', 'Mettre l’ordinateur en veille'],
  ['shutdown', 'Éteindre l’ordinateur'],
];

/**
 * The window's menu bar as an Electron menu template. Everything it does goes through `actions`, so the
 * template is plain data that can be checked without a window.
 */
function menuTemplate({ settings, actions, devTools = false }) {
  const limit = settings.speedLimitKBps;
  return [
    {
      label: 'Fichier',
      submenu: [
        { label: 'Ajouter un lien…', accelerator: 'CmdOrCtrl+N', click: () => actions.ui('add') },
        { label: 'Ouvrir le dossier de téléchargement', accelerator: 'CmdOrCtrl+Shift+O', click: actions.openFolder },
        { type: 'separator' },
        { label: 'Paramètres', accelerator: 'CmdOrCtrl+,', click: () => actions.ui('settings') },
        { type: 'separator' },
        { label: 'Quitter', accelerator: 'CmdOrCtrl+Q', click: actions.quit },
      ],
    },
    {
      label: 'Téléchargements',
      submenu: [
        { label: 'Tout mettre en pause', accelerator: 'CmdOrCtrl+Shift+P', click: actions.pauseAll },
        { label: 'Tout reprendre', accelerator: 'CmdOrCtrl+Shift+R', click: actions.resumeAll },
        { label: 'Retirer les téléchargements terminés de la liste', click: actions.clearFinished },
        { type: 'separator' },
        {
          label: 'Limite de vitesse',
          submenu: [
            ...LIMITS.map(([kbps, label]) => ({ label, type: 'radio', checked: limit === kbps, click: () => actions.setLimit(kbps) })),
            { type: 'separator' },
            { label: 'Autre valeur…', click: () => actions.ui('settings') },
          ],
        },
        {
          label: 'Quand tout est terminé',
          submenu: AFTER.map(([value, label]) => ({ label, type: 'radio', checked: settings.afterAll === value, click: () => actions.setAfterAll(value) })),
        },
      ],
    },
    {
      label: 'Affichage',
      submenu: [
        { label: 'Rechercher', accelerator: 'CmdOrCtrl+F', click: () => actions.ui('search') },
        { type: 'separator' },
        {
          label: 'Thème',
          submenu: [
            { label: 'Automatique', click: () => actions.ui('theme', 'auto') },
            { label: 'Clair', click: () => actions.ui('theme', 'light') },
            { label: 'Sombre', click: () => actions.ui('theme', 'dark') },
          ],
        },
        { type: 'separator' },
        { label: 'Zoom avant', role: 'zoomIn' },
        { label: 'Zoom arrière', role: 'zoomOut' },
        { label: 'Taille normale', role: 'resetZoom' },
        { type: 'separator' },
        { label: 'Plein écran', role: 'togglefullscreen' },
        ...(devTools ? [{ type: 'separator' }, { label: 'Outils de développement', role: 'toggleDevTools' }] : []),
      ],
    },
    {
      label: 'Aide',
      submenu: [
        { label: 'Rechercher des mises à jour…', click: actions.checkUpdate },
        { label: 'Copier le diagnostic yt-dlp', click: actions.copyDiagnostic },
        { type: 'separator' },
        { label: 'Page du projet', click: actions.openProject },
        { label: 'À propos de Tachy Download', click: actions.about },
      ],
    },
  ];
}

module.exports = { menuTemplate, LIMITS, AFTER };
