//! DATA step executor — v0.4.
//!
//! Supports: `set` (1+ sources, concatenated), `merge` (match-merge with `by`),
//! `by` + `first./last.`, `retain`, `array` declarations + indexed refs,
//! iterative `do var = a to b [by c]`, plus everything from v0.3.

use std::collections::HashMap;

use duckdb::types::Value as DV;
use duckdb::Connection;

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
    pub fn missing() -> Self { RtValue::Num(f64::NAN) }

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
            WriteTarget::Parquet { display, .. } | WriteTarget::Csv { display, .. } => display.clone(),
        }
    }
}

/// Input resolved to concrete SQL FROM-expressions.
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
    /// Per-var: comes from a source row (not derived). Used for reset
    /// semantics. Auto vars (first./last.) are also marked as "source"
    /// because the executor sets them directly each iteration.
    from_source: Vec<bool>,
    /// Per-var: retain — don't reset between iterations.
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
        self.vals.push(if is_char { RtValue::Str(String::new()) } else { RtValue::missing() });
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

/// One row sourced from an input — sparse map keyed by lowercased column name.
type SourceRow = HashMap<String, RtValue>;

struct ArrayBinding {
    /// Lowercase element variable names.
    elements: Vec<String>,
}

pub fn run_data_step(
    conn: &Connection,
    plan: &ResolvedDataStep,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<DataStepResult, DataStepError> {
    use std::sync::atomic::Ordering;

    let ds = plan.ast;
    let mut pdv = Pdv::new();

    // 1. Pre-declare PDV layout from length / retain / array declarations.
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
    // 2. Retain (and apply initial values once).
    for r in &ds.retain {
        let i = pdv.ensure(&r.name, false);
        pdv.retained[i] = true;
        if let Some(init) = r.initial {
            pdv.vals[i] = RtValue::Num(init);
        }
    }
    // Variables in `by` get first./last. auto-vars — declared lazily once
    // we know we'll actually be iterating with `by`.
    let by_active = !ds.by.is_empty();
    if by_active {
        for v in &ds.by {
            let i_first = pdv.ensure(&format!("first.{}", v), false);
            pdv.from_source[i_first] = true;
            let i_last = pdv.ensure(&format!("last.{}", v), false);
            pdv.from_source[i_last] = true;
        }
    }

    // 3. Pre-declare input vars from the `input` statement so they appear
    //    in PDV order regardless of whether datalines lines populate them.
    for iv in &ds.input_vars {
        let i = pdv.ensure(&iv.name, iv.is_char);
        pdv.from_source[i] = true;
    }

    // 4. Load + materialize input rows.
    let input_rows: Vec<SourceRow> = if let Some(infile) = &ds.infile {
        if ds.input_vars.is_empty() {
            return Err(DataStepError::Runtime(
                "infile requires an input statement".into(),
            ));
        }
        rows_from_infile(infile, &ds.input_vars)?
    } else if !ds.datalines.is_empty() && !ds.input_vars.is_empty() {
        rows_from_datalines(&ds.datalines, &ds.input_vars)
    } else { match &plan.input {
        None => vec![SourceRow::new()],
        Some(ResolvedInput::Set(sources)) => {
            let mut all = Vec::new();
            for from in sources {
                let (cols, rows) = load_source(conn, from, &ds.by)?;
                for (name, is_char) in &cols {
                    let i = pdv.ensure(name, *is_char);
                    pdv.from_source[i] = true;
                }
                for raw in rows {
                    let mut m = SourceRow::new();
                    for ((name, _), val) in cols.iter().zip(raw.into_iter()) {
                        m.insert(name.clone(), val);
                    }
                    all.push(m);
                }
            }
            all
        }
        Some(ResolvedInput::Merge(sources)) => {
            // Load each source sorted by `by` vars.
            let mut per_source: Vec<(Vec<(String, bool)>, Vec<Vec<RtValue>>)> = Vec::new();
            for from in sources {
                per_source.push(load_source(conn, from, &ds.by)?);
            }
            // Register all columns in the PDV.
            for (cols, _) in &per_source {
                for (name, is_char) in cols {
                    let i = pdv.ensure(name, *is_char);
                    pdv.from_source[i] = true;
                }
            }
            merge_rows(&per_source, &ds.by)?
        }
    }};

    // 4. Iterate.
    let mut out_buffers: Vec<Vec<HashMap<String, RtValue>>> =
        ds.outputs.iter().map(|_| Vec::new()).collect();
    let mut rows_in = 0u64;

    let mut prev_by: Option<Vec<RtValue>> = None;
    let n = input_rows.len();

    for (idx, row) in input_rows.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err(DataStepError::Runtime("cancelled".into()));
        }
        // Reset non-retained, non-source-bound vars to missing/empty.
        for i in 0..pdv.vals.len() {
            if !pdv.retained[i] && !pdv.from_source[i] {
                pdv.vals[i] = if pdv.is_char[i] {
                    RtValue::Str(String::new())
                } else {
                    RtValue::missing()
                };
            }
        }
        // Also clear source-bound vars not present in this particular row
        // (sparse case e.g. merge / concat with mismatched schemas).
        for (i, name) in pdv.names.iter().enumerate() {
            if pdv.retained[i] || !pdv.from_source[i] { continue; }
            if name.starts_with("first.") || name.starts_with("last.") { continue; }
            if !row.contains_key(name) {
                pdv.vals[i] = if pdv.is_char[i] {
                    RtValue::Str(String::new())
                } else {
                    RtValue::missing()
                };
            }
        }
        // Populate from input row.
        for (name, val) in row.iter() {
            if let Some(&i) = pdv.index.get(name) {
                pdv.vals[i] = coerce_to(pdv.is_char[i], val.clone());
            }
        }
        rows_in += 1;

        // first./last. for by vars.
        if by_active {
            let this_by: Vec<RtValue> = ds.by.iter().map(|v| pdv.get(v)).collect();
            for (j, by_var) in ds.by.iter().enumerate() {
                let is_first = match &prev_by {
                    None => true,
                    Some(p) => any_changed(&p[..=j], &this_by[..=j]),
                };
                let fi = pdv.index[&format!("first.{}", by_var)];
                pdv.vals[fi] = RtValue::Num(if is_first { 1.0 } else { 0.0 });
            }
            // Look ahead one row for last.
            let next_by: Option<Vec<RtValue>> = if idx + 1 < n {
                let next = &input_rows[idx + 1];
                Some(ds.by.iter().map(|v| {
                    next.get(v).cloned().unwrap_or_else(RtValue::missing)
                }).collect())
            } else {
                None
            };
            for (j, by_var) in ds.by.iter().enumerate() {
                let is_last = match &next_by {
                    None => true,
                    Some(nb) => any_changed(&nb[..=j], &this_by[..=j]),
                };
                let li = pdv.index[&format!("last.{}", by_var)];
                pdv.vals[li] = RtValue::Num(if is_last { 1.0 } else { 0.0 });
            }
            prev_by = Some(this_by);
        }

        // `where` filter.
        if let Some(w) = &ds.where_expr {
            let v = eval(w, &pdv, &arrays)?;
            if !v.truthy() {
                continue;
            }
        }

        // Body.
        let mut explicit_outputs: Vec<usize> = Vec::new();
        let mut deleted = false;
        for s in &ds.body {
            match exec_stmt(s, &mut pdv, &arrays, &ds.outputs, &mut explicit_outputs)? {
                StmtFlow::Continue => {}
                StmtFlow::Delete => { deleted = true; break; }
            }
        }
        if deleted { continue; }

        if explicit_outputs.is_empty() {
            for (i, _) in ds.outputs.iter().enumerate() {
                emit(&pdv, ds, &mut out_buffers[i]);
            }
        } else {
            for i in explicit_outputs {
                emit(&pdv, ds, &mut out_buffers[i]);
            }
        }
    }

    // 5. Write outputs.
    let mut result_outputs = Vec::new();
    for ((out_ref, target), buf) in ds
        .outputs
        .iter()
        .zip(plan.outputs.iter())
        .zip(out_buffers.into_iter())
    {
        let n = write_output(conn, target, &pdv, ds, &buf)?;
        result_outputs.push((out_ref.clone(), target.clone(), n));
    }

    Ok(DataStepResult { outputs: result_outputs, rows_in })
}

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

fn exec_stmt(
    s: &Stmt,
    pdv: &mut Pdv,
    arrays: &HashMap<String, ArrayBinding>,
    outs: &[TableRef],
    explicit: &mut Vec<usize>,
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
        Stmt::IfThen { cond, then_stmt, else_stmt } => {
            let v = eval(cond, pdv, arrays)?;
            if v.truthy() {
                exec_stmt(then_stmt, pdv, arrays, outs, explicit)
            } else if let Some(e) = else_stmt {
                exec_stmt(e, pdv, arrays, outs, explicit)
            } else {
                Ok(StmtFlow::Continue)
            }
        }
        Stmt::SubsetIf { cond } => {
            let v = eval(cond, pdv, arrays)?;
            if v.truthy() { Ok(StmtFlow::Continue) } else { Ok(StmtFlow::Delete) }
        }
        Stmt::Output { dataset } => {
            let idx = match dataset {
                None => {
                    for i in 0..outs.len() {
                        if !explicit.contains(&i) { explicit.push(i); }
                    }
                    return Ok(StmtFlow::Continue);
                }
                Some(t) => outs
                    .iter()
                    .position(|o| o.name.eq_ignore_ascii_case(&t.name))
                    .ok_or_else(|| DataStepError::Runtime(format!(
                        "`output {}` refers to a dataset not listed on the DATA statement",
                        t.qualified()
                    )))?,
            };
            if !explicit.contains(&idx) { explicit.push(idx); }
            Ok(StmtFlow::Continue)
        }
        Stmt::Delete => Ok(StmtFlow::Delete),
        Stmt::Block(stmts) => {
            for s in stmts {
                match exec_stmt(s, pdv, arrays, outs, explicit)? {
                    StmtFlow::Continue => {}
                    StmtFlow::Delete => return Ok(StmtFlow::Delete),
                }
            }
            Ok(StmtFlow::Continue)
        }
        Stmt::Select { switch, branches, otherwise } => {
            match switch {
                Some(sw) => {
                    let switch_val = eval(sw, pdv, arrays)?;
                    for branch in branches {
                        for v in &branch.values {
                            let candidate = eval(v, pdv, arrays)?;
                            if compare(&switch_val, &candidate) == 0 {
                                return exec_stmt(&branch.stmt, pdv, arrays, outs, explicit);
                            }
                        }
                    }
                }
                None => {
                    for branch in branches {
                        for v in &branch.values {
                            if eval(v, pdv, arrays)?.truthy() {
                                return exec_stmt(&branch.stmt, pdv, arrays, outs, explicit);
                            }
                        }
                    }
                }
            }
            if let Some(o) = otherwise {
                return exec_stmt(o, pdv, arrays, outs, explicit);
            }
            Ok(StmtFlow::Continue)
        }
        Stmt::DoLoop { var, start, stop, step, body } => {
            let start_v = eval(start, pdv, arrays)?.as_num()
                .ok_or_else(|| DataStepError::Runtime("do loop start is missing".into()))?;
            let stop_v = eval(stop, pdv, arrays)?.as_num()
                .ok_or_else(|| DataStepError::Runtime("do loop stop is missing".into()))?;
            let step_v = match step {
                Some(e) => eval(e, pdv, arrays)?.as_num()
                    .ok_or_else(|| DataStepError::Runtime("do loop step is missing".into()))?,
                None => 1.0,
            };
            if step_v == 0.0 {
                return Err(DataStepError::Runtime("do loop step cannot be 0".into()));
            }
            let ascending = step_v > 0.0;
            let mut i = start_v;
            loop {
                if (ascending && i > stop_v) || (!ascending && i < stop_v) {
                    break;
                }
                pdv.set(var, RtValue::Num(i));
                for s in body {
                    match exec_stmt(s, pdv, arrays, outs, explicit)? {
                        StmtFlow::Continue => {}
                        StmtFlow::Delete => return Ok(StmtFlow::Delete),
                    }
                }
                i += step_v;
            }
            Ok(StmtFlow::Continue)
        }
    }
}

fn resolve_array_index(
    name: &str,
    index: &Expr,
    pdv: &Pdv,
    arrays: &HashMap<String, ArrayBinding>,
) -> Result<String, DataStepError> {
    let arr = arrays.get(&name.to_ascii_lowercase())
        .ok_or_else(|| DataStepError::Runtime(format!("unknown array '{}'", name)))?;
    let idx_v = eval(index, pdv, arrays)?;
    let idx = idx_v.as_num()
        .ok_or_else(|| DataStepError::Runtime(format!("array '{}' index is missing", name)))?;
    let i = idx as isize;
    if i < 1 || (i as usize) > arr.elements.len() {
        return Err(DataStepError::Runtime(format!(
            "array '{}' index {} out of range (1..{})",
            name, i, arr.elements.len()
        )));
    }
    Ok(arr.elements[i as usize - 1].clone())
}

fn emit(pdv: &Pdv, ds: &DataStep, buf: &mut Vec<HashMap<String, RtValue>>) {
    let mut row = HashMap::new();
    for (i, name) in pdv.names.iter().enumerate() {
        // Skip auto vars (first./last.) from output.
        if name.starts_with("first.") || name.starts_with("last.") { continue; }
        if let Some(keep) = &ds.keep {
            if !keep.iter().any(|k| k.eq_ignore_ascii_case(name)) { continue; }
        }
        if let Some(drop) = &ds.drop {
            if drop.iter().any(|d| d.eq_ignore_ascii_case(name)) { continue; }
        }
        row.insert(name.clone(), pdv.vals[i].clone());
    }
    buf.push(row);
}

fn eval(e: &Expr, pdv: &Pdv, arrays: &HashMap<String, ArrayBinding>) -> Result<RtValue, DataStepError> {
    use BinOp::*;
    use UnaryOp::*;
    Ok(match e {
        Expr::NumLit(n) => RtValue::Num(*n),
        Expr::StrLit(s) => RtValue::Str(s.clone()),
        Expr::Ident(name) => pdv.get(name),
        Expr::Call { name, args } => {
            let evaluated: Vec<RtValue> = args
                .iter()
                .map(|a| eval(a, pdv, arrays))
                .collect::<Result<_, _>>()?;
            funcs::call(name, &evaluated).map_err(DataStepError::Runtime)?
        }
        Expr::ArrayRef { name, index } => {
            let element = resolve_array_index(name, index, pdv, arrays)?;
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
                        Div => if b == 0.0 { f64::NAN } else { a / b },
                        Pow => a.powf(b),
                        Mod => if b == 0.0 { f64::NAN } else { a - (a / b).trunc() * b },
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
        if x < y { -1 } else if x > y { 1 } else { 0 }
    } else {
        let sa = a.as_str();
        let sb = b.as_str();
        sa.cmp(&sb) as i32
    }
}

fn load_source(
    conn: &Connection,
    from: &str,
    by: &[String],
) -> Result<(Vec<(String, bool)>, Vec<Vec<RtValue>>), DataStepError> {
    let order = if by.is_empty() {
        String::new()
    } else {
        let cols: Vec<String> = by.iter().map(|v| format!("\"{}\"", v)).collect();
        format!(" ORDER BY {}", cols.join(", "))
    };
    let sql = format!("SELECT * FROM {}{}", from, order);
    let mut stmt = conn.prepare(&sql)?;
    let mut rows_iter = stmt.query([])?;
    let col_count = rows_iter.as_ref().map(|s| s.column_count()).unwrap_or(0);
    let col_names: Vec<String> = (0..col_count)
        .map(|i| {
            rows_iter
                .as_ref()
                .and_then(|s| s.column_name(i).ok())
                .map(|n| n.to_ascii_lowercase())
                .unwrap_or_else(|| format!("col{}", i))
        })
        .collect();

    let mut rows: Vec<Vec<RtValue>> = Vec::new();
    let mut col_types: Vec<bool> = vec![false; col_count];
    let mut types_set = false;

    while let Some(row) = rows_iter.next()? {
        let mut vals = Vec::with_capacity(col_count);
        for i in 0..col_count {
            let v: DV = row.get(i)?;
            if !types_set {
                col_types[i] = matches!(v, DV::Text(_) | DV::Blob(_));
            }
            vals.push(rt_from_duckdb(v));
        }
        types_set = true;
        rows.push(vals);
    }
    Ok((col_names.into_iter().zip(col_types).collect(), rows))
}

/// K-way match-merge by `by` variables. Within each group, emits
/// max(rows_in_each_source) rows; smaller sources pad with their last value
/// in the group (broadcast).
fn merge_rows(
    sources: &[(Vec<(String, bool)>, Vec<Vec<RtValue>>)],
    by: &[String],
) -> Result<Vec<SourceRow>, DataStepError> {
    if by.is_empty() {
        return Err(DataStepError::Runtime(
            "merge requires a `by` statement".into(),
        ));
    }

    // Precompute (sorted) by-key per row, per source.
    let mut per_source_keys: Vec<Vec<Vec<RtValue>>> = sources
        .iter()
        .map(|(cols, rows)| {
            let by_idx: Vec<Option<usize>> = by
                .iter()
                .map(|b| cols.iter().position(|(c, _)| c.eq_ignore_ascii_case(b)))
                .collect();
            rows.iter()
                .map(|r| {
                    by_idx
                        .iter()
                        .map(|opt| match opt {
                            Some(i) => r[*i].clone(),
                            None => RtValue::missing(),
                        })
                        .collect()
                })
                .collect()
        })
        .collect();

    // Cursors per source.
    let mut cursors = vec![0usize; sources.len()];
    let mut out: Vec<SourceRow> = Vec::new();

    loop {
        // Find smallest current key across all sources that still have rows.
        let mut min_key: Option<Vec<RtValue>> = None;
        for (i, cur) in cursors.iter().enumerate() {
            if *cur >= sources[i].1.len() { continue; }
            let key = &per_source_keys[i][*cur];
            min_key = Some(match min_key {
                None => key.clone(),
                Some(mk) => if compare_keys(key, &mk) < 0 { key.clone() } else { mk },
            });
        }
        let Some(group_key) = min_key else { break; };

        // For each source, take contiguous rows matching this group_key.
        let mut group_rows_per_src: Vec<Vec<usize>> = Vec::with_capacity(sources.len());
        for i in 0..sources.len() {
            let mut indices = Vec::new();
            while cursors[i] < sources[i].1.len()
                && compare_keys(&per_source_keys[i][cursors[i]], &group_key) == 0
            {
                indices.push(cursors[i]);
                cursors[i] += 1;
            }
            group_rows_per_src.push(indices);
        }
        let group_size = group_rows_per_src.iter().map(|v| v.len()).max().unwrap_or(0);
        if group_size == 0 { break; }

        for r in 0..group_size {
            let mut merged = SourceRow::new();
            for (i, (cols, rows)) in sources.iter().enumerate() {
                let indices = &group_rows_per_src[i];
                if indices.is_empty() { continue; }
                // Broadcast: clamp to last row of group.
                let idx = indices[r.min(indices.len() - 1)];
                for ((name, _), val) in cols.iter().zip(rows[idx].iter()) {
                    merged.insert(name.clone(), val.clone());
                }
            }
            // Also force by-vars into the row (in case schema mismatch).
            for (j, var) in by.iter().enumerate() {
                merged.insert(var.to_ascii_lowercase(), group_key[j].clone());
            }
            out.push(merged);
        }
    }

    Ok(out)
}

fn compare_keys(a: &[RtValue], b: &[RtValue]) -> i32 {
    for i in 0..a.len().max(b.len()) {
        let av = a.get(i).cloned().unwrap_or_else(RtValue::missing);
        let bv = b.get(i).cloned().unwrap_or_else(RtValue::missing);
        let c = compare(&av, &bv);
        if c != 0 { return c; }
    }
    0
}

/// Read rows from a delimited or whitespace-separated file via `infile`.
fn rows_from_infile(
    infile: &InfileSpec,
    input_vars: &[InputVar],
) -> Result<Vec<SourceRow>, DataStepError> {
    let text = std::fs::read_to_string(&infile.path)
        .map_err(|e| DataStepError::Runtime(format!("infile read: {}: {}", infile.path, e)))?;
    let mut rows = Vec::new();
    let firstobs = infile.firstobs.max(1) as usize;
    for (idx, line) in text.lines().enumerate() {
        if idx + 1 < firstobs { continue; }
        let trimmed_line = line.trim_end_matches('\r');
        if trimmed_line.is_empty() { continue; }
        let toks: Vec<String> = match &infile.dlm {
            None => trimmed_line.split_whitespace().map(|s| s.to_string()).collect(),
            Some(d) if infile.dsd => split_dsd(trimmed_line, d.chars().next().unwrap_or(',')),
            Some(d) => trimmed_line
                .split(d.as_str())
                .map(|s| s.to_string())
                .collect(),
        };
        rows.push(input_vars_to_row(input_vars, &toks));
    }
    Ok(rows)
}

/// RFC-4180-ish parser: respect double-quoted fields with `""` escape.
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
    let mut row = SourceRow::new();
    for (i, iv) in input_vars.iter().enumerate() {
        let key = iv.name.to_ascii_lowercase();
        let tok = toks.get(i).map(|s| s.trim());
        let val = match (iv.is_char, tok) {
            (true, Some(t)) => RtValue::Str(t.to_string()),
            (true, None) => RtValue::Str(String::new()),
            (false, Some(t)) if !t.is_empty() => match t.parse::<f64>() {
                Ok(n) => RtValue::Num(n),
                Err(_) => RtValue::missing(),
            },
            (false, _) => RtValue::missing(),
        };
        row.insert(key, val);
    }
    row
}

/// Parse free-form datalines: split each line on whitespace, map tokens to
/// input variables in order. Empty rows are skipped. A line with fewer
/// tokens than vars pads the rest with missing.
fn rows_from_datalines(lines: &[String], input_vars: &[InputVar]) -> Vec<SourceRow> {
    let mut rows = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        let toks: Vec<String> = trimmed.split_whitespace().map(|s| s.to_string()).collect();
        rows.push(input_vars_to_row(input_vars, &toks));
    }
    rows
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

fn write_output(
    conn: &Connection,
    target: &WriteTarget,
    pdv: &Pdv,
    ds: &DataStep,
    rows: &[HashMap<String, RtValue>],
) -> Result<u64, DataStepError> {
    let cols: Vec<(String, bool)> = pdv
        .names
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            if n.starts_with("first.") || n.starts_with("last.") { return false; }
            if let Some(keep) = &ds.keep {
                if !keep.iter().any(|k| k.eq_ignore_ascii_case(n)) { return false; }
            }
            if let Some(drop) = &ds.drop {
                if drop.iter().any(|d| d.eq_ignore_ascii_case(n)) { return false; }
            }
            true
        })
        .map(|(i, n)| (n.clone(), pdv.is_char[i]))
        .collect();

    match target {
        WriteTarget::DuckDb { schema, name } => write_duckdb_table(conn, schema, name, &cols, rows),
        WriteTarget::Parquet { path, .. } | WriteTarget::Csv { path, .. } => {
            let temp = format!("pas_ds_tmp_{}", uuid::Uuid::new_v4().simple());
            let n = write_duckdb_table(conn, "main", &temp, &cols, rows)?;
            let fmt = match target {
                WriteTarget::Parquet { .. } => "PARQUET",
                WriteTarget::Csv { .. } => "CSV",
                _ => unreachable!(),
            };
            let qualified = format!("\"main\".\"{}\"", temp);
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

fn write_duckdb_table(
    conn: &Connection,
    schema: &str,
    name: &str,
    cols: &[(String, bool)],
    rows: &[HashMap<String, RtValue>],
) -> Result<u64, DataStepError> {
    let qualified = format!("\"{}\".\"{}\"", schema, name);
    let mut create = format!("CREATE OR REPLACE TABLE {} (", qualified);
    for (i, (n, is_char)) in cols.iter().enumerate() {
        if i > 0 { create.push_str(", "); }
        create.push('"');
        create.push_str(n);
        create.push('"');
        create.push(' ');
        create.push_str(if *is_char { "VARCHAR" } else { "DOUBLE" });
    }
    create.push(')');
    conn.execute(&create, [])?;

    if rows.is_empty() {
        return Ok(0);
    }

    let mut app = conn.appender_to_db(name, schema)?;
    for row in rows {
        let vals: Vec<DV> = cols
            .iter()
            .map(|(n, is_char)| value_for_appender(*is_char, row.get(n)))
            .collect();
        app.append_row(duckdb::appender_params_from_iter(vals))?;
    }
    app.flush()?;
    Ok(rows.len() as u64)
}

fn value_for_appender(is_char: bool, v: Option<&RtValue>) -> DV {
    match (is_char, v) {
        (true, Some(RtValue::Str(s))) => DV::Text(s.clone()),
        (true, Some(RtValue::Num(n))) if !n.is_nan() => DV::Text(RtValue::Num(*n).as_str()),
        (true, _) => DV::Text(String::new()),
        (false, Some(RtValue::Num(n))) if !n.is_nan() => DV::Double(*n),
        (false, Some(RtValue::Str(s))) => s.trim().parse::<f64>().map(DV::Double).unwrap_or(DV::Null),
        (false, _) => DV::Null,
    }
}
