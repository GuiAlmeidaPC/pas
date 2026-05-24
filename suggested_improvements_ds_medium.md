Medium Priority Improvements — Detailed
1. Merge Cursor Uses O(n×k) Linear Scan; Should Be a Binary Heap
File: crates/pas-engine/src/datastep/exec.rs:1055-1111
Current state: The stream_merge function processes multiple sorted input cursors by scanning all cursors linearly to find the minimum-key row on each iteration. For k sources, every row processed requires k comparisons.
Problem: With many merge sources, this becomes O(n × k). If a user merges 10+ datasets, performance degrades noticeably.
Fix: Replace the linear scan with a std::collections::BinaryHeap keyed by the by-variable values. On each iteration, pop the minimum, consume rows from that source until the group changes, then push back. This drops complexity to O(n × log k).
2. Extract Duplicated Block-Dispatch Logic in submit() into execute_block()
File: crates/pas-engine/src/lib.rs:286-516
Current state: The submit() method contains ~230 lines where the same block-handling code appears twice: once at the top-level block iteration and again inside the Block::Statement sub-block loop. Lines 340-480 inside the Block::Statement arm duplicate the match logic from lines 350-515 in the outer loop (DATA step dispatch, PROC SQL dispatch, PROC dispatch, cancellation checks, macro variable cloning).
Problem: Every change to block dispatch (e.g. adding a new PROC) requires modifying code in two places, doubling the risk of drift.
Fix: Extract a private method fn execute_block(&self, block: &Block, ...) -> Result<Vec<Event>> that dispatches a single block. Call it both from the outer block loop and from within Block::Statement's sub-block iteration.
3. do while() / do until() Loop Variants Not Implemented
Files: crates/pas-engine/src/datastep/parse.rs, crates/pas-engine/src/datastep/exec.rs
Current state: Only do i = 1 to n; (indexed loops) are supported. SAS also defines:
- do while(condition); — evaluates at the top, executes body while true
- do until(condition); — evaluates at the bottom, executes body at least once until true
DIVERGENCE.md §1.7 notes these are "trivial to add" but they remain unimplemented.
Problem: Common SAS DATA step patterns using do while(last.id) or do until(eof) fail to parse.
Fix: Add While(Expression) and Until(Expression) variants to the DoSpec AST node. In the parser, after consuming do, check for while( / until( before assuming an indexed loop. In the executor, implement loop evaluation: for while, evaluate at top and break if false; for until, evaluate at bottom and break if true.
4. Implement LRU Eviction for Output Blocks
Files: crates/pas-engine/src/lib.rs (Session struct, submit method)
Current state: SPEC §7.4 specifies that output datasets should be evicted after N=20 submissions (LRU policy) to bound memory. The engine currently keeps all output blocks indefinitely — only MAX_PREVIEW_ROWS (1000) caps row count per preview, but obsolete datasets from prior runs persist in DuckDB.
Problem: Long-running sessions with repeated submissions accumulate stale datasets, consuming memory and disk.
Fix: Add a VecDeque<u64> tracking submission IDs to the Session struct. After each submit(), push the ID and, if len() > 20, drop the oldest submission's output datasets. Use DROP TABLE IF EXISTS for DuckDB-backed results and remove entries from self.result_blocks.
5. Missing Golden Tests for Error Paths
Files: tests/golden/ (add new .sas + .expected.json pairs)
Current state: All 9 golden tests assert "no_errors": true. There is no test that verifies the engine produces correct error diagnostics (line numbers, messages, severity) for malformed input.
Problem: Error-reporting code (span calculation, diagnostic messages, severity levels) is completely untested. Regressions in error quality go undetected.
Fix: Add 6-8 golden test pairs expecting "no_errors": false with specific error messages, covering:
- 01_syntax_error.sas — misspelled keyword, missing semicolon
- 02_undefined_dataset.sas — set on nonexistent table
- 03_merge_without_by.sas — merge a b; with no by statement
- 04_invalid_libname.sas — referencing an unassigned libref
- 05_type_mismatch.sas — arithmetic on character variables
- 06_unterminated_string.sas — missing closing quote in datalines
- 07_invalid_proc_sql.sas — from clause with syntax error
- 08_undefined_macro_var.sas — unresolved &var
6. Add cargo-fuzz Target for the Lexer/Parser
Files: New crates/pas-engine/fuzz/ directory
Current state: The hand-written lexer (449 lines in datastep/lex.rs) and recursive-descent parser (955 lines in datastep/parse.rs) have only 12 unit tests total. No fuzzing exists. SPEC §12.3 explicitly calls for cargo-fuzz.
Problem: Malformed SAS input can trigger panics, infinite loops, or crashes in the parser. Without fuzzing, these edge cases are found only by users.
Fix:
1. Add cargo-fuzz as a dev-dependency to the workspace
2. Create crates/pas-engine/fuzz/fuzz_targets/lex.rs that feeds arbitrary bytes to datastep::lex::tokenize() and verifies no panic
3. Create crates/pas-engine/fuzz/fuzz_targets/parse.rs that feeds tokenize() output to parse_data_step() and verifies no panic
4. Add a CI job (not on every PR; run nightly or on-demand) with cargo fuzz run parse -- -max_total_time=300
7. Tighten CSP for Production Builds
File: crates/pas-app/tauri.conf.json
Current state: The Content Security Policy allows:
- script-src 'unsafe-eval' (needed for Monaco)
- style-src 'unsafe-inline' (broad)
- connect-src http://ipc.localhost http://localhost:5173 ws://localhost:5173 (Vite HMR dev URLs)
The same CSP is used for both dev and production builds.
Problem: In production, localhost:5173 WebSocket and HTTP connections should not be permitted — they only exist for Vite dev server HMR.
Fix: Either:
1. Define separate dev and prod CSPs in tauri.conf.json (Tauri 2 supports "security": { "dev": {...}, "prod": {...} })
2. Or limit connect-src to http://ipc.localhost only in the production CSP
3. Consider adding require-trusted-types-for 'script' to further restrict injection vectors
8. No API Documentation on Tauri Commands
Files: crates/pas-app/src/lib.rs, crates/pas-engine/src/lib.rs
Current state: pas-app exposes 16 Tauri commands (submit, submit_files, cancel, read_file, write_file, read_project, save_project, list_libraries, list_datasets, dataset_schema, dataset_page, assign_libname, drop_libname, set_ai_config, get_ai_config_public, ai_chat). None have doc comments. The same is true for pas-engine's public API (Session, submit, run_sql, list_libraries, etc.).
Problem: A developer integrating with the engine or adding a new Tauri command has no reference for parameter shapes, return types, error conditions, or side effects. They must read the source.
Fix: Add /// doc comments on every #[tauri::command] and every pub fn / pub struct in the engine, describing:
- Parameter meaning and constraints
- Return value shape
- Error conditions and what each variant means
- Side effects (mutates session state, writes to DuckDB, etc.)
- Example usage in a one-liner
9. Add Datalines/Output/Delete/Stop/Rename/Label/Format DATA Step Golden Tests
Files: New .sas + .expected.json pairs in tests/golden/
Current state: The 9 golden tests cover core DATA step patterns (set, merge, by, retain, array, do, select/when, datalines). But ~60% of DATA step statements remain uncovered by golden tests:
- datalines with dsd, dlm=',', truncover, missover
- output <dataset> — explicit output to multiple datasets
- delete — conditional row removal
- stop — early termination
- rename old=new — column renaming
- label col='description' — column labels
- format col date9.; informat col mmddyy10.; — format/informat bindings
- attrib — multi-attribute column declarations
- where — DATA step WHERE statement (not clause)
- if _n_ = 1 — automatic variable usage
- infile with external file reading (DIR library)
Problem: Untested statements silently regress or produce wrong results.
Fix: Add 10-12 new golden test pairs, one per uncovered statement family. Each .expected.json should assert correct row counts, column counts, and column presence.
10. Replace All Mutex::lock().unwrap() Calls with Proper Error Handling
Files: crates/pas-engine/src/lib.rs, crates/pas-engine/src/datastep/exec.rs, crates/pas-engine/src/macros.rs
Current state: ~20 calls to self.conn.lock().unwrap(), self.macro_vars.lock().unwrap(), self.libraries.lock().unwrap() exist throughout the engine. If a mutex is poisoned (a prior panic in another thread), these cause a secondary panic with no useful context. One call at lib.rs:302 already uses .expect("engine mutex poisoned") — inconsistently.
Problem: A single panic anywhere in the engine poisons the session mutex, and all subsequent operations crash with opaque unwrap failures instead of clean errors.
Fix (two options):
1. Quick: Replace all .unwrap() with .expect("libraries lock poisoned") with a descriptive label. This at least gives a meaningful message.
2. Better: Implement From<PoisonError<T>> for EngineError and propagate errors through the Result chain. This lets the UI display "Session state corrupted — please restart" instead of crashing.
11. Deduplicate String Literal / Comment Scanning Logic
Files: crates/pas-engine/src/split.rs:39-59, 330-378, crates/pas-engine/src/sas_sql.rs:73-129, crates/pas-engine/src/lib.rs:1008-1023
Current state: Four separate implementations exist for the byte-level state machine that skips SAS string literals ('...', "...") and block comments (/* ... */):
- split.rs strip_comments() — strips /* */ and * ; comments
- split.rs split_on_semicolons() — respects string boundaries when splitting
- sas_sql.rs tokenize() — must skip over string contents during SQL tokenization
- lib.rs rewrite_librefs() — skips strings during libref substitution
Each has subtle differences. The lib.rs version doesn't handle escaped quotes via doubling; the sas_sql.rs version does.
Problem: A fix to string handling must be applied in 4 places. Missing a spot causes bugs where e.g. a semicolon inside a quoted string in PROC SQL is incorrectly treated as a statement terminator.
Fix: Extract a single fn skip_string_literal(input: &[u8], pos: usize, quote: u8) -> Option<usize> and fn skip_comment(input: &[u8], pos: usize) -> Option<usize> in a new crates/pas-engine/src/scan.rs module. Update all call sites to use the central implementations.
12. CI: No macOS Validation; Windows Build-Only (No Tests)
Files: .github/workflows/validate.yml, .github/workflows/build-windows.yml
Current state:
- validate.yml runs only on ubuntu-latest — fmt, clippy, test on Linux only
- build-windows.yml compiles on Windows but runs zero tests, no clippy, no fmt
- macOS has no CI at all — no validation, no build
The project targets all three desktop platforms via Tauri.
Problem: Platform-specific bugs (path handling on Windows, file permissions on macOS, DuckDB native library linking differences) are discovered only by users.
Fix:
1. Add macos-latest to the validate.yml matrix (full fmt + clippy + test)
2. Add windows-latest to validate matrix (at minimum cargo check + cargo test — Tauri deps may need WebView2 runtime workarounds)
3. Change cargo test -p pas-engine to cargo test --workspace to also test pas-app
4. Add cargo build --workspace step to verify pas-app compiles (currently not verified in CI)
13. Inconsistent Error Handling in build_where_clause (Parameterized Queries)
File: crates/pas-engine/src/lib.rs:1079-1096
Current state: The build_where_clause function constructs SQL WHERE clauses for the dataset viewer by escaping single quotes ('') and wrapping filter text in ILIKE '%...%'.
Problem: While not directly exploitable (DuckDB has no dangerous functions accessible via LIKE), string interpolation for user-supplied filter text is fragile. A filter containing % or _ (LIKE wildcards) produces unexpected matches. A filter containing '; DROP TABLE is safe only because DuckDB has no multiple-statement execution by default — this relies on implicit DuckDB behavior, not explicit safety.
Fix: Use DuckDB's ? parameterized query support instead of string interpolation. The appender-based DATA step already uses parameters. Apply the same pattern to the dataset viewer filter query: WHERE col ILIKE '%' || ? || '%' with a bound parameter.
14. Frontend: No Behavioral Component Tests
Files: ui/src/ (all .tsx files)
Current state: 2 tests exist, both static source-code regex scans (checking for the presence of string patterns in .tsx files, not executing them). Zero runtime tests for any React component.
Components with the most state logic and no tests:
- App.tsx (1,614 lines): tab management, event application, dirty-state tracking, submit dispatch, zoom persistence
- DatasetViewer.tsx (427 lines): pagination, sorting, filtering, stale request guard, Arrow IPC decoding
- AIChatPanel.tsx (427 lines): message state machine, code insertion, markdown rendering
- Tabs.tsx: close-confirmation dialogs, dirty flag propagation
Problem: Every UI change risks regressions in complex state machines. Manual testing is the only verification.
Fix:
1. Add vitest, @testing-library/react, jsdom to ui/package.json devDependencies
2. Create ui/src/__tests__/ with tests for:
- DatasetViewer: filter debounce timing, stale request rejection, Arrow batch decoding
- Tabs: dirty state propagation, close confirmation dialog logic
- App.tsx event application (applyEvent): error markers, dataset names, output rendering
3. Mock @tauri-apps/api calls to avoid needing the Tauri runtime in test
