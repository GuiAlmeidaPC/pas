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

const LINUX_FORMAT_ORDER = [
  { format: 'AppImage', ext: '.appimage', hint: 'Portable — runs anywhere, no install' },
  { format: 'deb', ext: '.deb', hint: 'Debian / Ubuntu and derivatives' },
  { format: 'rpm', ext: '.rpm', hint: 'Fedora / RHEL / openSUSE' },
];

/**
 * From the Linux asset list, produce an ordered [{ format, hint, url }] of the
 * formats actually present in the release.
 */
export function linuxFormats(linuxAssets) {
  const out = [];
  for (const { format, ext, hint } of LINUX_FORMAT_ORDER) {
    const match = (linuxAssets || []).find(
      (a) => String(a?.name || '').toLowerCase().endsWith(ext));
    if (match) out.push({ format, hint, url: match.browser_download_url });
  }
  return out;
}

/** Return the browser_download_url of the SHA256SUMS asset, or null. */
export function findChecksums(assets) {
  const match = (assets || []).find(
    (a) => String(a?.name || '').toLowerCase().includes('sha256sums'));
  return match ? match.browser_download_url : null;
}
