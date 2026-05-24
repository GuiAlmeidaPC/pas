# Suggested Improvements

Analysis date: 2026-05-24

Validation performed:

- `cargo test --workspace` passed.
- `pnpm --dir ui run build` passed.
- Rust emitted warnings for unused variables/fields.

## Recommendations

1. Harden AI credential handling

   API keys are stored directly in `localStorage` in `ui/src/AIChatPanel.tsx`. Move secrets to the Tauri backend or OS credential storage, and route LLM calls through Rust instead of browser `fetch`. The Anthropic browser header is another sign this should be backend-mediated.

2. Add a real CSP

   CSP is disabled in `crates/pas-app/tauri.conf.json`. Define a production CSP that permits only the app bundle and explicitly needed AI endpoints. This matters more because the app can read/write files and stores credentials.

3. Scope file-system commands

   `read_file`, `write_file`, `read_project`, and `save_project` accept arbitrary paths from the frontend in `crates/pas-app/src/lib.rs`. Consider constraining operations to dialog-selected paths, project roots, or a backend-maintained allowlist.

4. Remove debug prints from engine execution

   The engine hot path logs full submitted programs and macro expansions with `println!` in `crates/pas-engine/src/lib.rs`. Replace these with `tracing::debug!` guarded by log level, or remove them. These can leak user data and slow large jobs.

5. Treat warnings as CI failures

   The current Rust suite passes but reports unused variables/fields. Add `cargo clippy --workspace -- -D warnings` and fix the warnings before they accumulate.

6. Expand CI beyond Windows packaging

   The workflow only builds the Windows app in `.github/workflows/build-windows.yml`. Add a fast validation workflow for PRs and pushes: `cargo fmt --check`, `cargo clippy`, `cargo test --workspace`, and `pnpm --dir ui run build`. Keep packaging separate.

7. Improve dataset viewer request safety

   `DatasetViewer` can issue overlapping page/filter requests without cancellation or stale-response protection. Add request IDs or cancellation so slow prior responses cannot overwrite newer filter/page results.

8. Parameterize or strongly validate generated SQL

   Filtering builds SQL strings in `crates/pas-engine/src/lib.rs`, and other query paths format identifiers and order clauses directly. Some escaping exists, but centralizing identifier quoting and using parameters for values would reduce injection and quoting bugs.

9. Use current, validated AI model defaults

   The model list in `ui/src/AISettingsModal.tsx` includes likely placeholder/future names. Either fetch supported models per provider or use a conservative custom-model-first flow so users are not led into failing defaults.

10. Add frontend tests for state-heavy UI

    The UI has many flows: project persistence, tab dirty state, event handling, dataset pagination, and AI snippet insertion. There are no frontend tests/scripts beyond build. Add Vitest/React Testing Library for reducers/helpers and Playwright smoke tests for the Tauri-facing workflow where practical.
