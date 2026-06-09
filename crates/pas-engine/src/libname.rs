//! Parse PAS-flavored `libname` statements.
//!
//! Supported forms (v0.2):
//! ```pas
//! libname work;                     -- ignored; built-in
//! libname raw   "/data/landing" format=parquet;   -- defaults to DIR
//! libname raw   dir "/data/landing" format=csv;
//! libname store duckdb "/data/store.duckdb";
//! libname x clear;                  -- v0.3
//! ```

use crate::library::{DirFormat, LibraryKind};

#[derive(Debug, Clone)]
pub struct LibnameDef {
    pub name: String,
    pub kind: LibraryKind,
    pub path: String,
    pub format: Option<DirFormat>,
}

#[derive(Debug, thiserror::Error)]
pub enum LibnameError {
    #[error("libname syntax: {0}")]
    Syntax(String),
}

/// Try to parse a single semicolon-stripped statement as a libname.
/// Returns `Ok(None)` if it's not a libname (and the caller should treat it
/// as SQL). Returns `Ok(Some(def))` on success.
pub fn parse(stmt: &str) -> Result<Option<LibnameDef>, LibnameError> {
    let s = stmt.trim();
    let lower = s.to_ascii_lowercase();
    if !lower.starts_with("libname") {
        return Ok(None);
    }
    let rest = &s["libname".len()..];
    let mut toks = tokenize(rest);

    let name = toks
        .next()
        .ok_or_else(|| LibnameError::Syntax("missing library name".into()))?;

    // Optional explicit kind.
    let mut kind: Option<LibraryKind> = None;
    let mut path: Option<String> = None;
    let mut format: Option<DirFormat> = None;

    let peek = toks.peek().cloned();
    if let Some(tok) = peek.as_ref() {
        let low = tok.to_ascii_lowercase();
        match low.as_str() {
            "duckdb" => {
                kind = Some(LibraryKind::Duckdb);
                toks.next();
            }
            "dir" => {
                kind = Some(LibraryKind::Dir);
                toks.next();
            }
            "memory" => {
                kind = Some(LibraryKind::Memory);
                toks.next();
            }
            _ => {}
        }
    }

    // Next non-option token is the path (must be quoted in our v0.2 syntax).
    for tok in toks {
        if let Some(eq) = tok.find('=') {
            let key = tok[..eq].to_ascii_lowercase();
            let val = &tok[eq + 1..];
            if key == "format" {
                format = DirFormat::from_ext(unquote(val))
                    .ok_or_else(|| LibnameError::Syntax(format!("unknown format: {}", val)))
                    .map(Some)?;
            }
            continue;
        }
        if path.is_none() {
            path = Some(unquote(&tok).to_string());
        }
    }

    let resolved_kind = kind.unwrap_or_else(|| {
        // Heuristic default.
        match path.as_deref() {
            Some(p) if p.ends_with(".duckdb") || p.ends_with(".db") => LibraryKind::Duckdb,
            Some(_) => LibraryKind::Dir,
            None => LibraryKind::Memory,
        }
    });

    if resolved_kind != LibraryKind::Memory && path.is_none() {
        return Err(LibnameError::Syntax("missing path".into()));
    }

    let raw_path = path.unwrap_or_default();
    let normalized_path = if raw_path.contains('\\') {
        raw_path.replace('\\', "/")
    } else {
        raw_path
    };

    Ok(Some(LibnameDef {
        name: name.to_ascii_lowercase(),
        kind: resolved_kind,
        path: normalized_path,
        format,
    }))
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'') && last == first {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Whitespace-separated tokenizer that respects quoted strings.
fn tokenize(s: &str) -> std::iter::Peekable<std::vec::IntoIter<String>> {
    let bytes = s.as_bytes();
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            let q = b;
            cur.push(b as char);
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                cur.push(c as char);
                i += 1;
                if c == q {
                    break;
                }
            }
            continue;
        }
        cur.push(b as char);
        i += 1;
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens.into_iter().peekable()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dir_default() {
        let def = parse(r#"libname raw "/data/landing""#).unwrap().unwrap();
        assert_eq!(def.name, "raw");
        assert_eq!(def.kind, LibraryKind::Dir);
        assert_eq!(def.path, "/data/landing");
    }

    #[test]
    fn parses_duckdb_explicit() {
        let def = parse(r#"libname store duckdb "/data/store.duckdb""#)
            .unwrap()
            .unwrap();
        assert_eq!(def.kind, LibraryKind::Duckdb);
    }

    #[test]
    fn parses_format_option() {
        let def = parse(r#"libname raw dir "/data" format=csv"#)
            .unwrap()
            .unwrap();
        assert_eq!(def.format, Some(DirFormat::Csv));
    }

    #[test]
    fn non_libname_returns_none() {
        assert!(parse("select 1").unwrap().is_none());
    }
}
