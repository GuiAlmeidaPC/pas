# PAS Marketing Website Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a single-page, dependency-free static marketing/download site for PAS, themed like the app, that pulls the latest GitHub release for one-click OS-aware downloads, and auto-deploys to GitHub Pages.

**Architecture:** Pure static `website/` folder (`index.html`, `styles.css`, `download.js`) — no build step, no runtime dependencies. `download.js` is an ES module exporting pure helper functions (OS detection, release-asset classification, Linux-format listing, checksum lookup) plus a browser-only `init()` that wires them to the DOM. The pure functions are unit-tested with Node's built-in test runner (`node --test`), matching the project's existing smoke-test style. A GitHub Actions workflow uploads `website/` to GitHub Pages.

**Tech Stack:** HTML5, CSS (custom properties mirroring `ui/src/styles.css`), vanilla JS ES modules, `node --test` (built-in, no deps), GitHub Actions (`actions/upload-pages-artifact`, `actions/deploy-pages`).

**Reference:** Spec at `docs/superpowers/specs/2026-06-07-marketing-website-design.md`.

**Repo facts:** GitHub repo is `GuiAlmeidaPC/pas`. App icons live in `crates/pas-app/icons/` (`icon.png`, `32x32.png`, etc.). Example scripts/data in `example_project/`. App version is currently `0.2.0`.

---

## File Structure

```
website/
  index.html                       # the whole page (header, hero, features, how-it-works, footer)
  styles.css                       # PAS-themed, responsive styles
  download.js                      # ES module: pure helpers + browser init()
  download.test.mjs                # node --test unit tests for the pure helpers
  assets/
    favicon.png                    # copied from crates/pas-app/icons/32x32.png
    logo.png                       # copied from crates/pas-app/icons/icon.png (or 128x128.png)
    screenshots/
      hero.png                     # placeholder until real capture (editor + results)
      ide-full.png                 # placeholder until real capture (full IDE)
      README.md                    # notes on how/what to capture
.github/workflows/
  deploy-website.yml               # upload website/ to GitHub Pages on push to main
```

Each file has one responsibility: `index.html` = structure, `styles.css` = presentation, `download.js` = behavior/data. Keeping the download logic in pure exported functions lets us test it without a browser.

---

## Task 1: Scaffold `website/` and the download-logic test harness

**Files:**
- Create: `website/download.js`
- Test: `website/download.test.mjs`

We start with the testable core: `detectOS`. The module is an ES module so both Node and the browser can import the pure functions. Browser-only DOM wiring comes in a later task and is guarded so importing the module under Node does not touch `document`.

- [ ] **Step 1: Write the failing test**

Create `website/download.test.mjs`:

```js
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { detectOS } from './download.js';

test('detectOS identifies Windows', () => {
  assert.equal(detectOS('Mozilla/5.0 (Windows NT 10.0; Win64; x64)', 'Win32'), 'windows');
});

test('detectOS identifies macOS', () => {
  assert.equal(detectOS('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', 'MacIntel'), 'macos');
});

test('detectOS identifies Linux', () => {
  assert.equal(detectOS('Mozilla/5.0 (X11; Linux x86_64)', 'Linux x86_64'), 'linux');
});

test('detectOS treats Android as linux', () => {
  assert.equal(detectOS('Mozilla/5.0 (Linux; Android 13)', 'Linux armv8l'), 'linux');
});

test('detectOS falls back to linux when unknown', () => {
  assert.equal(detectOS('something weird', ''), 'linux');
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test website/download.test.mjs`
Expected: FAIL — cannot find/parse `./download.js` or `detectOS is not a function` (the file does not exist yet).

- [ ] **Step 3: Write the minimal implementation**

Create `website/download.js`:

```js
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
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `node --test website/download.test.mjs`
Expected: PASS — 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add website/download.js website/download.test.mjs
git commit -m "feat(website): add OS detection helper with tests"
```

---

## Task 2: Classify release assets by OS

**Files:**
- Modify: `website/download.js`
- Test: `website/download.test.mjs`

GitHub's release API returns `assets[]` where each asset has `name` and `browser_download_url`. We group them by OS using the file extension.

- [ ] **Step 1: Write the failing test**

Append to `website/download.test.mjs`:

```js
import { classifyAssets } from './download.js';

const SAMPLE_ASSETS = [
  { name: 'PAS_0.2.0_amd64.AppImage', browser_download_url: 'https://x/app.AppImage' },
  { name: 'PAS_0.2.0_amd64.deb', browser_download_url: 'https://x/app.deb' },
  { name: 'PAS-0.2.0-1.x86_64.rpm', browser_download_url: 'https://x/app.rpm' },
  { name: 'PAS_0.2.0_x64_en-US.msi', browser_download_url: 'https://x/app.msi' },
  { name: 'PAS_0.2.0_x64-setup.exe', browser_download_url: 'https://x/app.exe' },
  { name: 'PAS_0.2.0_universal.dmg', browser_download_url: 'https://x/app.dmg' },
  { name: 'SHA256SUMS.txt', browser_download_url: 'https://x/SHA256SUMS.txt' },
];

test('classifyAssets groups assets by OS via extension', () => {
  const g = classifyAssets(SAMPLE_ASSETS);
  assert.deepEqual(g.windows.map(a => a.name).sort(),
    ['PAS_0.2.0_x64-setup.exe', 'PAS_0.2.0_x64_en-US.msi']);
  assert.deepEqual(g.macos.map(a => a.name), ['PAS_0.2.0_universal.dmg']);
  assert.deepEqual(g.linux.map(a => a.name).sort(),
    ['PAS-0.2.0-1.x86_64.rpm', 'PAS_0.2.0_amd64.AppImage', 'PAS_0.2.0_amd64.deb']);
});

test('classifyAssets ignores non-installer assets like checksums', () => {
  const g = classifyAssets(SAMPLE_ASSETS);
  const all = [...g.windows, ...g.macos, ...g.linux].map(a => a.name);
  assert.equal(all.includes('SHA256SUMS.txt'), false);
});

test('classifyAssets is case-insensitive on extensions', () => {
  const g = classifyAssets([{ name: 'PAS.APPIMAGE', browser_download_url: 'u' }]);
  assert.equal(g.linux.length, 1);
});

test('classifyAssets handles empty/missing input', () => {
  assert.deepEqual(classifyAssets([]), { windows: [], macos: [], linux: [] });
  assert.deepEqual(classifyAssets(undefined), { windows: [], macos: [], linux: [] });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test website/download.test.mjs`
Expected: FAIL — `classifyAssets is not a function`.

- [ ] **Step 3: Write the minimal implementation**

Append to `website/download.js`:

```js
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
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `node --test website/download.test.mjs`
Expected: PASS — all Task 1 + Task 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add website/download.js website/download.test.mjs
git commit -m "feat(website): classify release assets by OS"
```

---

## Task 3: Build the Linux-format list and checksum lookup

**Files:**
- Modify: `website/download.js`
- Test: `website/download.test.mjs`

The Linux button opens a modal listing only the formats present in the release, in a fixed friendly order with hints. We also need to find the checksum asset for the verification link.

- [ ] **Step 1: Write the failing test**

Append to `website/download.test.mjs`:

```js
import { linuxFormats, findChecksums } from './download.js';

test('linuxFormats returns present formats in order with hints', () => {
  const linux = [
    { name: 'PAS-0.2.0.x86_64.rpm', browser_download_url: 'u-rpm' },
    { name: 'PAS_0.2.0_amd64.AppImage', browser_download_url: 'u-appimage' },
    { name: 'PAS_0.2.0_amd64.deb', browser_download_url: 'u-deb' },
  ];
  const out = linuxFormats(linux);
  assert.deepEqual(out.map(f => f.format), ['AppImage', 'deb', 'rpm']);
  assert.equal(out[0].url, 'u-appimage');
  assert.equal(out[1].url, 'u-deb');
  assert.equal(out[2].url, 'u-rpm');
  assert.ok(out[0].hint.length > 0);
});

test('linuxFormats omits formats not present in the release', () => {
  const linux = [{ name: 'PAS_0.2.0_amd64.AppImage', browser_download_url: 'u' }];
  const out = linuxFormats(linux);
  assert.deepEqual(out.map(f => f.format), ['AppImage']);
});

test('findChecksums returns the SHA256SUMS url or null', () => {
  assert.equal(
    findChecksums([{ name: 'SHA256SUMS.txt', browser_download_url: 'u-sum' }]),
    'u-sum');
  assert.equal(findChecksums([{ name: 'app.deb', browser_download_url: 'u' }]), null);
  assert.equal(findChecksums([]), null);
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test website/download.test.mjs`
Expected: FAIL — `linuxFormats is not a function`.

- [ ] **Step 3: Write the minimal implementation**

Append to `website/download.js`:

```js
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
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `node --test website/download.test.mjs`
Expected: PASS — all tests pass.

- [ ] **Step 5: Commit**

```bash
git add website/download.js website/download.test.mjs
git commit -m "feat(website): add linux format list and checksum lookup"
```

---

## Task 4: Browser init() — fetch release and render download buttons

**Files:**
- Modify: `website/download.js`

This is the browser glue. It is not unit-tested (DOM/network), but is kept small and delegates all logic to the tested pure functions. It must degrade gracefully if the API call fails.

- [ ] **Step 1: Add the constants, fetch, and render logic**

Append to `website/download.js`:

```js
export const REPO = 'GuiAlmeidaPC/pas';
const LATEST_API = `https://api.github.com/repos/${REPO}/releases/latest`;
const RELEASES_PAGE = `https://github.com/${REPO}/releases/latest`;

const OS_META = {
  windows: { label: 'Windows', icon: '🪟', ext: '.msi installer' },
  macos: { label: 'macOS', icon: '🍎', ext: '.dmg (universal)' },
  linux: { label: 'Linux', icon: '🐧', ext: 'AppImage · .deb · .rpm' },
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

function osButtonHTML(os, asset, isPrimary) {
  const meta = OS_META[os];
  const href = asset ? asset.browser_download_url : RELEASES_PAGE;
  return { meta, href };
}

async function init() {
  const primary = document.querySelector('[data-dl-primary]');
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
    const firstAsset = groups[os][0];
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

    if (isPrimary) primary.replaceWith(el), (primary = el);
    else secondary.appendChild(el);
  });

  if (versionEl) {
    versionEl.innerHTML = checksums
      ? `${version} · latest release · <a href="${checksums}">verify with SHA256SUMS.txt</a>`
      : `${version} · latest release`;
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
```

- [ ] **Step 2: Verify the module still imports cleanly under Node**

Run: `node --test website/download.test.mjs`
Expected: PASS — the `typeof document !== 'undefined'` guard prevents DOM access, so all existing tests still pass.

- [ ] **Step 3: Lint-check by importing in Node directly**

Run: `node -e "import('./website/download.js').then(m => console.log(Object.keys(m).join(',')))"`
Expected: prints `detectOS,classifyAssets,linuxFormats,findChecksums,REPO` (no error).

- [ ] **Step 4: Commit**

```bash
git add website/download.js
git commit -m "feat(website): wire release fetch, OS-aware buttons, and linux modal"
```

---

## Task 5: Write `index.html`

**Files:**
- Create: `website/index.html`

Static structure with `data-*` hooks that `download.js` targets. Screenshots reference placeholder files (created in Task 7). Uses real copy from the README/spec.

- [ ] **Step 1: Create the page**

Create `website/index.html`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>PAS — Practical Analytics Studio</title>
  <meta name="description" content="The data-wrangling power of SAS, open and offline. Run the DATA step and PROC SQL natively on your desktop — no server, no license." />
  <link rel="icon" href="assets/favicon.png" />
  <link rel="stylesheet" href="styles.css" />
</head>
<body>
  <header class="site-header">
    <div class="wordmark"><img src="assets/logo.png" alt="" class="logo" /> <span class="cy">PAS</span>
      <span class="tagline">Practical Analytics Studio</span></div>
    <a class="nav-dl" href="#download">Download</a>
  </header>

  <main>
    <!-- HERO -->
    <section class="hero" id="download">
      <div class="hero-copy">
        <h1>The data-wrangling power of <span class="cy">SAS</span>,<br>open &amp; offline.</h1>
        <p class="lede">DATA step + PROC SQL that run unmodified on your desktop — backed by
          DuckDB &amp; Apache Arrow. No server, no license.</p>
        <div class="downloads">
          <a class="dlbtn primary" data-dl-primary href="https://github.com/GuiAlmeidaPC/pas/releases/latest">
            <span class="dl-os">⬇</span>
            <span class="dl-text"><span class="dl-label" data-dl-label>Download PAS</span>
              <span class="dl-ext" data-dl-ext>detecting your platform…</span></span>
          </a>
          <div class="downloads-secondary" data-dl-secondary></div>
        </div>
        <p class="dl-meta" data-dl-version>latest release · verify with SHA256SUMS.txt</p>
      </div>
      <div class="hero-shot">
        <figure class="window">
          <figcaption class="titlebar"><span class="dot red"></span><span class="dot amber"></span><span class="dot green"></span> PAS — sales.sas</figcaption>
          <img src="assets/screenshots/hero.png" alt="PAS editor running a PROC SQL query with results" />
        </figure>
      </div>
    </section>

    <!-- FEATURES -->
    <section class="features">
      <p class="eyebrow">Why PAS</p>
      <h2>Everything you need to wrangle data</h2>
      <div class="fgrid">
        <article class="fcard"><div class="ic">⚙️</div><h3>DATA step &amp; PROC SQL</h3>
          <p>SAS semantics close enough that common scripts run unmodified.</p></article>
        <article class="fcard"><div class="ic">⚡</div><h3>DuckDB-powered</h3>
          <p>SQL executes on a highly optimized engine with zero-copy Arrow transfer.</p></article>
        <article class="fcard"><div class="ic">📊</div><h3>Million-row viewer</h3>
          <p>Paginated virtual scrolling handles huge tables smoothly.</p></article>
        <article class="fcard"><div class="ic">🔌</div><h3>Offline &amp; native</h3>
          <p>A single redistributable binary. No server, no license, no cloud.</p></article>
        <article class="fcard"><div class="ic">🧩</div><h3>Macro language</h3>
          <p><code>%macro</code>, <code>%if</code>/<code>%do</code>, macro functions and <code>&amp;</code>/<code>%</code> substitution.</p></article>
        <article class="fcard"><div class="ic">🖥️</div><h3>Cross-platform</h3>
          <p>Windows, macOS and Linux from one codebase.</p></article>
      </div>
    </section>

    <!-- HOW IT WORKS -->
    <section class="how">
      <p class="eyebrow">How it works</p>
      <h2>A familiar Enterprise Guide–style IDE</h2>
      <figure class="window wide">
        <figcaption class="titlebar"><span class="dot red"></span><span class="dot amber"></span><span class="dot green"></span> PAS</figcaption>
        <img src="assets/screenshots/ide-full.png" alt="The PAS IDE: library and project browsers, code editor, log pane, and output viewer" />
      </figure>
      <p class="how-caption">Editor with SAS syntax highlighting, a real-time log pane, a paginated
        output viewer, and library/project browsers — all streaming rows through the DATA step
        without holding everything in memory.</p>
    </section>
  </main>

  <footer class="site-footer">
    <span>PAS — MIT licensed</span>
    <nav>
      <a href="https://github.com/GuiAlmeidaPC/pas/releases">Releases</a>
      <a href="https://github.com/GuiAlmeidaPC/pas/blob/main/CHANGELOG.md">Changelog</a>
      <a href="https://github.com/GuiAlmeidaPC/pas/releases/latest">SHA256SUMS</a>
    </nav>
  </footer>

  <!-- LINUX FORMAT MODAL -->
  <div class="modal" data-linux-modal>
    <div class="modal-box">
      <div class="modal-head"><h3>Choose a Linux package</h3>
        <button class="modal-x" data-modal-close aria-label="Close">×</button></div>
      <div class="linux-list" data-linux-list></div>
    </div>
  </div>

  <script type="module" src="download.js"></script>
</body>
</html>
```

- [ ] **Step 2: Sanity-check the HTML opens**

Run: `node -e "const s=require('fs').readFileSync('website/index.html','utf8'); if(!s.includes('data-dl-primary')||!s.includes('data-linux-list')) process.exit(1); console.log('hooks present')"`
Expected: prints `hooks present`.

- [ ] **Step 3: Commit**

```bash
git add website/index.html
git commit -m "feat(website): add landing page markup"
```

---

## Task 6: Write `styles.css`

**Files:**
- Create: `website/styles.css`

PAS palette as custom properties; responsive layout for hero, feature grid, how-it-works, and the modal.

- [ ] **Step 1: Create the stylesheet**

Create `website/styles.css`:

```css
:root {
  --bg: #181818;
  --bg-alt: #161616;
  --panel: #252526;
  --panel-2: #2d2d30;
  --border: #3c3c3c;
  --text: #d4d4d4;
  --muted: #888;
  --soft: #9aa3b2;
  --accent: #22d3ee;
  --primary: #155e75;
  --primary-hover: #0e7490;
  --primary-text: #e0f7fa;
  --mono: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
}

* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--text);
  font-family: -apple-system, system-ui, "Segoe UI", Roboto, sans-serif;
  line-height: 1.5;
}
a { color: inherit; text-decoration: none; }
.cy { color: var(--accent); }
code { font-family: var(--mono); background: var(--panel-2); padding: 1px 5px; border-radius: 3px; font-size: 0.9em; }

/* Header */
.site-header {
  position: sticky; top: 0; z-index: 10;
  display: flex; justify-content: space-between; align-items: center;
  padding: 12px 24px; background: rgba(30,30,30,0.92);
  backdrop-filter: blur(6px); border-bottom: 1px solid var(--border);
}
.wordmark { display: flex; align-items: center; gap: 8px; font-weight: 700; color: #fff; font-size: 16px; }
.wordmark .logo { width: 22px; height: 22px; border-radius: 5px; }
.wordmark .tagline { color: var(--muted); font-weight: 400; font-size: 12px; }
.nav-dl { background: var(--primary); color: var(--primary-text); padding: 7px 16px; border-radius: 6px; font-size: 13px; font-weight: 600; }
.nav-dl:hover { background: var(--primary-hover); }

/* Layout sections */
main { max-width: 1080px; margin: 0 auto; padding: 0 24px; }
.hero, .features, .how { padding: 56px 0; border-bottom: 1px solid #242424; }
.eyebrow { color: var(--accent); letter-spacing: 2px; text-transform: uppercase; font-size: 12px; margin: 0 0 6px; }
h1 { font-size: clamp(28px, 4vw, 40px); line-height: 1.12; margin: 0 0 16px; color: #fff; }
h2 { font-size: clamp(22px, 3vw, 28px); color: #fff; margin: 0 0 28px; }
h3 { color: #fff; }

/* Hero */
.hero { display: grid; grid-template-columns: 1fr 1.1fr; gap: 36px; align-items: center; }
.lede { color: var(--soft); font-size: 16px; max-width: 44ch; margin: 0 0 24px; }
.downloads { display: flex; flex-direction: column; gap: 10px; max-width: 360px; }
.downloads-secondary { display: flex; gap: 10px; }
.dlbtn {
  display: flex; align-items: center; gap: 11px;
  background: var(--panel); border: 1px solid var(--border); border-radius: 8px;
  padding: 11px 15px; transition: border-color .15s, background .15s; flex: 1;
}
.dlbtn:hover { border-color: var(--accent); }
.dlbtn.primary { background: var(--primary); border-color: var(--primary-hover); }
.dlbtn.primary:hover { background: var(--primary-hover); }
.dl-os { font-size: 20px; }
.dl-text { display: flex; flex-direction: column; }
.dl-label { font-weight: 600; color: #fff; font-size: 14px; }
.dl-ext { font-size: 11px; color: var(--muted); }
.dlbtn.primary .dl-ext { color: #bae6fd; }
.dl-meta { font-size: 12px; color: var(--muted); margin-top: 14px; }
.dl-meta a { color: var(--accent); }

/* Window frame for screenshots */
.window { margin: 0; background: var(--panel); border: 1px solid var(--border); border-radius: 10px; overflow: hidden; box-shadow: 0 12px 40px rgba(0,0,0,.4); }
.window.wide { margin-bottom: 18px; }
.titlebar { display: flex; align-items: center; gap: 7px; padding: 8px 12px; background: var(--panel-2); border-bottom: 1px solid var(--border); font-size: 12px; color: var(--muted); }
.dot { width: 11px; height: 11px; border-radius: 50%; display: inline-block; }
.dot.red { background: #f48771; } .dot.amber { background: #f0a020; } .dot.green { background: #4ade80; }
.window img { display: block; width: 100%; height: auto; }

/* Features */
.fgrid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 16px; }
.fcard { background: var(--bg); border: 1px solid var(--border); border-radius: 10px; padding: 18px; }
.fcard .ic { font-size: 22px; }
.fcard h3 { margin: 10px 0 6px; font-size: 15px; }
.fcard p { margin: 0; font-size: 13px; color: var(--soft); }
.features { background: var(--bg); }

/* How it works */
.how { background: var(--bg-alt); }
.how-caption { color: var(--soft); font-size: 14px; max-width: 64ch; }

/* Footer */
.site-footer { max-width: 1080px; margin: 0 auto; padding: 20px 24px; display: flex; justify-content: space-between; align-items: center; font-size: 12px; color: var(--muted); flex-wrap: wrap; gap: 10px; }
.site-footer nav { display: flex; gap: 18px; }
.site-footer a:hover { color: var(--accent); }

/* Linux modal */
.modal { display: none; position: fixed; inset: 0; background: rgba(0,0,0,.6); z-index: 20; align-items: center; justify-content: center; padding: 20px; }
.modal.open { display: flex; }
.modal-box { background: var(--panel); border: 1px solid var(--border); border-radius: 12px; width: 100%; max-width: 420px; overflow: hidden; }
.modal-head { display: flex; justify-content: space-between; align-items: center; padding: 14px 18px; background: var(--panel-2); border-bottom: 1px solid var(--border); }
.modal-head h3 { margin: 0; font-size: 15px; }
.modal-x { background: none; border: none; color: var(--muted); font-size: 22px; cursor: pointer; line-height: 1; }
.modal-x:hover { color: var(--text); }
.linux-list { padding: 10px; display: flex; flex-direction: column; gap: 8px; }
.linux-row { display: flex; flex-direction: column; padding: 12px 14px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg); }
.linux-row:hover { border-color: var(--accent); }
.linux-fmt { font-weight: 600; color: #fff; }
.linux-hint { font-size: 12px; color: var(--muted); }

/* Responsive */
@media (max-width: 760px) {
  .hero { grid-template-columns: 1fr; }
  .fgrid { grid-template-columns: 1fr; }
  .downloads-secondary { flex-direction: column; }
  .site-header .tagline { display: none; }
}
```

- [ ] **Step 2: Sanity-check the CSS covers the markup hooks**

Run: `node -e "const c=require('fs').readFileSync('website/styles.css','utf8'); ['.dlbtn','.modal','.fgrid','.window','.hero'].forEach(k=>{if(!c.includes(k)){console.error('missing',k);process.exit(1)}}); console.log('classes present')"`
Expected: prints `classes present`.

- [ ] **Step 3: Commit**

```bash
git add website/styles.css
git commit -m "feat(website): add PAS-themed responsive styles"
```

---

## Task 7: Assets and placeholder screenshots

**Files:**
- Create: `website/assets/favicon.png`, `website/assets/logo.png`
- Create: `website/assets/screenshots/hero.png`, `website/assets/screenshots/ide-full.png`
- Create: `website/assets/screenshots/README.md`

Copy real icons from the app. Create placeholder screenshots so the page is never broken before real captures exist.

- [ ] **Step 1: Copy app icons**

```bash
mkdir -p website/assets/screenshots
cp crates/pas-app/icons/32x32.png website/assets/favicon.png
cp crates/pas-app/icons/128x128.png website/assets/logo.png
```

- [ ] **Step 2: Generate clearly-marked placeholder screenshots**

Create the placeholders as SVG-rendered PNGs is overkill; use a tiny committed SVG-to-file approach with a plain note image. Simplest dependency-free route — write SVG placeholders and reference them. Update the two `<img>` tags to point at `.svg` placeholders for now is NOT desired (we want real PNGs later at the same path). Instead, create minimal valid PNG placeholders:

Run:
```bash
node -e '
const fs=require("fs");
// 1x1 dark-gray PNG, base64 — valid placeholder so <img> never breaks.
const png=Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNk+M9QDwAEhgGAhqmMUQAAAABJRU5ErkJggg==","base64");
fs.writeFileSync("website/assets/screenshots/hero.png",png);
fs.writeFileSync("website/assets/screenshots/ide-full.png",png);
console.log("placeholders written");
'
```
Expected: prints `placeholders written` and both files exist.

- [ ] **Step 3: Document the capture requirements**

Create `website/assets/screenshots/README.md`:

```markdown
# Screenshots

These are **placeholders** until real captures replace them at the same paths.

Capture from the running PAS app (`example_project/` has scripts + data to load):

- `hero.png` — the code editor with a PROC SQL query and its results visible.
  Roughly 16:10, window ~1200px wide, dark theme.
- `ide-full.png` — the full IDE: library/project browser (left), editor (center),
  log pane (bottom), output/dataset viewer (right).

Keep them PNG, dark theme, and reasonably high-DPI. Replace the placeholder files
in place — `index.html` references these exact filenames.
```

- [ ] **Step 4: Commit**

```bash
git add website/assets
git commit -m "feat(website): add icons and placeholder screenshots"
```

---

## Task 8: Capture real screenshots from the running app

**Files:**
- Replace: `website/assets/screenshots/hero.png`, `website/assets/screenshots/ide-full.png`

This task depends on running the GUI. Use the project's `run` skill / Tauri dev to launch PAS, load `example_project/`, and capture the two views.

- [ ] **Step 1: Launch the app**

Build and run PAS in dev mode (from repo root):
Run: `cd crates/pas-app && cargo tauri dev` (or the project's documented run command; consult `AGENTS.md`/`CONTRIBUTING.md`).
Expected: the PAS window opens.

- [ ] **Step 2: Load example content and capture `hero.png`**

In the app, open `example_project/00_test.sas` (or a PROC SQL script), run it so results show, then capture the editor+results region to `website/assets/screenshots/hero.png`.

- [ ] **Step 3: Capture `ide-full.png`**

Arrange the window so the library/project browser, editor, log, and output panes are all visible; capture the full window to `website/assets/screenshots/ide-full.png`.

- [ ] **Step 4: Verify the page renders with real images**

Run: `python3 -m http.server 8080 --directory website` then open `http://localhost:8080` in a browser and confirm both screenshots display and download buttons populate.
Expected: real screenshots visible; primary button shows the detected OS.

> **If the GUI cannot run in this environment:** stop here, leave the placeholders in place, and hand off to the owner with the instructions in `website/assets/screenshots/README.md`. Do NOT block the rest of the plan on this task.

- [ ] **Step 5: Commit (only if real captures were taken)**

```bash
git add website/assets/screenshots/hero.png website/assets/screenshots/ide-full.png
git commit -m "feat(website): add real app screenshots"
```

---

## Task 9: GitHub Pages auto-deploy workflow

**Files:**
- Create: `.github/workflows/deploy-website.yml`

- [ ] **Step 1: Create the workflow**

Create `.github/workflows/deploy-website.yml`:

```yaml
name: Deploy Website

on:
  push:
    branches: ["main"]
    paths: ["website/**", ".github/workflows/deploy-website.yml"]
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

# Allow one concurrent deployment; newer runs cancel in-progress ones.
concurrency:
  group: "pages"
  cancel-in-progress: true

jobs:
  deploy:
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v5

      - name: Run download-logic tests
        run: node --test website/download.test.mjs

      - name: Configure Pages
        uses: actions/configure-pages@v5

      - name: Upload website artifact
        uses: actions/upload-pages-artifact@v3
        with:
          path: website

      - name: Deploy to GitHub Pages
        id: deployment
        uses: actions/deploy-pages@v4
```

- [ ] **Step 2: Validate the workflow YAML**

Run: `node -e "const f=require('fs').readFileSync('.github/workflows/deploy-website.yml','utf8'); ['upload-pages-artifact','deploy-pages','node --test'].forEach(k=>{if(!f.includes(k)){console.error('missing',k);process.exit(1)}}); console.log('workflow ok')"`
Expected: prints `workflow ok`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/deploy-website.yml
git commit -m "ci(website): auto-deploy website to GitHub Pages"
```

- [ ] **Step 4: One-time owner action (document, do not automate)**

In the GitHub repo: **Settings → Pages → Build and deployment → Source = "GitHub Actions"**. Note this in the PR description. Without it, the deploy job's `deploy-pages` step will fail until enabled.

---

## Task 10: Final verification

- [ ] **Step 1: Run the full test suite for the website module**

Run: `node --test website/download.test.mjs`
Expected: PASS — all OS-detection, classification, linux-format, and checksum tests pass.

- [ ] **Step 2: Serve locally and smoke-test**

Run: `python3 -m http.server 8080 --directory website`
Open `http://localhost:8080` and confirm:
- Header, hero, 6 feature cards, how-it-works, footer all render in the PAS dark theme.
- Download buttons populate from the latest release; the detected OS is the primary button.
- Clicking the Linux button opens the modal listing the available formats.
- Footer has Releases / Changelog / SHA256SUMS links and **no source-repo link**.

- [ ] **Step 3: Confirm no stray dependencies**

Run: `test ! -f website/package.json && echo "no build deps — good"`
Expected: prints `no build deps — good`.

- [ ] **Step 4: Final commit if anything was adjusted**

```bash
git add -A website .github/workflows/deploy-website.yml
git commit -m "chore(website): final polish and verification" || echo "nothing to commit"
```

---

## Self-Review Notes

- **Spec coverage:** tech stack (Task 1–6), file layout (all), slim header (Task 5/6), split hero (5/6), 6-feature grid (5/6), how-it-works (5/6), slim footer **without repo link** (5/6, verified Task 10), OS detection + highlight (1,4), latest-release fetch + extension classification (2,4), Linux modal of present formats (3,4), checksum link (3,4), failure fallback (4), PAS palette + JetBrains Mono (6), responsive (6), real screenshots with placeholder fallback (7,8), auto-deploy workflow (9), custom-domain note via CNAME (left to owner; documented in spec). All covered.
- **Placeholder scan:** screenshot images are intentional placeholders with a documented capture task (8); no TODO/TBD code steps.
- **Type/name consistency:** exported names `detectOS`, `classifyAssets`, `linuxFormats`, `findChecksums`, `REPO` are used identically across tasks and `index.html` `data-*` hooks (`data-dl-primary`, `data-dl-secondary`, `data-dl-version`, `data-linux-modal`, `data-linux-list`, `data-modal-close`) match between Task 4 and Task 5.
```
