# PAS vs SAS — Known Divergences

This document enumerates every place PAS v1.0 behaves differently from the SAS
language reference. Each entry says **what SAS does**, **what PAS does**, and
**why** — so we can decide later whether to close the gap or codify it as
intentional.

Out-of-scope items (`.sas7bdat`, statistical procedures, macro `%macro`/`%if`,
`PROC IMPORT`, etc.) are not divergences — they are not implemented at all and
are listed in §1.2 of `SPEC.md`.

---

## 1. Language

### 1.1 Macro language is preprocessor-only

| | |
|---|---|
| **SAS** | Full programmable macro language with `%macro/%mend`, `%if/%then/%else`, `%do`, `%sysfunc`, etc. Macros can generate arbitrary code. |
| **PAS** | Only `%let`, `%put`, and `&var` (with `.` terminator) substitution. Unrecognized `%` directives pass through and are reported by the downstream parser. |
| **Why** | Full macro is a large surface; spec defers it to v2. |

### 1.2 Format token shape in expressions

| | |
|---|---|
| **SAS** | Formats appear bare in expressions: `put(x, date9.)`, `input(s, comma8.2)`. |
| **PAS** | Format must be a quoted string: `put(x, 'date9.')`, `input(s, 'comma8.2')`. The `format` statement, however, accepts bare format names. |
| **Why** | The lexer turns `date9.` into surprising tokens (`Ident("date9") Dot`); a context-sensitive re-lex would be brittle. Requiring quotes is unambiguous and easy to fix later. |

### 1.3 DATA step set + by interleave

| | |
|---|---|
| **SAS** | `set a b c; by id;` interleaves rows from all sources in by-order. |
| **PAS** | `set` with multiple sources concatenates. `by` with multi-source `set` is not yet supported. |
| **Why** | Less common; deferred. `merge ...; by ...;` is supported. |

### 1.4 Special missing values (.a–.z, ._)

| | |
|---|---|
| **SAS** | 28 distinct numeric missing values, ordered between `._ < . < .a < .b < ... < .z`. |
| **PAS** | Only the standard numeric missing (NaN). Special missing values are not parsed and not represented. |
| **Why** | Listed as an open question in `SPEC.md` §16. |

### 1.5 Variable lengths are advisory

| | |
|---|---|
| **SAS** | `length` declarations are enforced — character vars are space-padded to width, numeric widths affect precision. |
| **PAS** | `length` registers the type (char vs numeric) and PDV order, but width is not enforced. Numeric values are always `f64`. Character values are always `String` (UTF-8) with no padding. |
| **Why** | Width-correct character storage adds complexity for very little wrangling benefit; revisit if a real use case appears. |

### 1.6 Datalines and infile column inputs

| | |
|---|---|
| **SAS** | Supports column-position input (`input name $1-10 age 11-13;`), pointer controls (`@col`), and modified-list input (`name : $20.`). |
| **PAS** | Only free-form, whitespace- (datalines) or delimiter- (infile) separated input. Each variable consumes one token. |
| **Why** | Free-form covers the modern CSV / TSV use case. Column-position input is most relevant to fixed-width legacy files; deferred. |

### 1.7 DO loop variants

| | |
|---|---|
| **SAS** | `do while(<expr>)`, `do until(<expr>)`, and `do i = 1, 3, 5;` (list form) are supported. |
| **PAS** | Only `do; ... end;` (block) and `do var = a to b [by c]; ... end;` (iterative). |
| **Why** | Trivial to add; not yet implemented. |

### 1.8 PROC step coverage

| | |
|---|---|
| **SAS** | Many PROCs: `PROC SORT`, `PROC TRANSPOSE`, `PROC FREQ`, `PROC PRINT`, `PROC MEANS`, `PROC IMPORT`, etc. |
| **PAS** | Only `PROC SQL` (passed through to DuckDB). All others raise an error. |
| **Why** | Data wrangling fits inside DATA step + PROC SQL. `PROC SORT` is the most common gap and is planned for v2. |

---

## 2. Execution model

### 2.1 In-memory materialization in DATA step

| | |
|---|---|
| **SAS** | DATA step streams rows one at a time. Output is appended without holding all rows in memory. |
| **PAS** | DATA step currently materializes all input rows (`Vec<SourceRow>`) and all output rows before writing. Memory usage scales with row count. |
| **Why** | Iteration semantics are easier to reason about when materialized. A streaming refactor is the headline item for v1.x. |

### 2.2 PROC SQL extensions

| | |
|---|---|
| **SAS** | PROC SQL with `calculated`, `outer union [corr]`, three-part names (`libref.table`), automatic remerging, `monotonic()`, truncated comparisons (`eqt`, `gtt`, `lt:` colon-modifier). |
| **PAS** | DuckDB-backed PROC SQL with a token-aware rewriter (`crates/pas-engine/src/sas_sql.rs`) handling: `calculated <col>` → `<col>`; `monotonic()` → `row_number() over ()`; `outer union corr` → `union all by name`; `outer union` → `union all`; `CREATE TABLE` → `CREATE OR REPLACE TABLE`; and `CREATE TABLE libref.ds AS …` for DIR libraries → `COPY (…) TO 'path' (FORMAT …)`. Three-part names are resolved by the libref rewriter. |
| **Not yet** | Automatic remerging (e.g. `select id, max(score) from t` without GROUP BY) and truncated comparisons (`eqt`/`gtt`). Both require fuller query parsing than a token-aware pass. |

### 2.3 CREATE TABLE overwrite

| | |
|---|---|
| **SAS** | `create table t as ...;` errors if `t` exists. Must `drop table t;` first. |
| **PAS** | `create table t as ...;` always overwrites (translated to `CREATE OR REPLACE TABLE` for DuckDB). |
| **Why** | Matches SAS PROC SQL `outobs=` / re-run ergonomics users actually expect; explicit `drop` is annoying in an interactive editor. |

### 2.4 Cancellation granularity

| | |
|---|---|
| **SAS** | "Cancel" usually halts immediately. |
| **PAS** | Cancel is cooperative — interrupts the running DuckDB query via `Connection::interrupt`, and the engine's per-statement and per-iteration cancel-flag checks stop the next operation. A `for` body that is purely Rust (no DuckDB calls) cancels at the next row boundary. |
| **Why** | Sufficient for interactive use. |

### 2.5 Sort stability for `by`

| | |
|---|---|
| **SAS** | `by` requires pre-sorted input (or you use `PROC SORT`); ordering within ties is unspecified. |
| **PAS** | Auto-sorts input via DuckDB `ORDER BY <by-vars>` before iterating; DuckDB sorts are not guaranteed stable. |
| **Why** | Convenience for `set ... ; by ...;` users. Logged as an open question. |

---

## 3. I/O and formats

### 3.1 Formats supported

PAS implements a useful subset of SAS formats. Numeric: `best.`, `8.`, `8.2`,
`comma.`/`comma8.2`, `date9.`, `mmddyy10.`, `ddmmyy10.`, `yymmdd10.`, `time8.`,
`datetime19.`. Character: `$char20.`, `$upcase.`, `$lowcase.`. Anything else
errors at `put`/`input` time.

### 3.2 No SAS7BDAT interop

PAS does not read or write `.sas7bdat`. Convert via SAS itself, or use a tool
like Stata's `usespss`, R's `haven`, or Python's `pyreadstat` to get to
Parquet/CSV first.

### 3.3 No encoding= on infile

| | |
|---|---|
| **SAS** | `infile '...' encoding='latin1';` transcodes on the fly. |
| **PAS** | Files are read as UTF-8 (Rust `read_to_string`); non-UTF-8 input errors. |
| **Why** | `encoding_rs` integration is straightforward but not yet plumbed through. |

---

## 4. UI / project

### 4.1 Single window, single session

PAS runs one engine session per process. SAS supports multiple servers / SAS
sessions. Not planned for v1.x.

### 4.2 No project autosave

Open tabs are saved when the user saves the project. There is no autosave; an
unsaved buffer is lost if the app is closed.

### 4.3 No log persistence

The log clears on each new submission. SAS Enterprise Guide writes per-program
logs to disk; PAS only shows the most recent log in the bottom pane.

---

## 5. Defaults and quirks

### 5.1 Library `WORK` is always present and in-memory

PAS uses an in-memory DuckDB for `WORK`. Closing the app loses all `WORK`
tables. Persist with a `libname` to DUCKDB or DIR.

### 5.2 Identifier case

PAS lowercases identifiers internally (matching SAS's case-insensitive
behavior). Column / dataset names are displayed lowercase except in log
notes, where dataset references are uppercased (`NOTE: The data set WORK.X has
N observations.`).

### 5.3 `%put` ordering

`%put` text is emitted as a NOTE event **at the top of the log**, before
program statements run. Real SAS interleaves macro execution with program
execution. For ad-hoc debugging this difference is rarely material; document
the value of intermediate variables with `%put` after the relevant `%let`s.

---

## Reporting a divergence

Open an issue with: the SAS program, what SAS produces, what PAS produces,
and a one-sentence claim about which behavior should win. Include `cargo
--version` and the PAS version from the status bar.
