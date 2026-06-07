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
