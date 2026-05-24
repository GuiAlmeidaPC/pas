use crate::library::{DirFormat, Library, LibraryKind};
use crate::query::{duckdb_error_span, materialize_select_into, run_one, run_query, StmtResult};
use crate::rewrite::dir_reader_expr;
use crate::split::{split_blocks, strip_comments, Block};
use crate::types::{Event, SourceSpan, Value};
use crate::{datastep, libname, procs, sas_sql, split, EngineError, MAX_PREVIEW_ROWS};
use duckdb::{Connection, InterruptHandle};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub struct Session {
    pub(crate) conn: Mutex<Connection>,
    pub(crate) read_conn: Mutex<Connection>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) interrupt: Arc<InterruptHandle>,
    pub(crate) libraries: Mutex<HashMap<String, Library>>,
    pub(crate) macro_vars: Mutex<HashMap<String, String>>,
    pub(crate) macro_defs: Mutex<HashMap<String, crate::macros::MacroDef>>,
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
        let mut v: Vec<Library> = self.libraries.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    /// Run an entire program, returning all events.
    pub fn submit(&self, program: &str) -> Vec<Event> {
        self.cancel.store(false, Ordering::SeqCst);
        let cleaned = strip_comments(program);

        let blocks = split_blocks(&cleaned);
        tracing::debug!(block_count = blocks.len(), "program split into blocks");
        let mut events = Vec::new();

        if blocks.is_empty() {
            events.push(Event::Note {
                text: "No statements found.".into(),
            });
            events.push(Event::Done);
            return events;
        }

        let conn = self.conn.lock().expect("engine mutex poisoned");
        for block in blocks {
            if self.cancel.load(Ordering::SeqCst) {
                events.push(Event::Warning {
                    text: "Execution cancelled by user.".into(),
                });
                break;
            }

            match block {
                Block::Statement {
                    text,
                    src_offset: _,
                } => {
                    let macro_result = {
                        let mut vars = self.macro_vars.lock().unwrap();
                        let mut defs = self.macro_defs.lock().unwrap();
                        crate::macros::preprocess(&text, &mut vars, &mut defs)
                    };

                    tracing::debug!(
                        statement = %text,
                        expanded = %macro_result.expanded,
                        puts = ?macro_result.puts,
                        "macro preprocessing complete",
                    );

                    for put_text in macro_result.puts {
                        events.push(Event::Note { text: put_text });
                    }

                    let expanded_trimmed = macro_result.expanded.trim();
                    if expanded_trimmed.is_empty() {
                        continue;
                    }

                    let sub_blocks = split_blocks(&macro_result.expanded);
                    tracing::debug!(sub_blocks = ?sub_blocks, "expanded statement split");
                    for sub_block in sub_blocks {
                        if self.cancel.load(Ordering::SeqCst) {
                            events.push(Event::Warning {
                                text: "Execution cancelled by user.".into(),
                            });
                            break;
                        }

                        match sub_block {
                            Block::Statement {
                                text: sub_text,
                                src_offset: sub_offset,
                            } => {
                                events.push(Event::Source {
                                    text: sub_text.clone(),
                                });
                                if let Some(handled) = self.try_libname(&conn, &sub_text) {
                                    events.extend(handled);
                                    continue;
                                }
                                self.run_sql_with_rewrites(
                                    &conn,
                                    &sub_text,
                                    sub_offset,
                                    &macro_result.expanded,
                                    &mut events,
                                );
                            }
                            Block::ProcSqlStmt {
                                text: sub_text,
                                src_offset: sub_offset,
                            } => {
                                events.push(Event::Source {
                                    text: sub_text.clone(),
                                });
                                self.run_sql_with_rewrites(
                                    &conn,
                                    &sub_text,
                                    sub_offset,
                                    &macro_result.expanded,
                                    &mut events,
                                );
                            }
                            Block::DataStep {
                                body: sub_body,
                                datalines: sub_datalines,
                                body_src_offset: sub_offset,
                            } => {
                                events.push(Event::Source {
                                    text: sub_body.clone(),
                                });
                                self.run_data_step(
                                    &conn,
                                    &sub_body,
                                    sub_datalines,
                                    sub_offset,
                                    &macro_result.expanded,
                                    &mut events,
                                );
                            }
                            Block::Proc {
                                name: sub_name,
                                body: sub_body,
                                ..
                            } => {
                                events.push(Event::Source {
                                    text: format!("proc {}; {} run;", sub_name, sub_body),
                                });
                                self.run_proc(&conn, &sub_name, &sub_body, &mut events);
                            }
                        }
                    }
                }
                Block::ProcSqlStmt { text, src_offset } => {
                    let macro_result = {
                        let mut vars = self.macro_vars.lock().unwrap();
                        let mut defs = self.macro_defs.lock().unwrap();
                        crate::macros::preprocess(&text, &mut vars, &mut defs)
                    };

                    for put_text in macro_result.puts {
                        events.push(Event::Note { text: put_text });
                    }

                    let expanded_trimmed = macro_result.expanded.trim();
                    if expanded_trimmed.is_empty() {
                        continue;
                    }

                    events.push(Event::Source {
                        text: expanded_trimmed.to_string(),
                    });
                    self.run_sql_with_rewrites(
                        &conn,
                        expanded_trimmed,
                        src_offset,
                        program,
                        &mut events,
                    );
                }
                Block::DataStep {
                    body,
                    datalines,
                    body_src_offset,
                } => {
                    let macro_result = {
                        let mut vars = self.macro_vars.lock().unwrap();
                        let mut defs = self.macro_defs.lock().unwrap();
                        crate::macros::preprocess(&body, &mut vars, &mut defs)
                    };

                    for put_text in macro_result.puts {
                        events.push(Event::Note { text: put_text });
                    }

                    let expanded = macro_result.expanded;
                    events.push(Event::Source {
                        text: expanded.clone(),
                    });
                    self.run_data_step(
                        &conn,
                        &expanded,
                        datalines,
                        body_src_offset,
                        program,
                        &mut events,
                    );
                }
                Block::Proc {
                    name,
                    body,
                    src_offset: _,
                } => {
                    let raw_proc = format!("proc {}; {} run;", name, body);
                    let macro_result = {
                        let mut vars = self.macro_vars.lock().unwrap();
                        let mut defs = self.macro_defs.lock().unwrap();
                        crate::macros::preprocess(&raw_proc, &mut vars, &mut defs)
                    };

                    for put_text in macro_result.puts {
                        events.push(Event::Note { text: put_text });
                    }

                    let expanded_trimmed = macro_result.expanded.trim();
                    if expanded_trimmed.is_empty() {
                        continue;
                    }

                    let sub_blocks = split_blocks(&macro_result.expanded);
                    for sub_block in sub_blocks {
                        if self.cancel.load(Ordering::SeqCst) {
                            events.push(Event::Warning {
                                text: "Execution cancelled by user.".into(),
                            });
                            break;
                        }

                        if let Block::Proc {
                            name: sub_name,
                            body: sub_body,
                            ..
                        } = sub_block
                        {
                            events.push(Event::Source {
                                text: format!("proc {}; {} run;", sub_name, sub_body),
                            });
                            self.run_proc(&conn, &sub_name, &sub_body, &mut events);
                        }
                    }
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
        let (clean_query, targets) = sas_sql::extract_into_clause(&rewritten);
        match run_one(conn, &clean_query, 1000) {
            Ok(StmtResult::Rows(block)) => {
                if !targets.is_empty() {
                    if let Some(first_row) = block.rows.first() {
                        let mut vars = self.macro_vars.lock().unwrap();
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
                    .map(|t| self.resolve_read(t))
                    .collect::<Result<Vec<_>, _>>()
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
                    .map(|t| self.resolve_read(t))
                    .collect::<Result<Vec<_>, _>>()
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
            .unwrap()
            .get(&libref.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| EngineError::Other(format!("library '{}' not assigned", libref)))
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
        self.libraries.lock().unwrap().insert(def.name.clone(), lib);
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
