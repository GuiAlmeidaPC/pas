# Contributing to PAS

Thanks for your interest in PAS (Practical Analytics Studio)! This guide covers
how to get a working build, the checks your change must pass, and the
conventions we follow.

## Before you start

- Read [`SPEC.md`](SPEC.md) for the supported PAS language subset.
- Read [`DIVERGENCE.md`](DIVERGENCE.md) for known, intentional differences from
  documented compatibility behavior.
- Statistical procedures and the proprietary binary dataset format are **out of scope** for
  the current version — please discuss in an issue before working on anything
  in that space. (The macro language is in scope and largely implemented; see
  `SPEC.md` §5.5 and `DIVERGENCE.md` §1.1.)

## Prerequisites

- **Rust** (stable; see [`rust-toolchain.toml`](rust-toolchain.toml))
- **Node.js** 20+
- **pnpm** 9+
- Tauri's platform build dependencies. On Debian/Ubuntu:
  ```bash
  sudo apt-get install -y libgtk-3-dev libwebkit2gtk-4.1-dev \
    libayatana-appindicator3-dev librsvg2-dev patchelf
  ```

## Getting started

```bash
# 1. Install frontend dependencies
cd ui && pnpm install

# 2. Run the app with hot-reloading
cd ../crates/pas-app && cargo tauri dev
```

## Checks your change must pass

CI (`.github/workflows/validate.yml`) runs these on every pull request. Run
them locally before pushing:

```bash
# Rust
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo test -p pas-engine

# Frontend (from ui/)
pnpm run build      # type-checks via tsc, then builds
pnpm test           # vitest
pnpm run test:smoke # security smoke tests
```

CI also runs dependency security audits. To reproduce them locally:

```bash
cargo install cargo-audit   # once
cargo audit                 # Rust deps; honors .cargo/audit.toml ignores

cd ui && pnpm audit --audit-level=high   # frontend deps
```

A change is not ready to merge unless all of the above pass.

## Commit and pull-request conventions

- Branch off `main` (e.g. `feat/...`, `fix/...`, `docs/...`).
- Use [Conventional Commits](https://www.conventionalcommits.org/) for commit
  subjects — this repo already follows that style
  (`feat(ui): ...`, `fix(ui): ...`, `docs: ...`, `style(ui): ...`).
- Keep pull requests focused; update [`CHANGELOG.md`](CHANGELOG.md) under
  `[Unreleased]` when your change is user-visible.
- New behavior needs tests. The engine has unit tests in `crates/pas-engine`;
  the UI has vitest suites under `ui/src/__tests__`.

## Releases

Releases are automated and triggered by pushing a `vX.Y.Z` tag. See
[`AGENTS.md`](AGENTS.md) and `.github/workflows/release.yml` for the full flow.

## License

By contributing, you agree that your contributions are licensed under the
[MIT License](LICENSE).
