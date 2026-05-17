//! Top-level program splitter.
//!
//! Walks a SAS-ish program (after comment stripping) and groups statements
//! into [`Block`]s the engine knows how to dispatch:
//!   - `proc sql; … quit|run;` → emits a [`Block::ProcSqlStmt`] per inner stmt
//!   - `data …; … run;`        → emits a single [`Block::DataStep`] holding
//!     the full body (header included, semicolons re-added)
//!   - anything else            → [`Block::Statement`]

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// A single statement that lived inside a `proc sql` wrapper.
    ProcSqlStmt(String),
    /// A DATA step: full body text plus any datalines payload that the
    /// preprocessor extracted from `datalines;` / `cards;` blocks.
    DataStep { body: String, datalines: Vec<String> },
    /// A global statement (libname, bare SQL, etc.).
    Statement(String),
}

/// Remove `/* ... */` block comments. String contents are preserved.
pub fn strip_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\'' || b == b'"' {
            let quote = b;
            out.push(b as char);
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                out.push(c as char);
                i += 1;
                if c == quote {
                    if i < bytes.len() && bytes[i] == quote {
                        out.push(quote as char);
                        i += 1;
                        continue;
                    }
                    break;
                }
            }
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            out.push(' ');
            continue;
        }
        out.push(b as char);
        i += 1;
    }
    out
}

/// Split a comment-stripped program into [`Block`]s. Datalines payloads
/// from `datalines;` / `cards;` / `lines;` blocks are extracted up front
/// and attached to the DATA step they originate from.
pub fn split_blocks(src: &str) -> Vec<Block> {
    let (program, mut datalines_queue) = extract_datalines(src);
    let stmts = split_on_semicolons(&program);
    let mut out = Vec::new();

    enum State {
        Normal,
        ProcSql,
        Data,
    }
    let mut state = State::Normal;
    let mut data_buf = String::new();
    let mut data_has_datalines = false;

    for raw in stmts {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();

        if matches!(state, State::Normal) && lower.starts_with('*') {
            continue;
        }

        match state {
            State::Normal => {
                if lower == "proc sql" || lower.starts_with("proc sql ") {
                    state = State::ProcSql;
                    continue;
                }
                if lower == "data" || lower.starts_with("data ") {
                    state = State::Data;
                    data_has_datalines = false;
                    data_buf.clear();
                    data_buf.push_str(trimmed);
                    data_buf.push(';');
                    continue;
                }
                out.push(Block::Statement(trimmed.to_string()));
            }
            State::ProcSql => {
                if lower == "quit" || lower == "run" {
                    state = State::Normal;
                    continue;
                }
                out.push(Block::ProcSqlStmt(trimmed.to_string()));
            }
            State::Data => {
                if lower == "run" {
                    let datalines = if data_has_datalines {
                        datalines_queue.pop_front().unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    out.push(Block::DataStep {
                        body: std::mem::take(&mut data_buf),
                        datalines,
                    });
                    state = State::Normal;
                    continue;
                }
                if matches!(lower.as_str(), "datalines" | "cards" | "lines") {
                    data_has_datalines = true;
                }
                data_buf.push_str(trimmed);
                data_buf.push(';');
            }
        }
    }

    if !data_buf.is_empty() {
        let datalines = if data_has_datalines {
            datalines_queue.pop_front().unwrap_or_default()
        } else {
            Vec::new()
        };
        out.push(Block::DataStep { body: data_buf, datalines });
    }

    out
}

/// Pre-process the source: find each `datalines;` (or `cards;` / `lines;`)
/// terminator on a line, then collect every following line until a line
/// whose only non-whitespace content is `;`. Returns the source with the
/// data lines stripped (the `datalines;` token is preserved so the
/// splitter still sees it) and a FIFO of extracted data blocks.
pub fn extract_datalines(src: &str) -> (String, std::collections::VecDeque<Vec<String>>) {
    let mut out = String::with_capacity(src.len());
    let mut blocks: std::collections::VecDeque<Vec<String>> = std::collections::VecDeque::new();
    let mut lines = src.split_inclusive('\n').peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        let lower = trimmed.trim().to_ascii_lowercase();
        let is_datalines = matches!(
            lower.trim_end_matches(';').trim(),
            "datalines" | "cards" | "lines"
        ) && lower.ends_with(';');

        out.push_str(line);
        if is_datalines {
            // Eat following lines until a line that, after trimming, is `;`.
            let mut data: Vec<String> = Vec::new();
            while let Some(d) = lines.next() {
                let dt = d.trim_end_matches(['\n', '\r']);
                if dt.trim() == ";" {
                    break;
                }
                data.push(dt.to_string());
            }
            blocks.push_back(data);
        }
    }
    (out, blocks)
}

/// Backward-compatible helper used by older tests/UI to flatten proc sql
/// content (and skip data steps). Prefer [`split_blocks`] in new code.
pub fn extract_sql_statements(src: &str) -> Vec<String> {
    split_blocks(src)
        .into_iter()
        .filter_map(|b| match b {
            Block::ProcSqlStmt(s) | Block::Statement(s) => Some(s),
            Block::DataStep { .. } => None,
        })
        .collect()
}

fn split_on_semicolons(src: &str) -> Vec<String> {
    let bytes = src.as_bytes();
    let mut stmts = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\'' || b == b'"' {
            let quote = b;
            current.push(b as char);
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                current.push(c as char);
                i += 1;
                if c == quote {
                    if i < bytes.len() && bytes[i] == quote {
                        current.push(quote as char);
                        i += 1;
                        continue;
                    }
                    break;
                }
            }
            continue;
        }
        if b == b';' {
            stmts.push(std::mem::take(&mut current));
            i += 1;
            continue;
        }
        current.push(b as char);
        i += 1;
    }
    if !current.trim().is_empty() {
        stmts.push(current);
    }
    stmts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_split_simple_sql() {
        let blocks = split_blocks("select 1; select 2;");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], Block::Statement(_)));
    }

    #[test]
    fn block_split_proc_sql() {
        let blocks = split_blocks("proc sql; select 1; quit;");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], Block::ProcSqlStmt(_)));
    }

    #[test]
    fn block_split_data_step() {
        let blocks = split_blocks("data out; set in; x = 1; run;");
        assert_eq!(blocks.len(), 1);
        let Block::DataStep { body, .. } = &blocks[0] else {
            panic!("expected DataStep, got {:?}", blocks)
        };
        assert!(body.contains("data out"));
        assert!(body.contains("x = 1"));
    }

    #[test]
    fn extracts_datalines() {
        let src = "data x;\n  input name $ age;\n  datalines;\nalice 30\nbob 25\n;\nrun;\n";
        let blocks = split_blocks(src);
        let Block::DataStep { body, datalines } = &blocks[0] else {
            panic!("expected DataStep");
        };
        assert_eq!(datalines.len(), 2);
        assert_eq!(datalines[0].trim(), "alice 30");
        assert_eq!(datalines[1].trim(), "bob 25");
        assert!(body.to_lowercase().contains("datalines"));
    }

    #[test]
    fn block_split_mixed() {
        let blocks = split_blocks(
            r#"
            libname foo "/tmp";
            data foo.out; set foo.in; run;
            proc sql; select 1; quit;
            "#,
        );
        assert!(matches!(blocks[0], Block::Statement(_)));
        assert!(matches!(blocks[1], Block::DataStep { .. }));
        assert!(matches!(blocks[2], Block::ProcSqlStmt(_)));
    }
}
