//! Minimal SAS macro preprocessor (v1.0).
//!
//! Supported:
//! - `%let name = value;`  — sets a session-scoped macro variable. The
//!   value may itself reference `&other` macro variables.
//! - `%put text;`          — captured and returned to the caller for
//!   emission as log Notes. `&var` references in the text are expanded.
//! - `&name` / `&name.`    — substitutes the value of the macro variable;
//!   leaves the reference untouched if no such variable exists.
//!
//! Macro expansion is **disabled** inside `'…'` single-quoted strings and
//! **enabled** inside `"…"` double-quoted strings, matching SAS.

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct PreprocessOutput {
    /// Source text with macro directives stripped and `&var` references
    /// expanded.
    pub expanded: String,
    /// Captured `%put` texts (already &-expanded), in source order.
    pub puts: Vec<String>,
}

pub fn preprocess(src: &str, vars: &mut HashMap<String, String>) -> PreprocessOutput {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut puts = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // Single-quoted: copy verbatim, honoring doubled-quote escape.
            out.push('\'');
            i += 1;
            while i < chars.len() {
                let d = chars[i];
                out.push(d);
                i += 1;
                if d == '\'' {
                    if i < chars.len() && chars[i] == '\'' {
                        out.push('\'');
                        i += 1;
                        continue;
                    }
                    break;
                }
            }
            continue;
        }
        if c == '"' {
            // Double-quoted: copy + expand & references inside.
            out.push('"');
            i += 1;
            while i < chars.len() {
                if chars[i] == '&' {
                    let (exp, adv) = expand_amp(&chars[i..], vars);
                    out.push_str(&exp);
                    i += adv;
                    continue;
                }
                let d = chars[i];
                out.push(d);
                i += 1;
                if d == '"' {
                    if i < chars.len() && chars[i] == '"' {
                        out.push('"');
                        i += 1;
                        continue;
                    }
                    break;
                }
            }
            continue;
        }
        if c == '%' && i + 1 < chars.len() {
            if let Some((kw, kw_len)) = match_macro_keyword(&chars[i + 1..]) {
                // Consume `%` and the keyword.
                i += 1 + kw_len;
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                match kw {
                    "let" => {
                        // %let name = value;
                        let (name, name_len) = read_ident(&chars[i..]);
                        i += name_len;
                        while i < chars.len() && chars[i].is_whitespace() {
                            i += 1;
                        }
                        if i < chars.len() && chars[i] == '=' {
                            i += 1;
                        }
                        while i < chars.len() && chars[i].is_whitespace() {
                            i += 1;
                        }
                        let mut value = String::new();
                        while i < chars.len() && chars[i] != ';' {
                            if chars[i] == '&' {
                                let (exp, adv) = expand_amp(&chars[i..], vars);
                                value.push_str(&exp);
                                i += adv;
                            } else {
                                value.push(chars[i]);
                                i += 1;
                            }
                        }
                        if i < chars.len() && chars[i] == ';' {
                            i += 1;
                        }
                        if !name.is_empty() {
                            vars.insert(name.to_ascii_lowercase(), value.trim().to_string());
                        }
                        continue;
                    }
                    "put" => {
                        let mut text = String::new();
                        while i < chars.len() && chars[i] != ';' {
                            if chars[i] == '&' {
                                let (exp, adv) = expand_amp(&chars[i..], vars);
                                text.push_str(&exp);
                                i += adv;
                            } else {
                                text.push(chars[i]);
                                i += 1;
                            }
                        }
                        if i < chars.len() && chars[i] == ';' {
                            i += 1;
                        }
                        puts.push(text.trim().to_string());
                        continue;
                    }
                    _ => unreachable!(),
                }
            }
            // Unknown macro directive — pass `%` through and let the
            // downstream parser complain.
            out.push('%');
            i += 1;
            continue;
        }
        if c == '&' {
            let (exp, adv) = expand_amp(&chars[i..], vars);
            out.push_str(&exp);
            i += adv;
            continue;
        }
        out.push(c);
        i += 1;
    }
    PreprocessOutput { expanded: out, puts }
}

fn match_macro_keyword(chars: &[char]) -> Option<(&'static str, usize)> {
    let s: String = chars.iter().take(4).collect();
    let lower = s.to_ascii_lowercase();
    let after_kw = |n: usize| -> bool {
        chars
            .get(n)
            .map(|c| !c.is_ascii_alphanumeric() && *c != '_')
            .unwrap_or(true)
    };
    if lower.starts_with("let") && after_kw(3) {
        return Some(("let", 3));
    }
    if lower.starts_with("put") && after_kw(3) {
        return Some(("put", 3));
    }
    None
}

fn read_ident(chars: &[char]) -> (String, usize) {
    let mut end = 0;
    while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    (chars[..end].iter().collect(), end)
}

fn expand_amp(chars: &[char], vars: &HashMap<String, String>) -> (String, usize) {
    debug_assert!(chars.first() == Some(&'&'));
    let mut end = 1;
    while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    if end == 1 {
        return ("&".to_string(), 1);
    }
    let name: String = chars[1..end].iter().collect();
    let key = name.to_ascii_lowercase();
    // The trailing `.` is a terminator for the reference (SAS lets you
    // write `&var.suffix` to glue them together).
    let consumed = if end < chars.len() && chars[end] == '.' { end + 1 } else { end };
    match vars.get(&key) {
        Some(v) => (v.clone(), consumed),
        None => (format!("&{}", name), consumed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn let_and_amp() {
        let mut vars = HashMap::new();
        let out = preprocess("%let x = 42; y = &x;", &mut vars);
        assert_eq!(vars.get("x"), Some(&"42".to_string()));
        assert_eq!(out.expanded.trim(), "y = 42;");
    }

    #[test]
    fn amp_with_dot_terminator() {
        let mut vars = HashMap::new();
        let out = preprocess("%let lib = work; data &lib..out; run;", &mut vars);
        assert!(out.expanded.contains("data work.out"));
    }

    #[test]
    fn single_quotes_disable_expansion() {
        let mut vars = HashMap::from([("x".into(), "42".into())]);
        let out = preprocess("a = '&x'; b = \"&x\";", &mut vars);
        assert!(out.expanded.contains("a = '&x'"));
        assert!(out.expanded.contains("b = \"42\""));
    }

    #[test]
    fn put_captures_text() {
        let mut vars = HashMap::from([("name".into(), "Ada".into())]);
        let out = preprocess("%put hello &name;", &mut vars);
        assert_eq!(out.puts, vec!["hello Ada".to_string()]);
        assert!(out.expanded.trim().is_empty());
    }

    #[test]
    fn unknown_var_left_in_place() {
        let mut vars = HashMap::new();
        let out = preprocess("y = &nope;", &mut vars);
        assert!(out.expanded.contains("&nope"));
    }

    #[test]
    fn let_value_can_reference_other_var() {
        let mut vars = HashMap::new();
        preprocess("%let a = 1; %let b = a is &a;", &mut vars);
        assert_eq!(vars.get("b"), Some(&"a is 1".to_string()));
    }
}
