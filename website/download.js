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

/**
 * Choose which asset a one-click button should download for an OS.
 * Windows prefers the .exe (NSIS) installer over the .msi; other OSes
 * just take the first asset. Returns null when the list is empty.
 */
export function primaryAsset(os, assets) {
  const list = assets || [];
  if (os === 'windows') {
    const exe = list.find((a) => String(a?.name || '').toLowerCase().endsWith('.exe'));
    if (exe) return exe;
  }
  return list[0] || null;
}

export const REPO = 'GuiAlmeidaPC/pas';
const LATEST_API = `https://api.github.com/repos/${REPO}/releases/latest`;
const RELEASES_PAGE = `https://github.com/${REPO}/releases/latest`;

const OS_META = {
  windows: { label: 'Windows', icon: '<img class="os-logo" src="assets/os/windows.svg" alt="" />', ext: '.exe installer' },
  macos: { label: 'macOS', icon: '<img class="os-logo" src="assets/os/apple.svg" alt="" />', ext: '.dmg (universal)' },
  linux: { label: 'Linux', icon: '<img class="os-logo" src="assets/os/linux.svg" alt="" />', ext: 'AppImage · .deb · .rpm' },
};

async function fetchLatestRelease() {
  const res = await fetch(LATEST_API, { headers: { Accept: 'application/vnd.github+json' } });
  if (!res.ok) throw new Error(`GitHub API ${res.status}`);
  return res.json();
}

// Render a fallback where every button points at the releases page.
function renderFallback() {
  const primary = document.querySelector('[data-dl-primary]');
  const secondary = document.querySelector('[data-dl-secondary]');
  const version = document.querySelector('[data-dl-version]');
  if (primary) {
    primary.href = RELEASES_PAGE;
    primary.querySelector('[data-dl-label]').textContent = 'Download PAS';
    primary.querySelector('[data-dl-ext]').textContent = 'choose your platform on GitHub';
  }
  if (secondary) secondary.innerHTML = '';
  if (version) version.textContent = 'latest release · verify with SHA256SUMS.txt';
}

async function init() {
  let primary = document.querySelector('[data-dl-primary]');
  const secondary = document.querySelector('[data-dl-secondary]');
  const versionEl = document.querySelector('[data-dl-version]');
  const modal = document.querySelector('[data-linux-modal]');
  const modalList = document.querySelector('[data-linux-list]');
  if (!primary || !secondary) return;

  const detected = detectOS(navigator.userAgent, navigator.platform);

  let release;
  try {
    release = await fetchLatestRelease();
  } catch {
    renderFallback();
    return;
  }

  const groups = classifyAssets(release.assets);
  const linux = linuxFormats(groups.linux);
  const checksums = findChecksums(release.assets);
  const version = release.tag_name || 'latest';

  // Primary = detected OS; secondaries = the other two.
  const order = [detected, ...['windows', 'macos', 'linux'].filter((o) => o !== detected)];
  secondary.innerHTML = '';

  order.forEach((os, idx) => {
    const meta = OS_META[os];
    const isPrimary = idx === 0;
    const firstAsset = primaryAsset(os, groups[os]);
    const isLinux = os === 'linux';

    const el = document.createElement('a');
    el.className = isPrimary ? 'dlbtn primary' : 'dlbtn';
    el.innerHTML =
      `<span class="dl-os">${meta.icon}</span>` +
      `<span class="dl-text"><span class="dl-label" data-dl-label>` +
      `${isPrimary ? 'Download for ' + meta.label : meta.label}</span>` +
      `<span class="dl-ext" data-dl-ext>${meta.ext}</span></span>`;

    if (isLinux && linux.length) {
      el.href = '#';
      el.addEventListener('click', (e) => { e.preventDefault(); openLinuxModal(modal, modalList, linux); });
    } else {
      el.href = firstAsset ? firstAsset.browser_download_url : RELEASES_PAGE;
    }

    if (isPrimary) { primary.replaceWith(el); primary = el; }
    else secondary.appendChild(el);
  });

  if (versionEl) {
    // Build via textContent/createElement so a crafted tag_name can't inject markup.
    versionEl.textContent = `${version} · latest release`;
    if (checksums) {
      versionEl.append(' · ');
      const link = document.createElement('a');
      link.href = checksums;
      link.textContent = 'verify with SHA256SUMS.txt';
      versionEl.append(link);
    }
  }

  // Modal close wiring.
  if (modal) {
    modal.querySelectorAll('[data-modal-close]').forEach((b) =>
      b.addEventListener('click', () => modal.classList.remove('open')));
    modal.addEventListener('click', (e) => { if (e.target === modal) modal.classList.remove('open'); });
  }
}

function openLinuxModal(modal, list, formats) {
  if (!modal || !list) return;
  list.innerHTML = formats
    .map((f) => `<a class="linux-row" href="${f.url}"><span class="linux-fmt">${f.format}</span>` +
      `<span class="linux-hint">${f.hint}</span></a>`)
    .join('');
  modal.classList.add('open');
}

// Only run in a browser; importing under Node (tests) must not touch the DOM.
if (typeof document !== 'undefined') {
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
  else init();
}
