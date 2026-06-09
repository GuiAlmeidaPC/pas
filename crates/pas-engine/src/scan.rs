//! Byte-level PAS source scanning primitives.
//!
//! Four places in the engine need to walk a PAS source byte-by-byte while
//! treating string literals and comments as opaque spans: the comment
//! stripper, the statement splitter, the PROC SQL preprocessor, and the
//! libref rewriter. Each used to roll its own quote/comment state machine,
//! and one of them (`rewrite_librefs`) silently disagreed with the others
//! on how to escape embedded quotes. Centralising the primitives here
//! keeps that interpretation in one place.

/// `bytes[start]` must be `'` or `"`. Returns the index one past the
/// closing quote.
///
/// PAS escapes an embedded quote by doubling it (`''` inside a `'...'`
/// literal is one literal apostrophe and does NOT terminate the string).
/// If the literal is unterminated, returns `bytes.len()`.
pub fn skip_string_literal(bytes: &[u8], start: usize) -> usize {
    debug_assert!(start < bytes.len());
    let quote = bytes[start];
    debug_assert!(quote == b'\'' || quote == b'"');
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == quote {
            if i + 1 < bytes.len() && bytes[i + 1] == quote {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// `bytes[start..]` must begin with `/*`. Returns the index one past the
/// closing `*/`. PAS block comments do not nest. If unterminated, returns
/// `bytes.len()`.
pub fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    debug_assert!(start + 1 < bytes.len() && bytes[start] == b'/' && bytes[start + 1] == b'*');
    let mut i = start + 2;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_simple_single_quotes() {
        let s = b"'hello' rest";
        assert_eq!(skip_string_literal(s, 0), 7);
    }

    #[test]
    fn string_simple_double_quotes() {
        let s = b"\"hi\" rest";
        assert_eq!(skip_string_literal(s, 0), 4);
    }

    #[test]
    fn string_doubled_single_quote_is_escape() {
        // 'O''Brien' → one literal token of length 10.
        let s = b"'O''Brien' rest";
        assert_eq!(skip_string_literal(s, 0), 10);
    }

    #[test]
    fn string_doubled_double_quote_is_escape() {
        let s = b"\"a\"\"b\" rest";
        assert_eq!(skip_string_literal(s, 0), 6);
    }

    #[test]
    fn string_unterminated_runs_to_end() {
        let s = b"'never ends";
        assert_eq!(skip_string_literal(s, 0), s.len());
    }

    #[test]
    fn string_quote_immediately_followed_by_eof() {
        // '' at EOF is an empty literal, not an open escape.
        let s = b"''";
        assert_eq!(skip_string_literal(s, 0), 2);
    }

    #[test]
    fn string_alternate_quote_inside_unaffected() {
        // Double quotes inside a single-quoted string are content, not
        // structure.
        let s = b"'a\"b' rest";
        assert_eq!(skip_string_literal(s, 0), 5);
    }

    #[test]
    fn block_comment_simple() {
        let s = b"/* xx */rest";
        assert_eq!(skip_block_comment(s, 0), 8);
    }

    #[test]
    fn block_comment_with_star_inside() {
        let s = b"/* a * b */rest";
        assert_eq!(skip_block_comment(s, 0), 11);
    }

    #[test]
    fn block_comment_unterminated_runs_to_end() {
        let s = b"/* never ends";
        assert_eq!(skip_block_comment(s, 0), s.len());
    }
}
