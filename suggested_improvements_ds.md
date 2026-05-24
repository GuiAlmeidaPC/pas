Critical / High Priority
1. Architecture — Monolithic lib.rs (2,549 lines)
The engine entry point is a kitchen sink. Split into:
- session.rs — session lifecycle, submit logic
- query.rs — DuckDB query execution, pagination
- rewrite.rs — SAS SQL → DuckDB rewrites
- types.rs — Event, Value, ResultBlock
The existing datastep submodules are well-organized; the problem is lib.rs hoards everything else.
2. Architecture — No separate read-only DuckDB connection
Session::submit() locks the mutex for the entire program run. dataset_page() also locks it, so UI paging blocks on a long-running DATA step. SPEC §6.5 already calls for a read-only connection. Add one.
3. Security — Remove 10 println! debug calls
lib.rs (lines 2276, 2287, 2296, 2528, 2540, 2547) and macros.rs (lines 1778, 1779, 1782, 1808) emit debug output unconditionally. Replace with tracing::debug! or proper assertions in tests.
4. Security — Remove #![allow(clippy::all, warnings)] from macros.rs
macros.rs line 11 suppresses all linting for 1,810 lines of code. Fix the warnings and remove the blanket allow.
5. Testing — pas-app crate has zero tests
The entire Tauri command layer (path security, AI config, file I/O, project save/restore) is untested. Add at minimum tests for the path traversal guards (normalize_path, ensure_under_project_root, allowed_paths).
6. Testing — datastep/exec.rs (1,555 lines) has zero unit tests
The most complex module in the project — DATA step executor, expression evaluator, merge logic, infile parsing — has no direct tests. Only exercised implicitly through integration tests.
7. Testing — No frontend component tests  
2 tests exist, both are static regex checks on source files (not runtime). Add Vitest + React Testing Library tests for DatasetViewer, Tabs, AIChatPanel.
8. Features — dataset_schema() returns ty: "?" for all columns
lib.rs:172-176 hardcodes column type to "?". Use DESCRIBE or PRAGMA table_info on DuckDB to return real types.
