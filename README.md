# PAS (Practical Analytics Studio)

**PAS** is a cross-platform desktop application that provides a full-featured analytics IDE for authoring and running a PAS language, specifically focusing on the **DATA step** and **PROC SQL**.

This project provides an offline, native experience for executing common data manipulation tasks without requiring an external analytics server.

## Features

- **Familiar IDE**: Includes a code editor, log pane, paginated output viewer, and library/project browsers.
- **DATA Step & PROC SQL**: Emulates PAS semantics closely enough that common data-wrangling scripts run unmodified.
- **High Performance**: 
  - Streams rows through the DATA step without holding everything in memory.
  - Backed by **DuckDB** for highly optimized SQL execution.
  - Utilizes **Apache Arrow** for zero-copy memory transfers between the engine and the UI.
- **Large Dataset Support**: The dataset viewer supports paginated scrolling for million-row tables using TanStack Virtual.
- **Cross-Platform**: Builds as a single native redistributable binary (Windows, macOS, Linux).

*Note: Statistical procedures (like `PROC MEANS`, `PROC FREQ`, `PROC REG`) and the proprietary binary dataset format are explicitly out of scope for the current version. The PAS macro language (`%macro`, `%if`, `%do`, macro functions, `&`/`%` substitution) **is** supported — see [`SPEC.md`](SPEC.md) §5.5 and [`DIVERGENCE.md`](DIVERGENCE.md) §1.1 for the exact subset.*

## Architecture

PAS is split into a robust Rust backend and a modern React frontend, connected via Tauri:

- **`pas-engine` (Rust)**: The core parser and executor. It tokenizes, parses, and interprets the PAS language. Data operations are delegated to DuckDB or handled natively by the DATA step streaming engine.
- **`pas-app` (Rust/Tauri)**: The desktop application shell. It manages windowing, native menus, IPC, and filesystem access.
- **`ui` (React/TypeScript)**: The frontend. Built with React 18, Vite, and the Monaco Editor (customized with a PAS syntax highlighter).

## Installation

Pre-built installers for **Linux**, **Windows**, and **macOS** are published on
the [Releases page](https://github.com/GuiAlmeidaPC/pas/releases):

- **Linux:** `.AppImage` (portable) or `.deb`
- **Windows:** `.msi` or `.exe` installer
- **macOS:** `.dmg` (unsigned — right-click → Open the first time to bypass
  Gatekeeper)

Verify your download against the `SHA256SUMS.txt` attached to each release:

```bash
sha256sum -c SHA256SUMS.txt --ignore-missing
```

## Building from Source

To build PAS from source, you will need **Node.js**, **pnpm**, and **Rust** installed on your system.

### 1. Install Frontend Dependencies

```bash
cd ui
pnpm install
```

### 2. Build the Application

**During Development:**
To run the app locally with hot-reloading for the frontend:
```bash
cd crates/pas-app
cargo tauri dev
```

**For Production:**
To build a release binary (which automatically builds the frontend):
```bash
cd crates/pas-app
cargo tauri build
```

The resulting executable will be placed in `crates/pas-app/target/release/bundle/`.

### Running without the Tauri CLI (throttled first build)

If you don't have the Tauri CLI (`cargo tauri`) installed, you can run the app
with plain `cargo`. Start the Vite dev server, then launch the debug binary
(which loads `http://localhost:5173` in dev builds):

```bash
# Terminal 1: frontend dev server
cd ui && pnpm dev

# Terminal 2: from the repo root
cargo run -p pas-app
```

The **first** build compiles heavy native dependencies — most notably DuckDB's
~600 MB C++ amalgamation. On a multi-core machine this can saturate every core
and make the desktop feel unresponsive. To keep the machine usable during that
first build, cap parallelism, drop debug info, and deprioritize the job:

```bash
# from the repo root
nice -n 19 ionice -c3 env RUSTFLAGS="-C debuginfo=0" cargo run -p pas-app -j 6
```

- `-j 6` — limit cargo to 6 parallel jobs (tune to roughly half your cores).
- `RUSTFLAGS="-C debuginfo=0"` — skip debug info; cuts peak memory and build time.
- `nice`/`ionice` — leave CPU and I/O headroom for the rest of the desktop.

Subsequent builds are incremental and fast, so the throttling mainly matters
for the initial compile.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for development setup, the checks your
change must pass, and commit/PR conventions. AI coding agents should also read
[`AGENTS.md`](AGENTS.md).

Before writing code, review [`SPEC.md`](SPEC.md) and [`DIVERGENCE.md`](DIVERGENCE.md)
for the supported language subset and known divergences from documented compatibility behavior. User-visible changes should be noted in [`CHANGELOG.md`](CHANGELOG.md).

## Releases

Releases are automated and triggered by pushing a `vX.Y.Z` tag, which builds the
Linux/Windows/macOS bundles, attaches SHA-256 checksums, and creates a draft
GitHub Release. See the "Releasing" section of [`AGENTS.md`](AGENTS.md) for the
step-by-step flow.

## License

MIT License
