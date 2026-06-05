# PAS — Specification

**PAS** (Practical Analytics Studio) is a cross-platform desktop application that clones the data-wrangling subset of SAS. It provides a SAS Enterprise Guide–style IDE for authoring and running a SAS-compatible language (PROC SQL + DATA step), with a log pane, a paginated dataset viewer, and a library/project browser.

Statistical procedures (`PROC MEANS`, `PROC FREQ`, `PROC REG`, etc.) are explicitly **out of scope**. Only data manipulation is supported.

---

## 1. Goals & Non-Goals

### 1.1 Goals
- Faithful enough emulation of SAS DATA step and PROC SQL semantics that common data-wrangling programs run unmodified.
- Familiar SAS EG–style UI: code editor, log, output, library tree, project tree.
- Submit selected code with **F3** (no selection → submit whole program).
- Output and dataset browsing must work on tables that don't fit in RAM (streamed pagination).
- Single redistributable binary per OS (Windows, macOS, Linux x86_64 + arm64).
- Fully offline; no telemetry by default.

### 1.2 Non-Goals
- Statistical procedures, graphing procs, ODS, IML, macro-heavy production workloads.
- SAS catalog (`.sas7bcat`) compatibility.
- Wire compatibility with SAS server protocols.
- Multi-user / collaborative editing.
- Mobile or web deployment.

### 1.3 v1 Scope Boundary
v1 ships with: PROC SQL, DATA step core (see §5), libraries against DuckDB and CSV/Parquet, a usable EG-like UI, and a near-complete macro processor (`%macro`/`%mend`, `%if`, `%do` loops, macro functions, and `&`/`%` substitution — see §5.5). `PROC IMPORT`/`PROC EXPORT` are v2. **`.sas7bdat` interop is explicitly out of scope** (the proprietary binary format is a deliberate non-goal).

---

## 2. Technology Stack

| Layer | Choice | Rationale |
|---|---|---|
| Shell | **Tauri 2.x** | Small binary, native webview, Rust-native IPC |
| Frontend | **React 18 + TypeScript + Vite** | Mature ecosystem for the editor + table widgets |
| Editor | **Monaco** | Selection API, custom tokenizer, F3 keybinding |
| Table viewer | **TanStack Virtual** + custom grid | Row virtualization for million-row paging |
| Engine | **Rust** (workspace crate `pas-engine`) | Same process as Tauri backend, no IPC overhead |
| Storage / SQL | **DuckDB** (via `duckdb` crate) | Columnar, embedded, ANSI SQL ≈ PROC SQL |
| In-memory format | **Apache Arrow** (`arrow-rs`) | Zero-copy batches between engine, DuckDB, and UI |
| Parser | Hand-written recursive-descent + Pratt for expressions | SAS grammar is too irregular for generators |
| Build | Cargo workspace + pnpm | Standard |
| Tests | `cargo test`, `insta` for snapshot, Playwright for UI smoke | — |

### 2.1 Workspace Layout
```
pas/
├─ Cargo.toml                  # workspace
├─ crates/
│  ├─ pas-lex/                 # lexer
│  ├─ pas-parse/               # parser, AST
│  ├─ pas-ast/                 # AST types shared by parse + interp
│  ├─ pas-engine/              # interpreter, DATA step executor, formats
│  ├─ pas-sql/                 # PROC SQL → DuckDB rewriter
│  ├─ pas-io/                  # readers/writers: csv, parquet
│  └─ pas-app/                 # Tauri binary + commands
├─ ui/                         # React app (Vite)
│  ├─ src/
│  └─ package.json
├─ examples/                   # sample .sas programs
├─ tests/                      # integration tests, golden programs
└─ SPEC.md
```

---

## 3. Application Model

### 3.1 Session
A **Session** is one running engine instance bound to one Tauri window. It owns:
- A DuckDB connection (single, serialized).
- A set of **Libraries** (named schemas in DuckDB or external file directories).
- A `WORK` library — temporary DuckDB schema, dropped on session close.
- The current **Project** (open files, layout, recent submissions).
- A submission queue (one program at a time; submissions are serialized).

### 3.2 Project
A **Project** is a directory containing:
```
my-project/
├─ project.pas.json            # layout, libname assignments, open tabs
├─ programs/*.sas              # user code
└─ work/                       # DuckDB file for persistent project tables (optional)
```
`project.pas.json` is human-readable and diffable.

### 3.3 Library
A `LIBNAME` maps a name to a backing store:
- `DUCKDB` — a schema inside the session DuckDB file
- `DIR` — a filesystem directory; each `.parquet`/`.csv` file appears as a dataset
- `MEMORY` — equivalent to `WORK`

Engine syntax mirrors SAS:
```sas
libname sales duckdb "C:/data/sales.duckdb";
libname raw   dir    "/data/landing" format=parquet;
```

---

## 4. UI Specification

### 4.1 Layout
Three-column resizable layout, matching SAS EG defaults:

```
┌──────────────┬──────────────────────────────┬──────────────┐
│ Project      │  Editor (Monaco, tabs)       │ Properties   │
│ Tree         │                              │ / Help       │
│              ├──────────────────────────────┤              │
│ Libraries    │  Log | Output | Datasets     │              │
│ Tree         │  (tab strip)                 │              │
└──────────────┴──────────────────────────────┴──────────────┘
```

Layout is persisted per-project in `project.pas.json`.

### 4.2 Editor
- Monaco with a custom `sas` language registration.
- Tokenizer recognizes: keywords, datalines block, macro tokens (`&x`, `%x`), string literals (single + double + `"..."x` hex), numeric literals, dates (`'01JAN2024'd`), comments (`*...;`, `/* */`).
- **Keybindings**:
  - `F3` — Submit. If a non-empty selection exists, submit only the selection; else submit the whole buffer.
  - `F4` — Cancel running submission.
  - `Ctrl/Cmd+Enter` — same as F3.
  - `Ctrl/Cmd+S` — Save file.
- Inline diagnostics from the parser (red squiggles for syntax errors, yellow for warnings).
- Bracket/quote auto-pair; smart indent after `data`, `proc`, `do`.

### 4.3 Log Pane
- Append-only, virtualized list of log records.
- Each record: `{level: NOTE|WARNING|ERROR, line_in_source, text, ts}`.
- Color coding: blue NOTE, green source echo, orange WARNING, red ERROR (configurable theme).
- Clicking a log line with a source-line reference jumps the editor cursor.
- "Clear log" button. Log persists per submission until cleared.

### 4.4 Output Pane
Two sibling tabs:
- **Output** — ordered list of result blocks produced by procs (e.g. PROC SQL with no `CREATE TABLE` outputs a result set). Rendered as a paginated grid.
- **Datasets** — last N (default 5) datasets created or modified in the submission, opened as paginated grids.

Grid behaviour (§7) is identical for both.

### 4.5 Library / Project Trees
- **Libraries**: top-level nodes are libnames; expand to list datasets; expand a dataset to list columns with type + format. Right-click → Open, Properties, Drop.
- **Project**: file tree rooted at project dir; double-click `.sas` opens a tab.

### 4.6 Status Bar
- Engine state: Idle / Running / Cancelling.
- Active library (current `WORK` size in rows + bytes).
- Cursor position, encoding, line endings.

### 4.7 Commands (Tauri `invoke` surface)
| Command | Args | Returns | Notes |
|---|---|---|---|
| `submit` | `{program: string, source_id: string}` | `submission_id` | Streams events via `pas://log`, `pas://output`, `pas://done` |
| `cancel` | `{submission_id}` | `()` | Cooperative |
| `list_libraries` | — | `Library[]` | |
| `list_datasets` | `{libref}` | `Dataset[]` | |
| `dataset_schema` | `{libref, name}` | `Schema` | |
| `dataset_page` | `{libref, name, offset, limit, sort?, filter?}` | `Arrow IPC bytes` | §7 |
| `output_page` | `{submission_id, block_idx, offset, limit}` | `Arrow IPC bytes` | |
| `assign_libname` | `{name, kind, path, opts}` | `()` | |
| `drop_libname` | `{name}` | `()` | |
| `open_project` / `save_project` | `{path}` | — | |

Events use Tauri's event system; payloads are JSON for log/control, binary (Arrow IPC) for data pages.

---

## 5. Language Specification

PAS implements a subset of the SAS language called **PAS/SAS**. Where behaviour differs from SAS, this document is authoritative.

### 5.1 Lexical
- Statements terminated by `;`. Whitespace-insensitive except inside strings and datalines.
- Comments: `* ... ;` (statement comment) and `/* ... */` (block, non-nesting in v1).
- Identifiers: `[A-Za-z_][A-Za-z0-9_]{0,31}`, case-insensitive, normalized to lowercase internally, preserved for display.
- String literals: `'...'` (no escapes except `''`; macro vars **not** resolved), `"..."` (with `""` escape; macro `&var`/`%macro` references **are** resolved), hex `"deadbeef"x`, date `'01JAN2024'd`, time `'13:30't`, datetime `'01JAN2024:13:30:00'dt`.
- Numbers: standard decimal, scientific.
- Special tokens: `&name` / `&name.` (macro var reference — resolved by the macro processor before lexing), `%name` (macro statement, function, or user-macro call — resolved by the macro processor; see §5.5).

### 5.2 Program Structure
A program is a sequence of **global statements**, **DATA steps**, and **PROCs**. Steps are delimited by `run;`, `quit;`, the next `data`/`proc`, or EOF.

Global statements in v1: `libname`, `filename`, `options`, `title`, `footnote`, and the macro statements `%let`, `%put`, `%global`, `%local`, `%macro`/`%mend`, `%if`/`%then`/`%else`, `%do`/`%end`, and user-macro calls (`%name`). See §5.5.

### 5.3 DATA Step

#### 5.3.1 Form
```sas
data <out-ds-list> [/ options];
    <statements>
run;
```

#### 5.3.2 Supported statements
| Statement | Notes |
|---|---|
| `set ds1 [ds2 ...] [(keep= drop= rename= where= in= obs= firstobs=)]` | Sequential, concatenating |
| `merge ds1 [(in=flag)] ds2 ...; by <vars>;` | Match-merge; requires sorted input or implicit sort. `in=` creates per-source 0/1 membership flags for the current BY group. |
| `by <vars>;` | Creates `first.var` / `last.var` automatic variables |
| `where <expr>;` | Pre-load filter (pushed to source) |
| `if <expr>;` / `if <expr> then <stmt>; [else ...]` | Subsetting if vs conditional |
| `keep <vars>;` / `drop <vars>;` / `rename old=new ...;` | Output-side |
| `retain <vars> [initial-values];` | Persists across iterations |
| `length <var> $n` / `length <var> n` | Declare type+width before first use |
| `format <var> fmt.;` / `informat ...;` | Display formats |
| `label <var>="..." ...;` | Stored in metadata |
| `array name{n} [$] [length] vars...;` | 1-based; `{*}` for "all matching" |
| `do ... end;`, `do i=a to b [by c]; ... end;`, `do while(...)`, `do until(...)` | |
| `if/else if/else`, `select/when/otherwise/end` | |
| `output [ds];` | Explicit emit; absence ⇒ implicit emit at iteration end |
| `delete;` | Skip emit, continue loop |
| `stop;` | Exit DATA step |
| `return;` | Jump to implicit loop top, emit unless `delete`d |
| `put <items>;` | Write to log (and to `file` target if set) |
| `call symput('name', value);` / `call symputx('name', value)` | Assign a macro variable from the DATA step at run time. `symputx` trims leading/trailing blanks. These are the only supported CALL routines. |
| `infile <path-or-fileref> [opts]; input <vars>;` | Free-form text read; v1 supports `dsd dlm=',' truncover` |
| `datalines; ... ;` | Inline data block |
| `attrib var length= label= format= informat= ...;` | Combined |

#### 5.3.3 Automatic variables
`_n_` (iteration counter, 1-based), `_error_` (0/1), `first.var`, `last.var` (with `by`).

#### 5.3.4 Variable typing
Two types only: **numeric** (f64, 8 bytes) and **character** (UTF-8, fixed declared length, space-padded on storage, trimmed on most ops). SAS missing numeric = NaN with a payload bit (we use Arrow null mask + a "special missing" side-table for `.a`–`.z`, `._`).

#### 5.3.5 Expressions & operators
Arithmetic `+ - * / **`, comparison `= ne lt le gt ge` and symbolic equivalents, logical `and or not`, `||` concat, `in (...)`, `between ... and ...` (PROC SQL only), `:` colon-modifier on comparisons (truncated char compare).

#### 5.3.6 Functions (v1 mandatory set)
- String: `substr`, `scan`, `index`, `find`, `tranwrd`, `translate`, `compress`, `compbl`, `strip`, `trim`, `left`, `right`, `upcase`, `lowcase`, `propcase`, `cats`, `catx`, `length`, `lengthn`, `lengthc`, `repeat`, `reverse`, `prxmatch`, `prxchange` (PCRE-backed).
- Numeric: `abs`, `ceil`, `floor`, `round`, `int`, `mod`, `min`, `max`, `sum`, `mean`, `sqrt`, `exp`, `log`, `log10`, `log2`.
- Date/time: `today`, `date`, `time`, `datetime`, `year`, `month`, `day`, `qtr`, `weekday`, `hour`, `minute`, `second`, `mdy`, `ymd`, `hms`, `dhms`, `intnx`, `intck`, `datepart`, `timepart`.
- Conversion: `put(value, fmt.)`, `input(text, informat.)`.
- Missing: `missing`, `coalesce`, `coalescec`, `nmiss`, `cmiss`.

Functions outside this list raise `ERROR: function X is not implemented in PAS v1.`

#### 5.3.7 Execution model
1. **Compile**: AST → typed IR. Determine PDV (Program Data Vector) layout from `set`/`merge`/`length`/first-assignment scans.
2. **Init**: Open all input sources as Arrow record-batch streams. Allocate PDV. Set non-retained vars to missing at top of each iteration.
3. **Iterate**: For each row from the driving source, copy into PDV, execute body, emit to one or more output writers on `output`/implicit emit.
4. **Finalize**: Close writers, register output datasets with the session, write log NOTE with row counts.

#### 5.3.8 Output dataset registration
Each output is written as a new DuckDB table (or Arrow IPC file for `MEMORY`) under the target libref, with a side metadata table `pas_meta_<table>` storing labels, formats, informats, lengths, and creation timestamp.

### 5.4 PROC SQL

```sas
proc sql [noprint outobs=n inobs=n];
    <sql-statements>
quit;
```

Supported:
- `select`, `create table ... as`, `insert into`, `update`, `delete`, `drop table`, `create view` (materialized — no lazy in v1).
- SAS-specific: `calculated <col>`, `outer union [corr]`, `monotonic()` → `row_number() over ()`, `eqt`/`gtt` truncated comparisons (rewritten), automatic remerging of summary stats (NOTE emitted).
- Three-part names: `libref.table` → `schema.table` in DuckDB.

Implementation: parse the SAS-flavoured SQL into our own AST, rewrite to DuckDB SQL, execute. Result sets without `create table` become **output blocks** displayed in the Output tab.

### 5.5 Macro Language
PAS ships a near-complete macro processor that runs as a text-substitution
pass over each block before lexing (`crates/pas-engine/src/macros.rs`).

Supported statements:
- `%let name = value;` — assign a macro variable in the current scope.
- `%put <text>;` — emit text to the log.
- `%global name1 name2;` / `%local name1 name2;` — scope declarations.
- `%macro name(<positional>, key=default); ... %mend [name];` — definitions
  with positional and keyword parameters (defaults supported).
- `%name(args)` / `%name` — user-macro invocation.
- `%if <cond> %then <action>; [%else <action>;]` — conditional logic.
- `%do; ... %end;` — block.
- `%do var = start %to end [%by step]; ... %end;` — iterative loop.
- `%do %while(<cond>); ... %end;` and `%do %until(<cond>); ... %end;`.

Variable references:
- `&name` / `&name.` — resolved from the macro symbol table. The trailing
  dot is an optional terminator. Resolution happens before lexing; references
  inside `"..."` double-quoted strings are resolved, those inside `'...'`
  single-quoted strings are not.

Built-in macro functions: `%eval`, `%sysevalf`, `%upcase`, `%lowcase`,
`%substr`, `%length`, `%index`, `%scan`, `%str`, `%quote`, `%bquote`,
`%superq`.

Automatic macro variables: `&sysdate`, `&sysday`, `&systime`, `&sysuserid`,
`&syscc`, `&syserr`.

DATA-step → macro binding is available via `call symput`/`call symputx`
(see §5.3.2).

Known gaps vs SAS are tracked in `DIVERGENCE.md` (notably: no `%sysfunc`, and
`%put` output is emitted ahead of program output — see DIVERGENCE §5.3).

### 5.6 Reserved Options (v1)
`options` accepts and stores (most are no-ops for now): `linesize`, `pagesize`, `nodate`, `nonumber`, `mprint`, `symbolgen`, `obs=`, `firstobs=`, `compress=`. Unknown options warn, do not error.

---

## 6. Engine Architecture

### 6.1 Pipeline
```
source text
   │  (5.5) macro pre-pass
   ▼
pas-lex tokens
   ▼
pas-parse AST (per-step)
   ▼
step planner → [Step]
   ▼
executor (DATA step interp | SQL rewriter→DuckDB | global)
   ▼
results: log events, output blocks, dataset writes
```

### 6.2 Step Execution
- Steps are run sequentially. A failing step emits ERROR and stops the submission unless `options errorabend=no` (default).
- Each step gets a `StepCtx` with: session handle, DuckDB conn, libname map, symbol table, cancellation token, log sink.

### 6.3 DATA Step Interpreter
- **PDV** is a `Vec<Cell>` where `Cell = Numeric(f64) | Char(Box<str>)` with a parallel `nulls: BitVec` and `special_missing: Vec<u8>`.
- The body compiles to a flat **bytecode** (`OpCode` enum: `LoadConst`, `LoadVar`, `StoreVar`, `BinOp`, `Call`, `JumpIfFalse`, `Output`, `Delete`, …). Tree-walking is acceptable in v0 but bytecode is the v1 target for `do` loop performance.
- Input is pulled as Arrow `RecordBatch`es (typically 8192 rows) from DuckDB or a file reader. The interpreter loops over batch rows in-place; this avoids per-row DuckDB calls.
- Output is appended into Arrow `RecordBatchBuilder`s; flushed every 64K rows into DuckDB via `appender` API or Parquet for `DIR` libraries.
- `merge` with `by` is implemented as a k-way ordered iterator over pre-sorted batches; if inputs aren't sorted, the engine emits a NOTE and sorts via DuckDB.

### 6.4 Cancellation
`StepCtx` holds an `Arc<AtomicBool>`. The interpreter checks every 4096 rows and between steps. SQL execution uses DuckDB's interrupt API.

### 6.5 Concurrency
- One submission at a time per session (matches SAS).
- The engine runs on a dedicated Tokio task; Tauri commands enqueue work.
- DuckDB connection is `!Send` safe via a `Mutex`-guarded handle owned by the engine task.
- UI paging requests (§7) run concurrently on a **separate read-only DuckDB connection** so they don't block submissions.

### 6.6 Errors
All engine errors implement `PasError` with: `code`, `severity`, `source_span: Option<(line, col, len)>`, `message`, `hint`. Errors with spans become editor diagnostics.

---

## 7. Paginated Output / Dataset Viewer

### 7.1 Requirements
- Open a 100 M-row dataset and scroll smoothly.
- Sort and filter without loading the whole table.
- Memory ceiling: < 200 MB resident for the UI process regardless of dataset size.

### 7.2 Protocol
- UI requests pages: `dataset_page({libref, name, offset, limit, sort, filter})`.
- Engine issues `SELECT ... FROM <table> [WHERE filter] [ORDER BY sort] LIMIT limit OFFSET offset` against DuckDB.
- Response is an **Arrow IPC stream** (binary), decoded in JS via `apache-arrow`.
- Default page size: 1000 rows. Grid prefetches ±2 pages around the viewport.

### 7.3 Sort/Filter
- Filter expressions are typed in a header row (per-column), translated to DuckDB `WHERE`.
- Sort indicators in headers, multi-column via shift-click.
- For unsorted scrolling without a `sort`, DuckDB row order is the table's natural order (insertion). For sorted scrolling, the engine creates a temporary `ORDER BY`-projected DuckDB view to keep pagination stable.

### 7.4 Output Blocks
Results from PROC SQL `select` (no `create table`) are written to ephemeral DuckDB tables under `pas_output.<submission>_<idx>`, dropped when the submission's results are dismissed or after N=20 submissions (LRU).

---

## 8. File I/O

### 8.1 Readers (v1)
| Format | Library kind | Notes |
|---|---|---|
| CSV | `DIR format=csv` | DuckDB `read_csv_auto` |
| Parquet | `DIR format=parquet` | DuckDB native |
| JSON-lines | `DIR format=jsonl` | DuckDB `read_json_auto` |

### 8.2 Writers (v1)
| Format | Notes |
|---|---|
| DuckDB table | Default for `DUCKDB`/`WORK` |
| Parquet | For `DIR format=parquet` writes |
| CSV | For `DIR format=csv` writes (UTF-8, RFC 4180) |

### 8.3 Encoding
UTF-8 internal. `infile` accepts `encoding=` and transcodes via `encoding_rs`.

---

## 9. Configuration & Persistence

### 9.1 User Config
Located at the OS config dir (`~/.config/pas/config.toml` on Linux, `%APPDATA%\pas\config.toml` on Windows, `~/Library/Application Support/pas/config.toml` on macOS).

```toml
[ui]
theme = "dark"
font = "JetBrains Mono"
font_size = 13

[engine]
batch_size = 8192
output_block_lru = 20
default_page_size = 1000

[paths]
projects_dir = "~/pas-projects"
```

### 9.2 Project File (`project.pas.json`)
```json
{
  "version": 1,
  "name": "sales-q1",
  "libnames": [
    {"name": "RAW", "kind": "DIR", "path": "./data", "format": "parquet"},
    {"name": "OUT", "kind": "DUCKDB", "path": "./work/out.duckdb"}
  ],
  "open_tabs": ["programs/load.sas", "programs/transform.sas"],
  "active_tab": "programs/transform.sas",
  "layout": {"left": 240, "right": 280, "bottom": 320}
}
```

### 9.3 Session State
- `WORK` lives in an in-memory DuckDB by default; can be promoted to disk via `options work="/path"`.
- Macro symbol table and `libname` assignments are session-scoped (not persisted unless saved into the project).

---

## 10. Logging & Diagnostics

### 10.1 Log Records
```rust
struct LogRecord {
    ts: DateTime<Utc>,
    submission_id: Uuid,
    level: Level,           // Note | Warning | Error | Source
    source_span: Option<Span>,
    text: String,
}
```

### 10.2 Standard Notes
After every DATA step:
```
NOTE: The data set WORK.OUT has 12,345 observations and 7 variables.
NOTE: DATA statement used (Total process time):
      real time           0.42 seconds
      cpu time            0.31 seconds
```

After PROC SQL:
```
NOTE: Table WORK.OUT created, with 12345 rows and 7 columns.
```

### 10.3 Engine Trace
With `options pasdebug=1;`, the engine additionally logs IR, planner decisions, and DuckDB SQL it generates. Off by default.

---

## 11. Performance Targets

| Operation | Target on a 2024 laptop |
|---|---|
| Cold start to interactive UI | < 1.5 s |
| Submit a 10-line program (no I/O) | < 50 ms latency to first log line |
| DATA step `set` + `if` over 10 M rows (Parquet input) | < 4 s |
| Open a 100 M-row Parquet dataset in viewer | < 500 ms to first page |
| Scroll to row 50 M in viewer | < 200 ms per page after seek |
| Memory ceiling, UI process | < 300 MB steady-state |
| Memory ceiling, engine, on 100 M-row workloads | < 1.5 GB (streaming) |

---

## 12. Testing Strategy

### 12.1 Unit
- `pas-lex`, `pas-parse`: snapshot tests via `insta` over a corpus of SAS programs.
- `pas-engine`: per-function tests (one per SAS function in §5.3.6).

### 12.2 Golden Programs
`tests/golden/` contains `.sas` programs paired with expected `.log` and expected output `.parquet`. CI runs each program and diffs results. The corpus must cover:
- Every supported DATA step statement.
- `by` group processing with first./last.
- `merge` (1:1, 1:n, n:m with warnings).
- `retain` with and without initial values.
- Arrays including `{*}` and temporary arrays.
- Each function in §5.3.6 with at least one missing-value case.
- PROC SQL: joins (inner/left/right/full), `calculated`, `outer union corr`, aggregation with remerge, three-part names.

### 12.3 Fuzz
`cargo-fuzz` target on the parser to ensure no panics on arbitrary input.

### 12.4 UI
- Playwright smoke: open app, type program, F3, assert log + output.
- Visual regression on the grid for fixed datasets.

### 12.5 Conformance
A "SAS divergence" doc tracks every known deviation from SAS behaviour, with rationale. Tests assert the divergence to prevent silent regressions.

---

## 13. Packaging & Distribution

- Built via `cargo tauri build` per OS.
- Targets: Windows MSI + portable EXE, macOS DMG (universal binary), Linux AppImage + .deb.
- Code signing: macOS notarization required for release builds; Windows Authenticode required.
- Auto-update via Tauri updater, opt-in, signed manifests.
- Version scheme: SemVer. v1.0 unblocks once all v1 scope items pass golden tests.

---

## 14. Security

- The engine executes arbitrary user code (data manipulation), but never shells out except for the `x` / `systask` statements which are **disabled** in v1 (parse error).
- Library paths are restricted to the project directory unless `options allow_external_paths=yes;` is set. Default: any path the user types in a `libname` is allowed (it's their machine), but the UI shows a one-time confirmation when assigning a libname outside the project tree.
- No network access from the engine. File reads honour OS permissions.
- Tauri allowlist limits the frontend to the `invoke` commands listed in §4.7.

---

## 15. Roadmap

### v0.1 (Walking Skeleton)
- Tauri shell, Monaco editor, F3 submit, log pane.
- Engine accepts a program, runs **PROC SQL only** against DuckDB, returns results.
- Single in-memory `WORK` library.

### v0.2
- Paginated output viewer with Arrow streaming.
- `libname` (DUCKDB + DIR), library tree.
- CSV/Parquet readers.

### v0.3 — DATA step v0
- `set`, `where`, `keep/drop`, `if/then/else`, `output`, `length`, basic functions (string + numeric).

### v0.4 — DATA step v1
- `by` + first./last., `merge`, `retain`, `array`, `do` loops, `select/when`, dates and date functions.

### v0.5
- Formats and informats (`put`/`input`, `format` statement).
- `datalines`, `infile`/`input`.

### v0.6
- Project files, multi-tab editor, save/restore layout.
- Cancellation, status bar, error squiggles wired through.

### v0.9 — Beta
- Performance pass: bytecode interpreter, batch tuning.
- Full golden-test suite green.
- Installers for all three OSes.

### v1.0
- Macro language (`%let`, `%put`, `&var`, plus `%macro`/`%mend`, `%if`,
  `%do` loops, and macro functions — see §5.5).
- Auto-update.  *Deferred: requires a real distribution server with signed manifests. Tracked separately.*
- SAS divergence document published. See `DIVERGENCE.md`.

### v2 (post-1.0)
- Macro language polish (`%sysfunc`, additional autocall built-ins, ordering
  parity for `%put`).
- `PROC IMPORT`/`PROC EXPORT`.
- ODS-lite for HTML output.
- Optional: scripting hooks (run a `.sas` from CLI without UI).

*Note: `PROC SORT`, `PROC PRINT`, and `PROC TRANSPOSE` shipped in v1.1.*

---

## 16. Open Questions

1. **Special missing values** (`.a`–`.z`, `._`): store as side-table or encode in NaN payload bits? Decision affects every numeric op.
2. **Sort stability for `by`**: rely on DuckDB sort (not stable by default) or implement a stable external merge?
3. **Macro pre-pass vs integrated lex**: a true SAS macro processor interleaves with tokenization. v1 runs macros as a per-block text pre-pass instead; this is why `%put` output is ordered ahead of program output (DIVERGENCE §5.3) and why `%sysfunc` (which calls DATA-step functions) is not yet supported. Revisit interleaving if these gaps matter.
4. **DuckDB versioning**: pin DuckDB and embed the storage version, or expose `ATTACH` for cross-version files?
5. **Plug-in surface**: should v2 expose a Rust trait for custom procs / functions? If yes, design now to avoid breaking changes.

---

*End of SPEC.md*
