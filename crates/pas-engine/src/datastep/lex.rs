//! Tokenizer for the DATA step body.

/// Half-open byte range in the body text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
    pub fn point(pos: usize) -> Self {
        Self {
            start: pos,
            end: pos,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Number(f64),
    Str(String),
    Semi,
    Comma,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Plus,
    Minus,
    Star,
    Slash,
    Power,
    Eq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Concat,
    Dot,
    Dollar,
    Colon,
    Eof,
}

#[derive(Debug)]
pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
        }
    }

    #[allow(dead_code)]
    pub fn tokens(self) -> Result<Vec<Tok>, String> {
        Ok(self
            .tokens_with_spans()?
            .into_iter()
            .map(|(t, _)| t)
            .collect())
    }

    pub fn tokens_with_spans(mut self) -> Result<Vec<(Tok, Span)>, String> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            let start = self.pos;
            let t = self.next_token()?;
            let span = Span::new(start, self.pos);
            let is_eof = matches!(t, Tok::Eof);
            out.push((t, span));
            if is_eof {
                break;
            }
        }
        Ok(out)
    }

    fn peek(&self) -> u8 {
        if self.pos < self.src.len() {
            self.src[self.pos]
        } else {
            0
        }
    }

    fn peek_at(&self, n: usize) -> u8 {
        if self.pos + n < self.src.len() {
            self.src[self.pos + n]
        } else {
            0
        }
    }

    fn bump(&mut self) -> u8 {
        let b = self.peek();
        self.pos += 1;
        b
    }

    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn next_token(&mut self) -> Result<Tok, String> {
        if self.pos >= self.src.len() {
            return Ok(Tok::Eof);
        }
        let b = self.peek();
        match b {
            b';' => {
                self.bump();
                Ok(Tok::Semi)
            }
            b',' => {
                self.bump();
                Ok(Tok::Comma)
            }
            b':' => {
                self.bump();
                Ok(Tok::Colon)
            }
            b'(' => {
                self.bump();
                Ok(Tok::LParen)
            }
            b')' => {
                self.bump();
                Ok(Tok::RParen)
            }
            b'{' => {
                self.bump();
                Ok(Tok::LBrace)
            }
            b'}' => {
                self.bump();
                Ok(Tok::RBrace)
            }
            b'[' => {
                self.bump();
                Ok(Tok::LBracket)
            }
            b']' => {
                self.bump();
                Ok(Tok::RBracket)
            }
            b'+' => {
                self.bump();
                Ok(Tok::Plus)
            }
            b'-' => {
                self.bump();
                Ok(Tok::Minus)
            }
            b'$' => {
                self.bump();
                Ok(Tok::Dollar)
            }
            b'.' => {
                if self.peek_at(1).is_ascii_digit() {
                    self.read_number()
                } else {
                    self.bump();
                    Ok(Tok::Dot)
                }
            }
            b'*' => {
                self.bump();
                if self.peek() == b'*' {
                    self.bump();
                    Ok(Tok::Power)
                } else {
                    Ok(Tok::Star)
                }
            }
            b'/' => {
                self.bump();
                Ok(Tok::Slash)
            }
            b'=' => {
                self.bump();
                Ok(Tok::Eq)
            }
            b'<' => {
                self.bump();
                match self.peek() {
                    b'=' => {
                        self.bump();
                        Ok(Tok::Le)
                    }
                    b'>' => {
                        self.bump();
                        Ok(Tok::NotEq)
                    }
                    _ => Ok(Tok::Lt),
                }
            }
            b'>' => {
                self.bump();
                if self.peek() == b'=' {
                    self.bump();
                    Ok(Tok::Ge)
                } else {
                    Ok(Tok::Gt)
                }
            }
            b'!' => {
                self.bump();
                if self.peek() == b'=' {
                    self.bump();
                    Ok(Tok::NotEq)
                } else {
                    Err("unexpected '!' (use 'ne' or '<>')".into())
                }
            }
            b'|' => {
                self.bump();
                if self.peek() == b'|' {
                    self.bump();
                    Ok(Tok::Concat)
                } else {
                    Err("unexpected '|' (use '||' for concat)".into())
                }
            }
            b'\'' | b'"' => self.read_string(),
            d if d.is_ascii_digit() => self.read_number(),
            a if a.is_ascii_alphabetic() || a == b'_' => self.read_ident(),
            other => Err(format!("unexpected character {:?}", other as char)),
        }
    }

    fn read_number(&mut self) -> Result<Tok, String> {
        let start = self.pos;
        while self.pos < self.src.len()
            && (self.src[self.pos].is_ascii_digit() || self.src[self.pos] == b'.')
        {
            self.pos += 1;
        }
        if self.pos < self.src.len() && (self.src[self.pos] == b'e' || self.src[self.pos] == b'E') {
            self.pos += 1;
            if self.pos < self.src.len()
                && (self.src[self.pos] == b'+' || self.src[self.pos] == b'-')
            {
                self.pos += 1;
            }
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        let slice = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|e| format!("bad number: {}", e))?;
        slice
            .parse::<f64>()
            .map(Tok::Number)
            .map_err(|_| format!("bad number {:?}", slice))
    }

    fn read_string(&mut self) -> Result<Tok, String> {
        let quote = self.bump();
        let mut out = String::new();
        loop {
            if self.pos >= self.src.len() {
                return Err("unterminated string".into());
            }
            let c = self.src[self.pos];
            self.pos += 1;
            if c == quote {
                if self.pos < self.src.len() && self.src[self.pos] == quote {
                    out.push(quote as char);
                    self.pos += 1;
                    continue;
                }
                break;
            }
            out.push(c as char);
        }

        // Date/time/datetime suffixes:
        //   'DDMMMYYYY'd        → SAS date (days since 1960-01-01)
        //   'HH:MM[:SS]'t       → SAS time (seconds since midnight)
        //   'DDMMMYYYY:HH:MM:SS'dt → SAS datetime (seconds since 1960-01-01)
        if self.pos < self.src.len() {
            let p = self.src[self.pos];
            if p == b'd' || p == b'D' {
                let is_dt = self.pos + 1 < self.src.len()
                    && (self.src[self.pos + 1] == b't' || self.src[self.pos + 1] == b'T');
                if is_dt {
                    self.pos += 2;
                    let n = parse_sas_datetime(&out)
                        .map_err(|e| format!("bad datetime literal {:?}: {}", out, e))?;
                    return Ok(Tok::Number(n));
                }
                self.pos += 1;
                let n = parse_sas_date(&out)
                    .map_err(|e| format!("bad date literal {:?}: {}", out, e))?;
                return Ok(Tok::Number(n));
            }
            if p == b't' || p == b'T' {
                self.pos += 1;
                let n = parse_sas_time(&out)
                    .map_err(|e| format!("bad time literal {:?}: {}", out, e))?;
                return Ok(Tok::Number(n));
            }
        }
        Ok(Tok::Str(out))
    }

    fn read_ident(&mut self) -> Result<Tok, String> {
        let start = self.pos;
        while self.pos < self.src.len()
            && (self.src[self.pos].is_ascii_alphanumeric() || self.src[self.pos] == b'_')
        {
            self.pos += 1;
        }
        let s = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|e| format!("bad ident: {}", e))?
            .to_ascii_lowercase();
        Ok(match s.as_str() {
            "eq" => Tok::Eq,
            "ne" => Tok::NotEq,
            "lt" => Tok::Lt,
            "le" => Tok::Le,
            "gt" => Tok::Gt,
            "ge" => Tok::Ge,
            _ => Tok::Ident(s),
        })
    }
}

/// SAS date string `DDMMMYYYY` → days since 1960-01-01.
pub(crate) fn parse_sas_date(s: &str) -> Result<f64, String> {
    use chrono::NaiveDate;
    let s = s.trim();
    if s.len() < 9 {
        return Err(format!("expected DDMMMYYYY, got {:?}", s));
    }
    let day: u32 = s[..2].parse().map_err(|_| "bad day")?;
    let mon = month_from_abbr(&s[2..5])?;
    let year: i32 = s[5..].parse().map_err(|_| "bad year")?;
    let d = NaiveDate::from_ymd_opt(year, mon, day).ok_or("invalid date")?;
    let base = NaiveDate::from_ymd_opt(1960, 1, 1).unwrap();
    Ok((d - base).num_days() as f64)
}

/// SAS time string `HH:MM[:SS[.fff]]` → seconds since midnight.
pub(crate) fn parse_sas_time(s: &str) -> Result<f64, String> {
    let s = s.trim();
    let mut parts = s.split(':');
    let h: u32 = parts
        .next()
        .ok_or("missing hour")?
        .parse()
        .map_err(|_| "bad hour")?;
    let m: u32 = parts
        .next()
        .ok_or("missing minute")?
        .parse()
        .map_err(|_| "bad minute")?;
    let sec: f64 = parts
        .next()
        .map(|p| p.parse().unwrap_or(0.0))
        .unwrap_or(0.0);
    Ok(h as f64 * 3600.0 + m as f64 * 60.0 + sec)
}

pub(crate) fn parse_sas_datetime(s: &str) -> Result<f64, String> {
    let s = s.trim();
    if s.len() < 9 {
        return Err(format!("expected DDMMMYYYY:HH..., got {:?}", s));
    }
    let (a, b) = s.split_at(9);
    let date = parse_sas_date(a)?;
    let time = parse_sas_time(b.trim_start_matches(':'))?;
    Ok(date * 86400.0 + time)
}

fn month_from_abbr(s: &str) -> Result<u32, String> {
    match s.to_ascii_uppercase().as_str() {
        "JAN" => Ok(1),
        "FEB" => Ok(2),
        "MAR" => Ok(3),
        "APR" => Ok(4),
        "MAY" => Ok(5),
        "JUN" => Ok(6),
        "JUL" => Ok(7),
        "AUG" => Ok(8),
        "SEP" => Ok(9),
        "OCT" => Ok(10),
        "NOV" => Ok(11),
        "DEC" => Ok(12),
        other => Err(format!("unknown month {:?}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(s: &str) -> Vec<Tok> {
        Lexer::new(s).tokens().unwrap()
    }

    #[test]
    fn numbers_and_idents() {
        let t = lex("x = 1.5 + foo");
        assert!(matches!(t[0], Tok::Ident(ref s) if s == "x"));
        assert!(matches!(t[1], Tok::Eq));
        assert!(matches!(t[2], Tok::Number(n) if (n - 1.5).abs() < 1e-9));
        assert!(matches!(t[3], Tok::Plus));
        assert!(matches!(t[4], Tok::Ident(ref s) if s == "foo"));
    }

    #[test]
    fn strings_with_doubled_quote() {
        let t = lex(r#"y = 'it''s'"#);
        assert!(matches!(&t[2], Tok::Str(s) if s == "it's"));
    }

    #[test]
    fn word_operators() {
        let t = lex("a eq b and c ne d");
        assert!(matches!(t[1], Tok::Eq));
        assert!(matches!(t[3], Tok::Ident(ref s) if s == "and"));
        assert!(matches!(t[5], Tok::NotEq));
    }

    #[test]
    fn date_literal() {
        let t = lex("d = '01JAN1960'd");
        // 1960-01-01 → 0 days
        assert!(matches!(t[2], Tok::Number(n) if n == 0.0));
    }

    #[test]
    fn time_literal() {
        let t = lex("t = '01:30:00't");
        assert!(matches!(t[2], Tok::Number(n) if (n - 5400.0).abs() < 1e-6));
    }
}
