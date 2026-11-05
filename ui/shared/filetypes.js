'use strict';

/*
 * The one list of file types. The engine files downloads into `folder`; the window groups them by `kind`
 * and shows `label` on the tile. Loaded with require() in the main process and with a <script> tag in
 * the window, where it defines the global FileTypes.
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.FileTypes = factory();
})(typeof self !== 'undefined' ? self : this, () => {
  const FILE_TYPES = [
    { kind: 'video', label: 'VID', folder: 'Vidéos', exts: ['mp4', 'mkv', 'avi', 'mov', 'webm', 'wmv', 'flv', 'm4v', 'ts'] },
    { kind: 'audio', label: 'MP3', folder: 'Musique', exts: ['mp3', 'wav', 'flac', 'aac', 'ogg', 'm4a', 'opus'] },
    { kind: 'image', label: 'IMG', folder: 'Images', exts: ['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'bmp', 'avif', 'tif', 'tiff'] },
    { kind: 'doc', label: 'DOC', folder: 'Documents', exts: ['pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'txt', 'odt', 'csv', 'epub'] },
    { kind: 'archive', label: 'ZIP', folder: 'Archives', exts: ['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz', 'tgz'] },
    { kind: 'app', label: 'APP', folder: 'Programmes', exts: ['exe', 'msi', 'msix', 'iso', 'dmg', 'apk', 'deb', 'rpm', 'img'] },
  ];
  const OTHER_FOLDER = 'Autres';

  /** Lower-case extension, ignoring a trailing ".part"; '' when there is none. */
  const extOf = (filename) => (/\.([A-Za-z0-9]+)(?:\.part)?$/.exec(filename)?.[1] ?? '').toLowerCase();

  /** The entry of FILE_TYPES for a file name, or null for an unknown type. */
  const typeOf = (filename) => {
    const ext = extOf(filename);
    return FILE_TYPES.find((t) => t.exts.includes(ext)) ?? null;
  };

  return { FILE_TYPES, OTHER_FOLDER, extOf, typeOf };
});
