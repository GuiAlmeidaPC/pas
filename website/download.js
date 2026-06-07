// download.js — PAS website download logic.
// Pure helpers are exported for unit testing; init() wires them to the DOM in the browser.

/**
 * Classify the visitor's OS from userAgent + platform strings.
 * Returns 'windows' | 'macos' | 'linux'. Defaults to 'linux' when unknown.
 */
export function detectOS(userAgent = '', platform = '') {
  const ua = String(userAgent).toLowerCase();
  const pf = String(platform).toLowerCase();
  if (ua.includes('windows') || pf.startsWith('win')) return 'windows';
  // Treat iOS/macOS together as 'macos' for download purposes.
  if (ua.includes('mac') || pf.startsWith('mac') || ua.includes('iphone') || ua.includes('ipad')) return 'macos';
  if (ua.includes('linux') || ua.includes('android') || ua.includes('x11')) return 'linux';
  return 'linux';
}

const EXT_TO_OS = {
  '.msi': 'windows',
  '.exe': 'windows',
  '.dmg': 'macos',
  '.appimage': 'linux',
  '.deb': 'linux',
  '.rpm': 'linux',
};

/**
 * Group release assets into { windows, macos, linux } by file extension.
 * Assets with no recognized installer extension (e.g. SHA256SUMS.txt) are dropped.
 */
export function classifyAssets(assets) {
  const groups = { windows: [], macos: [], linux: [] };
  for (const asset of assets || []) {
    const name = String(asset?.name || '').toLowerCase();
    const dot = name.lastIndexOf('.');
    if (dot === -1) continue;
    const os = EXT_TO_OS[name.slice(dot)];
    if (os) groups[os].push(asset);
  }
  return groups;
}
