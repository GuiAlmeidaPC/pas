//! PAS engine — v0.2.
//!
//! Adds `libname` support (DUCKDB attach + DIR), library/dataset listing
//! commands, and paginated dataset reads.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use duckdb::{Connection, InterruptHandle};
use serde::Serialize;
use thiserror::Error;

mod datastep;
mod libname;
mod library;
mod macros;
mod procs;
mod sas_sql;
mod split;

pub use library::{ColumnInfo, DatasetInfo, DirFormat, Library, LibraryKind};
pub use split::{extract_sql_statements, split_blocks, strip_comments, Block};

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("duckdb: {0}")]
    DuckDb(#[from] duckdb::Error),
    #[error("libname: {0}")]
    Libname(#[from] libname::LibnameError),
    #[error("data step: {0}")]
    DataStep(#[from] datastep::DataStepError),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Source { text: String },
    Note { text: String },
    Warning { text: String },
    Error { text: String, source_span: Option<SourceSpan> },
    Output { block: ResultBlock },
    Done,
}

/// 1-based line/column range in the submitted program (after macro
/// expansion). `start` and `end` mark the offending token.
#[derive(Debug, Clone, Serialize)]
pub struct SourceSpan {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResultBlock {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Column {
    pub name: String,
    pub ty: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

/// Cap on preview rows in an Output event (paginated viewer fetches more).
const MAX_PREVIEW_ROWS: usize = 1000;

pub struct Session {
    conn: Mutex<Connection>,
    cancel: Arc<AtomicBool>,
    interrupt: Arc<InterruptHandle>,
    libraries: Mutex<HashMap<String, Library>>,
    macro_vars: Mutex<HashMap<String, String>>,
}

impl Session {
    pub fn new_in_memory() -> Result<Self, EngineError> {
        let conn = Connection::open_in_memory()?;
        let interrupt = conn.interrupt_handle();
        let mut libs = HashMap::new();
        // WORK is always present and points at the default in-memory schema.
        libs.insert(
            "work".to_string(),
            Library { name: "work".to_string(), kind: LibraryKind::Memory, path: String::new(), format: None },
        );
        Ok(Self {
            conn: Mutex::new(conn),
            cancel: Arc::new(AtomicBool::new(false)),
            interrupt,
            libraries: Mutex::new(libs),
            macro_vars: Mutex::new(HashMap::new()),
        })
    }

    /// Stop any in-flight submission. Interrupts the running DuckDB query
    /// (if any) and also sets a cooperative flag so the engine bails out
    /// before starting the next statement.
    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.interrupt.interrupt();
    }

    pub fn list_libraries(&self) -> Vec<Library> {
        let mut v: Vec<Library> =
            self.libraries.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn list_datasets(&self, libref: &str) -> Result<Vec<DatasetInfo>, EngineError> {
        let lib = self.lookup_library(libref)?;
        let conn = self.conn.lock().unwrap();
        match lib.kind {
            LibraryKind::Memory => list_schema_tables(&conn, "main", &lib.name),
            LibraryKind::Duckdb => list_schema_tables(&conn, &lib.name, &lib.name),
            LibraryKind::Dir => list_dir_datasets(&lib),
        }
    }

    pub fn dataset_schema(
        &self,
        libref: &str,
        name: &str,
    ) -> Result<Vec<ColumnInfo>, EngineError> {
        let lib = self.lookup_library(libref)?;
        let from_clause = dataset_from_clause(&lib, name)?;
        let conn = self.conn.lock().unwrap();
        let sql = format!("SELECT * FROM {} LIMIT 0", from_clause);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query([])?;
        let col_count = rows.as_ref().map(|s| s.column_count()).unwrap_or(0);
        let cols = (0..col_count)
            .map(|i| {
                let name = rows
                    .as_ref()
                    .and_then(|s| s.column_name(i).ok())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| format!("col{}", i));
                ColumnInfo { name, ty: "?".to_string() }
            })
            .collect();
        Ok(cols)
    }

    /// Fetch a row page encoded as an Arrow IPC stream. The schema carries
    /// `total_rows` and `offset` in its metadata so the UI can size the
    /// scrollbar without a separate round-trip.
    pub fn dataset_page_arrow(
        &self,
        libref: &str,
        name: &str,
        offset: u64,
        limit: u64,
        filters: Option<&HashMap<String, String>>,
    ) -> Result<Vec<u8>, EngineError> {
        use arrow::array::RecordBatch;
        use arrow::datatypes::Schema;
        use arrow::ipc::writer::StreamWriter;
        use std::sync::Arc;

        let lib = self.lookup_library(libref)?;
        let from_clause = dataset_from_clause(&lib, name)?;
        let where_sql = build_where_clause(filters);
        let conn = self.conn.lock().unwrap();

        let total: u64 = {
            let count_sql = format!("SELECT count(*) FROM {}{}", from_clause, where_sql);
            let mut stmt = conn.prepare(&count_sql)?;
            let mut rows = stmt.query([])?;
            match rows.next()? {
                Some(r) => r.get::<_, i64>(0).unwrap_or(0).max(0) as u64,
                None => 0,
            }
        };

        let sql = format!(
            "SELECT * FROM {}{} LIMIT {} OFFSET {}",
            from_clause, where_sql, limit, offset
        );
        let mut stmt = conn.prepare(&sql)?;
        let arrow_iter = stmt.query_arrow([])?;
        let base_schema = arrow_iter.get_schema();
        let mut md = base_schema.metadata().clone();
        md.insert("total_rows".to_string(), total.to_string());
        md.insert("offset".to_string(), offset.to_string());
        let schema = Arc::new(Schema::new_with_metadata(
            base_schema.fields().clone(),
            md,
        ));

        let mut buf: Vec<u8> = Vec::new();
        {
            let mut writer = StreamWriter::try_new(&mut buf, &schema)
                .map_err(|e| EngineError::Other(format!("arrow writer: {}", e)))?;
            for batch in arrow_iter {
                let rebatched = RecordBatch::try_new(schema.clone(), batch.columns().to_vec())
                    .map_err(|e| EngineError::Other(format!("arrow batch: {}", e)))?;
                writer
                    .write(&rebatched)
                    .map_err(|e| EngineError::Other(format!("arrow write: {}", e)))?;
            }
            writer
                .finish()
                .map_err(|e| EngineError::Other(format!("arrow finish: {}", e)))?;
        }
        Ok(buf)
    }

    /// Fetch a row page from a dataset. Both rows and total row count are
    /// returned so the UI can size the scrollbar.
    pub fn dataset_page(
        &self,
        libref: &str,
        name: &str,
        offset: u64,
        limit: u64,
        filters: Option<&HashMap<String, String>>,
    ) -> Result<DatasetPage, EngineError> {
        let lib = self.lookup_library(libref)?;
        let from_clause = dataset_from_clause(&lib, name)?;
        let where_sql = build_where_clause(filters);
        let conn = self.conn.lock().unwrap();

        let total: u64 = {
            let count_sql = format!("SELECT count(*) FROM {}{}", from_clause, where_sql);
            let mut stmt = conn.prepare(&count_sql)?;
            let mut rows = stmt.query([])?;
            match rows.next()? {
                Some(r) => r.get::<_, i64>(0).unwrap_or(0).max(0) as u64,
                None => 0,
            }
        };

        let sql = format!(
            "SELECT * FROM {}{} LIMIT {} OFFSET {}",
            from_clause, where_sql, limit, offset
        );
        let block = match run_query(&conn, &sql, limit as usize)? {
            StmtResult::Rows(b) => b,
            _ => ResultBlock { columns: vec![], rows: vec![], truncated: false },
        };
        Ok(DatasetPage { columns: block.columns, rows: block.rows, total_rows: total })
    }

    /// Run an entire program, returning all events.
    pub fn submit(&self, program: &str) -> Vec<Event> {
        self.cancel.store(false, Ordering::SeqCst);
        let cleaned = strip_comments(program);

        // Macro pre-pass: `%let` / `%put` / `&var`. Macro variables persist
        // across submissions on the same session.
        let macro_result = {
            let mut vars = self.macro_vars.lock().unwrap();
            macros::preprocess(&cleaned, &mut vars)
        };
        let mut events = Vec::new();
        for text in macro_result.puts {
            events.push(Event::Note { text });
        }

        let blocks = split_blocks(&macro_result.expanded);

        if blocks.is_empty() {
            // Only complain about an empty program if there was no macro
            // activity either — a `%let` alone is legitimately empty after
            // pre-processing.
            if events.is_empty() {
                events.push(Event::Note { text: "No statements found.".into() });
            }
            events.push(Event::Done);
            return events;
        }

        let conn = self.conn.lock().expect("engine mutex poisoned");
        for block in blocks {
            if self.cancel.load(Ordering::SeqCst) {
                events.push(Event::Warning { text: "Execution cancelled by user.".into() });
                break;
            }
            match block {
                Block::Statement { text, src_offset } => {
                    events.push(Event::Source { text: text.clone() });
                    if let Some(handled) = self.try_libname(&conn, &text) {
                        events.extend(handled);
                        continue;
                    }
                    self.run_sql_with_rewrites(
                        &conn,
                        &text,
                        src_offset,
                        &macro_result.expanded,
                        &mut events,
                    );
                }
                Block::ProcSqlStmt { text, src_offset } => {
                    events.push(Event::Source { text: text.clone() });
                    self.run_sql_with_rewrites(
                        &conn,
                        &text,
                        src_offset,
                        &macro_result.expanded,
                        &mut events,
                    );
                }
                Block::DataStep { body, datalines, body_src_offset } => {
                    events.push(Event::Source { text: body.clone() });
                    self.run_data_step(
                        &conn,
                        &body,
                        datalines,
                        body_src_offset,
                        &macro_result.expanded,
                        &mut events,
                    );
                }
                Block::Proc { name, body, .. } => {
                    events.push(Event::Source { text: format!("proc {}; {} run;", name, body) });
                    self.run_proc(&conn, &name, &body, &mut events);
                }
            }
        }

        events.push(Event::Done);
        events
    }

    fn try_libname(&self, conn: &Connection, stmt: &str) -> Option<Vec<Event>> {
        match libname::parse(stmt) {
            Ok(Some(def)) => Some(match self.apply_libname(conn, &def) {
                Ok(msg) => vec![Event::Note { text: msg }],
                Err(e) => vec![Event::Error { text: e.to_string(), source_span: None }],
            }),
            Ok(None) => None,
            Err(e) => Some(vec![Event::Error { text: e.to_string(), source_span: None }]),
        }
    }

    fn run_sql_with_rewrites(
        &self,
        conn: &Connection,
        stmt: &str,
        src_offset: usize,
        program: &str,
        events: &mut Vec<Event>,
    ) {
        let after_create = self.rewrite_create_for_dir(stmt);
        let rewritten = self.rewrite_librefs(&after_create);
        match run_one(conn, &rewritten) {
            Ok(StmtResult::Rows(block)) => {
                let suffix = if block.truncated {
                    format!(" (showing first {})", block.rows.len())
                } else {
                    String::new()
                };
                events.push(Event::Note {
                    text: format!("Statement returned {} row(s){}.", block.rows.len(), suffix),
                });
                events.push(Event::Output { block });
            }
            Ok(StmtResult::Affected(n)) => events.push(Event::Note {
                text: format!("Statement executed ({} row(s) affected).", n),
            }),
            Ok(StmtResult::Done) => events.push(Event::Note { text: "Statement executed.".into() }),
            Err(e) => {
                let text = e.to_string();
                let source_span = duckdb_error_span(&text, stmt, src_offset, program);
                events.push(Event::Error { text, source_span });
            }
        }
    }

    fn run_proc(&self, conn: &Connection, name: &str, body: &str, events: &mut Vec<Event>) {
        let result = match name {
            "sort" => self.proc_sort(conn, body),
            "print" => self.proc_print(conn, body),
            "transpose" => self.proc_transpose(conn, body),
            other => Err(EngineError::Other(format!(
                "PROC {} is not implemented in PAS",
                other.to_uppercase()
            ))),
        };
        match result {
            Ok(notes) => {
                for n in notes {
                    events.push(n);
                }
            }
            Err(e) => events.push(Event::Error { text: e.to_string(), source_span: None }),
        }
    }

    fn proc_sort(&self, conn: &Connection, body: &str) -> Result<Vec<Event>, EngineError> {
        let spec = procs::sort::parse(body).map_err(EngineError::Other)?;
        let from = self.resolve_read(&spec.data_in)?;
        let target = self.resolve_write(&spec.data_out)?;
        let select_sql = procs::sort::build_select_sql(&from, &spec);
        let rows = materialize_select_into(conn, &target, &select_sql)?;
        Ok(vec![Event::Note {
            text: format!("The data set {} has {} observations.", target.display(), rows),
        }])
    }

    fn proc_print(&self, conn: &Connection, body: &str) -> Result<Vec<Event>, EngineError> {
        let spec = procs::print::parse(body).map_err(EngineError::Other)?;
        let from = self.resolve_read(&spec.data)?;
        let sql = procs::print::build_select_sql(&from, &spec);
        match run_query(conn, &sql, MAX_PREVIEW_ROWS)? {
            StmtResult::Rows(block) => Ok(vec![
                Event::Note {
                    text: format!("PROC PRINT showing {} row(s).", block.rows.len()),
                },
                Event::Output { block },
            ]),
            _ => Ok(vec![]),
        }
    }

    fn proc_transpose(&self, conn: &Connection, body: &str) -> Result<Vec<Event>, EngineError> {
        let spec = procs::transpose::parse(body).map_err(EngineError::Other)?;
        let from = self.resolve_read(&spec.data_in)?;
        let target = self.resolve_write(&spec.data_out)?;
        let select_sql = procs::transpose::build_select_sql(&from, &spec);
        let rows = materialize_select_into(conn, &target, &select_sql)?;
        Ok(vec![Event::Note {
            text: format!("The data set {} has {} observations.", target.display(), rows),
        }])
    }

    fn run_data_step(
        &self,
        conn: &Connection,
        body: &str,
        datalines: Vec<String>,
        body_src_offset: usize,
        program: &str,
        events: &mut Vec<Event>,
    ) {
        let ds = match datastep::parse::parse_data_step_with_datalines(body, datalines) {
            Ok(ds) => ds,
            Err(e) => {
                let abs_start = body_src_offset + e.span.start;
                let abs_end = body_src_offset + e.span.end.max(e.span.start);
                let (sl, sc) = split::byte_to_line_col(program, abs_start);
                let (el, ec) = split::byte_to_line_col(program, abs_end);
                events.push(Event::Error {
                    text: format!("data step parse: {}", e),
                    source_span: Some(SourceSpan {
                        start_line: sl,
                        start_col: sc,
                        end_line: el,
                        end_col: ec.max(sc + 1),
                    }),
                });
                return;
            }
        };
        // Resolve input FROM-expressions.
        let input = match ds.input.as_ref() {
            None => None,
            Some(datastep::ast::DataInput::Set(tables)) => {
                match tables.iter().map(|t| self.resolve_read(t)).collect::<Result<Vec<_>, _>>() {
                    Ok(v) => Some(datastep::exec::ResolvedInput::Set(v)),
                    Err(e) => {
                        events.push(Event::Error { text: e.to_string(), source_span: None });
                        return;
                    }
                }
            }
            Some(datastep::ast::DataInput::Merge(tables)) => {
                match tables.iter().map(|t| self.resolve_read(t)).collect::<Result<Vec<_>, _>>() {
                    Ok(v) => Some(datastep::exec::ResolvedInput::Merge(v)),
                    Err(e) => {
                        events.push(Event::Error { text: e.to_string(), source_span: None });
                        return;
                    }
                }
            }
        };
        let mut outputs = Vec::with_capacity(ds.outputs.len());
        for t in &ds.outputs {
            match self.resolve_write(t) {
                Ok(w) => outputs.push(w),
                Err(e) => {
                    events.push(Event::Error { text: e.to_string(), source_span: None });
                    return;
                }
            }
        }
        let plan = datastep::exec::ResolvedDataStep { ast: &ds, input, outputs };

        match datastep::run_data_step(conn, &plan, &self.cancel) {
            Ok(res) => {
                for (_, target, rows) in &res.outputs {
                    events.push(Event::Note {
                        text: format!("The data set {} has {} observations.", target.display(), rows),
                    });
                }
                events.push(Event::Note {
                    text: format!("DATA statement read {} observation(s).", res.rows_in),
                });
            }
            Err(e) => {
                // Runtime errors may carry a span pointing at the offending
                // expression in the body. Convert to absolute source span.
                let source_span = match &e {
                    datastep::DataStepError::Runtime(_, Some(s)) => {
                        let abs_start = body_src_offset + s.start;
                        let abs_end = body_src_offset + s.end.max(s.start);
                        let (sl, sc) = split::byte_to_line_col(program, abs_start);
                        let (el, ec) = split::byte_to_line_col(program, abs_end);
                        Some(SourceSpan {
                            start_line: sl,
                            start_col: sc,
                            end_line: el,
                            end_col: ec.max(sc + 1),
                        })
                    }
                    _ => None,
                };
                events.push(Event::Error { text: e.to_string(), source_span });
            }
        }
    }

    /// Resolve a `libref.dataset` reference (or unqualified) to the SQL
    /// FROM expression DuckDB should use.
    fn resolve_read(&self, t: &datastep::ast::TableRef) -> Result<String, EngineError> {
        match &t.libref {
            None => Ok(format!("\"main\".\"{}\"", t.name)),
            Some(l) => {
                let lib = self.lookup_library(l)?;
                match lib.kind {
                    LibraryKind::Memory => Ok(format!("\"main\".\"{}\"", t.name)),
                    LibraryKind::Duckdb => Ok(format!("\"{}\".\"{}\"", lib.name, t.name)),
                    LibraryKind::Dir => Ok(dir_reader_expr(&lib, &t.name)),
                }
            }
        }
    }

    fn resolve_write(
        &self,
        t: &datastep::ast::TableRef,
    ) -> Result<datastep::exec::WriteTarget, EngineError> {
        use datastep::exec::WriteTarget;
        match &t.libref {
            None => Ok(WriteTarget::DuckDb { schema: "main".into(), name: t.name.clone() }),
            Some(l) => {
                let lib = self.lookup_library(l)?;
                match lib.kind {
                    LibraryKind::Memory => Ok(WriteTarget::DuckDb {
                        schema: "main".into(),
                        name: t.name.clone(),
                    }),
                    LibraryKind::Duckdb => Ok(WriteTarget::DuckDb {
                        schema: lib.name.clone(),
                        name: t.name.clone(),
                    }),
                    LibraryKind::Dir => {
                        let fmt = lib.format.unwrap_or(DirFormat::Parquet);
                        let path = format!(
                            "{}/{}.{}",
                            lib.path.trim_end_matches('/'),
                            t.name,
                            fmt.extension()
                        );
                        let display = format!(
                            "{}.{}",
                            lib.name.to_uppercase(),
                            t.name.to_uppercase()
                        );
                        Ok(match fmt {
                            DirFormat::Parquet => WriteTarget::Parquet { path, display },
                            DirFormat::Csv => WriteTarget::Csv { path, display },
                        })
                    }
                }
            }
        }
    }

    fn lookup_library(&self, libref: &str) -> Result<Library, EngineError> {
        self.libraries
            .lock()
            .unwrap()
            .get(&libref.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| EngineError::Other(format!("library '{}' not assigned", libref)))
    }

    fn apply_libname(
        &self,
        conn: &Connection,
        def: &libname::LibnameDef,
    ) -> Result<String, EngineError> {
        match def.kind {
            LibraryKind::Memory => {}
            LibraryKind::Duckdb => {
                let sql = format!(
                    "ATTACH IF NOT EXISTS '{}' AS \"{}\"",
                    def.path.replace('\'', "''"),
                    def.name
                );
                conn.execute(&sql, [])?;
            }
            LibraryKind::Dir => {
                let p = Path::new(&def.path);
                if !p.exists() {
                    return Err(EngineError::Other(format!(
                        "path does not exist: {}",
                        def.path
                    )));
                }
                if !p.is_dir() {
                    return Err(EngineError::Other(format!(
                        "not a directory: {}",
                        def.path
                    )));
                }
            }
        }
        let lib = Library {
            name: def.name.clone(),
            kind: def.kind,
            path: def.path.clone(),
            format: def.format,
        };
        self.libraries.lock().unwrap().insert(def.name.clone(), lib);
        Ok(format!(
            "Library {} assigned as {:?}{}.",
            def.name.to_uppercase(),
            def.kind,
            if def.path.is_empty() { String::new() } else { format!(" → {}", def.path) }
        ))
    }

    /// If `sql` is `CREATE [OR REPLACE] TABLE <libref>.<ds> AS <body>` and
    /// `<libref>` is a DIR library, rewrite the whole statement into a
    /// `COPY (<body>) TO '<path>/<ds>.<ext>' (FORMAT <ext>)`. Otherwise
    /// returns the input unchanged.
    fn rewrite_create_for_dir(&self, sql: &str) -> String {
        let trimmed_start = sql.trim_start();
        let leading_ws = &sql[..sql.len() - trimmed_start.len()];
        let lower = trimmed_start.to_ascii_lowercase();

        let prefix_len = if lower.starts_with("create or replace table") {
            "create or replace table".len()
        } else if lower.starts_with("create table") {
            "create table".len()
        } else {
            return sql.to_string();
        };

        let after_prefix = &trimmed_start[prefix_len..];
        let after_prefix_trim = after_prefix.trim_start();

        // Parse `<libref>.<ds>` (identifier . identifier).
        let mut end_lib = 0;
        for (i, c) in after_prefix_trim.char_indices() {
            if c.is_ascii_alphanumeric() || c == '_' {
                end_lib = i + c.len_utf8();
            } else {
                break;
            }
        }
        if end_lib == 0 {
            return sql.to_string();
        }
        let libref = &after_prefix_trim[..end_lib];
        let after_lib = &after_prefix_trim[end_lib..];
        if !after_lib.starts_with('.') {
            return sql.to_string();
        }
        let after_dot = &after_lib[1..];
        let mut end_ds = 0;
        for (i, c) in after_dot.char_indices() {
            if c.is_ascii_alphanumeric() || c == '_' {
                end_ds = i + c.len_utf8();
            } else {
                break;
            }
        }
        if end_ds == 0 {
            return sql.to_string();
        }
        let dataset = &after_dot[..end_ds];
        let after_ds = after_dot[end_ds..].trim_start();

        // Need an `as` keyword.
        let lower_after = after_ds.to_ascii_lowercase();
        let body = if let Some(rest) = lower_after.strip_prefix("as ") {
            let consumed = after_ds.len() - rest.len();
            after_ds[consumed..].trim()
        } else if let Some(rest) = lower_after.strip_prefix("as\t") {
            let consumed = after_ds.len() - rest.len();
            after_ds[consumed..].trim()
        } else if lower_after.starts_with("as\n") {
            after_ds[3..].trim()
        } else {
            return sql.to_string();
        };

        let lib = {
            let libs = self.libraries.lock().unwrap();
            match libs.get(&libref.to_ascii_lowercase()) {
                Some(l) if l.kind == LibraryKind::Dir => l.clone(),
                _ => return sql.to_string(),
            }
        };

        let fmt = lib.format.unwrap_or(DirFormat::Parquet);
        let ext = fmt.extension();
        let fmt_name = match fmt {
            DirFormat::Parquet => "PARQUET",
            DirFormat::Csv => "CSV",
        };
        let path = format!("{}/{}.{}", lib.path.trim_end_matches('/'), dataset, ext);
        format!(
            "{}COPY ({}) TO '{}' (FORMAT {})",
            leading_ws,
            body,
            path.replace('\'', "''"),
            fmt_name
        )
    }

    /// Replace `libref.dataset` tokens with the actual reader expression
    /// for DIR libraries. DUCKDB libraries already resolve via attached
    /// schemas; MEMORY refs use the default schema.
    fn rewrite_librefs(&self, sql: &str) -> String {
        let libs = self.libraries.lock().unwrap();
        let dir_libs: Vec<Library> = libs
            .values()
            .filter(|l| l.kind == LibraryKind::Dir)
            .cloned()
            .collect();
        drop(libs);
        if dir_libs.is_empty() {
            return sql.to_string();
        }
        let mut out = String::with_capacity(sql.len());
        let bytes = sql.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            // Skip string literals.
            if c == b'\'' || c == b'"' {
                let q = c;
                out.push(c as char);
                i += 1;
                while i < bytes.len() {
                    let b = bytes[i];
                    out.push(b as char);
                    i += 1;
                    if b == q { break; }
                }
                continue;
            }
            if c.is_ascii_alphabetic() || c == b'_' {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                {
                    i += 1;
                }
                let ident = &sql[start..i];
                // libref.dataset?
                if i < bytes.len() && bytes[i] == b'.' {
                    let after_dot = i + 1;
                    let mut j = after_dot;
                    while j < bytes.len()
                        && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_')
                    {
                        j += 1;
                    }
                    if j > after_dot {
                        let ds = &sql[after_dot..j];
                        if let Some(lib) = dir_libs
                            .iter()
                            .find(|l| l.name.eq_ignore_ascii_case(ident))
                        {
                            let reader = dir_reader_expr(lib, ds);
                            out.push_str(&reader);
                            i = j;
                            continue;
                        }
                    }
                }
                out.push_str(ident);
                continue;
            }
            out.push(c as char);
            i += 1;
        }
        out
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetPage {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    pub total_rows: u64,
}

/// Build a `WHERE col1 LIKE '%v%' AND col2 LIKE '%w%'` clause for the
/// dataset viewer. Empty values and missing maps return an empty string.
fn build_where_clause(filters: Option<&HashMap<String, String>>) -> String {
    let Some(map) = filters else { return String::new(); };
    let mut parts: Vec<String> = map
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| {
            let needle = v.replace('\'', "''");
            format!("CAST(\"{}\" AS VARCHAR) ILIKE '%{}%'", k.replace('"', ""), needle)
        })
        .collect();
    if parts.is_empty() { return String::new(); }
    parts.sort();
    format!(" WHERE {}", parts.join(" AND "))
}

fn dir_reader_expr(lib: &Library, dataset: &str) -> String {
    let fmt = lib.format.unwrap_or(DirFormat::Parquet);
    let path = format!("{}/{}.{}", lib.path.trim_end_matches('/'), dataset, fmt.extension());
    let escaped = path.replace('\'', "''");
    match fmt {
        DirFormat::Parquet => format!("read_parquet('{}')", escaped),
        DirFormat::Csv => format!("read_csv_auto('{}')", escaped),
    }
}

fn dataset_from_clause(lib: &Library, dataset: &str) -> Result<String, EngineError> {
    Ok(match lib.kind {
        LibraryKind::Memory => format!("\"main\".\"{}\"", dataset),
        LibraryKind::Duckdb => format!("\"{}\".\"{}\"", lib.name, dataset),
        LibraryKind::Dir => dir_reader_expr(lib, dataset),
    })
}

fn list_schema_tables(
    conn: &Connection,
    schema: &str,
    libref: &str,
) -> Result<Vec<DatasetInfo>, EngineError> {
    let sql = "SELECT table_name FROM information_schema.tables \
               WHERE table_schema = ? ORDER BY table_name";
    let mut stmt = conn.prepare(sql)?;
    let names: Vec<String> = stmt
        .query_map([schema], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(names
        .into_iter()
        .map(|name| DatasetInfo { libref: libref.to_string(), name, rows: None })
        .collect())
}

fn list_dir_datasets(lib: &Library) -> Result<Vec<DatasetInfo>, EngineError> {
    let fmt = lib.format.unwrap_or(DirFormat::Parquet);
    let ext = fmt.extension();
    let dir = std::fs::read_dir(&lib.path)
        .map_err(|e| EngineError::Other(format!("read_dir: {}", e)))?;
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case(ext))
            == Some(true)
        {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                out.push(DatasetInfo {
                    libref: lib.name.clone(),
                    name: stem.to_string(),
                    rows: None,
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

enum StmtResult {
    Rows(ResultBlock),
    Affected(usize),
    Done,
}

fn run_one(conn: &Connection, sql: &str) -> Result<StmtResult, EngineError> {
    let trimmed = sas_sql::rewrite(sql.trim());
    let trimmed = trimmed.as_str();
    if trimmed.is_empty() {
        return Ok(StmtResult::Done);
    }
    if is_query(trimmed) {
        run_query(conn, trimmed, MAX_PREVIEW_ROWS)
    } else {
        let mut stmt = conn.prepare(trimmed)?;
        let n = stmt.execute([])?;
        Ok(StmtResult::Affected(n))
    }
}

fn run_query(conn: &Connection, sql: &str, max_rows: usize) -> Result<StmtResult, EngineError> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows_iter = stmt.query([])?;

    let col_count = rows_iter.as_ref().map(|s| s.column_count()).unwrap_or(0);
    let col_names: Vec<String> = (0..col_count)
        .map(|i| {
            rows_iter
                .as_ref()
                .and_then(|s| s.column_name(i).ok())
                .map(|n| n.to_string())
                .unwrap_or_else(|| format!("col{}", i))
        })
        .collect();

    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut truncated = false;
    let mut col_types: Vec<String> = vec![String::new(); col_count];
    let mut types_filled = false;

    while let Some(row) = rows_iter.next()? {
        if rows.len() >= max_rows {
            truncated = true;
            break;
        }
        let mut vals = Vec::with_capacity(col_count);
        for i in 0..col_count {
            let v: duckdb::types::Value = row.get(i)?;
            if !types_filled {
                col_types[i] = type_name(&v).to_string();
            }
            vals.push(value_from_duckdb(v));
        }
        types_filled = true;
        rows.push(vals);
    }

    let columns = col_names
        .into_iter()
        .zip(col_types)
        .map(|(name, ty)| Column { name, ty: if ty.is_empty() { "?".into() } else { ty } })
        .collect();

    Ok(StmtResult::Rows(ResultBlock { columns, rows, truncated }))
}

/// Run `select_sql` and route the result rows into `target`. Returns the
/// number of rows written. Used by PROC SORT and PROC TRANSPOSE; shaped to
/// match the data-step writer's contract (CREATE OR REPLACE for DuckDB
/// targets, COPY (...) TO 'path' for DIR libraries).
fn materialize_select_into(
    conn: &Connection,
    target: &datastep::exec::WriteTarget,
    select_sql: &str,
) -> Result<u64, EngineError> {
    use datastep::exec::WriteTarget;
    match target {
        WriteTarget::DuckDb { schema, name } => {
            let qualified = format!("\"{}\".\"{}\"", schema, name);
            let create = format!("CREATE OR REPLACE TABLE {} AS {}", qualified, select_sql);
            conn.execute(&create, [])?;
            let count_sql = format!("SELECT count(*) FROM {}", qualified);
            let mut stmt = conn.prepare(&count_sql)?;
            let mut rows = stmt.query([])?;
            Ok(match rows.next()? {
                Some(r) => r.get::<_, i64>(0).unwrap_or(0).max(0) as u64,
                None => 0,
            })
        }
        WriteTarget::Parquet { path, .. } | WriteTarget::Csv { path, .. } => {
            let fmt = match target {
                WriteTarget::Parquet { .. } => "PARQUET",
                WriteTarget::Csv { .. } => "CSV",
                _ => unreachable!(),
            };
            // Stage into a temp table so we can count rows before writing.
            let temp = format!("pas_proc_tmp_{}", uuid::Uuid::new_v4().simple());
            let qualified = format!("\"main\".\"{}\"", temp);
            let create = format!("CREATE OR REPLACE TABLE {} AS {}", qualified, select_sql);
            conn.execute(&create, [])?;
            let count_sql = format!("SELECT count(*) FROM {}", qualified);
            let mut stmt = conn.prepare(&count_sql)?;
            let mut rows = stmt.query([])?;
            let n = match rows.next()? {
                Some(r) => r.get::<_, i64>(0).unwrap_or(0).max(0) as u64,
                None => 0,
            };
            drop(stmt);
            let copy = format!(
                "COPY (SELECT * FROM {}) TO '{}' (FORMAT {})",
                qualified,
                path.replace('\'', "''"),
                fmt
            );
            conn.execute(&copy, [])?;
            conn.execute(&format!("DROP TABLE {}", qualified), [])?;
            Ok(n)
        }
    }
}

/// Parse the `LINE N: ... ^` pointer that DuckDB embeds in parser /
/// catalog errors into an absolute source span in `program`.
///
/// The format we look for:
///
/// ```text
/// LINE 1: SELECT * FRM users
///                  ^
/// ```
///
/// Returns `None` if the marker isn't present (e.g. runtime error with
/// no position info).
fn duckdb_error_span(
    err: &str,
    stmt: &str,
    src_offset: usize,
    program: &str,
) -> Option<SourceSpan> {
    // Find the LINE N marker.
    let prefix = "LINE ";
    let line_idx = err.find(prefix)?;
    let after = &err[line_idx + prefix.len()..];
    let colon = after.find(':')?;
    let line_no: u32 = after[..colon].trim().parse().ok()?;
    let after_colon = &after[colon + 1..];
    // The remainder of that line is the source fragment DuckDB echoed
    // (starting with a single space). The line below it is the `^`
    // pointer line.
    let mut lines = after_colon.lines();
    let _echo_line = lines.next()?;
    let caret_line = lines.next()?;
    // Column = position of `^` in the caret line (1-based, byte-counted).
    // The echoed source line lives in the original source verbatim, so the
    // caret position lines up with that source's character offset.
    let col = caret_line.find('^').map(|p| p as u32 + 1)?;

    // Convert (line_no, col) in `stmt` to an absolute byte offset inside
    // `stmt`, then add `src_offset` and convert back to program (line, col).
    let mut byte = 0usize;
    let mut current_line = 1u32;
    for ch in stmt.chars() {
        if current_line == line_no {
            // Walk forward `col - 1` columns on this line.
            let mut col_count = 1u32;
            let mut sub = 0usize;
            for ch2 in stmt[byte..].chars() {
                if col_count == col {
                    break;
                }
                if ch2 == '\n' {
                    break;
                }
                sub += ch2.len_utf8();
                col_count += 1;
            }
            let abs = src_offset + byte + sub;
            let (sl, sc) = split::byte_to_line_col(program, abs);
            return Some(SourceSpan {
                start_line: sl,
                start_col: sc,
                end_line: sl,
                end_col: sc + 1,
            });
        }
        let len = ch.len_utf8();
        byte += len;
        if ch == '\n' {
            current_line += 1;
        }
    }
    None
}

fn is_query(sql: &str) -> bool {
    let s = sql.trim_start();
    let head: String = s
        .chars()
        .take_while(|c| c.is_alphabetic())
        .flat_map(|c| c.to_lowercase())
        .collect();
    matches!(
        head.as_str(),
        "select" | "with" | "show" | "describe" | "explain" | "pragma" | "values" | "table"
            | "from" | "summarize"
    )
}

fn type_name(v: &duckdb::types::Value) -> &'static str {
    use duckdb::types::Value as DV;
    match v {
        DV::Null => "null",
        DV::Boolean(_) => "boolean",
        DV::TinyInt(_) | DV::SmallInt(_) | DV::Int(_) | DV::BigInt(_) => "integer",
        DV::HugeInt(_) => "hugeint",
        DV::UTinyInt(_) | DV::USmallInt(_) | DV::UInt(_) | DV::UBigInt(_) => "uinteger",
        DV::Float(_) | DV::Double(_) => "double",
        DV::Text(_) => "varchar",
        DV::Blob(_) => "blob",
        _ => "other",
    }
}

fn value_from_duckdb(v: duckdb::types::Value) -> Value {
    use duckdb::types::Value as DV;
    match v {
        DV::Null => Value::Null,
        DV::Boolean(b) => Value::Bool(b),
        DV::TinyInt(i) => Value::Int(i as i64),
        DV::SmallInt(i) => Value::Int(i as i64),
        DV::Int(i) => Value::Int(i as i64),
        DV::BigInt(i) => Value::Int(i),
        DV::HugeInt(i) => Value::Text(i.to_string()),
        DV::UTinyInt(i) => Value::Int(i as i64),
        DV::USmallInt(i) => Value::Int(i as i64),
        DV::UInt(i) => Value::Int(i as i64),
        DV::UBigInt(i) => Value::Int(i as i64),
        DV::Float(f) => Value::Float(f as f64),
        DV::Double(f) => Value::Float(f),
        DV::Text(s) => Value::Text(s),
        DV::Blob(b) => Value::Text(format!("<blob {} bytes>", b.len())),
        other => Value::Text(format!("{:?}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_bare_select() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit("select 1 as a, 'hi' as b;");
        assert!(matches!(evs.last(), Some(Event::Done)));
        assert!(evs.iter().any(|e| matches!(e, Event::Output { .. })));
    }

    #[test]
    fn runs_proc_sql_block() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(
            r#"
            proc sql;
                create table t as select 1 as a union all select 2;
                select count(*) as n from t;
            quit;
            "#,
        );
        let outputs: Vec<_> = evs.iter().filter(|e| matches!(e, Event::Output { .. })).collect();
        assert_eq!(outputs.len(), 1, "got events: {:?}", evs);
    }

    #[test]
    fn surfaces_sql_error() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit("select * from no_such_table;");
        assert!(evs.iter().any(|e| matches!(e, Event::Error { .. })));
    }

    #[test]
    fn create_table_overwrites_on_second_run() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table t as select 1 as x;");
        let evs = s.submit("create table t as select 2 as x;");
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
    }

    #[test]
    fn lists_work_dataset_after_create() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table foo as select 1 as a;");
        let ds = s.list_datasets("work").unwrap();
        assert!(ds.iter().any(|d| d.name == "foo"));
    }

    #[test]
    fn create_into_dir_library_writes_parquet() {
        let dir = tempdir_path();
        std::fs::create_dir_all(&dir).unwrap();
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(&format!(r#"libname out "{}";"#, dir));
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let evs = s.submit("create table out.demo as select 1 as a union all select 2;");
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        assert!(std::path::Path::new(&format!("{}/demo.parquet", dir)).exists());
        // Read back via libref.
        let evs = s.submit("select count(*) as n from out.demo;");
        assert!(evs.iter().any(|e| matches!(e, Event::Output { .. })), "{:?}", evs);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn tempdir_path() -> String {
        let p = std::env::temp_dir().join(format!("pas-test-{}", uuid::Uuid::new_v4()));
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn data_step_filters_and_derives() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x union all select 2 union all select 5;");
        let evs = s.submit(
            r#"
            data work.out;
                set src;
                if x > 1;
                y = x * 10;
                msg = cats('hello-', x);
            run;
            "#,
        );
        let errs: Vec<_> = evs.iter().filter(|e| matches!(e, Event::Error { .. })).collect();
        assert!(errs.is_empty(), "errors: {:?}", errs);
        let page = s.dataset_page("work", "out", 0, 100, None).unwrap();
        assert_eq!(page.total_rows, 2);
        // Columns: x, y, msg
        let names: Vec<_> = page.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"x"));
        assert!(names.contains(&"y"));
        assert!(names.contains(&"msg"));
    }

    #[test]
    fn data_step_keep_drop() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as a, 2 as b, 3 as c;");
        s.submit("data o; set src; keep a c; run;");
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        let names: Vec<_> = page.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"a") && names.contains(&"c") && !names.contains(&"b"));
    }

    #[test]
    fn data_step_retain_accumulator() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select * from (values (1), (2), (3), (4)) as t(x);");
        let evs = s.submit(
            r#"
            data o;
                set src;
                retain total 0;
                total = total + x;
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 100, None).unwrap();
        assert_eq!(page.total_rows, 4);
        // Last row's `total` should be 1+2+3+4 = 10.
        let total_idx = page.columns.iter().position(|c| c.name == "total").unwrap();
        if let crate::Value::Float(t) = &page.rows[3][total_idx] {
            assert!((t - 10.0).abs() < 1e-9, "expected 10, got {}", t);
        } else {
            panic!("expected float, got {:?}", page.rows[3][total_idx]);
        }
    }

    #[test]
    fn data_step_by_first_last() {
        let s = Session::new_in_memory().unwrap();
        s.submit(
            "create table src as select * from (values \
             ('a',1),('a',2),('b',1),('c',1),('c',2),('c',3)) as t(grp, x);",
        );
        // Keep only the last row per group.
        let evs = s.submit(
            r#"
            data o;
                set src;
                by grp;
                if last.grp;
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 100, None).unwrap();
        assert_eq!(page.total_rows, 3);
    }

    #[test]
    fn data_step_array_and_do_loop() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as a1, 2 as a2, 3 as a3;");
        let evs = s.submit(
            r#"
            data o;
                set src;
                array a{3} a1 a2 a3;
                total = 0;
                do i = 1 to 3;
                    total = total + a{i};
                end;
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 100, None).unwrap();
        let total_idx = page.columns.iter().position(|c| c.name == "total").unwrap();
        if let crate::Value::Float(t) = &page.rows[0][total_idx] {
            assert!((t - 6.0).abs() < 1e-9);
        } else { panic!("not float: {:?}", page.rows[0][total_idx]); }
    }

    #[test]
    fn data_step_infile_csv() {
        let dir = tempdir_path();
        std::fs::create_dir_all(&dir).unwrap();
        let path = format!("{}/people.csv", dir);
        std::fs::write(&path, "name,age\nalice,30\nbob,25\ncarol,41\n").unwrap();
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(&format!(
            r#"
            data work.people;
                infile '{}' dlm=',' dsd firstobs=2;
                input name $ age;
            run;
            "#,
            path
        ));
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "people", 0, 100, None).unwrap();
        assert_eq!(page.total_rows, 3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn data_step_put_formats() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x;");
        let evs = s.submit(
            r#"
            data o;
                set src;
                d_fmt = put('15FEB2024'd, 'date9.');
                iso   = put('15FEB2024'd, 'yymmdd10.');
                money = put(1234567.89, 'comma14.2');
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        let by = |n: &str| page.columns.iter().position(|c| c.name == n).unwrap();
        let txt = |idx: usize| match &page.rows[0][idx] {
            crate::Value::Text(s) => s.clone(),
            other => panic!("expected text, got {:?}", other),
        };
        assert_eq!(txt(by("d_fmt")), "15FEB2024");
        assert_eq!(txt(by("iso")), "2024-02-15");
        assert_eq!(txt(by("money")).trim(), "1,234,567.89");
    }

    #[test]
    fn data_step_input_function() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select '01JAN2024' as s;");
        let evs = s.submit(
            r#"
            data o;
                set src;
                d = input(s, 'date9.');
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        let d_idx = page.columns.iter().position(|c| c.name == "d").unwrap();
        let expected = (chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
            - chrono::NaiveDate::from_ymd_opt(1960, 1, 1).unwrap()).num_days() as f64;
        match &page.rows[0][d_idx] {
            crate::Value::Float(f) => assert!((f - expected).abs() < 1e-9, "{} vs {}", f, expected),
            other => panic!("expected float, got {:?}", other),
        }
    }

    #[test]
    fn data_step_datalines_input() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(
            "data work.people;\n  input name $ age;\n  datalines;\nalice 30\nbob 25\ncarol 41\n;\nrun;\n",
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "people", 0, 100, None).unwrap();
        assert_eq!(page.total_rows, 3);
        let name_i = page.columns.iter().position(|c| c.name == "name").unwrap();
        let age_i = page.columns.iter().position(|c| c.name == "age").unwrap();
        let names: Vec<String> = page.rows.iter().map(|r| match &r[name_i] {
            crate::Value::Text(s) => s.clone(),
            _ => String::new(),
        }).collect();
        let ages: Vec<f64> = page.rows.iter().map(|r| match &r[age_i] {
            crate::Value::Float(f) => *f,
            _ => f64::NAN,
        }).collect();
        assert_eq!(names, vec!["alice", "bob", "carol"]);
        assert_eq!(ages, vec![30.0, 25.0, 41.0]);
    }

    #[test]
    fn data_step_merge_streams_through_cursors() {
        // 200K left + 200K right with overlapping by-keys. Old impl held
        // both fully materialized in Rust HashMaps. New impl snapshots
        // each to a DuckDB temp table and streams 4K rows at a time per
        // cursor.
        let s = Session::new_in_memory().unwrap();
        s.submit("create table lefts  as select x as id, x * 2  as a from range(0, 200000) t(x);");
        s.submit("create table rights as select x as id, x * 10 as b from range(0, 200000) t(x);");
        let evs = s.submit(
            r#"
            data merged;
                merge lefts rights;
                by id;
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "merged", 0, 1, None).unwrap();
        assert_eq!(page.total_rows, 200000);
    }

    #[test]
    fn tier1_function_library() {
        // One golden program that exercises every tier-1 addition end
        // to end. Inputs are crafted so the expected outputs are easy to
        // read off without computing dates by hand.
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x;");
        let evs = s.submit(
            r#"
            data o;
                set src;
                /* string */
                word3 = scan('alpha beta gamma delta', 3);
                where_be = find('the quick brown fox', 'brown');
                no_dashes = tranwrd('a-b-c', '-', ':');
                masked = translate('hello', '*', 'l');
                blamt = compbl('one   two    three');
                title = propcase('the quick brown fox');
                rev = reverse('abc');
                rep = repeat('ab', 2);
                /* numeric */
                sgn_pos = sign(7);
                sgn_neg = sign(-3);
                largest_3 = largest(2, 5, 1, 9, 3);
                smallest_2 = smallest(2, 5, 1, 9, 3);
                ifn_val = ifn(x > 0, 100, -1);
                ifc_val = ifc(x > 0, 'pos', 'neg');
                pos = whichn(9, 5, 1, 9, 3);
                cpos = whichc('b', 'a', 'b', 'c');
                /* missing */
                nm = notmissing(x);
                /* date — yrdif from 1JAN2000 to 1JAN2025 ≈ 25.0 */
                years = yrdif('01JAN2000'd, '01JAN2025'd, 'act/365');
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        let by = |n: &str| page.columns.iter().position(|c| c.name == n).unwrap();
        let txt = |n: &str| match &page.rows[0][by(n)] {
            crate::Value::Text(s) => s.clone(),
            other => panic!("{}: expected text, got {:?}", n, other),
        };
        let num = |n: &str| match &page.rows[0][by(n)] {
            crate::Value::Float(f) => *f,
            crate::Value::Int(i) => *i as f64,
            other => panic!("{}: expected number, got {:?}", n, other),
        };
        assert_eq!(txt("word3"), "gamma");
        assert_eq!(num("where_be"), 11.0);
        assert_eq!(txt("no_dashes"), "a:b:c");
        assert_eq!(txt("masked"), "he**o");
        assert_eq!(txt("blamt"), "one two three");
        assert_eq!(txt("title"), "The Quick Brown Fox");
        assert_eq!(txt("rev"), "cba");
        assert_eq!(txt("rep"), "ababab"); // repeat 'ab' with n=2 → 3 copies
        assert_eq!(num("sgn_pos"), 1.0);
        assert_eq!(num("sgn_neg"), -1.0);
        assert_eq!(num("largest_3"), 5.0); // 2nd largest of {5,1,9,3}
        assert_eq!(num("smallest_2"), 3.0); // 2nd smallest of {5,1,9,3}
        assert_eq!(num("ifn_val"), 100.0);
        assert_eq!(txt("ifc_val"), "pos");
        assert_eq!(num("pos"), 3.0);
        assert_eq!(num("cpos"), 2.0);
        assert_eq!(num("nm"), 1.0);
        // 25 years ÷ 365-basis ≈ 9131/365 = 25.0164…
        assert!((num("years") - 25.0).abs() < 0.05, "got {}", num("years"));
    }

    #[test]
    fn runtime_call_error_carries_span() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x;");
        let program = "data o;\n  set src;\n  y = some_function_that_doesnt_exist(x);\nrun;\n";
        let evs = s.submit(program);
        let span = evs.iter().find_map(|e| match e {
            Event::Error { source_span, .. } => source_span.clone(),
            _ => None,
        });
        let span = span.expect("expected runtime error span");
        // Line 3 is the assignment with the bad function.
        assert_eq!(span.start_line, 3);
        assert!(span.start_col >= 5, "expected the call to be past the indent, got col {}", span.start_col);
    }

    #[test]
    fn runtime_array_out_of_range_carries_span() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x;");
        let program = "data o;\n  set src;\n  array a{3} a1 a2 a3;\n  y = a{5};\nrun;\n";
        let evs = s.submit(program);
        let span = evs.iter().find_map(|e| match e {
            Event::Error { source_span, .. } => source_span.clone(),
            _ => None,
        });
        let span = span.expect("expected runtime error span");
        // Line 4 is `y = a{5};`.
        assert_eq!(span.start_line, 4);
    }

    #[test]
    fn proc_sql_error_carries_source_span() {
        let s = Session::new_in_memory().unwrap();
        // FRM instead of FROM — DuckDB emits `LINE 1: SELECT * FRM ...^`.
        let evs = s.submit("select * frm sqlite_master;\n");
        let span = evs.iter().find_map(|e| match e {
            Event::Error { source_span, .. } => source_span.clone(),
            _ => None,
        });
        let span = span.expect("expected PROC SQL error span");
        assert_eq!(span.start_line, 1);
        // Column should be somewhere around the offending `frm` token
        // (col 10 in `select * frm ...` — DuckDB's caret tends to land
        // there). Don't pin to an exact number to stay robust across
        // DuckDB versions, but it must be past the first column.
        assert!(span.start_col >= 5, "got col {}", span.start_col);
    }

    #[test]
    fn data_step_parse_error_carries_source_span() {
        let s = Session::new_in_memory().unwrap();
        // Bad syntax inside the body: missing semicolon after the assignment.
        let program = "data out;\n  set src;\n  x = 1\n  y = 2;\nrun;\n";
        let evs = s.submit(program);
        let err_span = evs.iter().find_map(|e| match e {
            Event::Error { source_span, .. } => source_span.clone(),
            _ => None,
        });
        let span = err_span.expect("expected an Event::Error with a source_span");
        // The offending token should land somewhere on or after the buggy
        // line 3 (which is the assignment without a trailing semicolon).
        assert!(span.start_line >= 3, "span at line {} should be >= 3", span.start_line);
        assert!(span.end_line >= span.start_line);
        assert!(span.start_col >= 1);
    }

    #[test]
    fn data_step_streams_large_input() {
        // 500K rows × a couple of derived columns. Materializing this as
        // Vec<HashMap<String, RtValue>> would cost ~150-200 MB. With the
        // streaming pipeline it should fit easily in normal test memory
        // and finish in a few seconds.
        let s = Session::new_in_memory().unwrap();
        s.submit("create table big as select * from range(0, 500000) t(x);");
        let evs = s.submit(
            r#"
            data work.big_out;
                set big;
                y = x * 2;
                bucket = mod(x, 100);
                if bucket = 0;
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "big_out", 0, 1, None).unwrap();
        assert_eq!(page.total_rows, 5000);
    }

    #[test]
    fn proc_sort_orders_with_nodupkey() {
        let s = Session::new_in_memory().unwrap();
        s.submit(
            "create table src as select * from (values \
             ('b', 2),('a', 1),('a', 3),('c', 5)) as t(grp, val);",
        );
        let evs = s.submit(
            "proc sort data=src out=sorted nodupkey; by grp; run;",
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "sorted", 0, 10, None).unwrap();
        // nodupkey keeps one row per by-group (grp): a, b, c → 3 rows.
        assert_eq!(page.total_rows, 3);
        let grp_idx = page.columns.iter().position(|c| c.name == "grp").unwrap();
        if let crate::Value::Text(g) = &page.rows[0][grp_idx] {
            assert_eq!(g, "a");
        }
    }

    #[test]
    fn proc_print_emits_output_block() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select * from (values (1),(2),(3)) as t(x);");
        let evs = s.submit("proc print data=src obs=2; var x; run;");
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let outputs: Vec<_> = evs.iter().filter(|e| matches!(e, Event::Output { .. })).collect();
        assert_eq!(outputs.len(), 1);
        if let Some(Event::Output { block }) = outputs.first() {
            assert_eq!(block.rows.len(), 2);
            assert_eq!(block.columns[0].name, "x");
        }
    }

    #[test]
    fn proc_transpose_pivots_long_to_wide() {
        let s = Session::new_in_memory().unwrap();
        s.submit(
            "create table sales as select * from (values \
             ('east','q1',10),('east','q2',20),('west','q1',5),('west','q2',8)) as t(region, qtr, amount);",
        );
        let evs = s.submit(
            "proc transpose data=sales out=wide; by region; id qtr; var amount; run;",
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "wide", 0, 10, None).unwrap();
        // 2 regions × (region + q1 + q2) = 2 rows, 3 columns
        assert_eq!(page.total_rows, 2);
        let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"region"));
        assert!(names.contains(&"q1"));
        assert!(names.contains(&"q2"));
    }

    #[test]
    fn proc_sql_calculated_keyword() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select * from (values (10), (20), (30)) as t(x);");
        let evs = s.submit(
            "proc sql; create table o as select x, x*2 as doubled, calculated doubled + 1 as plus1 from src; quit;",
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        assert_eq!(page.total_rows, 3);
    }

    #[test]
    fn proc_sql_monotonic_function() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select * from (values ('a'),('b'),('c')) as t(letter);");
        let evs = s.submit(
            "proc sql; create table o as select monotonic() as rn, letter from src; quit;",
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        assert_eq!(page.total_rows, 3);
        let rn_idx = page.columns.iter().position(|c| c.name == "rn").unwrap();
        // First row's rn should be 1.
        match &page.rows[0][rn_idx] {
            crate::Value::Int(n) => assert_eq!(*n, 1),
            other => panic!("expected int rn, got {:?}", other),
        }
    }

    #[test]
    fn proc_sql_outer_union_corr_aligns_by_name() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table a as select 1 as id, 'ada' as name;");
        s.submit("create table b as select 'alan' as name, 2 as id;");
        let evs = s.submit(
            "proc sql; create table merged as select * from a outer union corr select * from b; quit;",
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "merged", 0, 10, None).unwrap();
        // Two rows, columns id + name regardless of declaration order.
        assert_eq!(page.total_rows, 2);
        let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"id") && names.contains(&"name"));
    }

    #[test]
    fn macro_let_and_put_drive_program() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(
            r#"
            %let target = 42;
            %put answer is &target;
            create table t as select &target as x;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        // The %put text appears as a NOTE.
        assert!(evs.iter().any(|e| matches!(e, Event::Note { text } if text.contains("answer is 42"))));
        // The table was created with x = 42.
        let page = s.dataset_page("work", "t", 0, 10, None).unwrap();
        assert_eq!(page.total_rows, 1);
        if let crate::Value::Int(n) = &page.rows[0][0] {
            assert_eq!(*n, 42);
        }
    }

    #[test]
    fn macro_vars_persist_across_submissions() {
        let s = Session::new_in_memory().unwrap();
        s.submit("%let name = ada;");
        let evs = s.submit("%put hi &name;");
        assert!(evs.iter().any(|e| matches!(e, Event::Note { text } if text.contains("hi ada"))));
    }

    #[test]
    fn data_step_select_when() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select * from (values (1),(2),(3),(4)) as t(x);");
        let evs = s.submit(
            r#"
            data o;
                set src;
                select (x);
                    when (1) label = 'one';
                    when (2, 3) label = 'middle';
                    otherwise label = 'big';
                end;
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 100, None).unwrap();
        let label_idx = page.columns.iter().position(|c| c.name == "label").unwrap();
        let labels: Vec<String> = page.rows.iter().map(|r| match &r[label_idx] {
            crate::Value::Text(s) => s.clone(),
            _ => String::new(),
        }).collect();
        assert_eq!(labels, vec!["one", "middle", "middle", "big"]);
    }

    #[test]
    fn data_step_date_funcs_and_literal() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x;");
        let evs = s.submit(
            r#"
            data o;
                set src;
                d  = '01JAN2024'd;
                yr = year(d);
                mn = month(d);
                dy = day(d);
                next = intnx('month', d, 1);
                gap  = intck('year', d, '01JAN2026'd);
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        let by_name = |n: &str| page.columns.iter().position(|c| c.name == n).unwrap();
        let row = &page.rows[0];
        let n = |i: usize| match &row[i] { crate::Value::Float(f) => *f, _ => f64::NAN };
        assert_eq!(n(by_name("yr")), 2024.0);
        assert_eq!(n(by_name("mn")), 1.0);
        assert_eq!(n(by_name("dy")), 1.0);
        // 01FEB2024 = days from 1960-01-01
        // intnx('month', 01JAN2024, 1) → 01FEB2024
        let feb1 = (chrono::NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
            - chrono::NaiveDate::from_ymd_opt(1960, 1, 1).unwrap()).num_days() as f64;
        assert_eq!(n(by_name("next")), feb1);
        assert_eq!(n(by_name("gap")), 2.0);
    }

    #[test]
    fn data_step_merge_one_to_many() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table people as select * from (values (1,'a'),(2,'b'),(3,'c')) as t(id, name);");
        s.submit("create table scores as select * from (values (1,10),(1,20),(2,30)) as t(id, score);");
        let evs = s.submit(
            r#"
            data o;
                merge people scores;
                by id;
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 100, None).unwrap();
        // id=1 → 2 rows (name=a broadcast), id=2 → 1 row, id=3 → 1 row (score null) = 4 total
        assert_eq!(page.total_rows, 4);
    }

    #[test]
    fn data_step_reads_and_writes_dir_library() {
        let dir = tempdir_path();
        std::fs::create_dir_all(&dir).unwrap();
        let s = Session::new_in_memory().unwrap();
        s.submit(&format!(r#"libname dados "{}";"#, dir));
        // Seed a parquet file via PROC SQL (CREATE TABLE dados.people).
        let evs = s.submit("create table dados.people as select 'Ada' as name, 1815 as born;");
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        // DATA step that reads + writes via the DIR libref.
        let evs = s.submit(
            r#"
            data dados.people_12;
                set dados.people;
                numero_12 = 12;
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        assert!(std::path::Path::new(&format!("{}/people_12.parquet", dir)).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn data_step_if_then_else() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 5 as x union all select 0 as x;");
        let evs = s.submit(
            r#"
            data o; set src;
                if x > 0 then sign = 'pos'; else sign = 'zero';
            run;
            "#,
        );
        assert!(!evs.iter().any(|e| matches!(e, Event::Error { .. })), "{:?}", evs);
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        assert_eq!(page.total_rows, 2);
    }

    #[test]
    fn dataset_page_arrow_produces_ipc_stream() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table foo as select * from range(0, 50) t(x);");
        let bytes = s.dataset_page_arrow("work", "foo", 0, 10, None).unwrap();
        // Arrow IPC stream starts with a 4-byte continuation token 0xFFFFFFFF.
        assert!(bytes.len() > 16, "page should be non-trivial");
        assert_eq!(&bytes[0..4], &[0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn dataset_page_returns_total() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table foo as select * from range(0, 50) t(x);");
        let page = s.dataset_page("work", "foo", 0, 10, None).unwrap();
        assert_eq!(page.total_rows, 50);
        assert_eq!(page.rows.len(), 10);
    }
}
