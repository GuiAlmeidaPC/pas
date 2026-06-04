//! DATA step executor.
//!
//! Streams rows through the program data vector and writes output via the
//! DuckDB Appender API. Memory stays bounded by the appender batch + one
//! lookahead row (needed for `last.var` detection), regardless of input
//! row count.
//!
//! For `merge`, the per-source materialization is intentionally retained
//! — k-way streaming across multiple DuckDB cursors requires self-
//! referential lifetimes that we sidestep here. The output side of merge
//! is still streamed: merged rows flow through the same per-row pipeline.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};

use duckdb::types::Value as DV;
use duckdb::{Appender, Connection};

/// Cursor batch size used by the streaming merge. Each source holds at
/// most this many rows in memory at a time.
const MERGE_CURSOR_BATCH: usize = 4096;

use super::ast::*;
use super::funcs;
use super::DataStepError;

/// Runtime value.
#[derive(Debug, Clone)]
pub enum RtValue {
    Num(f64),
    Str(String),
}

impl RtValue {
    pub fn missing() -> Self {
        RtValue::Num(f64::NAN)
    }

    pub fn as_num(&self) -> Option<f64> {
        match self {
            RtValue::Num(n) if !n.is_nan() => Some(*n),
            RtValue::Num(_) => None,
            RtValue::Str(s) => s.trim().parse::<f64>().ok(),
        }
    }

    pub fn as_str(&self) -> String {
        match self {
            RtValue::Num(n) if n.is_nan() => String::new(),
            RtValue::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e16 {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            RtValue::Str(s) => s.clone(),
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            RtValue::Num(n) => !n.is_nan() && *n != 0.0,
            RtValue::Str(s) => !s.trim().is_empty(),
        }
    }
}

pub fn is_missing(v: &RtValue) -> bool {
    match v {
        RtValue::Num(n) => n.is_nan(),
        RtValue::Str(s) => s.trim().is_empty(),
    }
}

#[derive(Debug, Clone)]
pub enum WriteTarget {
    DuckDb { schema: String, name: String },
    Parquet { path: String, display: String },
    Csv { path: String, display: String },
}

impl WriteTarget {
    pub fn display(&self) -> String {
        match self {
            WriteTarget::DuckDb { schema, name } => {
                format!("{}.{}", schema.to_uppercase(), name.to_uppercase())
            }
            WriteTarget::Parquet { display, .. } | WriteTarget::Csv { display, .. } => {
                display.clone()
            }
        }
    }
}

pub enum ResolvedInput {
    Set(Vec<String>),
    Merge(Vec<String>),
}

pub struct ResolvedDataStep<'a> {
    pub ast: &'a DataStep,
    pub input: Option<ResolvedInput>,
    pub outputs: Vec<WriteTarget>,
}

#[derive(Debug)]
pub struct DataStepResult {
    pub outputs: Vec<(TableRef, WriteTarget, u64)>,
    pub rows_in: u64,
}

struct Pdv {
    names: Vec<String>,
    index: HashMap<String, usize>,
    is_char: Vec<bool>,
    vals: Vec<RtValue>,
    from_source: Vec<bool>,
    retained: Vec<bool>,
}

impl Pdv {
    fn new() -> Self {
        Self {
            names: Vec::new(),
            index: HashMap::new(),
            is_char: Vec::new(),
            vals: Vec::new(),
            from_source: Vec::new(),
            retained: Vec::new(),
        }
    }

    fn ensure(&mut self, name: &str, is_char: bool) -> usize {
        let lower = name.to_ascii_lowercase();
        if let Some(&i) = self.index.get(&lower) {
            return i;
        }
        let i = self.names.len();
        self.names.push(lower.clone());
        self.index.insert(lower, i);
        self.is_char.push(is_char);
        self.vals.push(if is_char {
            RtValue::Str(String::new())
        } else {
            RtValue::missing()
        });
        self.from_source.push(false);
        self.retained.push(false);
        i
    }

    fn get(&self, name: &str) -> RtValue {
        self.index
            .get(&name.to_ascii_lowercase())
            .map(|&i| self.vals[i].clone())
            .unwrap_or_else(RtValue::missing)
    }

    fn set(&mut self, name: &str, v: RtValue) {
        let lower = name.to_ascii_lowercase();
        if let Some(&i) = self.index.get(&lower) {
            self.vals[i] = coerce_to(self.is_char[i], v);
            return;
        }
        let is_char = matches!(v, RtValue::Str(_));
        let i = self.ensure(name, is_char);
        self.vals[i] = v;
    }
}

fn coerce_to(want_char: bool, v: RtValue) -> RtValue {
    match (want_char, &v) {
        (true, RtValue::Str(_)) => v,
        (true, RtValue::Num(_)) => RtValue::Str(v.as_str()),
        (false, RtValue::Num(_)) => v,
        (false, RtValue::Str(_)) => match v.as_num() {
            Some(n) => RtValue::Num(n),
            None => RtValue::missing(),
        },
    }
}

/// A single input row keyed by lowercased column name. Allocations per row
/// are unavoidable here since column sets can vary (merge / multi-source
/// set with mismatched schemas).
type SourceRow = HashMap<String, RtValue>;

struct ArrayBinding {
    elements: Vec<String>,
}

/// Output table created up front. For DIR libraries the table is a staging
/// `pas_ds_tmp_*` whose contents get COPYed out at the end.
struct WriterSpec {
    schema: String,
    name: String,
    cols: Vec<(String, bool)>,
    /// `Some(...)` when we have to COPY the staging table to a Parquet/CSV
    /// file after streaming completes.
    copy_to: Option<CopyToFile>,
    target: WriteTarget,
}

struct CopyToFile {
    path: String,
    fmt: &'static str,
}

// ── Streaming runtime ──────────────────────────────────────────────────────

/// Per-output appender + counter wired up to the writer specs. Lives only
/// for the duration of the streaming row loop.
struct OutputAppender<'conn> {
    appender: Appender<'conn>,
    cols: Vec<(String, bool)>,
    count: u64,
}

impl<'conn> OutputAppender<'conn> {
    fn append(&mut self, pdv: &Pdv) -> Result<(), DataStepError> {
        let vals: Vec<DV> = self
            .cols
            .iter()
            .map(|(name, is_char)| {
                let v = pdv.index.get(name).map(|&i| &pdv.vals[i]);
                value_for_appender(*is_char, v)
            })
            .collect();
        self.appender
            .append_row(duckdb::appender_params_from_iter(vals))?;
        self.count += 1;
        Ok(())
    }
}

/// Mutable streaming state. Holds the PDV, array bindings, output
/// appenders, and the one-row lookahead buffer that drives first./last..
struct Runtime<'a, 'conn> {
    pdv: Pdv,
    arrays: HashMap<String, ArrayBinding>,
    ds: &'a DataStep,
    appenders: Vec<OutputAppender<'conn>>,
    cancel: &'a std::sync::atomic::AtomicBool,
    rows_in: u64,
    prev_by: Option<Vec<RtValue>>,
    pending: Option<SourceRow>,
    macro_vars: &'a std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl<'a, 'conn> Runtime<'a, 'conn> {
    /// Push one row into the pipeline. Implements a 1-row lookahead so the
    /// processor can see the next row's by-values when computing `last.var`.
    fn feed(&mut self, row: SourceRow) -> Result<(), DataStepError> {
        match self.pending.take() {
            None => {
                self.pending = Some(row);
                Ok(())
            }
            Some(prev) => {
                self.process(&prev, Some(&row))?;
                self.pending = Some(row);
                Ok(())
            }
        }
    }

    /// Flush the last pending row (no lookahead).
    fn finish(&mut self) -> Result<(), DataStepError> {
        if let Some(last) = self.pending.take() {
            self.process(&last, None)?;
        }
        Ok(())
    }

    fn process(
        &mut self,
        current: &SourceRow,
        next: Option<&SourceRow>,
    ) -> Result<(), DataStepError> {
        use std::sync::atomic::Ordering;
        if self.cancel.load(Ordering::SeqCst) {
            return Err(DataStepError::runtime("cancelled"));
        }
        self.rows_in += 1;

        // Reset non-retained, non-source-bound vars.
        for i in 0..self.pdv.vals.len() {
            if !self.pdv.retained[i] && !self.pdv.from_source[i] {
                self.pdv.vals[i] = if self.pdv.is_char[i] {
                    RtValue::Str(String::new())
                } else {
                    RtValue::missing()
                };
            }
        }
        // Clear source-bound vars that the current row doesn't provide.
        for i in 0..self.pdv.names.len() {
            let name = &self.pdv.names[i];
            if self.pdv.retained[i] || !self.pdv.from_source[i] {
                continue;
            }
            if name.starts_with("first.") || name.starts_with("last.") {
                continue;
            }
            if !current.contains_key(name) {
                self.pdv.vals[i] = if self.pdv.is_char[i] {
                    RtValue::Str(String::new())
                } else {
                    RtValue::missing()
                };
            }
        }
        // Populate from row.
        for (name, val) in current.iter() {
            if let Some(&i) = self.pdv.index.get(name) {
                self.pdv.vals[i] = coerce_to(self.pdv.is_char[i], val.clone());
            }
        }

        // first./last. for by vars.
        if !self.ds.by.is_empty() {
            let this_by: Vec<RtValue> = self.ds.by.iter().map(|v| self.pdv.get(v)).collect();
            for (j, by_var) in self.ds.by.iter().enumerate() {
                let is_first = match &self.prev_by {
                    None => true,
                    Some(p) => any_changed(&p[..=j], &this_by[..=j]),
                };
                let fi = self.pdv.index[&format!("first.{}", by_var)];
                self.pdv.vals[fi] = RtValue::Num(if is_first { 1.0 } else { 0.0 });
            }
            let next_by: Option<Vec<RtValue>> = next.map(|nr| {
                self.ds
                    .by
                    .iter()
                    .map(|v| nr.get(v).cloned().unwrap_or_else(RtValue::missing))
                    .collect()
            });
            for (j, by_var) in self.ds.by.iter().enumerate() {
                let is_last = match &next_by {
                    None => true,
                    Some(nb) => any_changed(&nb[..=j], &this_by[..=j]),
                };
                let li = self.pdv.index[&format!("last.{}", by_var)];
                self.pdv.vals[li] = RtValue::Num(if is_last { 1.0 } else { 0.0 });
            }
            self.prev_by = Some(this_by);
        }

        // where filter.
        if let Some(w) = &self.ds.where_expr {
            if !eval(w, &self.pdv, &self.arrays)?.truthy() {
                return Ok(());
            }
        }

        // Body.
        let mut deleted = false;
        for s in &self.ds.body {
            match exec_stmt(
                s,
                &mut self.pdv,
                &self.arrays,
                &self.ds.outputs,
                &mut self.appenders,
                self.macro_vars,
            )? {
                StmtFlow::Continue => {}
                StmtFlow::Delete => {
                    deleted = true;
                    break;
                }
            }
        }
        if deleted {
            return Ok(());
        }

        // Emit.
        if !has_explicit_output(&self.ds.body) {
            for i in 0..self.appenders.len() {
                self.appenders[i].append(&self.pdv)?;
            }
        }
        Ok(())
    }
}

// ── Entry point ───────────────────────────────────────────────────────────

pub fn run_data_step(
    conn: &Connection,
    plan: &ResolvedDataStep,
    cancel: &std::sync::atomic::AtomicBool,
    macro_vars: &std::sync::Mutex<std::collections::HashMap<String, String>>,
) -> Result<DataStepResult, DataStepError> {
    let ds = plan.ast;
    let mut pdv = Pdv::new();

    // 1. PDV declarations from length / array / retain.
    for d in &ds.lengths {
        pdv.ensure(&d.name, d.is_char);
    }
    let mut arrays: HashMap<String, ArrayBinding> = HashMap::new();
    for a in &ds.arrays {
        let elements: Vec<String> = if a.elements.is_empty() {
            (1..=a.size).map(|i| format!("{}{}", a.name, i)).collect()
        } else {
            a.elements.iter().map(|s| s.to_ascii_lowercase()).collect()
        };
        for el in &elements {
            pdv.ensure(el, a.is_char);
        }
        arrays.insert(a.name.to_ascii_lowercase(), ArrayBinding { elements });
    }
    for r in &ds.retain {
        let i = pdv.ensure(&r.name, false);
        pdv.retained[i] = true;
        if let Some(init) = r.initial {
            pdv.vals[i] = RtValue::Num(init);
        }
    }
    if !ds.by.is_empty() {
        for v in &ds.by {
            let i_first = pdv.ensure(&format!("first.{}", v), false);
            pdv.from_source[i_first] = true;
            let i_last = pdv.ensure(&format!("last.{}", v), false);
            pdv.from_source[i_last] = true;
        }
    }
    for iv in &ds.input_vars {
        let i = pdv.ensure(&iv.name, iv.is_char);
        pdv.from_source[i] = true;
    }

    // 2. Discover source schemas via DESCRIBE (cheap, no row scan).
    let source_schemas = discover_source_schemas(conn, &plan.input)?;
    for schema in &source_schemas {
        for (name, is_char) in schema {
            let i = pdv.ensure(name, *is_char);
            pdv.from_source[i] = true;
        }
    }

    // 3. Static analysis: pre-declare body-assigned vars so the output
    //    table schema is finalized before streaming begins.
    let analyzed = analyze_body_assignments(&ds.body);
    for (name, is_char) in &analyzed {
        pdv.ensure(name, *is_char);
    }
    // do-loop counter vars are introduced by the static analyzer; mark them
    // as transient so they aren't preserved between iterations.
    // (They're already declared with `retained=false`, which is correct.)

    // 4. Create output tables (or staging tables for DIR targets).
    let writer_specs = create_output_tables(conn, &plan.outputs, &pdv, ds)?;

    // 5. Stream rows through the runtime.
    let (rows_in, counts) = {
        let mut appenders: Vec<OutputAppender> = Vec::with_capacity(writer_specs.len());
        for spec in &writer_specs {
            let app = conn.appender_to_db(&spec.name, &spec.schema)?;
            appenders.push(OutputAppender {
                appender: app,
                cols: spec.cols.clone(),
                count: 0,
            });
        }
        let mut rt = Runtime {
            pdv,
            arrays,
            ds,
            appenders,
            cancel,
            rows_in: 0,
            prev_by: None,
            pending: None,
            macro_vars,
        };
        iterate_input(conn, plan, &source_schemas, |row| rt.feed(row))?;
        rt.finish()?;
        let counts: Vec<u64> = rt.appenders.iter().map(|a| a.count).collect();
        (rt.rows_in, counts)
        // appenders dropped here → flushed automatically
    };

    // 6. Finalize: COPY staging tables to files for DIR libraries.
    let mut results = Vec::with_capacity(writer_specs.len());
    for (i, spec) in writer_specs.into_iter().enumerate() {
        if let Some(copy) = &spec.copy_to {
            let qualified = format!("\"{}\".\"{}\"", spec.schema, spec.name);
            let copy_sql = format!(
                "COPY (SELECT * FROM {}) TO '{}' (FORMAT {})",
                qualified,
                copy.path.replace('\'', "''"),
                copy.fmt
            );
            conn.execute(&copy_sql, [])?;
            conn.execute(&format!("DROP TABLE {}", qualified), [])?;
        }
        results.push((ds.outputs[i].clone(), spec.target, counts[i]));
    }
    Ok(DataStepResult {
        outputs: results,
        rows_in,
    })
}

// ── Output table setup ────────────────────────────────────────────────────

fn create_output_tables(
    conn: &Connection,
    targets: &[WriteTarget],
    pdv: &Pdv,
    ds: &DataStep,
) -> Result<Vec<WriterSpec>, DataStepError> {
    let cols = pdv_output_columns(pdv, ds);
    let mut out = Vec::with_capacity(targets.len());
    for target in targets {
        let spec = match target {
            WriteTarget::DuckDb { schema, name } => {
                create_table_with_schema(conn, schema, name, &cols)?;
                WriterSpec {
                    schema: schema.clone(),
                    name: name.clone(),
                    cols: cols.clone(),
                    copy_to: None,
                    target: target.clone(),
                }
            }
            WriteTarget::Parquet { path, .. } | WriteTarget::Csv { path, .. } => {
                let temp = format!("pas_ds_tmp_{}", uuid::Uuid::new_v4().simple());
                create_table_with_schema(conn, "main", &temp, &cols)?;
                let fmt = match target {
                    WriteTarget::Parquet { .. } => "PARQUET",
                    WriteTarget::Csv { .. } => "CSV",
                    _ => unreachable!(),
                };
                WriterSpec {
                    schema: "main".into(),
                    name: temp,
                    cols: cols.clone(),
                    copy_to: Some(CopyToFile {
                        path: path.clone(),
                        fmt,
                    }),
                    target: target.clone(),
                }
            }
        };
        out.push(spec);
    }
    Ok(out)
}

fn pdv_output_columns(pdv: &Pdv, ds: &DataStep) -> Vec<(String, bool)> {
    pdv.names
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            if n.starts_with("first.") || n.starts_with("last.") {
                return false;
            }
            if let Some(keep) = &ds.keep {
                if !keep.iter().any(|k| k.eq_ignore_ascii_case(n)) {
                    return false;
                }
            }
            if let Some(drop) = &ds.drop {
                if drop.iter().any(|d| d.eq_ignore_ascii_case(n)) {
                    return false;
                }
            }
            true
        })
        .map(|(i, n)| (n.clone(), pdv.is_char[i]))
        .collect()
}

fn create_table_with_schema(
    conn: &Connection,
    schema: &str,
    name: &str,
    cols: &[(String, bool)],
) -> Result<(), DataStepError> {
    let qualified = format!("\"{}\".\"{}\"", schema, name);
    let mut create = format!("CREATE OR REPLACE TABLE {} (", qualified);
    if cols.is_empty() {
        // DuckDB rejects empty column lists; insert a placeholder column we
        // never write to. This only happens for empty DATA steps.
        create.push_str("\"__pas_empty\" INTEGER");
    } else {
        for (i, (n, is_char)) in cols.iter().enumerate() {
            if i > 0 {
                create.push_str(", ");
            }
            create.push('"');
            create.push_str(n);
            create.push('"');
            create.push(' ');
            create.push_str(if *is_char { "VARCHAR" } else { "DOUBLE" });
        }
    }
    create.push(')');
    conn.execute(&create, [])?;
    Ok(())
}

// ── Input streaming ───────────────────────────────────────────────────────

fn discover_source_schemas(
    conn: &Connection,
    input: &Option<ResolvedInput>,
) -> Result<Vec<Vec<(String, bool)>>, DataStepError> {
    let sources: Vec<&String> = match input {
        Some(ResolvedInput::Set(s)) => s.iter().collect(),
        Some(ResolvedInput::Merge(s)) => s.iter().collect(),
        None => return Ok(Vec::new()),
    };
    let mut out = Vec::with_capacity(sources.len());
    for from in sources {
        out.push(describe_query(conn, from)?);
    }
    Ok(out)
}

fn describe_query(conn: &Connection, from: &str) -> Result<Vec<(String, bool)>, DataStepError> {
    let sql = format!("DESCRIBE SELECT * FROM {}", from);
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        let type_name: String = row.get(1)?;
        out.push((name.to_ascii_lowercase(), is_char_type(&type_name)));
    }
    Ok(out)
}

fn is_char_type(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.contains("VARCHAR")
        || upper.contains("CHAR")
        || upper.contains("TEXT")
        || upper.contains("STRING")
        || upper.contains("BLOB")
}

/// Pump rows through `visit` for the configured input. The DuckDB
/// statement / Rows iterator lives only inside this function, so its
/// lifetime never escapes.
fn iterate_input<F>(
    conn: &Connection,
    plan: &ResolvedDataStep,
    source_schemas: &[Vec<(String, bool)>],
    mut visit: F,
) -> Result<(), DataStepError>
where
    F: FnMut(SourceRow) -> Result<(), DataStepError>,
{
    let ds = plan.ast;

    match &plan.input {
        None => {
            // A sourceless DATA step still iterates once (unless datalines
            // or infile take over below).
            if ds.datalines.is_empty() && ds.infile.is_none() {
                visit(SourceRow::new())?;
            }
        }
        Some(ResolvedInput::Set(sources)) => {
            for (src_idx, from) in sources.iter().enumerate() {
                stream_set_source(conn, from, &ds.by, &source_schemas[src_idx], &mut visit)?;
            }
        }
        Some(ResolvedInput::Merge(sources)) => {
            stream_merge(conn, sources, &ds.by, source_schemas, &mut visit)?;
        }
    }

    if !ds.datalines.is_empty() && !ds.input_vars.is_empty() {
        for line in &ds.datalines {
            if let Some(row) = parse_datalines_line(line, &ds.input_vars) {
                visit(row)?;
            }
        }
    }
    if let Some(infile) = &ds.infile {
        if ds.input_vars.is_empty() {
            return Err(DataStepError::runtime("infile requires an input statement"));
        }
        stream_infile_rows(infile, &ds.input_vars, &mut visit)?;
    }

    Ok(())
}

fn stream_set_source<F>(
    conn: &Connection,
    from: &str,
    by: &[String],
    schema: &[(String, bool)],
    visit: &mut F,
) -> Result<(), DataStepError>
where
    F: FnMut(SourceRow) -> Result<(), DataStepError>,
{
    let order_sql = if by.is_empty() {
        String::new()
    } else {
        let cols: Vec<String> = by.iter().map(|v| crate::quote_ident(v)).collect();
        format!(" ORDER BY {}", cols.join(", "))
    };
    let sql = format!("SELECT * FROM {}{}", from, order_sql);
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let col_count = rows.as_ref().map(|s| s.column_count()).unwrap_or(0);
    while let Some(row) = rows.next()? {
        let mut src_row = SourceRow::with_capacity(col_count);
        for i in 0..col_count {
            let v: DV = row.get(i)?;
            let name = schema
                .get(i)
                .map(|(n, _)| n.clone())
                .unwrap_or_else(|| format!("col{}", i));
            src_row.insert(name, rt_from_duckdb(v));
        }
        visit(src_row)?;
    }
    Ok(())
}

fn parse_datalines_line(line: &str, input_vars: &[InputVar]) -> Option<SourceRow> {
    // Trim only the trailing newline/CR; leading/inner columns matter for
    // formatted (column) input.
    let line = line.trim_end_matches(['\r', '\n']);
    if line.trim().is_empty() {
        return None;
    }
    Some(read_row_columnar(line, input_vars))
}

/// Read a row using a column pointer, honoring each variable's reader:
/// formatted input consumes a fixed column width; list/modified input consumes
/// the next whitespace-delimited token.
fn read_row_columnar(line: &str, input_vars: &[InputVar]) -> SourceRow {
    let chars: Vec<char> = line.chars().collect();
    let mut pos = 0usize;
    let mut row = SourceRow::with_capacity(input_vars.len());
    for iv in input_vars {
        let key = iv.name.to_ascii_lowercase();
        let field: Option<String> = match iv.reader {
            InputReader::Formatted => {
                if pos >= chars.len() {
                    None
                } else {
                    let width = iv.informat.map(|f| f.width).unwrap_or(8);
                    let end = (pos + width).min(chars.len());
                    let s: String = chars[pos..end].iter().collect();
                    pos = end;
                    Some(s)
                }
            }
            InputReader::List | InputReader::Modified => {
                while pos < chars.len() && chars[pos].is_whitespace() {
                    pos += 1;
                }
                if pos >= chars.len() {
                    None
                } else {
                    let start = pos;
                    while pos < chars.len() && !chars[pos].is_whitespace() {
                        pos += 1;
                    }
                    Some(chars[start..pos].iter().collect())
                }
            }
        };
        row.insert(key, field_to_value(iv, field.as_deref()));
    }
    row
}

/// Convert a raw field to a value, applying the variable's informat (or the
/// default list-input typing when there is none).
fn field_to_value(iv: &InputVar, field: Option<&str>) -> RtValue {
    if let Some(inf) = &iv.informat {
        return apply_informat(inf, field);
    }
    match (iv.is_char, field) {
        (true, Some(t)) => RtValue::Str(t.trim().to_string()),
        (true, None) => RtValue::Str(String::new()),
        (false, Some(t)) if !t.trim().is_empty() => parse_num(t.trim()),
        (false, _) => RtValue::missing(),
    }
}

fn parse_num(s: &str) -> RtValue {
    match s.parse::<f64>() {
        Ok(n) => RtValue::Num(n),
        Err(_) => RtValue::missing(),
    }
}

fn apply_informat(inf: &Informat, field: Option<&str>) -> RtValue {
    let raw = match field {
        Some(s) => s,
        None => {
            return if inf.is_char() {
                RtValue::Str(String::new())
            } else {
                RtValue::missing()
            }
        }
    };
    match inf.kind {
        // $charW. preserves leading blanks; $w. left-aligns (trims leading).
        InformatKind::CharPreserve => RtValue::Str(raw.trim_end().to_string()),
        InformatKind::CharTrim => RtValue::Str(raw.trim().to_string()),
        InformatKind::Numeric => {
            let t = raw.trim();
            match parse_num(t) {
                RtValue::Num(n) if inf.decimals > 0 && !t.contains('.') => {
                    RtValue::Num(n / 10f64.powi(inf.decimals as i32))
                }
                other => other,
            }
        }
        InformatKind::Date => {
            let t = raw.trim();
            if t.is_empty() {
                RtValue::missing()
            } else {
                super::lex::parse_sas_date(t)
                    .map(RtValue::Num)
                    .unwrap_or_else(|_| RtValue::missing())
            }
        }
        InformatKind::NumericSymbol => {
            let neg = raw.contains('(');
            let cleaned: String = raw
                .chars()
                .filter(|c| !matches!(c, '$' | ',' | ' ' | '(' | ')'))
                .collect();
            match parse_num(cleaned.trim()) {
                RtValue::Num(n) if neg => RtValue::Num(-n),
                other => other,
            }
        }
    }
}

fn stream_infile_rows<F>(
    infile: &InfileSpec,
    input_vars: &[InputVar],
    visit: &mut F,
) -> Result<(), DataStepError>
where
    F: FnMut(SourceRow) -> Result<(), DataStepError>,
{
    let file = std::fs::File::open(&infile.path)
        .map_err(|e| DataStepError::runtime(format!("infile open: {}: {}", infile.path, e)))?;
    let reader = BufReader::new(file);
    let firstobs = infile.firstobs.max(1) as usize;
    for (idx, line) in reader.lines().enumerate() {
        if idx + 1 < firstobs {
            continue;
        }
        let line = line
            .map_err(|e| DataStepError::runtime(format!("infile read: {}: {}", infile.path, e)))?;
        let trimmed_line = line.trim_end_matches('\r');
        if trimmed_line.is_empty() {
            continue;
        }
        match &infile.dlm {
            // Whitespace input honors column/formatted readers.
            None => visit(read_row_columnar(trimmed_line, input_vars))?,
            Some(d) => {
                let toks: Vec<String> = if infile.dsd {
                    split_dsd(trimmed_line, d.chars().next().unwrap_or(','))
                } else {
                    trimmed_line
                        .split(d.as_str())
                        .map(|s| s.to_string())
                        .collect()
                };
                visit(input_vars_to_row(input_vars, &toks))?;
            }
        }
    }
    Ok(())
}

fn split_dsd(line: &str, dlm: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' && cur.is_empty() {
            in_quotes = true;
        } else if c == dlm {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn input_vars_to_row(input_vars: &[InputVar], toks: &[String]) -> SourceRow {
    let mut row = SourceRow::with_capacity(input_vars.len());
    for (i, iv) in input_vars.iter().enumerate() {
        let key = iv.name.to_ascii_lowercase();
        row.insert(key, field_to_value(iv, toks.get(i).map(|s| s.as_str())));
    }
    row
}

// ── Static analysis ───────────────────────────────────────────────────────

/// Walk the body collecting `(name, is_char)` for every assignment target
/// that isn't already declared by `length`, `array`, `retain`, `input`,
/// or the source schema. Type is inferred from the right-hand side.
/// Insertion order is preserved so the output table's columns appear in
/// source order, matching SAS PDV semantics.
fn analyze_body_assignments(body: &[Stmt]) -> Vec<(String, bool)> {
    let mut order: Vec<(String, bool)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    walk_stmts(body, &mut order, &mut seen);
    order
}

fn walk_stmts(
    stmts: &[Stmt],
    order: &mut Vec<(String, bool)>,
    seen: &mut std::collections::HashSet<String>,
) {
    for s in stmts {
        walk_stmt(s, order, seen);
    }
}

fn walk_stmt(
    s: &Stmt,
    order: &mut Vec<(String, bool)>,
    seen: &mut std::collections::HashSet<String>,
) {
    match s {
        Stmt::Assign {
            target: AssignTarget::Var(name),
            expr,
        } => {
            let key = name.to_ascii_lowercase();
            if seen.insert(key.clone()) {
                order.push((key, infer_expr_is_char(expr)));
            }
        }
        Stmt::Assign { .. } => {}
        Stmt::IfThen {
            then_stmt,
            else_stmt,
            ..
        } => {
            walk_stmt(then_stmt, order, seen);
            if let Some(e) = else_stmt {
                walk_stmt(e, order, seen);
            }
        }
        Stmt::Block(stmts) => walk_stmts(stmts, order, seen),
        Stmt::DoLoop { var, body, .. } => {
            let key = var.to_ascii_lowercase();
            if seen.insert(key.clone()) {
                order.push((key, false));
            }
            walk_stmts(body, order, seen);
        }
        Stmt::DoWhile { body, .. } | Stmt::DoUntil { body, .. } => {
            walk_stmts(body, order, seen);
        }
        Stmt::Select {
            branches,
            otherwise,
            ..
        } => {
            for b in branches {
                walk_stmt(&b.stmt, order, seen);
            }
            if let Some(o) = otherwise {
                walk_stmt(o, order, seen);
            }
        }
        _ => {}
    }
}

fn infer_expr_is_char(e: &Expr) -> bool {
    match e {
        Expr::StrLit(_) => true,
        Expr::Binary {
            op: BinOp::Concat, ..
        } => true,
        Expr::Call { name, .. } => is_char_returning_function(name),
        _ => false,
    }
}

fn is_char_returning_function(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "substr"
            | "upcase"
            | "lowcase"
            | "trim"
            | "strip"
            | "left"
            | "right"
            | "cats"
            | "catx"
            | "compress"
            | "compbl"
            | "tranwrd"
            | "translate"
            | "put"
            | "coalescec"
            | "propcase"
            | "reverse"
            | "repeat"
            | "scan"
            | "ifc"
            | "prxchange"
    )
}

// ── Streaming k-way merge ────────────────────────────────────────────────
//
// We don't keep DuckDB Rows iterators alive across the merge — the
// self-referential lifetimes are awkward. Instead, each source is
// snapshotted into a sorted TEMP table once, and a paged cursor refills
// a small `VecDeque` from that temp table on demand. Memory per cursor
// stays bounded by `MERGE_CURSOR_BATCH`; DuckDB owns the sort buffer
// and spills to disk if needed.

struct MergeCursor {
    table: String,
    schema: Vec<(String, bool)>,
    buffer: VecDeque<SourceRow>,
    offset: usize,
    exhausted: bool,
}

impl MergeCursor {
    fn refill(&mut self, conn: &Connection) -> Result<(), DataStepError> {
        if self.exhausted {
            return Ok(());
        }
        let sql = format!(
            "SELECT * FROM \"main\".\"{}\" LIMIT {} OFFSET {}",
            self.table, MERGE_CURSOR_BATCH, self.offset
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        let col_count = rows.as_ref().map(|s| s.column_count()).unwrap_or(0);
        let mut filled = 0usize;
        while let Some(row) = rows.next()? {
            let mut src_row = SourceRow::with_capacity(col_count);
            for i in 0..col_count {
                let v: DV = row.get(i)?;
                let name = self
                    .schema
                    .get(i)
                    .map(|(n, _)| n.clone())
                    .unwrap_or_else(|| format!("col{}", i));
                src_row.insert(name, rt_from_duckdb(v));
            }
            self.buffer.push_back(src_row);
            filled += 1;
        }
        self.offset += filled;
        if filled == 0 {
            self.exhausted = true;
        }
        Ok(())
    }

    fn peek(&mut self, conn: &Connection) -> Result<Option<&SourceRow>, DataStepError> {
        if self.buffer.is_empty() {
            self.refill(conn)?;
        }
        Ok(self.buffer.front())
    }

    fn pop(&mut self, conn: &Connection) -> Result<Option<SourceRow>, DataStepError> {
        if self.buffer.is_empty() {
            self.refill(conn)?;
        }
        Ok(self.buffer.pop_front())
    }

    fn extract_key(row: &SourceRow, by: &[String]) -> Vec<RtValue> {
        by.iter()
            .map(|b| {
                row.get(&b.to_ascii_lowercase())
                    .cloned()
                    .unwrap_or_else(RtValue::missing)
            })
            .collect()
    }
}

fn stream_merge<F>(
    conn: &Connection,
    sources: &[String],
    by: &[String],
    source_schemas: &[Vec<(String, bool)>],
    visit: &mut F,
) -> Result<(), DataStepError>
where
    F: FnMut(SourceRow) -> Result<(), DataStepError>,
{
    if by.is_empty() {
        return Err(DataStepError::runtime("merge requires a `by` statement"));
    }
    let order_cols: Vec<String> = by.iter().map(|v| crate::quote_ident(v)).collect();
    let order_clause = order_cols.join(", ");

    // 1. Snapshot each source into a TEMP table sorted by the by-vars.
    let mut cursors: Vec<MergeCursor> = Vec::with_capacity(sources.len());
    let mut temp_names: Vec<String> = Vec::with_capacity(sources.len());
    for (i, from) in sources.iter().enumerate() {
        let temp = format!("pas_merge_tmp_{}", uuid::Uuid::new_v4().simple());
        let create = format!(
            "CREATE OR REPLACE TEMP TABLE \"{}\" AS SELECT * FROM {} ORDER BY {}",
            temp, from, order_clause
        );
        conn.execute(&create, [])?;
        let schema = source_schemas.get(i).cloned().unwrap_or_default();
        cursors.push(MergeCursor {
            table: temp.clone(),
            schema,
            buffer: VecDeque::new(),
            offset: 0,
            exhausted: false,
        });
        temp_names.push(temp);
    }

    // 2. K-way merge. Each iteration picks the smallest current key
    //    across all cursors, gathers the matching group from each, and
    //    emits the broadcast cross-product (shorter sides padded to the
    //    longest with their last row in the group).
    let merge_result = (|| -> Result<(), DataStepError> {
        loop {
            // Find smallest current key.
            let mut min_key: Option<Vec<RtValue>> = None;
            for cursor in cursors.iter_mut() {
                if let Some(row) = cursor.peek(conn)? {
                    let key = MergeCursor::extract_key(row, by);
                    min_key = Some(match min_key {
                        None => key,
                        Some(mk) if compare_keys(&key, &mk) < 0 => key,
                        Some(mk) => mk,
                    });
                }
            }
            let Some(group_key) = min_key else { break };

            // Drain matching rows from each cursor.
            let mut group_per_src: Vec<Vec<SourceRow>> = Vec::with_capacity(cursors.len());
            for cursor in cursors.iter_mut() {
                let mut group: Vec<SourceRow> = Vec::new();
                loop {
                    let matches = match cursor.peek(conn)? {
                        Some(row) => {
                            compare_keys(&MergeCursor::extract_key(row, by), &group_key) == 0
                        }
                        None => false,
                    };
                    if !matches {
                        break;
                    }
                    if let Some(row) = cursor.pop(conn)? {
                        group.push(row);
                    }
                }
                group_per_src.push(group);
            }

            let group_size = group_per_src.iter().map(|g| g.len()).max().unwrap_or(0);
            for r in 0..group_size {
                let mut merged = SourceRow::new();
                for group in &group_per_src {
                    if group.is_empty() {
                        continue;
                    }
                    let row = &group[r.min(group.len() - 1)];
                    for (k, v) in row {
                        merged.insert(k.clone(), v.clone());
                    }
                }
                for (j, var) in by.iter().enumerate() {
                    merged.insert(var.to_ascii_lowercase(), group_key[j].clone());
                }
                visit(merged)?;
            }
        }
        Ok(())
    })();

    // 3. Best-effort cleanup; ignore drop errors so we never mask the
    //    real error from merge_result.
    for temp in &temp_names {
        let _ = conn.execute(&format!("DROP TABLE IF EXISTS \"main\".\"{}\"", temp), []);
    }
    merge_result
}

fn compare_keys(a: &[RtValue], b: &[RtValue]) -> i32 {
    for i in 0..a.len().max(b.len()) {
        let av = a.get(i).cloned().unwrap_or_else(RtValue::missing);
        let bv = b.get(i).cloned().unwrap_or_else(RtValue::missing);
        let c = compare(&av, &bv);
        if c != 0 {
            return c;
        }
    }
    0
}

// ── Statement execution ───────────────────────────────────────────────────

fn any_changed(a: &[RtValue], b: &[RtValue]) -> bool {
    a.len() != b.len() || a.iter().zip(b.iter()).any(|(x, y)| !values_equal(x, y))
}

fn values_equal(a: &RtValue, b: &RtValue) -> bool {
    match (a.as_num(), b.as_num()) {
        (Some(x), Some(y)) => x == y,
        _ => a.as_str() == b.as_str(),
    }
}

enum StmtFlow {
    Continue,
    Delete,
}

fn exec_stmt<'conn>(
    s: &Stmt,
    pdv: &mut Pdv,
    arrays: &HashMap<String, ArrayBinding>,
    outs: &[TableRef],
    appenders: &mut [OutputAppender<'conn>],
    macro_vars: &std::sync::Mutex<std::collections::HashMap<String, String>>,
) -> Result<StmtFlow, DataStepError> {
    match s {
        Stmt::Assign { target, expr } => {
            let v = eval(expr, pdv, arrays)?;
            match target {
                AssignTarget::Var(name) => pdv.set(name, v),
                AssignTarget::ArrayElem { name, index } => {
                    let element = resolve_array_index(name, index, pdv, arrays)?;
                    pdv.set(&element, v);
                }
            }
            Ok(StmtFlow::Continue)
        }
        Stmt::IfThen {
            cond,
            then_stmt,
            else_stmt,
        } => {
            let v = eval(cond, pdv, arrays)?;
            if v.truthy() {
                exec_stmt(then_stmt, pdv, arrays, outs, appenders, macro_vars)
            } else if let Some(e) = else_stmt {
                exec_stmt(e, pdv, arrays, outs, appenders, macro_vars)
            } else {
                Ok(StmtFlow::Continue)
            }
        }
        Stmt::SubsetIf { cond } => {
            let v = eval(cond, pdv, arrays)?;
            if v.truthy() {
                Ok(StmtFlow::Continue)
            } else {
                Ok(StmtFlow::Delete)
            }
        }
        Stmt::Output { dataset } => {
            match dataset {
                None => {
                    for app in appenders.iter_mut() {
                        app.append(pdv)?;
                    }
                }
                Some(t) => {
                    let idx = outs
                        .iter()
                        .position(|o| o.name.eq_ignore_ascii_case(&t.name))
                        .ok_or_else(|| {
                            DataStepError::runtime(format!(
                                "`output {}` refers to a dataset not listed on the DATA statement",
                                t.qualified()
                            ))
                        })?;
                    if let Some(app) = appenders.get_mut(idx) {
                        app.append(pdv)?;
                    }
                }
            }
            Ok(StmtFlow::Continue)
        }
        Stmt::Delete => Ok(StmtFlow::Delete),
        Stmt::Block(stmts) => {
            for s in stmts {
                match exec_stmt(s, pdv, arrays, outs, appenders, macro_vars)? {
                    StmtFlow::Continue => {}
                    StmtFlow::Delete => return Ok(StmtFlow::Delete),
                }
            }
            Ok(StmtFlow::Continue)
        }
        Stmt::Select {
            switch,
            branches,
            otherwise,
        } => {
            match switch {
                Some(sw) => {
                    let switch_val = eval(sw, pdv, arrays)?;
                    for branch in branches {
                        for v in &branch.values {
                            let candidate = eval(v, pdv, arrays)?;
                            if compare(&switch_val, &candidate) == 0 {
                                return exec_stmt(
                                    &branch.stmt,
                                    pdv,
                                    arrays,
                                    outs,
                                    appenders,
                                    macro_vars,
                                );
                            }
                        }
                    }
                }
                None => {
                    for branch in branches {
                        for v in &branch.values {
                            if eval(v, pdv, arrays)?.truthy() {
                                return exec_stmt(
                                    &branch.stmt,
                                    pdv,
                                    arrays,
                                    outs,
                                    appenders,
                                    macro_vars,
                                );
                            }
                        }
                    }
                }
            }
            if let Some(o) = otherwise {
                return exec_stmt(o, pdv, arrays, outs, appenders, macro_vars);
            }
            Ok(StmtFlow::Continue)
        }
        Stmt::DoLoop {
            var,
            start,
            stop,
            step,
            body,
        } => {
            let start_v = eval(start, pdv, arrays)?
                .as_num()
                .ok_or_else(|| DataStepError::runtime("do loop start is missing"))?;
            let stop_v = eval(stop, pdv, arrays)?
                .as_num()
                .ok_or_else(|| DataStepError::runtime("do loop stop is missing"))?;
            let step_v = match step {
                Some(e) => eval(e, pdv, arrays)?
                    .as_num()
                    .ok_or_else(|| DataStepError::runtime("do loop step is missing"))?,
                None => 1.0,
            };
            if step_v == 0.0 {
                return Err(DataStepError::runtime("do loop step cannot be 0"));
            }
            let ascending = step_v > 0.0;
            let mut i = start_v;
            loop {
                if (ascending && i > stop_v) || (!ascending && i < stop_v) {
                    break;
                }
                pdv.set(var, RtValue::Num(i));
                for s in body {
                    match exec_stmt(s, pdv, arrays, outs, appenders, macro_vars)? {
                        StmtFlow::Continue => {}
                        StmtFlow::Delete => return Ok(StmtFlow::Delete),
                    }
                }
                i += step_v;
            }
            Ok(StmtFlow::Continue)
        }
        Stmt::DoWhile { cond, body } => loop {
            let c = eval(cond, pdv, arrays)?
                .as_num()
                .ok_or_else(|| DataStepError::runtime("do while condition is missing"))?;
            if c == 0.0 {
                break Ok(StmtFlow::Continue);
            }
            for s in body {
                match exec_stmt(s, pdv, arrays, outs, appenders, macro_vars)? {
                    StmtFlow::Continue => {}
                    StmtFlow::Delete => return Ok(StmtFlow::Delete),
                }
            }
        },
        Stmt::DoUntil { cond, body } => loop {
            for s in body {
                match exec_stmt(s, pdv, arrays, outs, appenders, macro_vars)? {
                    StmtFlow::Continue => {}
                    StmtFlow::Delete => return Ok(StmtFlow::Delete),
                }
            }
            let c = eval(cond, pdv, arrays)?
                .as_num()
                .ok_or_else(|| DataStepError::runtime("do until condition is missing"))?;
            if c != 0.0 {
                break Ok(StmtFlow::Continue);
            }
        },
        Stmt::Call { name, args } => {
            if name.eq_ignore_ascii_case("symput") || name.eq_ignore_ascii_case("symputx") {
                if args.len() != 2 {
                    return Err(DataStepError::runtime(format!(
                        "CALL {} requires exactly 2 arguments, got {}",
                        name,
                        args.len()
                    )));
                }
                let name_val = eval(&args[0], pdv, arrays)?;
                let val_val = eval(&args[1], pdv, arrays)?;
                let var_name = name_val.as_str().trim().to_string();
                if var_name.is_empty() {
                    return Err(DataStepError::runtime(format!(
                        "CALL {} first argument (macro variable name) cannot be empty",
                        name
                    )));
                }
                let var_value = if name.eq_ignore_ascii_case("symputx") {
                    val_val.as_str().trim().to_string()
                } else {
                    val_val.as_str()
                };
                let mut vars = macro_vars.lock().unwrap();
                vars.insert(var_name, var_value);
            } else {
                return Err(DataStepError::runtime(format!(
                    "unknown CALL routine '{}'",
                    name
                )));
            }
            Ok(StmtFlow::Continue)
        }
    }
}

fn has_explicit_output(stmts: &[Stmt]) -> bool {
    for s in stmts {
        if has_explicit_stmt_output(s) {
            return true;
        }
    }
    false
}

fn has_explicit_stmt_output(s: &Stmt) -> bool {
    match s {
        Stmt::Output { .. } => true,
        Stmt::IfThen {
            then_stmt,
            else_stmt,
            ..
        } => {
            has_explicit_stmt_output(then_stmt)
                || else_stmt
                    .as_ref()
                    .map(|e| has_explicit_stmt_output(e))
                    .unwrap_or(false)
        }
        Stmt::Block(inner) => has_explicit_output(inner),
        Stmt::DoLoop { body, .. } => has_explicit_output(body),
        Stmt::DoWhile { body, .. } | Stmt::DoUntil { body, .. } => has_explicit_output(body),
        Stmt::Select {
            branches,
            otherwise,
            ..
        } => {
            branches.iter().any(|b| has_explicit_stmt_output(&b.stmt))
                || otherwise
                    .as_ref()
                    .map(|o| has_explicit_stmt_output(o))
                    .unwrap_or(false)
        }
        _ => false,
    }
}

/// Attach a span to a runtime error if it doesn't already have one.
fn with_span(e: DataStepError, span: super::lex::Span) -> DataStepError {
    match e {
        DataStepError::Runtime(msg, None) => DataStepError::Runtime(msg, Some(span)),
        other => other,
    }
}

fn resolve_array_index(
    name: &str,
    index: &Expr,
    pdv: &Pdv,
    arrays: &HashMap<String, ArrayBinding>,
) -> Result<String, DataStepError> {
    let arr = arrays
        .get(&name.to_ascii_lowercase())
        .ok_or_else(|| DataStepError::runtime(format!("unknown array '{}'", name)))?;
    let idx_v = eval(index, pdv, arrays)?;
    let idx = idx_v
        .as_num()
        .ok_or_else(|| DataStepError::runtime(format!("array '{}' index is missing", name)))?;
    let i = idx as isize;
    if i < 1 || (i as usize) > arr.elements.len() {
        return Err(DataStepError::runtime(format!(
            "array '{}' index {} out of range (1..{})",
            name,
            i,
            arr.elements.len()
        )));
    }
    Ok(arr.elements[i as usize - 1].clone())
}

fn eval(
    e: &Expr,
    pdv: &Pdv,
    arrays: &HashMap<String, ArrayBinding>,
) -> Result<RtValue, DataStepError> {
    use BinOp::*;
    use UnaryOp::*;
    Ok(match e {
        Expr::NumLit(n) => RtValue::Num(*n),
        Expr::StrLit(s) => RtValue::Str(s.clone()),
        Expr::Ident(name) => pdv.get(name),
        Expr::Call { name, args, span } => {
            let evaluated: Vec<RtValue> = args
                .iter()
                .map(|a| eval(a, pdv, arrays))
                .collect::<Result<_, _>>()?;
            funcs::call(name, &evaluated).map_err(|m| DataStepError::runtime_at(m, *span))?
        }
        Expr::ArrayRef { name, index, span } => {
            let element =
                resolve_array_index(name, index, pdv, arrays).map_err(|e| with_span(e, *span))?;
            pdv.get(&element)
        }
        Expr::Unary { op, expr } => {
            let v = eval(expr, pdv, arrays)?;
            match op {
                Neg => match v.as_num() {
                    Some(n) => RtValue::Num(-n),
                    None => RtValue::missing(),
                },
                Not => RtValue::Num(if v.truthy() { 0.0 } else { 1.0 }),
            }
        }
        Expr::Binary { op, lhs, rhs } => {
            let l = eval(lhs, pdv, arrays)?;
            let r = eval(rhs, pdv, arrays)?;
            match op {
                Concat => RtValue::Str(format!("{}{}", l.as_str(), r.as_str())),
                Add | Sub | Mul | Div | Pow | Mod => {
                    let (a, b) = match (l.as_num(), r.as_num()) {
                        (Some(a), Some(b)) => (a, b),
                        _ => return Ok(RtValue::missing()),
                    };
                    let n = match op {
                        Add => a + b,
                        Sub => a - b,
                        Mul => a * b,
                        Div => {
                            if b == 0.0 {
                                f64::NAN
                            } else {
                                a / b
                            }
                        }
                        Pow => a.powf(b),
                        Mod => {
                            if b == 0.0 {
                                f64::NAN
                            } else {
                                a - (a / b).trunc() * b
                            }
                        }
                        _ => unreachable!(),
                    };
                    RtValue::Num(n)
                }
                Eq | Ne | Lt | Le | Gt | Ge => {
                    let cmp = compare(&l, &r);
                    let result = match op {
                        Eq => cmp == 0,
                        Ne => cmp != 0,
                        Lt => cmp < 0,
                        Le => cmp <= 0,
                        Gt => cmp > 0,
                        Ge => cmp >= 0,
                        _ => unreachable!(),
                    };
                    RtValue::Num(if result { 1.0 } else { 0.0 })
                }
                And => RtValue::Num(if l.truthy() && r.truthy() { 1.0 } else { 0.0 }),
                Or => RtValue::Num(if l.truthy() || r.truthy() { 1.0 } else { 0.0 }),
            }
        }
    })
}

fn compare(a: &RtValue, b: &RtValue) -> i32 {
    if let (Some(x), Some(y)) = (a.as_num(), b.as_num()) {
        if x < y {
            -1
        } else if x > y {
            1
        } else {
            0
        }
    } else {
        let sa = a.as_str();
        let sb = b.as_str();
        sa.cmp(&sb) as i32
    }
}

fn rt_from_duckdb(v: DV) -> RtValue {
    match v {
        DV::Null => RtValue::missing(),
        DV::Boolean(b) => RtValue::Num(if b { 1.0 } else { 0.0 }),
        DV::TinyInt(i) => RtValue::Num(i as f64),
        DV::SmallInt(i) => RtValue::Num(i as f64),
        DV::Int(i) => RtValue::Num(i as f64),
        DV::BigInt(i) => RtValue::Num(i as f64),
        DV::UTinyInt(i) => RtValue::Num(i as f64),
        DV::USmallInt(i) => RtValue::Num(i as f64),
        DV::UInt(i) => RtValue::Num(i as f64),
        DV::UBigInt(i) => RtValue::Num(i as f64),
        DV::Float(f) => RtValue::Num(f as f64),
        DV::Double(f) => RtValue::Num(f),
        DV::Text(s) => RtValue::Str(s),
        other => RtValue::Str(format!("{:?}", other)),
    }
}

fn value_for_appender(is_char: bool, v: Option<&RtValue>) -> DV {
    match (is_char, v) {
        (true, Some(RtValue::Str(s))) => DV::Text(s.clone()),
        (true, Some(RtValue::Num(n))) if !n.is_nan() => DV::Text(RtValue::Num(*n).as_str()),
        (true, _) => DV::Text(String::new()),
        (false, Some(RtValue::Num(n))) if !n.is_nan() => DV::Double(*n),
        (false, Some(RtValue::Str(s))) => {
            s.trim().parse::<f64>().map(DV::Double).unwrap_or(DV::Null)
        }
        (false, _) => DV::Null,
    }
}

#[cfg(test)]
mod input_tests {
    use super::super::parse::parse_data_step_with_datalines;
    use super::*;

    fn row(line: &str, vars: &[InputVar]) -> SourceRow {
        read_row_columnar(line, vars)
    }

    fn vars_from(input: &str) -> Vec<InputVar> {
        // Parse just an input statement via a minimal data step.
        let src = format!("data t; {} ; run;", input);
        parse_data_step_with_datalines(&src, vec![])
            .expect("parse")
            .input_vars
    }

    #[test]
    fn modified_list_date_informat() {
        let vars = vars_from("input id name $ d :date9. amt");
        // id(list) name(list char) d(:date9.) amt(list)
        let r = row("7 Grace 22JUL2019 95000", &vars);
        assert_eq!(r["id"].as_num(), Some(7.0));
        assert_eq!(r["name"].as_str(), "Grace");
        // 22JUL2019 → SAS date serial.
        assert_eq!(r["d"].as_num(), Some(21752.0));
        assert_eq!(r["amt"].as_num(), Some(95000.0));
    }

    #[test]
    fn formatted_char_reads_embedded_spaces() {
        // emp_id list, then a 40-column $char field starting at the blank in col 4.
        // Field cols 4..43 hold " Jane Doe" + padding; dept_id begins at col 44.
        let name_field = format!(" {:<39}", "Jane Doe"); // 40 chars total
        let line = format!("101{}10", name_field);
        let vars = vars_from("input emp_id name $char40. dept_id");
        let r = row(&line, &vars);
        assert_eq!(r["emp_id"].as_num(), Some(101.0));
        // $char preserves the leading blank from the pointer position.
        assert_eq!(r["name"].as_str(), " Jane Doe");
        assert_eq!(r["dept_id"].as_num(), Some(10.0));
    }

    #[test]
    fn dollar_trim_informat_left_aligns() {
        let vars = vars_from("input emp_id name $40. dept_id");
        let name_field = format!(" {:<39}", "Jane Doe");
        let line = format!("101{}10", name_field);
        let r = row(&line, &vars);
        // $w. left-aligns (trims leading blanks).
        assert_eq!(r["name"].as_str(), "Jane Doe");
    }

    #[test]
    fn numeric_symbol_informat_strips() {
        let vars = vars_from("input x :dollar12.2 y :comma8.");
        let r = row("$1,234.50 2,000", &vars);
        assert_eq!(r["x"].as_num(), Some(1234.50));
        assert_eq!(r["y"].as_num(), Some(2000.0));
    }

    #[test]
    fn unsupported_informat_is_an_error() {
        let src = "data t; input x :weird9.; run;";
        let err = parse_data_step_with_datalines(src, vec![]).unwrap_err();
        assert!(
            err.message.contains("unsupported informat"),
            "{}",
            err.message
        );
    }
}

#[cfg(test)]
pub(crate) fn test_eval_expression(
    expr_str: &str,
    vars: &HashMap<String, RtValue>,
) -> Result<RtValue, super::DataStepError> {
    let parsed_expr = super::parse::parse_expr_for_test(expr_str)
        .map_err(|e| super::DataStepError::Parse(e.message))?;
    let mut pdv = Pdv::new();
    for (name, val) in vars {
        pdv.set(name, val.clone());
    }
    let arrays = HashMap::new();
    eval(&parsed_expr, &pdv, &arrays)
}
