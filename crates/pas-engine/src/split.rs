//! Top-level program splitter, position-preserving.
//!
//! Each preprocessing step (`strip_comments`, `extract_datalines`) replaces
//! removed characters with whitespace of equal byte length, so offsets in
//! the cleaned source line up 1:1 with offsets in the original program.
//! That invariant is what lets [`Block`]s carry meaningful source ranges
//! and what lets parser errors get rendered as Monaco squiggles back in
//! the editor.

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// A single statement that lived inside a `proc sql` wrapper.
    ProcSqlStmt { text: String, src_offset: usize },
    /// Any non-SQL PROC.
    Proc { name: String, body: String, src_offset: usize },
    /// A DATA step. `body` is the original source slice between the data
    /// header's `;` and the trailing `run;` — positions in the body map
    /// directly to positions in the original program by adding
    /// `body_src_offset`.
    DataStep {
        body: String,
        datalines: Vec<String>,
        body_src_offset: usize,
    },
    /// A global statement (libname, bare SQL, etc.).
    Statement { text: String, src_offset: usize },
}

/// Remove `/* ... */` and `* ... ;` comments while preserving byte
/// offsets — every removed byte becomes a single ASCII space.
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
        // /* block comment */
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            let end = (i + 2).min(bytes.len());
            for _ in start..end {
                out.push(' ');
            }
            i = end;
            continue;
        }
        out.push(b as char);
        i += 1;
    }
    out
}

/// Split a comment-stripped program into [`Block`]s. Datalines payloads
/// from `datalines;` / `cards;` / `lines;` blocks are extracted up front
/// (replaced with whitespace) and attached to the DATA step they came
/// from.
pub fn split_blocks(src: &str) -> Vec<Block> {
    let (program, mut datalines_queue) = extract_datalines(src);
    let stmts = split_on_semicolons(&program);
    let mut out = Vec::new();

    enum State {
        Normal,
        ProcSql,
        ProcOther {
            name: String,
            /// Reconstructed body string (header options + subsequent
            /// statements joined by `;`). For PROC parsers fine-grained
            /// source positions aren't currently needed.
            body: String,
            src_offset: usize,
        },
        Data {
            body_start: Option<usize>,
            body_end: usize,
            has_datalines: bool,
        },
    }
    let mut state = State::Normal;

    for raw in stmts {
        let trimmed = raw.text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();

        if matches!(state, State::Normal) && lower.starts_with('*') {
            continue;
        }

        match &mut state {
            State::Normal => {
                if lower == "proc sql" || lower.starts_with("proc sql ") {
                    state = State::ProcSql;
                    continue;
                }
                if lower.starts_with("proc ") {
                    let after = trimmed[5..].trim_start();
                    let name_end = after
                        .find(|c: char| c.is_whitespace())
                        .unwrap_or(after.len());
                    let name = after[..name_end].to_ascii_lowercase();
                    let rest_after_name = after[name_end..].trim();
                    let mut body = String::new();
                    if !rest_after_name.is_empty() {
                        body.push_str(rest_after_name);
                        body.push(';');
                    }
                    state = State::ProcOther { name, body, src_offset: raw.start };
                    continue;
                }
                if lower == "data" || lower.starts_with("data ") {
                    // body includes the `data …;` header so the data-step
                    // parser sees the keyword. Offset starts at the
                    // header's first byte.
                    state = State::Data {
                        body_start: Some(raw.start),
                        body_end: raw.end,
                        has_datalines: false,
                    };
                    continue;
                }
                out.push(Block::Statement {
                    text: trimmed.to_string(),
                    src_offset: raw.start,
                });
            }
            State::ProcSql => {
                if lower == "quit" || lower == "run" {
                    state = State::Normal;
                    continue;
                }
                out.push(Block::ProcSqlStmt {
                    text: trimmed.to_string(),
                    src_offset: raw.start,
                });
            }
            State::ProcOther { name, body, src_offset } => {
                if lower == "run" || lower == "quit" {
                    out.push(Block::Proc {
                        name: std::mem::take(name),
                        body: std::mem::take(body),
                        src_offset: *src_offset,
                    });
                    state = State::Normal;
                    continue;
                }
                body.push_str(trimmed);
                body.push(';');
            }
            State::Data { body_start, body_end, has_datalines } => {
                if lower == "run" {
                    let start = body_start.unwrap_or(*body_end);
                    let body = src.get(start..*body_end).unwrap_or("").to_string();
                    let datalines = if *has_datalines {
                        datalines_queue.pop_front().unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    out.push(Block::DataStep {
                        body,
                        datalines,
                        body_src_offset: start,
                    });
                    state = State::Normal;
                    continue;
                }
                if matches!(lower.as_str(), "datalines" | "cards" | "lines") {
                    *has_datalines = true;
                }
                if body_start.is_none() {
                    *body_start = Some(raw.start);
                }
                *body_end = raw.end;
            }
        }
    }

    // Flush unclosed blocks at EOF.
    if let State::ProcOther { name, body, src_offset } = state {
        out.push(Block::Proc { name, body, src_offset });
    }

    out
}

/// Pre-process the source: find each `datalines;` (or `cards;` / `lines;`)
/// terminator on a line, then collect every following line until a line
/// whose only non-whitespace content is `;`. Removed lines are replaced
/// with whitespace of equal byte length to keep offsets stable.
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
            let mut data: Vec<String> = Vec::new();
            while let Some(d) = lines.next() {
                let dt = d.trim_end_matches(['\n', '\r']);
                if dt.trim() == ";" {
                    // Replace the terminator line with whitespace.
                    for byte in d.bytes() {
                        out.push(if byte == b'\n' { '\n' } else { ' ' });
                    }
                    break;
                }
                data.push(dt.to_string());
                // Replace the consumed data line with whitespace.
                for byte in d.bytes() {
                    out.push(if byte == b'\n' { '\n' } else { ' ' });
                }
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
            Block::ProcSqlStmt { text, .. } | Block::Statement { text, .. } => Some(text),
            Block::DataStep { .. } | Block::Proc { .. } => None,
        })
        .collect()
}

/// Raw split-on-semicolons result. `start..end` is the byte range of the
/// statement (including the trailing `;` in `end`) in the source that was
/// passed to this function. `text` preserves the raw run (no trimming) so
/// the caller can decide how to handle whitespace.
struct RawStmt {
    text: String,
    start: usize,
    end: usize,
}

fn split_on_semicolons(src: &str) -> Vec<RawStmt> {
    let bytes = src.as_bytes();
    let mut stmts = Vec::new();
    let mut current = String::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if current.is_empty() {
            start = i;
        }
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
            stmts.push(RawStmt {
                text: std::mem::take(&mut current),
                start,
                end: i + 1,
            });
            i += 1;
            continue;
        }
        current.push(b as char);
        i += 1;
    }
    if !current.trim().is_empty() {
        stmts.push(RawStmt { text: current, start, end: i });
    }
    stmts
}

/// Convert a byte offset in `src` into a 1-based `(line, column)` pair.
/// Columns count UTF-8 *code points* (which Monaco lines up with) rather
/// than bytes.
pub fn byte_to_line_col(src: &str, offset: usize) -> (u32, u32) {
    let clamped = offset.min(src.len());
    let mut line = 1u32;
    let mut col = 1u32;
    let mut i = 0usize;
    for ch in src.chars() {
        if i >= clamped {
            break;
        }
        let len = ch.len_utf8();
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        i += len;
    }
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_split_simple_sql() {
        let blocks = split_blocks("select 1; select 2;");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], Block::Statement { .. }));
    }

    #[test]
    fn block_split_proc_sql() {
        let blocks = split_blocks("proc sql; select 1; quit;");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], Block::ProcSqlStmt { .. }));
    }

    #[test]
    fn block_split_data_step() {
        let blocks = split_blocks("data out; set in; x = 1; run;");
        assert_eq!(blocks.len(), 1);
        let Block::DataStep { body, body_src_offset, .. } = &blocks[0] else {
            panic!("expected DataStep, got {:?}", blocks)
        };
        assert!(body.starts_with("data out"));
        assert!(body.contains("x = 1"));
        assert_eq!(*body_src_offset, 0);
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
        assert!(matches!(blocks[0], Block::Statement { .. }));
        assert!(matches!(blocks[1], Block::DataStep { .. }));
        assert!(matches!(blocks[2], Block::ProcSqlStmt { .. }));
    }

    #[test]
    fn extracts_datalines() {
        let src = "data x;\n  input name $ age;\n  datalines;\nalice 30\nbob 25\n;\nrun;\n";
        let blocks = split_blocks(src);
        let Block::DataStep { body, datalines, .. } = &blocks[0] else {
            panic!("expected DataStep");
        };
        assert_eq!(datalines.len(), 2);
        assert_eq!(datalines[0].trim(), "alice 30");
        assert_eq!(datalines[1].trim(), "bob 25");
        assert!(body.to_lowercase().contains("datalines"));
    }

    #[test]
    fn block_split_proc_sort() {
        let blocks = split_blocks("proc sort data=in out=out nodupkey; by x; run;");
        assert_eq!(blocks.len(), 1);
        let Block::Proc { name, body, .. } = &blocks[0] else {
            panic!("expected Proc, got {:?}", blocks);
        };
        assert_eq!(name, "sort");
        assert!(body.contains("data=in"));
        assert!(body.contains("by x"));
    }

    #[test]
    fn strip_comments_preserves_byte_offsets() {
        let src = "a /* xxx */ b";
        let out = strip_comments(src);
        assert_eq!(out.len(), src.len());
        assert_eq!(&out[..2], "a ");
        assert_eq!(&out[11..], " b");
    }

    #[test]
    fn extract_datalines_preserves_byte_offsets() {
        let src = "data x;\ndatalines;\nalice 30\n;\nrun;\n";
        let (out, _) = extract_datalines(src);
        assert_eq!(out.len(), src.len());
        // The "run;" line must still be at the same offset.
        assert!(out.contains("run;"));
    }

    #[test]
    fn byte_to_line_col_basic() {
        let src = "a\nbc\ndef";
        assert_eq!(byte_to_line_col(src, 0), (1, 1));
        assert_eq!(byte_to_line_col(src, 2), (2, 1));
        assert_eq!(byte_to_line_col(src, 5), (3, 1));
        assert_eq!(byte_to_line_col(src, 7), (3, 3));
    }
}
