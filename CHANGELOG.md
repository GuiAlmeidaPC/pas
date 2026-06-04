# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Renamed the AI assistant to **"Agent"** throughout the UI (panel title, View
  menu, setup dialog, and the editor right-click actions).

### Added
- DATA step **informat / column input**: modified-list input (`var :informat.`),
  fixed-width formatted input (`var informat.`), and **column-range input**
  (`var [$] start-end`), all driven by a column pointer. Informats: `$charW.`,
  `$w.`, `w.d`, `dateW.` (→ SAS date serial), and `commaW.d` / `dollarW.d`. The
  `format` / `informat` / `label` statements are accepted (not yet applied).
  See `DIVERGENCE.md` §1.6.
- Agent panel: a **model selector** in the header for switching models on the
  fly without opening Setup. The list is fetched live from the provider's
  model endpoint (cached locally) and falls back to a curated list offline;
  the ChatGPT/Codex list stays curated.
- AI assistant: **"Sign in with ChatGPT"** (OAuth) as an alternative to an
  OpenAI API key. Uses the PKCE OAuth flow (loopback callback on port 1455) and
  calls the Codex Responses API with the resulting subscription token, so usage
  bills against the ChatGPT plan. Tokens are stored AES-GCM-encrypted in the app
  data directory and survive restarts; sign-in lives behind an auth-mode toggle
  on the OpenAI provider in AI Setup.

### Documentation
- Corrected `SPEC.md`, `DIVERGENCE.md`, `README.md`, `AGENTS.md`, and
  `CONTRIBUTING.md` to document the macro language as implemented (`%macro`,
  `%if`, `%do` loops, macro functions, `&`/`%` resolution, automatic vars, and
  `call symput`/`symputx`) rather than the obsolete "preprocessor-only / out of
  scope" description. Remaining gaps (e.g. `%sysfunc`) are noted as divergences.

## [0.1.0] - 2026-06-02

First tagged release.

### Added
- Initial PAS (Practical Analytics Studio) implementation: a cross-platform
  Tauri desktop app cloning the data-wrangling subset of SAS.
- `pas-engine` Rust crate: tokenizer, parser, and interpreter for a
  SAS-compatible language focused on the DATA step and PROC SQL, backed by
  DuckDB and Apache Arrow. Includes `PROC PRINT`, `PROC SORT`, and
  `PROC TRANSPOSE`.
- `pas-app` Tauri shell: windowing, native menus, IPC, and filesystem access.
- React/TypeScript frontend with a Monaco editor (SAS syntax highlighting),
  log pane, paginated dataset viewer (TanStack Virtual), and library/project
  browsers.
- AI chat assistant that proposes file edits via `pas-edit` blocks, with a
  Monaco diff review modal, per-file Accept/Reject/Review cards, tab-aware
  reads, and apply-time revalidation that blocks stale edits.
- Project documentation: `SPEC.md`, `DIVERGENCE.md`, `LICENSE`,
  `CONTRIBUTING.md`, and `AGENTS.md`.
- CI security scanning (`cargo audit`, `pnpm audit`).
- Tag-based release workflow producing Linux (`.AppImage`, `.deb`), Windows
  (`.msi`, `.exe`), and unsigned macOS (`.dmg`) bundles with SHA-256 checksums.

[Unreleased]: https://github.com/GuiAlmeidaPC/pas/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/GuiAlmeidaPC/pas/releases/tag/v0.1.0
