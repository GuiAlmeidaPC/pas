//! Small parsing helpers shared across PROC handlers.

use std::collections::HashMap;

/// Split a proc body (which is the joined-by-`;` text after the proc name)
/// into the header-options chunk and a list of subsequent statements.
///
/// Example body: `"data=in out=out nodupkey;by x descending y"`
/// returns `("data=in out=out nodupkey", ["by x descending y"])`.
pub fn split_body(body: &str) -> (String, Vec<String>) {
    let parts: Vec<&str> = body
        .split(';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let header = parts.first().copied().unwrap_or("").to_string();
    let rest = parts.iter().skip(1).map(|s| s.to_string()).collect();
    (header, rest)
}

/// Parse `key=value` and bare-flag options, e.g.
/// `data=in out=out nodupkey label`.
///
/// The value runs from `=` until the next whitespace. Bare flags map to
/// `None`. Keys are lowercased; values keep their case.
pub fn parse_options(s: &str) -> HashMap<String, Option<String>> {
    let mut out = HashMap::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_alphanumeric() || c == '_' {
                key.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if key.is_empty() {
            chars.next();
            continue;
        }
        if chars.peek() == Some(&'=') {
            chars.next();
            let mut value = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                value.push(c);
                chars.next();
            }
            out.insert(key.to_ascii_lowercase(), Some(value));
        } else {
            out.insert(key.to_ascii_lowercase(), None);
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct ByVar {
    pub name: String,
    pub descending: bool,
}

/// Parse a `by` clause body (everything after the `by` keyword). The
/// `descending` keyword applies to the variable that immediately follows.
pub fn parse_by_clause(s: &str) -> Vec<ByVar> {
    let mut out = Vec::new();
    let mut next_desc = false;
    for word in s.split_whitespace() {
        let lw = word.to_ascii_lowercase();
        if lw == "descending" || lw == "desc" {
            next_desc = true;
        } else {
            out.push(ByVar { name: lw, descending: next_desc });
            next_desc = false;
        }
    }
    out
}

/// Parse a whitespace- or comma-separated identifier list.
pub fn parse_name_list(s: &str) -> Vec<String> {
    s.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_flags_and_kv() {
        let m = parse_options("data=lib.in out=lib.out nodupkey");
        assert_eq!(m["data"], Some("lib.in".to_string()));
        assert_eq!(m["out"], Some("lib.out".to_string()));
        assert!(m.contains_key("nodupkey"));
        assert_eq!(m["nodupkey"], None);
    }

    #[test]
    fn by_descending() {
        let v = parse_by_clause("x descending y z");
        assert_eq!(v[0], ByVar { name: "x".into(), descending: false });
        assert_eq!(v[1], ByVar { name: "y".into(), descending: true });
        assert_eq!(v[2], ByVar { name: "z".into(), descending: false });
    }

    #[test]
    fn split_body_extracts_header() {
        let (h, rest) = split_body("data=in out=out;by x;var y");
        assert_eq!(h, "data=in out=out");
        assert_eq!(rest, vec!["by x".to_string(), "var y".to_string()]);
    }
}
