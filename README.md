# PAS (Practical Analytics Studio)

**PAS** is a cross-platform desktop application that clones the data-wrangling subset of SAS. It provides a SAS Enterprise Guide–style IDE for authoring and running a SAS-compatible language, specifically focusing on the **DATA step** and **PROC SQL**.

This project provides an offline, native experience for executing common data manipulation tasks without requiring a full SAS server.

## Features

- **Familiar IDE**: Includes a code editor, log pane, paginated output viewer, and library/project browsers.
- **DATA Step & PROC SQL**: Emulates SAS semantics closely enough that common data-wrangling scripts run unmodified.
- **High Performance**: 
  - Streams rows through the DATA step without holding everything in memory.
  - Backed by **DuckDB** for highly optimized SQL execution.
  - Utilizes **Apache Arrow** for zero-copy memory transfers between the engine and the UI.
- **Large Dataset Support**: The dataset viewer supports paginated scrolling for million-row tables using TanStack Virtual.
- **Cross-Platform**: Builds as a single native redistributable binary (Windows, macOS, Linux).

*Note: Statistical procedures (like `PROC MEANS`, `PROC FREQ`, `PROC REG`), macros, and the proprietary `.sas7bdat` format are explicitly out of scope for the current version.*

## Architecture

PAS is split into a robust Rust backend and a modern React frontend, connected via Tauri:

- **`pas-engine` (Rust)**: The core parser and executor. It tokenizes, parses, and interprets the SAS-compatible language. Data operations are delegated to DuckDB or handled natively by the DATA step streaming engine.
- **`pas-app` (Rust/Tauri)**: The desktop application shell. It manages windowing, native menus, IPC, and filesystem access.
- **`ui` (React/TypeScript)**: The frontend. Built with React 18, Vite, and the Monaco Editor (customized with a SAS syntax highlighter).

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

## Contributing

Please review `SPEC.md` and `DIVERGENCE.md` in the root directory for a detailed specification of the supported language subset and known divergences from standard SAS behavior.

## License

MIT License
