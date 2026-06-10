use crate::library::{DirFormat, Library, LibraryKind};
use crate::query::{duckdb_error_span, materialize_select_into, run_one, run_query, StmtResult};
use crate::rewrite::dir_reader_expr;
use crate::split::{split_blocks_checked, strip_comments, Block, SplitError};
use crate::types::{Event, SourceSpan, Value};
use crate::{datastep, libname, pas_sql, procs, split, EngineError, MAX_PREVIEW_ROWS};
use duckdb::{Connection, InterruptHandle};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

fn parse_title_value(raw: &str) -> String {
    let s = raw.trim();
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        let quote = bytes[0];
        if (quote == b'\'' || quote == b'"') && bytes[s.len() - 1] == quote {
            let inner = &s[1..s.len() - 1];
            let doubled = if quote == b'\'' { "''" } else { "\"\"" };
            let single = if quote == b'\'' { "'" } else { "\"" };
            return inner.replace(doubled, single);
        }
    }
    s.to_string()
}

fn split_error_event(program: &str, err: SplitError) -> Event {
    match err {
        SplitError::UnterminatedDatalines { start_offset } => {
            let (line, col) = split::byte_to_line_col(program, start_offset);
            Event::Error {
                text: err.to_string(),
                source_span: Some(SourceSpan {
                    start_line: line,
                    start_col: col,
                    end_line: line,
                    end_col: col + 1,
                }),
            }
        }
    }
}

/// Extract a human-readable message from a caught panic payload. Rust panics
/// carry either a `&str` (string-literal `panic!`) or a `String` (formatted).
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

pub struct Session {
    pub(crate) conn: Mutex<Connection>,
    pub(crate) read_conn: Mutex<Connection>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) interrupt: Arc<InterruptHandle>,
    pub(crate) libraries: Mutex<HashMap<String, Library>>,
    pub(crate) macro_vars: Mutex<HashMap<String, String>>,
    pub(crate) macro_defs: Mutex<HashMap<String, crate::macros::MacroDef>>,
    pub(crate) dataset_formats: Mutex<HashMap<String, HashMap<String, String>>>,
    pub(crate) title: Mutex<Option<String>>,
}

impl Session {
    pub fn new_in_memory() -> Result<Self, EngineError> {
        let conn = Connection::open_in_memory()?;
        let read_conn = conn.try_clone()?;
        let interrupt = conn.interrupt_handle();
        let mut libs = HashMap::new();
        // WORK is always present and points at the default in-memory schema.
        libs.insert(
            "work".to_string(),
            Library {
                name: "work".to_string(),
                kind: LibraryKind::Memory,
                path: String::new(),
                format: None,
            },
        );
        Ok(Self {
            conn: Mutex::new(conn),
            read_conn: Mutex::new(read_conn),
            cancel: Arc::new(AtomicBool::new(false)),
            interrupt,
            libraries: Mutex::new(libs),
            macro_vars: Mutex::new(HashMap::new()),
            macro_defs: Mutex::new(HashMap::new()),
            dataset_formats: Mutex::new(HashMap::new()),
            title: Mutex::new(None),
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
        let mut v: Vec<Library> = self
            .libraries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    /// Run an entire program, returning all events.
    ///
    /// Convenience wrapper over [`Session::submit_with`] that buffers every
    /// event; interactive callers should prefer `submit_with` so the log can
    /// update while the program is still running.
    pub fn submit(&self, program: &str) -> Vec<Event> {
        let mut events = Vec::new();
        self.submit_with(program, |e| events.push(e));
        events
    }

    /// Run an entire program, delivering events to `on_event` as each block
    /// finishes executing — not buffered until the end of the run. The final
    /// event is always [`Event::Done`].
    pub fn submit_with<F: FnMut(Event)>(&self, program: &str, mut on_event: F) {
        let cleaned = strip_comments(program);

        let blocks = match split_blocks_checked(&cleaned) {
            Ok(blocks) => blocks,
            Err(err) => {
                on_event(split_error_event(program, err));
                on_event(Event::Done);
                return;
            }
        };
        tracing::debug!(block_count = blocks.len(), "program split into blocks");

        if blocks.is_empty() {
            on_event(Event::Note {
                text: "No statements found.".into(),
            });
            on_event(Event::Done);
            return;
        }

        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Reset the cancel flag only after this run owns the connection: a
        // submission queued behind an in-flight run must not clear a cancel
        // aimed at that run (see queued_submission_does_not_clear_a_pending_cancel).
        self.cancel.store(false, Ordering::SeqCst);
        for block in blocks {
            if self.cancel.load(Ordering::SeqCst) {
                on_event(Event::Warning {
                    text: "Execution cancelled by user.".into(),
                });
                break;
            }

            // Reconstruct the raw textual form of this block for macro
            // preprocessing. The macro processor operates on text, so each
            // block kind is serialized back to the form a user would have
            // typed.
            let raw = match &block {
                Block::Statement { text, .. } | Block::ProcSqlStmt { text, .. } => text.clone(),
                Block::DataStep { body, .. } => body.clone(),
                Block::Proc { name, body, .. } => format!("proc {}; {} run;", name, body),
            };
            let macro_result = {
                let mut vars = self.macro_vars.lock().unwrap_or_else(|e| e.into_inner());
                let mut defs = self.macro_defs.lock().unwrap_or_else(|e| e.into_inner());
                crate::macros::preprocess(&raw, &mut vars, &mut defs)
            };
            tracing::debug!(
                raw = %raw,
                expanded = %macro_result.expanded,
                puts = ?macro_result.puts,
                "macro preprocessing complete",
            );
            for put_text in macro_result.puts {
                on_event(Event::Note { text: put_text });
            }
            // A DataStep with empty expansion is still treated as a parse
            // error (matches pre-refactor behavior); for the other kinds an
            // empty expansion is silently skipped.
            if !matches!(block, Block::DataStep { .. }) && macro_result.expanded.trim().is_empty() {
                continue;
            }

            // Dispatch. Statement and Proc admit macro expansions that
            // introduce additional sibling blocks, so the expanded text is
            // re-split. DataStep and ProcSqlStmt are dispatched as a single
            // expanded block.
            match block {
                Block::DataStep {
                    datalines,
                    body_src_offset,
                    ..
                } => {
                    let synthetic = Block::DataStep {
                        body: macro_result.expanded,
                        datalines,
                        body_src_offset,
                    };
                    self.dispatch_block_streamed(&conn, synthetic, program, &mut on_event);
                }
                Block::ProcSqlStmt { src_offset, .. } => {
                    let synthetic = Block::ProcSqlStmt {
                        text: macro_result.expanded.trim().to_string(),
                        src_offset,
                    };
                    self.dispatch_block_streamed(&conn, synthetic, program, &mut on_event);
                }
                Block::Statement { .. } => {
                    let sub_blocks = match split_blocks_checked(&macro_result.expanded) {
                        Ok(blocks) => blocks,
                        Err(err) => {
                            on_event(split_error_event(&macro_result.expanded, err));
                            continue;
                        }
                    };
                    tracing::debug!(sub_blocks = ?sub_blocks, "expanded statement split");
                    for sub_block in sub_blocks {
                        if self.cancel.load(Ordering::SeqCst) {
                            on_event(Event::Warning {
                                text: "Execution cancelled by user.".into(),
                            });
                            break;
                        }
                        self.dispatch_block_streamed(
                            &conn,
                            sub_block,
                            &macro_result.expanded,
                            &mut on_event,
                        );
                    }
                }
                Block::Proc { .. } => {
                    // Historical quirk: a PROC whose macro expansion yields
                    // non-PROC sibling blocks silently drops them.
                    let sub_blocks = match split_blocks_checked(&macro_result.expanded) {
                        Ok(blocks) => blocks,
                        Err(err) => {
                            on_event(split_error_event(&macro_result.expanded, err));
                            continue;
                        }
                    };
                    for sub_block in sub_blocks {
                        if self.cancel.load(Ordering::SeqCst) {
                            on_event(Event::Warning {
                                text: "Execution cancelled by user.".into(),
                            });
                            break;
                        }
                        if matches!(sub_block, Block::Proc { .. }) {
                            self.dispatch_block_streamed(
                                &conn,
                                sub_block,
                                &macro_result.expanded,
                                &mut on_event,
                            );
                        }
                    }
                }
            }
        }

        on_event(Event::Done);
    }

    /// Run one block through the guarded dispatcher and forward its events to
    /// the streaming callback. The block runners accumulate into a `Vec`, so
    /// delivery granularity is one dispatched block.
    fn dispatch_block_streamed<F: FnMut(Event)>(
        &self,
        conn: &Connection,
        block: Block,
        program_for_spans: &str,
        on_event: &mut F,
    ) {
        let mut events = Vec::new();
        self.dispatch_block_guarded(conn, block, program_for_spans, &mut events);
        for event in events {
            on_event(event);
        }
    }

    /// Execute one block, converting any internal panic into an Error event.
    ///
    /// The engine runs arbitrary user-authored programs, and a bug in any
    /// statement runner (bad date arithmetic, an out-of-range index, etc.)
    /// must never abort the whole submission or leave the engine unusable.
    /// `catch_unwind` keeps the panic from unwinding past `submit` (so the
    /// run still ends with `Event::Done`), and because every shared mutex is
    /// acquired with poison recovery, a panic that fires while a lock is held
    /// does not brick subsequent submissions. See the
    /// `engine_panic_in_block_is_isolated_and_session_recovers` test.
    fn dispatch_block_guarded(
        &self,
        conn: &Connection,
        block: Block,
        program_for_spans: &str,
        events: &mut Vec<Event>,
    ) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.dispatch_block(conn, block, program_for_spans, events);
        }));
        if let Err(payload) = outcome {
            events.push(Event::Error {
                text: format!("internal engine error: {}", panic_message(&payload)),
                source_span: None,
            });
        }
    }

    /// Execute one concrete block. The caller has already performed any
    /// macro preprocessing; this method only emits a Source event and
    /// dispatches to the kind-specific runner.
    fn dispatch_block(
        &self,
        conn: &Connection,
        block: Block,
        program_for_spans: &str,
        events: &mut Vec<Event>,
    ) {
        match block {
            Block::Statement { text, src_offset } => {
                events.push(Event::Source { text: text.clone() });
                if let Some(handled) = self.try_libname(conn, &text) {
                    events.extend(handled);
                    return;
                }
                if let Some(handled) = self.try_title(&text) {
                    events.extend(handled);
                    return;
                }
                self.run_sql_with_rewrites(conn, &text, src_offset, program_for_spans, events);
            }
            Block::ProcSqlStmt { text, src_offset } => {
                events.push(Event::Source { text: text.clone() });
                self.run_sql_with_rewrites(conn, &text, src_offset, program_for_spans, events);
            }
            Block::DataStep {
                body,
                datalines,
                body_src_offset,
            } => {
                events.push(Event::Source { text: body.clone() });
                self.run_data_step(
                    conn,
                    &body,
                    datalines,
                    body_src_offset,
                    program_for_spans,
                    events,
                );
            }
            Block::Proc { name, body, .. } => {
                events.push(Event::Source {
                    text: format!("proc {}; {} run;", name, body),
                });
                self.run_proc(conn, &name, &body, events);
            }
        }
    }

    fn try_libname(&self, conn: &Connection, stmt: &str) -> Option<Vec<Event>> {
        match libname::parse(stmt) {
            Ok(Some(def)) => Some(match self.apply_libname(conn, &def) {
                Ok(msg) => vec![Event::Note { text: msg }],
                Err(e) => vec![Event::Error {
                    text: e.to_string(),
                    source_span: None,
                }],
            }),
            Ok(None) => None,
            Err(e) => Some(vec![Event::Error {
                text: e.to_string(),
                source_span: None,
            }]),
        }
    }

    fn try_title(&self, stmt: &str) -> Option<Vec<Event>> {
        let trimmed = stmt.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower != "title" && !lower.starts_with("title ") {
            return None;
        }

        let value = trimmed
            .get(5..)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(parse_title_value);
        *self.title.lock().unwrap_or_else(|e| e.into_inner()) = value.clone();

        let text = match value {
            Some(title) => format!("Title set to \"{}\".", title),
            None => "Title cleared.".to_string(),
        };
        Some(vec![Event::Note { text }])
    }

    fn active_title(&self) -> Option<String> {
        self.title.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
        let (clean_query, targets) = pas_sql::extract_into_clause(&rewritten);
        match run_one(conn, &clean_query, 1000) {
            Ok(StmtResult::Rows(mut block)) => {
                if !targets.is_empty() {
                    if let Some(first_row) = block.rows.first() {
                        let mut vars = self.macro_vars.lock().unwrap_or_else(|e| e.into_inner());
                        for (idx, target) in targets.iter().enumerate() {
                            if let Some(val) = first_row.get(idx) {
                                let mut val_str = match val {
                                    Value::Null => String::new(),
                                    Value::Bool(b) => b.to_string(),
                                    Value::Int(i) => i.to_string(),
                                    Value::Float(f) => f.to_string(),
                                    Value::Text(s) => s.clone(),
                                };
                                if target.trimmed {
                                    val_str = val_str.trim().to_string();
                                }
                                vars.insert(target.name.clone(), val_str);
                            }
                        }
                    }
                    events.push(Event::Note {
                        text: "Statement executed, macro variables assigned.".into(),
                    });
                } else {
                    block.title = self.active_title();
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
            }
            Ok(StmtResult::Affected(n)) => events.push(Event::Note {
                text: format!("Statement executed ({} row(s) affected).", n),
            }),
            Ok(StmtResult::Done) => events.push(Event::Note {
                text: "Statement executed.".into(),
            }),
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
            Err(e) => events.push(Event::Error {
                text: e.to_string(),
                source_span: None,
            }),
        }
    }

    fn proc_sort(&self, conn: &Connection, body: &str) -> Result<Vec<Event>, EngineError> {
        let spec = procs::sort::parse(body).map_err(EngineError::Other)?;
        let from = self.resolve_read(&spec.data_in)?;
        let target = self.resolve_write(&spec.data_out)?;
        let select_sql = procs::sort::build_select_sql(&from, &spec);
        let rows = materialize_select_into(conn, &target, &select_sql)?;
        Ok(vec![Event::Note {
            text: format!(
                "The data set {} has {} observations.",
                target.display(),
                rows
            ),
        }])
    }

    fn proc_print(&self, conn: &Connection, body: &str) -> Result<Vec<Event>, EngineError> {
        let spec = procs::print::parse(body).map_err(EngineError::Other)?;
        let lib = match &spec.data.libref {
            None => self.lookup_library("work")?,
            Some(l) => self.lookup_library(l)?,
        };
        let from = self.resolve_read(&spec.data)?;
        let sql = procs::print::build_select_sql(&from, &spec);
        match run_query(conn, &sql, MAX_PREVIEW_ROWS)? {
            StmtResult::Rows(block) => {
                let mut block = self.apply_dataset_formats(&lib, &spec.data.name, block);
                block.title = self.active_title();
                Ok(vec![
                    Event::Note {
                        text: format!("PROC PRINT showing {} row(s).", block.rows.len()),
                    },
                    Event::Output { block },
                ])
            }
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
            text: format!(
                "The data set {} has {} observations.",
                target.display(),
                rows
            ),
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
                match tables
                    .iter()
                    .map(|t| {
                        Ok(datastep::exec::ResolvedSource {
                            from: self.resolve_read(t)?,
                            in_var: t.in_var.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>, EngineError>>()
                {
                    Ok(v) => Some(datastep::exec::ResolvedInput::Set(v)),
                    Err(e) => {
                        events.push(Event::Error {
                            text: e.to_string(),
                            source_span: None,
                        });
                        return;
                    }
                }
            }
            Some(datastep::ast::DataInput::Merge(tables)) => {
                match tables
                    .iter()
                    .map(|t| {
                        Ok(datastep::exec::ResolvedSource {
                            from: self.resolve_read(t)?,
                            in_var: t.in_var.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>, EngineError>>()
                {
                    Ok(v) => Some(datastep::exec::ResolvedInput::Merge(v)),
                    Err(e) => {
                        events.push(Event::Error {
                            text: e.to_string(),
                            source_span: None,
                        });
                        return;
                    }
                }
            }
        };
        let mut outputs = Vec::with_capacity(ds.outputs.len());
        for t in &ds.outputs {
            if t.libref.is_none() && t.name.eq_ignore_ascii_case("_null_") {
                continue;
            }
            match self.resolve_write(t) {
                Ok(w) => outputs.push(w),
                Err(e) => {
                    events.push(Event::Error {
                        text: e.to_string(),
                        source_span: None,
                    });
                    return;
                }
            }
        }
        let plan = datastep::exec::ResolvedDataStep {
            ast: &ds,
            input,
            outputs,
        };

        match datastep::run_data_step(conn, &plan, &self.cancel, &self.macro_vars) {
            Ok(res) => {
                self.register_data_step_formats(&ds, &res.outputs);
                for (_, target, rows) in &res.outputs {
                    events.push(Event::Note {
                        text: format!(
                            "The data set {} has {} observations.",
                            target.display(),
                            rows
                        ),
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
                events.push(Event::Error {
                    text: e.to_string(),
                    source_span,
                });
            }
        }
    }

    /// Resolve a `libref.dataset` reference (or unqualified) to the SQL
    /// FROM expression DuckDB should use.
    pub(crate) fn resolve_read(&self, t: &datastep::ast::TableRef) -> Result<String, EngineError> {
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

    pub(crate) fn resolve_write(
        &self,
        t: &datastep::ast::TableRef,
    ) -> Result<datastep::exec::WriteTarget, EngineError> {
        use datastep::exec::WriteTarget;
        match &t.libref {
            None => Ok(WriteTarget::DuckDb {
                schema: "main".into(),
                name: t.name.clone(),
            }),
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
                        let display =
                            format!("{}.{}", lib.name.to_uppercase(), t.name.to_uppercase());
                        Ok(match fmt {
                            DirFormat::Parquet => WriteTarget::Parquet { path, display },
                            DirFormat::Csv => WriteTarget::Csv { path, display },
                        })
                    }
                }
            }
        }
    }

    pub(crate) fn lookup_library(&self, libref: &str) -> Result<Library, EngineError> {
        self.libraries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&libref.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| EngineError::Other(format!("library '{}' not assigned", libref)))
    }

    fn register_data_step_formats(
        &self,
        ds: &datastep::ast::DataStep,
        outputs: &[(datastep::ast::TableRef, datastep::exec::WriteTarget, u64)],
    ) {
        let formats: HashMap<String, String> = ds
            .formats
            .iter()
            .map(|f| (f.name.to_ascii_lowercase(), f.format.clone()))
            .collect();
        let mut registry = self
            .dataset_formats
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for (_, target, _) in outputs {
            if let datastep::exec::WriteTarget::DuckDb { schema, name } = target {
                let key = dataset_format_key(schema, name);
                if formats.is_empty() {
                    registry.remove(&key);
                } else {
                    registry.insert(key, formats.clone());
                }
            }
        }
    }

    pub(crate) fn formats_for_dataset(&self, schema: &str, name: &str) -> HashMap<String, String> {
        self.dataset_formats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&dataset_format_key(schema, name))
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn apply_libname(
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
                    return Err(EngineError::Other(format!("not a directory: {}", def.path)));
                }
            }
        }
        let lib = Library {
            name: def.name.clone(),
            kind: def.kind,
            path: def.path.clone(),
            format: def.format,
        };
        self.libraries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(def.name.clone(), lib);
        Ok(format!(
            "Library {} assigned as {:?}{}.",
            def.name.to_uppercase(),
            def.kind,
            if def.path.is_empty() {
                String::new()
            } else {
                format!(" → {}", def.path)
            }
        ))
    }
}

pub(crate) fn dataset_format_key(schema: &str, name: &str) -> String {
    format!(
        "{}.{}",
        schema.to_ascii_lowercase(),
        name.to_ascii_lowercase()
    )
}
