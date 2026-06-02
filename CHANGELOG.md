# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Nothing has been released yet. Everything below ships in the first tagged
release, `v0.1.0`. When that tag is cut, move this content under a
`## [0.1.0] - YYYY-MM-DD` heading.

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

[Unreleased]: https://github.com/GuiAlmeidaPC/pas/commits/main
