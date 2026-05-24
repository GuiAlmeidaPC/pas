use crate::library::{ColumnInfo, DatasetInfo, DirFormat, Library, LibraryKind};
use crate::rewrite::{build_where_clause, dataset_from_clause, is_query};
use crate::session::Session;
use crate::types::{Column, DatasetPage, ResultBlock, SourceSpan, Value};
use crate::EngineError;
use duckdb::Connection;
use std::collections::HashMap;

impl Session {
    pub fn list_datasets(&self, libref: &str) -> Result<Vec<DatasetInfo>, EngineError> {
        let lib = self.lookup_library(libref)?;
        let conn = self.read_conn.lock().unwrap();
        match lib.kind {
            LibraryKind::Memory => list_schema_tables(&conn, "main", &lib.name),
            LibraryKind::Duckdb => list_schema_tables(&conn, &lib.name, &lib.name),
            LibraryKind::Dir => list_dir_datasets(&lib),
        }
    }

    pub fn dataset_schema(&self, libref: &str, name: &str) -> Result<Vec<ColumnInfo>, EngineError> {
        let lib = self.lookup_library(libref)?;
        let from_clause = dataset_from_clause(&lib, name)?;
        let conn = self.read_conn.lock().unwrap();
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
                let ty = rows
                    .as_ref()
                    .map(|s| format!("{:?}", s.column_type(i)).to_lowercase())
                    .unwrap_or_else(|| "?".to_string());
                ColumnInfo { name, ty }
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
        let (where_sql, where_params) = build_where_clause(filters);
        let conn = self.read_conn.lock().unwrap();

        let total: u64 = {
            let count_sql = format!("SELECT count(*) FROM {}{}", from_clause, where_sql);
            let mut stmt = conn.prepare(&count_sql)?;
            let mut rows = stmt.query(duckdb::params_from_iter(where_params.iter()))?;
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
        let arrow_iter = stmt.query_arrow(duckdb::params_from_iter(where_params.iter()))?;
        let base_schema = arrow_iter.get_schema();
        let mut md = base_schema.metadata().clone();
        md.insert("total_rows".to_string(), total.to_string());
        md.insert("offset".to_string(), offset.to_string());
        let schema = Arc::new(Schema::new_with_metadata(base_schema.fields().clone(), md));

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
        let (where_sql, where_params) = build_where_clause(filters);
        let conn = self.read_conn.lock().unwrap();

        let total: u64 = {
            let count_sql = format!("SELECT count(*) FROM {}{}", from_clause, where_sql);
            let mut stmt = conn.prepare(&count_sql)?;
            let mut rows = stmt.query(duckdb::params_from_iter(where_params.iter()))?;
            match rows.next()? {
                Some(r) => r.get::<_, i64>(0).unwrap_or(0).max(0) as u64,
                None => 0,
            }
        };

        let sql = format!(
            "SELECT * FROM {}{} LIMIT {} OFFSET {}",
            from_clause, where_sql, limit, offset
        );
        let block = match run_query_params(&conn, &sql, limit as usize, &where_params)? {
            crate::query::StmtResult::Rows(b) => b,
            _ => ResultBlock {
                columns: vec![],
                rows: vec![],
                truncated: false,
            },
        };
        Ok(DatasetPage {
            columns: block.columns,
            rows: block.rows,
            total_rows: total,
        })
    }
}

pub(crate) fn list_schema_tables(
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
        .map(|name| DatasetInfo {
            libref: libref.to_string(),
            name,
            rows: None,
        })
        .collect())
}

pub(crate) fn list_dir_datasets(lib: &Library) -> Result<Vec<DatasetInfo>, EngineError> {
    let fmt = lib.format.unwrap_or(DirFormat::Parquet);
    let ext = fmt.extension();
    let dir =
        std::fs::read_dir(&lib.path).map_err(|e| EngineError::Other(format!("read_dir: {}", e)))?;
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let p = entry.path();
        if p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case(ext))
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

pub(crate) enum StmtResult {
    Rows(ResultBlock),
    Affected(usize),
    Done,
}

pub(crate) fn run_one(
    conn: &Connection,
    sql: &str,
    max_preview: usize,
) -> Result<StmtResult, EngineError> {
    let trimmed = crate::sas_sql::rewrite(sql.trim());
    let trimmed = trimmed.as_str();
    if trimmed.is_empty() {
        return Ok(StmtResult::Done);
    }
    if is_query(trimmed) {
        run_query(conn, trimmed, max_preview)
    } else {
        let mut stmt = conn.prepare(trimmed)?;
        let n = stmt.execute([])?;
        Ok(StmtResult::Affected(n))
    }
}

pub(crate) fn run_query(
    conn: &Connection,
    sql: &str,
    max_rows: usize,
) -> Result<StmtResult, EngineError> {
    run_query_params(conn, sql, max_rows, &[] as &[String])
}

pub(crate) fn run_query_params<P>(
    conn: &Connection,
    sql: &str,
    max_rows: usize,
    params: &[P],
) -> Result<StmtResult, EngineError>
where
    P: duckdb::ToSql,
{
    let mut stmt = conn.prepare(sql)?;
    let mut rows_iter = stmt.query(duckdb::params_from_iter(params.iter()))?;

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
        for (i, ty) in col_types.iter_mut().enumerate() {
            let v: duckdb::types::Value = row.get(i)?;
            if !types_filled {
                *ty = type_name(&v).to_string();
            }
            vals.push(value_from_duckdb(v));
        }
        types_filled = true;
        rows.push(vals);
    }

    let columns = col_names
        .into_iter()
        .zip(col_types)
        .map(|(name, ty)| Column {
            name,
            ty: if ty.is_empty() { "?".into() } else { ty },
        })
        .collect();

    Ok(StmtResult::Rows(ResultBlock {
        columns,
        rows,
        truncated,
    }))
}

/// Run `select_sql` and route the result rows into `target`. Returns the
/// number of rows written. Used by PROC SORT and PROC TRANSPOSE; shaped to
/// match the data-step writer's contract (CREATE OR REPLACE for DuckDB
/// targets, COPY (...) TO 'path' for DIR libraries).
pub(crate) fn materialize_select_into(
    conn: &Connection,
    target: &crate::datastep::exec::WriteTarget,
    select_sql: &str,
) -> Result<u64, EngineError> {
    use crate::datastep::exec::WriteTarget;
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
pub(crate) fn duckdb_error_span(
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
    let col = caret_line.find('^').map(|p| p as u32 + 1)?;

    let mut byte = 0usize;
    let mut current_line = 1u32;
    for ch in stmt.chars() {
        if current_line == line_no {
            // Walk forward `col - 1` columns on this line.
            let mut sub = 0usize;
            for (col_count, ch2) in (1u32..).zip(stmt[byte..].chars()) {
                if col_count == col {
                    break;
                }
                if ch2 == '\n' {
                    break;
                }
                sub += ch2.len_utf8();
            }
            let abs = src_offset + byte + sub;
            let (sl, sc) = crate::split::byte_to_line_col(program, abs);
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
