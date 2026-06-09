//! Rewriters that turn PAS PROC-SQL extensions into plain DuckDB SQL.
//!
//! Two passes run in order:
//!
//!   1. `rewrite_create_or_replace` — adds `OR REPLACE` to bare
//!      `CREATE TABLE` / `CREATE VIEW` so re-running a program doesn't
//!      error on existing objects.
//!   2. `rewrite_extensions` — token-aware pass that handles:
//!        - `calculated <col>`         → `<col>`
//!        - `monotonic()`              → `row_number() over ()`
//!        - `outer union corr`         → `union all by name`
//!        - `outer union`              → `union all`
//!
//! Both passes preserve string literals and `/* … */` / `-- …` comments
//! verbatim (so a column literally named "calculated" inside a string
//! survives untouched).

pub fn rewrite(sql: &str) -> String {
    let s = rewrite_create_or_replace(sql);
    rewrite_extensions(&s)
}

pub fn rewrite_create_or_replace(sql: &str) -> String {
    let lower = sql.to_ascii_lowercase();
    let stripped = lower.trim_start();
    let leading_ws = sql.len() - sql.trim_start().len();

    if stripped.starts_with("create or replace") {
        return sql.to_string();
    }

    let (prefix_lower, replacement) = if stripped.starts_with("create table") {
        ("create table", "CREATE OR REPLACE TABLE")
    } else if stripped.starts_with("create temp table") {
        ("create temp table", "CREATE OR REPLACE TEMP TABLE")
    } else if stripped.starts_with("create temporary table") {
        (
            "create temporary table",
            "CREATE OR REPLACE TEMPORARY TABLE",
        )
    } else if stripped.starts_with("create view") {
        ("create view", "CREATE OR REPLACE VIEW")
    } else {
        return sql.to_string();
    };

    let rest_start = leading_ws + prefix_lower.len();
    format!(
        "{}{}{}",
        &sql[..leading_ws],
        replacement,
        &sql[rest_start..]
    )
}

// ── Token-aware extension rewriter ─────────────────────────────────────────

pub fn rewrite_extensions(sql: &str) -> String {
    let toks = tokenize(sql);
    let rewritten = apply_extension_rewrites(&toks);
    emit(&rewritten)
}

#[derive(Debug, Clone)]
enum Tok {
    /// A SQL identifier or keyword (sequence of `[A-Za-z_][A-Za-z0-9_]*`).
    Word(String),
    /// Anything else: whitespace, punctuation, string literals, comments,
    /// numbers. Emitted verbatim.
    Other(String),
}

fn tokenize(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            out.push(Tok::Word(src[start..i].to_string()));
            continue;
        }
        let start = i;
        // Quoted string literal — keep entire span including quotes.
        if b == b'\'' || b == b'"' {
            i = crate::scan::skip_string_literal(bytes, i);
            out.push(Tok::Other(src[start..i].to_string()));
            continue;
        }
        // /* block comment */
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i = crate::scan::skip_block_comment(bytes, i);
            out.push(Tok::Other(src[start..i].to_string()));
            continue;
        }
        // -- line comment
        if b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            out.push(Tok::Other(src[start..i].to_string()));
            continue;
        }
        // Single non-word character (whitespace, punctuation, digit, etc.).
        i += 1;
        out.push(Tok::Other(src[start..i].to_string()));
    }
    out
}

fn emit(toks: &[Tok]) -> String {
    let mut out = String::new();
    for t in toks {
        match t {
            Tok::Word(w) => out.push_str(w),
            Tok::Other(s) => out.push_str(s),
        }
    }
    out
}

fn is_whitespace_or_comment(t: &Tok) -> bool {
    match t {
        Tok::Word(_) => false,
        Tok::Other(s) => {
            s.starts_with("/*") || s.starts_with("--") || s.chars().all(|c| c.is_whitespace())
        }
    }
}

/// Find the index of the next `Word` token, skipping over whitespace and
/// comments. Returns `None` if the next significant token isn't a word.
fn next_word(toks: &[Tok], from: usize) -> Option<(usize, &str)> {
    let mut i = from;
    while i < toks.len() {
        match &toks[i] {
            Tok::Word(w) => return Some((i, w.as_str())),
            _ if is_whitespace_or_comment(&toks[i]) => i += 1,
            _ => return None,
        }
    }
    None
}

/// If `toks[from..]` begins (modulo whitespace) with `(` then `)`, returns
/// the index *after* the closing paren. Used for `monotonic()` detection.
fn match_empty_parens(toks: &[Tok], from: usize) -> Option<usize> {
    let mut i = from;
    while i < toks.len() && is_whitespace_or_comment(&toks[i]) {
        i += 1;
    }
    let Tok::Other(s) = toks.get(i)? else {
        return None;
    };
    if s != "(" {
        return None;
    }
    let mut j = i + 1;
    while j < toks.len() && is_whitespace_or_comment(&toks[j]) {
        j += 1;
    }
    let Tok::Other(s2) = toks.get(j)? else {
        return None;
    };
    if s2 != ")" {
        return None;
    }
    Some(j + 1)
}

fn apply_extension_rewrites(toks: &[Tok]) -> Vec<Tok> {
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if let Tok::Word(w) = &toks[i] {
            let lw = w.to_ascii_lowercase();

            // `calculated <col>` → `<col>` (drop the keyword and its
            // trailing space).
            if lw == "calculated" {
                i += 1;
                if let Some(Tok::Other(s)) = toks.get(i) {
                    if s.chars().all(|c| c.is_whitespace()) {
                        // Preserve one space so the next token isn't glued.
                        out.push(Tok::Other(" ".to_string()));
                        i += 1;
                        continue;
                    }
                }
                continue;
            }

            // `monotonic()` → `row_number() over ()`
            if lw == "monotonic" {
                if let Some(after) = match_empty_parens(toks, i + 1) {
                    out.push(Tok::Word("row_number".to_string()));
                    out.push(Tok::Other("() over ()".to_string()));
                    i = after;
                    continue;
                }
            }

            // `outer union [corr]` → `union all [by name]`
            if lw == "outer" {
                if let Some((j, w2)) = next_word(toks, i + 1) {
                    if w2.eq_ignore_ascii_case("union") {
                        if let Some((k, w3)) = next_word(toks, j + 1) {
                            if w3.eq_ignore_ascii_case("corr") {
                                out.push(Tok::Word("union".to_string()));
                                out.push(Tok::Other(" all by name".to_string()));
                                i = k + 1;
                                continue;
                            }
                        }
                        out.push(Tok::Word("union".to_string()));
                        out.push(Tok::Other(" all".to_string()));
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntoTarget {
    pub name: String,
    pub trimmed: bool,
}

pub fn extract_into_clause(sql: &str) -> (String, Vec<IntoTarget>) {
    let toks = tokenize(sql);
    let mut clean_toks = Vec::with_capacity(toks.len());
    let mut targets = Vec::new();

    let mut i = 0;
    let mut paren_depth: usize = 0;

    while i < toks.len() {
        match &toks[i] {
            Tok::Other(s) if s == "(" => {
                paren_depth += 1;
                clean_toks.push(toks[i].clone());
                i += 1;
            }
            Tok::Other(s) if s == ")" => {
                paren_depth = paren_depth.saturating_sub(1);
                clean_toks.push(toks[i].clone());
                i += 1;
            }
            Tok::Word(w) if paren_depth == 0 && w.eq_ignore_ascii_case("into") => {
                let mut into_toks = Vec::new();
                i += 1;
                while i < toks.len() {
                    let mut stop = false;
                    match &toks[i] {
                        Tok::Word(w_next) => {
                            let lw = w_next.to_ascii_lowercase();
                            if lw == "from"
                                || lw == "where"
                                || lw == "group"
                                || lw == "having"
                                || lw == "order"
                            {
                                stop = true;
                            }
                        }
                        Tok::Other(s_next) if s_next == ";" => {
                            stop = true;
                        }
                        _ => {}
                    }
                    if stop {
                        break;
                    }
                    into_toks.push(toks[i].clone());
                    i += 1;
                }
                targets = parse_into_targets(&into_toks);
            }
            _ => {
                clean_toks.push(toks[i].clone());
                i += 1;
            }
        }
    }

    (emit(&clean_toks), targets)
}

fn parse_into_targets(toks: &[Tok]) -> Vec<IntoTarget> {
    let mut targets = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if is_whitespace_or_comment(&toks[i]) {
            i += 1;
            continue;
        }

        if let Tok::Other(s) = &toks[i] {
            if s.contains(':') {
                let mut var_name = String::new();
                if s == ":" {
                    i += 1;
                    if i < toks.len() {
                        if let Tok::Word(w) = &toks[i] {
                            var_name = w.clone();
                        }
                    }
                } else if let Some(stripped) = s.strip_prefix(':') {
                    var_name = stripped.to_string();
                }

                if !var_name.is_empty() {
                    let mut trimmed = false;
                    let mut j = i + 1;
                    while j < toks.len() {
                        match &toks[j] {
                            _ if is_whitespace_or_comment(&toks[j]) => j += 1,
                            Tok::Word(w) if w.eq_ignore_ascii_case("trimmed") => {
                                trimmed = true;
                                i = j;
                                break;
                            }
                            Tok::Other(s) if s == "," => {
                                break;
                            }
                            _ => break,
                        }
                    }

                    targets.push(IntoTarget {
                        name: var_name,
                        trimmed,
                    });
                }
            }
        }
        i += 1;
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_table_gains_or_replace() {
        assert_eq!(
            rewrite("create table t as select 1;"),
            "CREATE OR REPLACE TABLE t as select 1;"
        );
    }

    #[test]
    fn create_or_replace_is_left_alone() {
        let s = "CREATE OR REPLACE TABLE t AS SELECT 1;";
        assert_eq!(rewrite(s), s);
    }

    #[test]
    fn calculated_keyword_is_stripped() {
        let s = "select x*2 as foo, calculated foo + 1 as bar from t;";
        let out = rewrite(s);
        // Crucially: no "calculated" token in the output.
        assert!(!out.to_lowercase().contains("calculated"));
        assert!(out.contains("foo + 1 as bar"));
    }

    #[test]
    fn calculated_inside_string_survives() {
        let s = "select 'calculated x' as note;";
        assert_eq!(rewrite(s), s);
    }

    #[test]
    fn monotonic_becomes_row_number() {
        let out = rewrite("select monotonic() as rn, x from t;");
        assert!(out.contains("row_number() over ()"));
        assert!(!out.contains("monotonic"));
    }

    #[test]
    fn outer_union_corr_becomes_union_all_by_name() {
        let out = rewrite("select a from t1 outer union corr select b from t2;");
        let low = out.to_ascii_lowercase();
        assert!(low.contains("union all by name"));
        assert!(!low.contains("outer union"));
    }

    #[test]
    fn outer_union_without_corr_becomes_union_all() {
        let out = rewrite("select a from t1 outer union select b from t2;");
        let low = out.to_ascii_lowercase();
        assert!(low.contains("union all"));
        assert!(!low.contains("by name"));
        assert!(!low.contains("outer union"));
    }

    #[test]
    fn left_outer_join_is_not_disturbed() {
        // `outer` followed by `join` (or anything other than `union`)
        // must pass through.
        let s = "select * from a left outer join b on a.id = b.id;";
        assert_eq!(rewrite(s), s);
    }

    #[test]
    fn comments_and_whitespace_preserved() {
        let s = "select /* keep me */ x as y, calculated y + 1 from t;";
        let out = rewrite(s);
        assert!(out.contains("/* keep me */"));
        assert!(out.contains("y + 1"));
    }

    #[test]
    fn test_extract_into_clause_single() {
        let sql = "select count(*) into :n_emp trimmed from raw_employees;";
        let (rewritten, targets) = extract_into_clause(sql);
        assert_eq!(rewritten.trim(), "select count(*) from raw_employees;");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "n_emp");
        assert!(targets[0].trimmed);
    }

    #[test]
    fn test_extract_into_clause_multiple() {
        let sql = "select count(*), max(salary) into :n_emp trimmed, :max_sal from raw_employees;";
        let (rewritten, targets) = extract_into_clause(sql);
        assert_eq!(
            rewritten.trim(),
            "select count(*), max(salary) from raw_employees;"
        );
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].name, "n_emp");
        assert!(targets[0].trimmed);
        assert_eq!(targets[1].name, "max_sal");
        assert!(!targets[1].trimmed);
    }
}
