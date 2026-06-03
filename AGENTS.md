# AGENTS.md

Guidance for AI coding agents (and humans) working in this repository. Keep it
short, accurate, and runnable. If a command here drifts from reality, fix it.

## What this project is

PAS (Practical Analytics Studio) is a cross-platform **Tauri desktop app** that
clones the data-wrangling subset of SAS (DATA step + PROC SQL). Rust backend,
React/TypeScript frontend, DuckDB + Apache Arrow under the hood.

## Repository layout

| Path | What lives here |
|------|-----------------|
| `crates/pas-engine/` | Core Rust engine: lexer, parser, DATA step executor, SAS→SQL rewriting, PROCs (`print`, `sort`, `transpose`). |
| `crates/pas-app/` | Tauri desktop shell: windowing, native menus, IPC, filesystem. Tauri config in `tauri.conf.json`. |
| `ui/` | React 18 + Vite + TypeScript frontend. Source in `ui/src/`, tests in `ui/src/__tests__/`. |
| `SPEC.md` | The supported SAS-compatible language subset. |
| `DIVERGENCE.md` | Intentional differences from standard SAS. |
| `docs/` | Design specs and implementation plans. |

## Build, test, lint — run these before claiming work is done

```bash
# Rust (from repo root)
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo test -p pas-engine

# Frontend (from ui/)
pnpm install --frozen-lockfile
pnpm run build       # tsc type-check + vite build
pnpm test            # vitest run
pnpm run test:smoke  # node --test security smoke tests
```

CI additionally runs dependency audits (`security` job): `cargo audit` (honors
`.cargo/audit.toml` ignores) and `pnpm audit --audit-level=high` in `ui/`.

Run the app locally: `cd crates/pas-app && cargo tauri dev`.

## Conventions

- **Commits:** Conventional Commits (`feat(ui): ...`, `fix(...): ...`,
  `docs: ...`). Match the surrounding style.
- **Scope guard:** statistical PROCs, macros, and `.sas7bdat` are out of scope.
- **Tests required** for new behavior. Engine tests live in `crates/pas-engine`;
  UI tests in `ui/src/__tests__`.
- **Changelog:** update `CHANGELOG.md` under `[Unreleased]` for user-visible
  changes.
- **Secrets:** never persist API keys to browser storage — there's a smoke test
  enforcing this (`pnpm run test:smoke`).

## Releasing

Releases are tag-driven. To cut a release:

1. Move `[Unreleased]` content in `CHANGELOG.md` under a `## [X.Y.Z] - DATE`
   heading.
2. Bump `version` in `Cargo.toml` (workspace) and `crates/pas-app/tauri.conf.json`.
3. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`.
4. `.github/workflows/release.yml` builds Linux/Windows/macOS bundles, attaches
   `SHA256SUMS.txt`, and creates a **draft** GitHub Release for review.
