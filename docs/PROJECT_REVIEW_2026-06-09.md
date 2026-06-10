# PAS Project Review — 2026-06-09

Deep analysis of the repository at commit `2c2981f` (main, clean tree). No code
was changed as part of this review.

## Verification performed

| Check | Result |
|---|---|
| `cargo test -p pas-engine` | 149 unit tests + golden suite, all pass |
| `cargo clippy --workspace` | clean (no warnings) |
| `pnpm test` (ui) | 7 files / 47 tests, all pass |
| `pnpm run test:smoke` (ui) | passes |

## Overall assessment

This is a healthy, well-engineered project that is far past the prototype
stage. The architecture (Rust engine → Tauri shell → React UI, DuckDB + Arrow
underneath) is sound and the layering is real: the engine has no UI knowledge,
the shell mediates all I/O and AI traffic, and the frontend never holds
secrets. Documentation quality is unusually high — `SPEC.md`, `DIVERGENCE.md`,
and `AGENTS.md` are accurate and current, and the codebase contains honest,
constraint-explaining comments (e.g. the panic-safety rationale in
`session.rs`).

Notably, almost every item from the May 2026 review files
(`suggested_improvements_*.md`) has since been addressed: debug `println!`s
are gone, a real CSP exists, filesystem commands are allowlist-scoped, AI
calls are backend-mediated, a separate read-only DuckDB connection exists,
CI validates fmt/clippy/tests/audits, and the engine `lib.rs` monolith was
split into modules. The remaining findings below are the next tier.

---

## 1. High-impact recommendations

### 1.1 Stream engine events to the UI during a run (UX)

`Session::submit()` returns `Vec<Event>` only after the **entire program**
finishes; `pas-app`'s `submit` command then emits the buffered events
(`crates/pas-app/src/lib.rs:150-161`). For a long-running DATA step or SQL
query the log pane shows nothing until completion — the defining feedback
loop of an analytics IDE is missing. Cancellation works, but the user can't
see what they're cancelling.

**Recommendation:** add a callback/channel-based variant, e.g.
`Session::submit_with(&self, program, on_event: impl FnMut(Event))`, keep the
`Vec`-returning method as a thin wrapper for tests, and emit each event as it
is produced. `submit_files` already demonstrates the per-event emit pattern;
only the engine API change is needed.

### 1.2 Run the whole workspace's tests in CI

CI runs `cargo test -p pas-engine` only (`validate.yml`), but `pas-app` now
has meaningful tests — path-traversal guards, SSE parsing, model-list
filtering, OAuth token round-trips (`crates/pas-app/src/lib.rs` tests,
`oauth.rs` tests). They currently never run in CI.

**Recommendation:** change the CI step (and `AGENTS.md`) to
`cargo test --workspace`. The cost is small since the workspace is already
compiled for clippy.

### 1.3 Cancellation race between overlapping submissions

`submit()` begins with `self.cancel.store(false)` and the flag is shared
session-wide (`session.rs:122`). Sequence: run A is executing → user clicks
Cancel (`cancel = true`) → run B is submitted before A observes the flag →
B resets `cancel = false` → A keeps running. Because `submit` also holds the
write-connection mutex for its whole duration, B's reset happens while
blocked-waiting, making this window very real in practice (cancel + immediate
resubmit is the natural user gesture).

**Recommendation:** give each submission its own cancel token (e.g. a
generation counter or per-submission `Arc<AtomicBool>` returned to the
caller), or only reset the flag after acquiring the connection lock.

### 1.4 OAuth callback listener can hang forever

`wait_for_code()` (`oauth.rs:226-261`) enforces its 5-minute deadline only
*after* a connection arrives; `listener.incoming()` blocks indefinitely if
the user closes the browser tab without completing login. The
`spawn_blocking` thread — and port 1455 — then leak for the life of the
process, and a retried login fails with "port 1455 unavailable".

**Recommendation:** set the listener nonblocking (or use
`set_read_timeout`/poll loop) so the deadline is honored with no traffic, and
make the login cancellable from the UI.

---

## 2. Security observations

The security posture is good for a local desktop tool: CSP locked down
(`connect-src 'self' ipc:` — the webview cannot reach the network), file
commands restricted to `.pas`/`.pas.json` under an allowlist, dataset filters
parameterized with `ESCAPE`d `ILIKE`, AI keys encrypted at rest with the
documented obfuscation-only caveat. Remaining items, in priority order:

1. **The engine bypasses the filesystem sandbox by design.** `read_file`/
   `write_file` are carefully scoped, but any PAS program can do
   `libname x dir '/anywhere'`, `read_csv('/etc/passwd')`, or `COPY ... TO`
   via DuckDB — the engine has unrestricted filesystem access. For a local
   tool running user-authored code this is defensible, but it deserves an
   explicit entry in `DIVERGENCE.md`/`SPEC.md` (threat model: the sandbox
   protects against a compromised *webview*, not against malicious *programs*).
   If AI-generated programs can be auto-run in the future, this becomes a real
   boundary: consider DuckDB's `enable_external_access=false` plus an engine-
   level allowlist for DIR libnames.
2. **`get_ai_config` returns the raw API key to the frontend**
   (`lib.rs:770-782`). The webview only needs to know *whether* a key exists
   (plus provider/model). Returning the key re-exposes it to the renderer
   process that the rest of the design works to keep secret-free
   (CSP allows `unsafe-eval` for Monaco, so renderer compromise is not
   far-fetched). Recommend returning a redacted config and keeping the key
   backend-only.
3. **Reusing the OpenAI Codex CLI `CLIENT_ID`** (`oauth.rs:21`) against
   `chatgpt.com/backend-api/codex/responses` makes PAS impersonate another
   vendor's app. This is a terms-of-service and breakage risk (OpenAI can
   rotate/revoke that client at any time). Worth documenting as an accepted
   risk, and isolating so its removal is cheap.
4. **`ATTACH` path injection is handled** (single quotes doubled in
   `apply_libname`), and libref rewriting has regression tests for doubled
   quotes — good. Identifier quoting is centralized in `quote_ident` but not
   used everywhere (e.g. `resolve_read` formats `\"{}\"` manually); minor
   consistency cleanup.

---

## 3. Code quality and architecture

1. **`ui/src/App.tsx` (1,769 lines) is the new monolith.** It owns tabs,
   project persistence, layout, run lifecycle, event handling, menus, and
   modals. The engine-side split (`session/query/rewrite/split/...`) shows
   the team knows how to do this; the same treatment is due here. Natural
   seams: a `useProject` hook (open/save/dirty-state), a `useRunner` hook
   (submit/cancel/event subscription), and a layout component.
2. **`save_project`/`read_project` duplicate ~40 lines of allowlist
   registration verbatim** (`lib.rs:533-565` vs `612-644`). Extract a
   `register_project_paths(&project, &root, &mut allowed)` helper.
3. **Magic number drift:** `run_sql_with_rewrites` passes a literal `1000`
   (`session.rs:397`) where `MAX_PREVIEW_ROWS` (also 1000) is meant.
4. **178 `unwrap()`s in pas-engine.** The `catch_unwind` +
   poison-recovering-mutex design makes these non-fatal, which is a
   reasonable backstop — but each panic becomes a vague "internal engine
   error" instead of a useful message with a source span. Worth burning down
   opportunistically in the exec/eval hot paths.
5. **`macros.rs` (1,814 lines)** still carries six targeted clippy `allow`s
   (down from a blanket allow — progress). It's the most likely module to
   need ongoing work (`%sysfunc` is the headline gap in DIVERGENCE §1.1);
   splitting lexing/parsing/eval into submodules like `datastep/` would pay
   off before that feature lands.
6. **Two SSE parsers exist** (`drain_sse_events` in `lib.rs` and
   `drain_responses_sse` in `oauth.rs`) with the same buffering logic.
   Unify into one module with provider-specific delta extractors.

## 4. Testing gaps

Coverage is genuinely decent (149 engine tests, 13 golden programs, 47 UI
tests, security smoke tests). Remaining gaps:

- **`submit_files` and project open/save round-trip** have no tests — the
  path-resolution + allowlist logic there is exactly the kind of code that
  regresses silently.
- **Golden tests don't cover error programs.** All 13 golden cases are
  happy-path; add a few that assert specific `Event::Error` text/spans so
  diagnostics don't degrade.
- **No E2E/UI integration test** drives the real Tauri IPC layer. A single
  WebDriver/Playwright smoke (open app → run program → see output) would
  catch wiring breaks that unit tests can't. Acceptable to defer, but note it.
- The UI smoke tests are static source-regex checks; they're brittle to
  refactors (e.g. renaming `requestSeqRef` breaks the "stale response" test
  without any behavior change). Fine as tripwires; don't grow this pattern.

## 5. Housekeeping

1. **Delete or archive `suggested_improvements_codex.md`,
   `suggested_improvements_ds.md`, `suggested_improvements_ds_medium.md`.**
   They are dated 2026-05-24 and nearly everything in them is done; as
   top-level files they now actively mislead (both humans and AI agents
   reading the repo root).
2. **README first sentence is circular:** "PAS … provides a full-featured
   analytics IDE for authoring and running a PAS language." After the brand-
   neutral rewrite the sentence lost its referent — say what the language *is*
   ("a SAS-compatible data-wrangling language" or similar, to whatever degree
   trademark caution allows; `AGENTS.md` already says "clones the
   data-wrangling subset of SAS").
3. **`ui/package.json` version is 0.1.0** while the workspace and
   `tauri.conf.json` are 0.2.0. Harmless (private package), but the release
   checklist in `AGENTS.md` bumps two of three version fields; either sync it
   or note it's intentionally unversioned.
4. **`example_project/pojeto_teste.pas.json`** looks like a typo'd test
   artifact ("projeto") committed by accident; verify and remove.
5. **macOS releases are unsigned** (README documents the Gatekeeper
   workaround). Fine for now; an Apple Developer ID + notarization step in
   `release.yml` is the eventual fix and the workflow is already structured
   to take it.

## 6. Suggested priority order

1. Stream events during runs (1.1) — biggest user-visible win.
2. `cargo test --workspace` in CI (1.2) — one-line change, real coverage.
3. Housekeeping sweep (5.1, 5.2, 5.4) — cheap, removes misleading content.
4. Cancel-token race (1.3) and OAuth listener hang (1.4).
5. Redact `get_ai_config` (2.2); document the engine sandbox boundary (2.1).
6. App.tsx decomposition (3.1) — do it before the next big UI feature, not
   after.
