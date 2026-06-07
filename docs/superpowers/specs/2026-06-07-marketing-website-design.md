# PAS Marketing Website — Design

**Date:** 2026-06-07
**Status:** Approved (pending spec review)

## Purpose

A single-page marketing and download site for PAS (Practical Analytics Studio).
It introduces the software, showcases the IDE, and gives visitors a one-click
download of the latest release for their operating system. It is hosted free on
GitHub Pages, with a custom domain planned for later.

## Goals

- One scrollable landing page — no docs, blog, or multi-page navigation.
- Visually on-brand: mirrors the PAS app's dark IDE theme and palette.
- Download buttons always point at the **latest** GitHub release with zero
  per-release maintenance.
- No build step, no runtime dependencies — pure static files.

## Non-Goals (YAGNI)

- Documentation, blog, changelog pages, or release-notes feeds.
- A "View on GitHub" call-to-action button in the hero or header.
- Any backend, analytics, or third-party scripts.
- Code signing or installer changes (explicitly out of scope — separate concern).

## Tech Stack

Plain static **HTML + CSS + vanilla JS**. No framework, no bundler, no
`package.json`. This is the least machinery for a single page, loads instantly,
and deploys to GitHub Pages with no build action.

## File Layout

```
website/
  index.html                  # the entire page
  styles.css                  # PAS-themed styles, responsive
  download.js                 # GitHub-release fetch + OS detection + Linux modal
  assets/
    logo.svg                  # PAS wordmark/mark (derived from existing icons)
    favicon.png               # from crates/pas-app/icons/
    screenshots/
      hero.png                # editor + PROC SQL results (hero, right column)
      ide-full.png            # full IDE: library browser, editor, log, output
  CNAME                       # added later when the custom domain is ready
```

Deployment workflow lives at `.github/workflows/deploy-website.yml`.

## Page Structure (top to bottom)

### 1. Slim sticky header
- Left: `▦ PAS` wordmark + muted subtitle "Practical Analytics Studio".
- Right: a teal **Download** button that anchors/scrolls to the hero.
- Sticky on scroll.

### 2. Split hero
- **Left column:** headline ("The data-wrangling power of SAS, open & offline."),
  one-sentence subhead, and the download control (see Download UX below).
  Small line beneath: version + "latest release" + SHA256 verification note.
- **Right column:** real screenshot `hero.png` (editor + results), framed in a
  mock window title bar (traffic-light dots) for product context.
- Responsive: columns stack vertically on narrow viewports.

### 3. Feature highlights
- Section label "Why PAS" + heading.
- A 6-card grid (3 columns desktop → 1 column mobile):
  1. **DATA step & PROC SQL** — SAS semantics close enough that common scripts run unmodified.
  2. **DuckDB-powered** — optimized SQL engine, zero-copy Apache Arrow transfer.
  3. **Million-row viewer** — paginated virtual scrolling for huge tables.
  4. **Offline & native** — single redistributable binary; no server, license, or cloud.
  5. **Macro language** — `%macro`, `%if`/`%do`, macro functions, `&`/`%` substitution.
  6. **Cross-platform** — Windows, macOS, Linux from one codebase.

### 4. How it works
- Section label "How it works" + heading "A familiar Enterprise Guide–style IDE".
- Large screenshot `ide-full.png` showing library/project browsers, editor with
  SAS highlighting, the log pane, and the output viewer, framed in a mock window.
- Short caption describing the panes and the streaming DATA-step execution.

### 5. Slim footer
- Muted single row. **No source-repository link.**
- Links: **Releases**, **Changelog**, **SHA256SUMS** — each pointing into the
  GitHub releases area. MIT license note. Project tagline.

## Download UX

### OS detection
On load, `download.js` inspects `navigator.userAgent`/`navigator.platform` to
classify the visitor as Windows, macOS, or Linux.

- The detected OS becomes the large **primary** (teal) button.
- The other two OSes render as smaller secondary buttons beside it.
- If detection is inconclusive, default the primary to Linux and show all three.

### Latest-release fetch
- `GET https://api.github.com/repos/GuiAlmeidaPC/pas/releases/latest`.
- Parse `assets[]`, classifying each by file extension:
  - `.msi`, `.exe` → Windows
  - `.dmg` → macOS
  - `.AppImage`, `.deb`, `.rpm` → Linux
- Read `tag_name` to display the version (e.g. `v0.2.0`).
- Locate the `SHA256SUMS.txt` asset for the verification link.
- Each download button links directly to the matching asset's
  `browser_download_url`.

### Linux format modal
- Clicking the **Linux** button opens a modal (styled in the PAS theme) listing
  the Linux formats **that actually exist in the release**: AppImage, .deb,
  and/or .rpm. Formats absent from the release are omitted.
- Each modal entry links to that asset's download URL with a one-line hint
  (e.g. "Portable, no install" / "Debian/Ubuntu" / "Fedora/RHEL").
- Windows and macOS buttons download their single asset directly (no modal).

### Failure fallback
If the API call fails (rate limit, offline, etc.), all download buttons fall
back to linking to
`https://github.com/GuiAlmeidaPC/pas/releases/latest`, and the version line
reads "latest release". The page must never present a broken/empty download.

## Styling

- `:root` palette mirroring `ui/src/styles.css`:
  `--bg #1e1e1e`, `--panel #252526`, `--panel-2 #2d2d30`, `--border #3c3c3c`,
  `--text #d4d4d4`, `--muted #888`, `--accent #22d3ee`, primary button
  `#155e75` → hover `#0e7490`.
- Code/monospace uses **JetBrains Mono** (with `ui-monospace` fallback); body
  uses the system UI font stack — matching the app.
- Fully responsive: hero stacks, feature grid collapses to one column, modal and
  buttons remain tappable on mobile.
- Self-contained: no web-font CDN required unless we choose to load JetBrains
  Mono from a font file in `assets/` (preferred over an external CDN to keep the
  page dependency-free and offline-friendly).

## Screenshots

Real screenshots replace HTML mockups.

- Captured by **running the actual PAS app** and loading the existing
  `example_project/` scripts and data so the editor, log, and output panes show
  real content.
- Two images required: `hero.png` (editor + results) and `ide-full.png` (full
  IDE with all panes).
- Stored in `website/assets/screenshots/`.
- **Risk:** this ties the page's final look to launching the GUI app
  successfully in the build environment. If the app cannot run cleanly here, the
  screenshots will be captured by the project owner on their own machine and
  dropped into the assets folder. Until real screenshots exist, the page uses
  clearly-marked placeholder images so it is never broken.

## Deployment

- `.github/workflows/deploy-website.yml`: on push to `main` (when `website/**`
  changes), build nothing — just upload the `website/` directory as a Pages
  artifact and deploy it via `actions/deploy-pages`.
- Requires GitHub Pages to be enabled for the repository with "GitHub Actions"
  as the source (one-time repo setting by the owner).
- Custom domain: when ready, add a `CNAME` file to `website/` and configure DNS;
  no other changes needed.

## Open Questions / Owner Actions

- Enable GitHub Pages (source = GitHub Actions) in repo settings — one-time.
- Confirm the release actually publishes `.rpm` (the bundle targets `"all"`, but
  the README lists only AppImage/.deb). The modal handles either case
  gracefully, so this is informational, not blocking.
- Provide/approve the two real screenshots.
