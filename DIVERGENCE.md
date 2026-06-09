# PAS Language Divergences

This document enumerates every place PAS v1.0 behaves differently from the
reference language. Each entry says **what the reference system does**, **what PAS does**, and
**why** — so we can decide later whether to close the gap or codify it as
intentional.

Out-of-scope items (proprietary binary datasets, statistical procedures, `PROC IMPORT`, etc.)
are not divergences — they are not implemented at all and are listed in §1.2 of
`SPEC.md`.

---

## 1. Language

### 1.1 Macro language is near-complete but not the full reference system

| | |
|---|---|
| **Reference** | Full programmable macro language with `%macro/%mend`, `%if/%then/%else`, `%do` (block / iterative / `%while` / `%until`), a large autocall/built-in library including `%sysfunc`, and macros that can generate arbitrary code. |
| **PAS** | A near-complete text-substitution macro processor (`crates/pas-engine/src/macros.rs`): `%let`, `%put`, `%global`, `%local`, `%macro`/`%mend` with positional + keyword (default) parameters, `%name(...)` calls, `%if/%then/%else`, all four `%do` loop forms, `&var`/`&var.` resolution (including inside `"..."`), and the built-in functions `%eval`, `%sysevalf`, `%upcase`, `%lowcase`, `%substr`, `%length`, `%index`, `%scan`, `%str`, `%quote`, `%bquote`, `%superq`. Automatic vars: `&sysdate`, `&sysday`, `&systime`, `&sysuserid`, `&syscc`, `&syserr`. `call symput`/`symputx` bind macro vars from the DATA step. |
| **Not yet** | `%sysfunc` / `%qsysfunc` (calling DATA-step functions from macro context) and other autocall functions outside the list above. Unrecognized `%` directives pass through and are reported by the downstream parser. |
| **Why** | The processor runs as a per-block text pre-pass rather than interleaving with the lexer (see SPEC §16.3); `%sysfunc` needs the function evaluator wired into that pass. |

### 1.2 Format token shape in expressions

| | |
|---|---|
| **Reference** | Formats appear bare in expressions: `put(x, date9.)`, `input(s, comma8.2)`. |
| **PAS** | Format must be a quoted string: `put(x, 'date9.')`, `input(s, 'comma8.2')`. The `format` statement, however, accepts bare format names. |
| **Why** | The lexer turns `date9.` into surprising tokens (`Ident("date9") Dot`); a context-sensitive re-lex would be brittle. Requiring quotes is unambiguous and easy to fix later. |

### 1.3 DATA step set + by interleave

| | |
|---|---|
| **Reference** | `set a b c; by id;` interleaves rows from all sources in by-order. |
| **PAS** | `set` with multiple sources concatenates. `by` with multi-source `set` is not yet supported. |
| **Why** | Less common; deferred. `merge ...; by ...;` is supported. |

### 1.4 Special missing values (.a–.z, ._)

| | |
|---|---|
| **Reference** | 28 distinct numeric missing values, ordered between `._ < . < .a < .b < ... < .z`. |
| **PAS** | Only the standard numeric missing (NaN). Special missing values are not parsed and not represented. |
| **Why** | Listed as an open question in `SPEC.md` §16. |

### 1.5 Variable lengths are advisory

| | |
|---|---|
| **Reference** | `length` declarations are enforced — character vars are space-padded to width, numeric widths affect precision. |
| **PAS** | `length` registers the type (char vs numeric) and PDV order, but width is not enforced. Numeric values are always `f64`. Character values are always `String` (UTF-8) with no padding. |
| **Why** | Width-correct character storage adds complexity for very little wrangling benefit; revisit if a real use case appears. |

### 1.6 Datalines and infile column inputs

| | |
|---|---|
| **Reference** | Supports column-range input (`input name $1-10 age 11-13;`), pointer controls (`@col`, `+n`), modified-list input (`name :$20.`), and formatted input with informats. |
| **PAS** | Supports list input, **modified-list input** (`var :informat.`), **formatted input** via informat width (`var informat.`), and **column-range input** (`var [$] start-end`, 1-based inclusive; a bare `start` reads one column), all driven by a column pointer that advances as fields are read. Informats: `$charW.` (preserves leading blanks), `$w.` (left-aligns), `w.d` numeric, `dateW.` → PAS date serial, and `commaW.d` / `dollarW.d` (strip `$ , ( )`). Unknown informats raise an error. The `format` statement is applied for supported display formats in dataset pages and PROC PRINT (`date9.`, `commaW.d`, `dollarW.d`, `best`/numeric); stored values remain raw numbers/strings for SQL, joins, filters, and calculations. **Not yet:** `@col` / `+n` pointer controls, other informats (time/datetime, `mmddyy`, `yymmdd`, etc.), persisted display-format metadata for directory-backed CSV/Parquet libraries, and applying `informat` / `label` statements. |
| **Why** | Informat and column input cover fixed-width and typed legacy files. Explicit `@`/`+n` pointer controls and broader metadata persistence are the remaining pieces. |

### 1.7 DO loop variants

| | |
|---|---|
| **Reference** | `do while(<expr>)`, `do until(<expr>)`, `do i = 1 to n [by s];`, and `do i = 1, 3, 5;` (value-list form) are supported. |
| **PAS** | Supports `do; ... end;` (block), `do var = a to b [by c]; ... end;` (iterative), `do while(<expr>); ... end;`, and `do until(<expr>); ... end;`. The value-list form (`do i = 1, 3, 5;`) is not yet implemented. |
| **Why** | The value-list form is rarely used outside index iteration that the indexed form already covers. |

### 1.8 PROC step coverage

| | |
|---|---|
| **Reference** | Many PROCs: `PROC SORT`, `PROC TRANSPOSE`, `PROC FREQ`, `PROC PRINT`, `PROC MEANS`, `PROC IMPORT`, etc. |
| **PAS** | `PROC SQL`, `PROC SORT`, `PROC PRINT`, `PROC TRANSPOSE`. Everything else errors with "PROC X is not implemented in PAS". |
| **Notes** | `PROC SORT` supports `data=`, `out=` (in-place if omitted), `by … descending …`, `nodupkey` (one row per by-key), `noduprecs` (drop exact duplicates). `PROC PRINT` supports `data=`, `obs=`, `var`. `PROC TRANSPOSE` supports `data=`, `out=`, `by`, single-`id`, single-`var` (translated to DuckDB `PIVOT`). |
| **Why** | Statistical procs (`MEANS`, `FREQ`, `REG`) remain explicitly out of scope per `SPEC.md` §1.2. |

---

## 2. Execution model

### 2.1 In-memory materialization in DATA step

| | |
|---|---|
| **Reference** | DATA step streams rows one at a time. Output is appended without holding all rows in memory. |
| **PAS** | DATA step streams source rows from DuckDB and writes output via the Appender API. The pipeline holds only the current row, a one-row lookahead (used for `last.var` detection), and the appender's own batch. `merge` snapshots each source into a sorted DuckDB TEMP table once, then walks the snapshots through paged cursors that refill 4K rows at a time — so the Rust process holds at most `N_sources × 4096` rows even for million-row merges. |

### 2.2 PROC SQL extensions

| | |
|---|---|
| **Reference** | PROC SQL with `calculated`, `outer union [corr]`, three-part names (`libref.table`), automatic remerging, `monotonic()`, truncated comparisons (`eqt`, `gtt`, `lt:` colon-modifier). |
| **PAS** | DuckDB-backed PROC SQL with a token-aware rewriter (`crates/pas-engine/src/pas_sql.rs`) handling: `calculated <col>` → `<col>`; `monotonic()` → `row_number() over ()`; `outer union corr` → `union all by name`; `outer union` → `union all`; `CREATE TABLE` → `CREATE OR REPLACE TABLE`; and `CREATE TABLE libref.ds AS …` for DIR libraries → `COPY (…) TO 'path' (FORMAT …)`. Three-part names are resolved by the libref rewriter. |
| **Not yet** | Automatic remerging (e.g. `select id, max(score) from t` without GROUP BY) and truncated comparisons (`eqt`/`gtt`). Both require fuller query parsing than a token-aware pass. |

### 2.3 CREATE TABLE overwrite

| | |
|---|---|
| **Reference** | `create table t as ...;` errors if `t` exists. Must `drop table t;` first. |
| **PAS** | `create table t as ...;` always overwrites (translated to `CREATE OR REPLACE TABLE` for DuckDB). |
| **Why** | Matches PAS PROC SQL `outobs=` / re-run ergonomics users actually expect; explicit `drop` is annoying in an interactive editor. |

### 2.4 Cancellation granularity

| | |
|---|---|
| **Reference** | "Cancel" usually halts immediately. |
| **PAS** | Cancel is cooperative — interrupts the running DuckDB query via `Connection::interrupt`, and the engine's per-statement and per-iteration cancel-flag checks stop the next operation. A `for` body that is purely Rust (no DuckDB calls) cancels at the next row boundary. |
| **Why** | Sufficient for interactive use. |

### 2.5 Sort stability for `by`

| | |
|---|---|
| **Reference** | `by` requires pre-sorted input (or you use `PROC SORT`); ordering within ties is unspecified. |
| **PAS** | Auto-sorts input via DuckDB `ORDER BY <by-vars>` before iterating; DuckDB sorts are not guaranteed stable. |
| **Why** | Convenience for `set ... ; by ...;` users. Logged as an open question. |

---

## 3. I/O and formats

### 3.1 Formats supported

PAS implements a useful subset of reference-system formats. Numeric: `best.`, `8.`, `8.2`,
`comma.`/`comma8.2`, `date9.`, `mmddyy10.`, `ddmmyy10.`, `yymmdd10.`, `time8.`,
`datetime19.`. Character: `$char20.`, `$upcase.`, `$lowcase.`. Anything else
errors at `put`/`input` time.

### 3.2 No proprietary dataset interop

PAS does not read or write proprietary binary datasets. Convert via the source system, or use a tool
like Stata's `usespss`, R's `haven`, or Python's `pyreadstat` to get to
Parquet/CSV first.

### 3.3 No encoding= on infile

| | |
|---|---|
| **Reference** | `infile '...' encoding='latin1';` transcodes on the fly. |
| **PAS** | Files are read as UTF-8 (Rust `read_to_string`); non-UTF-8 input errors. |
| **Why** | `encoding_rs` integration is straightforward but not yet plumbed through. |

---

## 4. UI / project

### 4.1 Single window, single session

PAS runs one engine session per process. The reference system supports multiple
server-backed sessions. Not planned for v1.x.

### 4.2 No project autosave

Open tabs are saved when the user saves the project. There is no autosave; an
unsaved buffer is lost if the app is closed.

### 4.3 No log persistence

The log clears on each new submission. desktop analytics writes per-program
logs to disk; PAS only shows the most recent log in the bottom pane.

### 4.4 AI assistant: ChatGPT OAuth and token storage

Not a language divergence — a PAS-specific integration note. The AI assistant offers
"Sign in with ChatGPT" alongside API-key providers. This embeds the **public**
OpenAI Codex OAuth `client_id` (`app_EMoamEEZ73f0CkXaXp7hrann`) and talks to the
Codex Responses API (`chatgpt.com/backend-api/codex/responses`), the same client
the Codex CLI uses; it is not an officially documented third-party API and may
change without notice. OAuth tokens are persisted in
`chatgpt_tokens.enc` (app data dir), AES-256-GCM-encrypted with a key **derived
from a stable install path** — this is obfuscation against casual inspection,
**not** strong at-rest protection (anyone with the file and the app binary can
decrypt). API keys, by contrast, remain in memory only and are never persisted.

---

## 5. Defaults and quirks

### 5.1 Library `WORK` is always present and in-memory

PAS uses an in-memory DuckDB for `WORK`. Closing the app loses all `WORK`
tables. Persist with a `libname` to DUCKDB or DIR.

### 5.2 Identifier case

PAS lowercases identifiers internally (matching the reference system's case-insensitive
behavior). Column / dataset names are displayed lowercase except in log
notes, where dataset references are uppercased (`NOTE: The data set WORK.X has
N observations.`).

### 5.3 `%put` ordering

`%put` text is emitted as a NOTE event **at the top of the log**, before
program statements run. The reference system interleaves macro execution with program
execution. For ad-hoc debugging this difference is rarely material; document
the value of intermediate variables with `%put` after the relevant `%let`s.

---

## Reporting a divergence

Open an issue with: the source program, what the reference system produces, what PAS produces,
and a one-sentence claim about which behavior should win. Include `cargo
--version` and the PAS version from the status bar.
