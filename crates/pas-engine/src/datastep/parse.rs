//! Recursive-descent parser for the v0.4 DATA step.

use super::ast::*;
use super::lex::{Lexer, Span, Tok};

/// Parse error with a source span (byte offsets relative to the body text
/// passed to the parser).
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[allow(dead_code)]
pub fn parse_data_step(src: &str) -> Result<DataStep, ParseError> {
    parse_data_step_with_datalines(src, Vec::new())
}

pub fn parse_data_step_with_datalines(
    src: &str,
    datalines: Vec<String>,
) -> Result<DataStep, ParseError> {
    let toks = Lexer::new(src)
        .tokens_with_spans()
        .map_err(|m| ParseError {
            message: m,
            span: Span::point(0),
        })?;
    let mut p = Parser { toks, pos: 0, src };
    let mut ds = p.parse_data_step()?;
    ds.datalines = datalines;
    Ok(ds)
}

struct Parser<'a> {
    toks: Vec<(Tok, Span)>,
    pos: usize,
    src: &'a str,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].0
    }
    fn current_span(&self) -> Span {
        self.toks[self.pos].1
    }
    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].0.clone();
        self.pos += 1;
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, t: &Tok, ctx: &str) -> Result<(), ParseError> {
        if self.eat(t) {
            Ok(())
        } else {
            Err(self.err(format!(
                "expected {:?} in {}, found {:?}",
                t,
                ctx,
                self.peek()
            )))
        }
    }
    fn at_keyword(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == kw)
    }
    fn eat_keyword(&mut self, kw: &str) -> bool {
        if self.at_keyword(kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn err(&self, message: String) -> ParseError {
        ParseError {
            message,
            span: self.current_span(),
        }
    }

    fn parse_data_step(&mut self) -> Result<DataStep, ParseError> {
        if !self.eat_keyword("data") {
            return Err(self.err(format!(
                "DATA step must start with `data`, got {:?}",
                self.peek()
            )));
        }
        let outputs = self.parse_table_list()?;
        self.expect(&Tok::Semi, "data header")?;

        let mut ds = DataStep {
            outputs,
            input: None,
            by: Vec::new(),
            where_expr: None,
            keep: None,
            drop: None,
            lengths: Vec::new(),
            retain: Vec::new(),
            arrays: Vec::new(),
            formats: Vec::new(),
            input_vars: Vec::new(),
            datalines: Vec::new(),
            infile: None,
            body: Vec::new(),
        };

        loop {
            while self.eat(&Tok::Semi) {}
            if matches!(self.peek(), Tok::Eof) || self.at_keyword("run") {
                self.eat_keyword("run");
                self.eat(&Tok::Semi);
                break;
            }
            self.parse_top_stmt(&mut ds)?;
        }

        Ok(ds)
    }

    fn parse_table_list(&mut self) -> Result<Vec<TableRef>, ParseError> {
        let mut out = vec![self.parse_table_ref()?];
        while !matches!(self.peek(), Tok::Semi | Tok::Eof) {
            out.push(self.parse_table_ref()?);
        }
        Ok(out)
    }

    fn parse_table_ref(&mut self) -> Result<TableRef, ParseError> {
        let first = match self.bump() {
            Tok::Ident(s) => s,
            other => return Err(self.err(format!("expected dataset name, got {:?}", other))),
        };
        let (libref, name) = if self.eat(&Tok::Dot) {
            let name = match self.bump() {
                Tok::Ident(s) => s,
                other => {
                    return Err(self.err(format!("expected table name after dot, got {:?}", other)))
                }
            };
            (Some(first), name)
        } else {
            (None, first)
        };
        let mut in_var = None;
        if self.eat(&Tok::LParen) {
            loop {
                if self.eat(&Tok::RParen) {
                    break;
                }
                let option = match self.bump() {
                    Tok::Ident(s) => s,
                    other => {
                        return Err(
                            self.err(format!("expected dataset option name, got {:?}", other))
                        )
                    }
                };
                self.expect(&Tok::Eq, "dataset option")?;
                match option.as_str() {
                    "in" => {
                        let flag = match self.bump() {
                            Tok::Ident(s) => s,
                            other => {
                                return Err(self.err(format!(
                                    "expected variable name for in=, got {:?}",
                                    other
                                )))
                            }
                        };
                        if in_var.replace(flag).is_some() {
                            return Err(self.err("duplicate in= dataset option".into()));
                        }
                    }
                    other => {
                        return Err(
                            self.err(format!("dataset option '{}' is not supported yet", other))
                        )
                    }
                }
            }
        }
        Ok(TableRef {
            libref,
            name,
            in_var,
        })
    }

    fn parse_top_stmt(&mut self, ds: &mut DataStep) -> Result<(), ParseError> {
        if self.eat_keyword("set") {
            let mut sources = vec![self.parse_table_ref()?];
            while !matches!(self.peek(), Tok::Semi | Tok::Eof) {
                sources.push(self.parse_table_ref()?);
            }
            self.expect(&Tok::Semi, "set")?;
            if ds.input.is_some() {
                return Err(ParseError {
                    message: "multiple set/merge statements in one data step are not supported"
                        .into(),
                    span: self.current_span(),
                });
            }
            ds.input = Some(DataInput::Set(sources));
            return Ok(());
        }
        if self.eat_keyword("merge") {
            let mut sources = vec![self.parse_table_ref()?];
            while !matches!(self.peek(), Tok::Semi | Tok::Eof) {
                sources.push(self.parse_table_ref()?);
            }
            self.expect(&Tok::Semi, "merge")?;
            if ds.input.is_some() {
                return Err(ParseError {
                    message: "multiple set/merge statements in one data step are not supported"
                        .into(),
                    span: self.current_span(),
                });
            }
            if sources.len() < 2 {
                return Err(ParseError {
                    message: "merge requires at least two datasets".into(),
                    span: self.current_span(),
                });
            }
            ds.input = Some(DataInput::Merge(sources));
            return Ok(());
        }
        if self.eat_keyword("by") {
            let names = self.parse_name_list()?;
            self.expect(&Tok::Semi, "by")?;
            ds.by = names;
            return Ok(());
        }
        if self.eat_keyword("where") {
            let e = self.parse_expr()?;
            self.expect(&Tok::Semi, "where")?;
            ds.where_expr = Some(e);
            return Ok(());
        }
        if self.eat_keyword("keep") {
            let names = self.parse_name_list()?;
            self.expect(&Tok::Semi, "keep")?;
            ds.keep = Some(names);
            return Ok(());
        }
        if self.eat_keyword("drop") {
            let names = self.parse_name_list()?;
            self.expect(&Tok::Semi, "drop")?;
            ds.drop = Some(names);
            return Ok(());
        }
        if self.eat_keyword("length") {
            let decls = self.parse_length_decls()?;
            self.expect(&Tok::Semi, "length")?;
            ds.lengths.extend(decls);
            return Ok(());
        }
        if self.eat_keyword("retain") {
            let decls = self.parse_retain_decls()?;
            self.expect(&Tok::Semi, "retain")?;
            ds.retain.extend(decls);
            return Ok(());
        }
        if self.eat_keyword("array") {
            let decl = self.parse_array_decl()?;
            self.expect(&Tok::Semi, "array")?;
            ds.arrays.push(decl);
            return Ok(());
        }
        if self.eat_keyword("input") {
            let vars = self.parse_input_vars()?;
            self.expect(&Tok::Semi, "input")?;
            ds.input_vars.extend(vars);
            return Ok(());
        }
        if self.eat_keyword("infile") {
            ds.infile = Some(self.parse_infile_spec()?);
            self.expect(&Tok::Semi, "infile")?;
            return Ok(());
        }
        // `datalines;` / `cards;` / `lines;` are placeholders here — the
        // actual data text was extracted before parsing. Just consume.
        if self.at_keyword("datalines") || self.at_keyword("cards") || self.at_keyword("lines") {
            self.bump();
            self.expect(&Tok::Semi, "datalines")?;
            return Ok(());
        }
        if self.eat_keyword("format") {
            let formats = self.parse_format_decls()?;
            self.expect(&Tok::Semi, "format")?;
            ds.formats.extend(formats);
            return Ok(());
        }
        // `informat` / `label` are accepted as no-ops for compatibility.
        if self.eat_keyword("informat") || self.eat_keyword("label") {
            while !matches!(self.peek(), Tok::Semi | Tok::Eof) {
                self.bump();
            }
            self.expect(&Tok::Semi, "informat/label")?;
            return Ok(());
        }

        let s = self.parse_stmt()?;
        ds.body.push(s);
        Ok(())
    }

    fn parse_name_list(&mut self) -> Result<Vec<String>, ParseError> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                Tok::Ident(_) => {
                    if let Tok::Ident(s) = self.bump() {
                        out.push(s);
                    }
                }
                Tok::Semi | Tok::Eof => break,
                Tok::Comma => {
                    self.bump();
                }
                other => return Err(self.err(format!("expected name, got {:?}", other))),
            }
        }
        Ok(out)
    }

    fn parse_length_decls(&mut self) -> Result<Vec<LengthDecl>, ParseError> {
        let mut out = Vec::new();
        while !matches!(self.peek(), Tok::Semi | Tok::Eof) {
            let name = match self.bump() {
                Tok::Ident(s) => s,
                other => return Err(self.err(format!("expected name in length, got {:?}", other))),
            };
            let is_char = self.eat(&Tok::Dollar);
            let width = match self.peek() {
                Tok::Number(n) => {
                    let n = *n;
                    self.bump();
                    n as u32
                }
                _ => 8,
            };
            out.push(LengthDecl {
                name,
                is_char,
                width,
            });
            if matches!(self.peek(), Tok::Comma) {
                self.bump();
            }
        }
        Ok(out)
    }

    fn parse_retain_decls(&mut self) -> Result<Vec<RetainDecl>, ParseError> {
        let mut out = Vec::new();
        while !matches!(self.peek(), Tok::Semi | Tok::Eof) {
            let name = match self.bump() {
                Tok::Ident(s) => s,
                Tok::Comma => continue,
                other => return Err(self.err(format!("expected name in retain, got {:?}", other))),
            };
            let initial = match self.peek() {
                Tok::Number(n) => {
                    let n = *n;
                    self.bump();
                    Some(n)
                }
                _ => None,
            };
            out.push(RetainDecl { name, initial });
        }
        Ok(out)
    }

    fn parse_format_decls(&mut self) -> Result<Vec<FormatDecl>, ParseError> {
        let mut out = Vec::new();
        let mut names = Vec::new();
        while !matches!(self.peek(), Tok::Semi | Tok::Eof) {
            match self.peek() {
                Tok::Ident(_) => {
                    let run = self.raw_run_at_current();
                    if run.contains('.') {
                        if names.is_empty() {
                            return Err(self.err("format requires at least one variable".into()));
                        }
                        let format = run.to_string();
                        self.advance_past(run);
                        for name in names.drain(..) {
                            out.push(FormatDecl {
                                name,
                                format: format.clone(),
                            });
                        }
                    } else if let Tok::Ident(name) = self.bump() {
                        names.push(name);
                    }
                }
                Tok::Dollar | Tok::Number(_) => {
                    let run = self.raw_run_at_current();
                    if run.is_empty() || !run.contains('.') {
                        return Err(self.err(format!("expected format, got {:?}", self.peek())));
                    }
                    if names.is_empty() {
                        return Err(self.err("format requires at least one variable".into()));
                    }
                    let format = run.to_string();
                    self.advance_past(run);
                    for name in names.drain(..) {
                        out.push(FormatDecl {
                            name,
                            format: format.clone(),
                        });
                    }
                }
                Tok::Comma => {
                    self.bump();
                }
                other => return Err(self.err(format!("unexpected token in format: {:?}", other))),
            }
        }
        if !names.is_empty() {
            return Err(self.err("format statement missing format specifier".into()));
        }
        Ok(out)
    }

    fn parse_infile_spec(&mut self) -> Result<InfileSpec, ParseError> {
        let raw_path = match self.bump() {
            Tok::Str(s) => s,
            other => {
                return Err(self.err(format!(
                    "expected quoted path after infile, got {:?}",
                    other
                )))
            }
        };
        let path = if raw_path.contains('\\') {
            raw_path.replace('\\', "/")
        } else {
            raw_path
        };
        let mut spec = InfileSpec {
            path,
            dlm: None,
            dsd: false,
            firstobs: 1,
        };
        loop {
            match self.peek().clone() {
                Tok::Semi | Tok::Eof => break,
                Tok::Ident(kw) => {
                    let kwl = kw.to_ascii_lowercase();
                    self.bump();
                    match kwl.as_str() {
                        "dsd" => spec.dsd = true,
                        "truncover" | "missover" | "stopover" | "flowover" => { /* tolerated; defaults match truncover */
                        }
                        "dlm" | "delimiter" => {
                            self.expect(&Tok::Eq, "dlm")?;
                            spec.dlm = Some(match self.bump() {
                                Tok::Str(s) => s,
                                other => {
                                    return Err(
                                        self.err(format!("expected dlm value, got {:?}", other))
                                    )
                                }
                            });
                        }
                        "firstobs" => {
                            self.expect(&Tok::Eq, "firstobs")?;
                            spec.firstobs = match self.bump() {
                                Tok::Number(n) => n as u64,
                                other => {
                                    return Err(self.err(format!(
                                        "expected number for firstobs, got {:?}",
                                        other
                                    )))
                                }
                            };
                        }
                        other => return Err(self.err(format!("unknown infile option {:?}", other))),
                    }
                }
                other => return Err(self.err(format!("unexpected token in infile: {:?}", other))),
            }
        }
        Ok(spec)
    }

    fn parse_input_vars(&mut self) -> Result<Vec<InputVar>, ParseError> {
        let mut out = Vec::new();
        while !matches!(self.peek(), Tok::Semi | Tok::Eof) {
            let name = match self.bump() {
                Tok::Ident(s) => s,
                Tok::Comma => continue,
                other => {
                    return Err(
                        self.err(format!("expected variable name in input, got {:?}", other))
                    )
                }
            };

            // `:informat.` modified-list input.
            let modified = self.eat(&Tok::Colon);

            // Column-range input: `[$] start[-end]` (no colon, no dot). Try this
            // before informat detection and backtrack if it doesn't match.
            if !modified {
                if let Some((is_char, start, end)) = self.try_column_range() {
                    out.push(InputVar {
                        name,
                        is_char,
                        informat: None,
                        reader: InputReader::Column { start, end },
                    });
                    continue;
                }
            }

            // An informat is the contiguous run of `$`/letters/digits/`.` that
            // starts at the current token. A lone `$` is plain character input;
            // a run containing `.` is an informat. We read it from raw source to
            // avoid the awkward tokenization of forms like `dollar12.2`.
            let run = self.raw_run_at_current();
            let (informat, reader) = if modified {
                let inf = parse_informat_str(run).map_err(|e| self.err(e))?;
                self.advance_past(run);
                (Some(inf), InputReader::Modified)
            } else if run == "$" {
                self.bump(); // consume the lone Dollar
                (None, InputReader::List)
            } else if run.starts_with('$') || run.contains('.') {
                let inf = parse_informat_str(run).map_err(|e| self.err(e))?;
                self.advance_past(run);
                (Some(inf), InputReader::Formatted)
            } else {
                (None, InputReader::List)
            };

            let is_char = match &informat {
                Some(inf) => inf.is_char(),
                None => matches!(reader, InputReader::List) && run == "$",
            };
            out.push(InputVar {
                name,
                is_char,
                informat,
                reader,
            });
        }
        Ok(out)
    }

    /// The contiguous `$`/alnum/`.`/`_` run beginning at the current token's
    /// source offset (used to read an informat verbatim).
    fn raw_run_at_current(&self) -> &'a str {
        let start = self.current_span().start;
        let bytes = self.src.as_bytes();
        let mut end = start;
        while end < bytes.len() {
            let c = bytes[end];
            if c.is_ascii_alphanumeric() || c == b'$' || c == b'.' || c == b'_' {
                end += 1;
            } else {
                break;
            }
        }
        &self.src[start..end]
    }

    /// Advance the token cursor past every token contained in `run` (matched by
    /// source offset).
    fn advance_past(&mut self, run: &str) {
        let start = self.current_span().start;
        let end = start + run.len();
        while self.pos < self.toks.len() && self.toks[self.pos].1.start < end {
            self.pos += 1;
        }
    }

    /// If the current token is an integer column number (a `Number` whose raw
    /// source has no `.`, distinguishing it from an informat width like `40.`),
    /// return its value. Does not consume.
    fn int_column_at_current(&self) -> Option<usize> {
        if let Tok::Number(n) = self.peek() {
            let span = self.current_span();
            let raw = &self.src[span.start..span.end];
            if !raw.contains('.') && *n >= 1.0 && n.fract() == 0.0 {
                return Some(*n as usize);
            }
        }
        None
    }

    /// Try to parse `[$] start[-end]` column-range input at the current
    /// position. Returns `(is_char, start, end)` (1-based inclusive) and leaves
    /// the cursor past the range, or restores the cursor and returns `None`.
    fn try_column_range(&mut self) -> Option<(bool, usize, usize)> {
        let save = self.pos;
        let is_char = self.eat(&Tok::Dollar);
        let start = match self.int_column_at_current() {
            Some(n) => {
                self.pos += 1;
                n
            }
            None => {
                self.pos = save;
                return None;
            }
        };
        let end = if matches!(self.peek(), Tok::Minus) {
            let before_minus = self.pos;
            self.pos += 1;
            match self.int_column_at_current() {
                Some(m) => {
                    self.pos += 1;
                    m
                }
                None => {
                    self.pos = before_minus; // lone column, leave the '-'
                    start
                }
            }
        } else {
            start
        };
        if end < start {
            self.pos = save;
            return None;
        }
        Some((is_char, start, end))
    }

    fn parse_array_decl(&mut self) -> Result<ArrayDecl, ParseError> {
        // array <name>{<size> | *} [$] [<width>] [<element list>]
        let name = match self.bump() {
            Tok::Ident(s) => s,
            other => return Err(self.err(format!("expected array name, got {:?}", other))),
        };
        // Subscript opener.
        let close = if self.eat(&Tok::LBrace) {
            Tok::RBrace
        } else if self.eat(&Tok::LBracket) {
            Tok::RBracket
        } else if self.eat(&Tok::LParen) {
            Tok::RParen
        } else {
            return Err(self.err(format!(
                "expected '{{' / '[' / '(' after array name, got {:?}",
                self.peek()
            )));
        };

        let size_tok = self.bump();
        let explicit_size: Option<usize> = match size_tok {
            Tok::Number(n) => Some(n as usize),
            Tok::Star => None, // {*} — size inferred from element list
            other => {
                return Err(self.err(format!("expected size or '*' in array, got {:?}", other)))
            }
        };
        self.expect(&close, "array size")?;

        let is_char = self.eat(&Tok::Dollar);
        // Optional width — skip if next is number followed by an identifier or end.
        if matches!(self.peek(), Tok::Number(_)) {
            // Treat as width and discard for v0.4; engine doesn't enforce widths yet.
            self.bump();
        }

        let mut elements = Vec::new();
        while let Tok::Ident(s) = self.peek().clone() {
            self.bump();
            elements.push(s);
            if matches!(self.peek(), Tok::Comma) {
                self.bump();
            }
        }

        let size = explicit_size.unwrap_or(elements.len());
        if size == 0 {
            return Err(self.err(format!("array {} has size 0", name)));
        }
        if !elements.is_empty() && elements.len() != size {
            return Err(self.err(format!(
                "array {} declared size {} but {} elements listed",
                name,
                size,
                elements.len()
            )));
        }
        Ok(ArrayDecl {
            name,
            size,
            is_char,
            elements,
        })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        if self.eat_keyword("if") {
            return self.parse_if();
        }
        if self.eat_keyword("call") {
            let name = match self.bump() {
                Tok::Ident(s) => s.to_lowercase(),
                other => {
                    return Err(self.err(format!("expected CALL routine name, got {:?}", other)))
                }
            };
            self.expect(&Tok::LParen, "call arguments")?;
            let mut args = Vec::new();
            if !self.eat(&Tok::RParen) {
                args.push(self.parse_expr()?);
                while self.eat(&Tok::Comma) {
                    args.push(self.parse_expr()?);
                }
                self.expect(&Tok::RParen, "call arguments list")?;
            }
            self.expect(&Tok::Semi, "call statement")?;
            return Ok(Stmt::Call { name, args });
        }
        if self.eat_keyword("output") {
            let target = if matches!(self.peek(), Tok::Ident(_)) {
                Some(self.parse_table_ref()?)
            } else {
                None
            };
            self.expect(&Tok::Semi, "output")?;
            return Ok(Stmt::Output { dataset: target });
        }
        if self.eat_keyword("delete") {
            self.expect(&Tok::Semi, "delete")?;
            return Ok(Stmt::Delete);
        }
        if self.eat_keyword("select") {
            let switch = if self.eat(&Tok::LParen) {
                let e = self.parse_expr()?;
                self.expect(&Tok::RParen, "select expression")?;
                Some(e)
            } else {
                None
            };
            self.expect(&Tok::Semi, "select")?;
            let mut branches = Vec::new();
            let mut otherwise: Option<Box<Stmt>> = None;
            loop {
                while self.eat(&Tok::Semi) {}
                if self.at_keyword("end") {
                    self.bump();
                    self.expect(&Tok::Semi, "select end")?;
                    break;
                }
                if self.eat_keyword("when") {
                    self.expect(&Tok::LParen, "when")?;
                    let mut values = vec![self.parse_expr()?];
                    while self.eat(&Tok::Comma) {
                        values.push(self.parse_expr()?);
                    }
                    self.expect(&Tok::RParen, "when")?;
                    let stmt = Box::new(self.parse_stmt()?);
                    branches.push(SelectBranch { values, stmt });
                    continue;
                }
                if self.eat_keyword("otherwise") {
                    otherwise = Some(Box::new(self.parse_stmt()?));
                    continue;
                }
                return Err(self.err(format!(
                    "expected when/otherwise/end inside select, got {:?}",
                    self.peek()
                )));
            }
            return Ok(Stmt::Select {
                switch,
                branches,
                otherwise,
            });
        }
        if self.eat_keyword("do") {
            // Three forms after `do`:
            //   do;                                   block
            //   do while(cond); / do until(cond);     conditional loop
            //   do var = a to b [by c];               iterative loop
            if self.eat(&Tok::Semi) {
                return Ok(Stmt::Block(self.parse_do_body()?));
            }
            if self.eat_keyword("while") {
                self.expect(&Tok::LParen, "do while")?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen, "do while")?;
                self.expect(&Tok::Semi, "do while header")?;
                let body = self.parse_do_body()?;
                return Ok(Stmt::DoWhile { cond, body });
            }
            if self.eat_keyword("until") {
                self.expect(&Tok::LParen, "do until")?;
                let cond = self.parse_expr()?;
                self.expect(&Tok::RParen, "do until")?;
                self.expect(&Tok::Semi, "do until header")?;
                let body = self.parse_do_body()?;
                return Ok(Stmt::DoUntil { cond, body });
            }
            // Iterative form.
            let var = match self.bump() {
                Tok::Ident(s) => s,
                other => return Err(self.err(format!("expected loop var, got {:?}", other))),
            };
            self.expect(&Tok::Eq, "iterative do")?;
            let start = self.parse_expr()?;
            if !self.eat_keyword("to") {
                return Err(self.err(format!(
                    "expected 'to' in iterative do, got {:?}",
                    self.peek()
                )));
            }
            let stop = self.parse_expr()?;
            let step = if self.eat_keyword("by") {
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.expect(&Tok::Semi, "iterative do header")?;
            let body = self.parse_do_body()?;
            return Ok(Stmt::DoLoop {
                var,
                start,
                stop,
                step,
                body,
            });
        }
        // Assignment: ident [`{expr}` | `[expr]`] = expr ;
        let name = match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                s
            }
            other => return Err(self.err(format!("unexpected statement start: {:?}", other))),
        };
        let target = if self.eat(&Tok::LBrace) {
            let idx = self.parse_expr()?;
            self.expect(&Tok::RBrace, "array index")?;
            AssignTarget::ArrayElem { name, index: idx }
        } else if self.eat(&Tok::LBracket) {
            let idx = self.parse_expr()?;
            self.expect(&Tok::RBracket, "array index")?;
            AssignTarget::ArrayElem { name, index: idx }
        } else {
            AssignTarget::Var(name)
        };
        self.expect(&Tok::Eq, "assignment")?;
        let expr = self.parse_expr()?;
        self.expect(&Tok::Semi, "assignment")?;
        Ok(Stmt::Assign { target, expr })
    }

    fn parse_do_body(&mut self) -> Result<Vec<Stmt>, ParseError> {
        let mut body = Vec::new();
        while !self.at_keyword("end") {
            if matches!(self.peek(), Tok::Eof) {
                return Err(ParseError {
                    message: "unterminated do/end".into(),
                    span: self.current_span(),
                });
            }
            while self.eat(&Tok::Semi) {}
            if self.at_keyword("end") {
                break;
            }
            body.push(self.parse_stmt()?);
        }
        self.eat_keyword("end");
        self.expect(&Tok::Semi, "end")?;
        Ok(body)
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        let cond = self.parse_expr()?;
        if self.eat_keyword("then") {
            let then_stmt = Box::new(self.parse_stmt()?);
            let else_stmt = if self.eat_keyword("else") {
                Some(Box::new(self.parse_stmt()?))
            } else {
                None
            };
            return Ok(Stmt::IfThen {
                cond,
                then_stmt,
                else_stmt,
            });
        }
        self.expect(&Tok::Semi, "subsetting if")?;
        Ok(Stmt::SubsetIf { cond })
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_and()?;
        while self.eat_keyword("or") {
            let rhs = self.parse_and()?;
            lhs = Expr::Binary {
                op: BinOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }
    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_not()?;
        while self.eat_keyword("and") {
            let rhs = self.parse_not()?;
            lhs = Expr::Binary {
                op: BinOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }
    fn parse_not(&mut self) -> Result<Expr, ParseError> {
        if self.eat_keyword("not") {
            let e = self.parse_not()?;
            Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(e),
            })
        } else {
            self.parse_cmp()
        }
    }
    fn parse_cmp(&mut self) -> Result<Expr, ParseError> {
        let lhs = self.parse_concat()?;
        let op = match self.peek() {
            Tok::Eq => Some(BinOp::Eq),
            Tok::NotEq => Some(BinOp::Ne),
            Tok::Lt => Some(BinOp::Lt),
            Tok::Le => Some(BinOp::Le),
            Tok::Gt => Some(BinOp::Gt),
            Tok::Ge => Some(BinOp::Ge),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let rhs = self.parse_concat()?;
            return Ok(Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            });
        }
        Ok(lhs)
    }
    fn parse_concat(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_add()?;
        while self.eat(&Tok::Concat) {
            let rhs = self.parse_add()?;
            lhs = Expr::Binary {
                op: BinOp::Concat,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }
    fn parse_add(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = if self.eat(&Tok::Plus) {
                BinOp::Add
            } else if self.eat(&Tok::Minus) {
                BinOp::Sub
            } else {
                break;
            };
            let rhs = self.parse_mul()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }
    fn parse_mul(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_power()?;
        loop {
            let op = if self.eat(&Tok::Star) {
                BinOp::Mul
            } else if self.eat(&Tok::Slash) {
                BinOp::Div
            } else {
                break;
            };
            let rhs = self.parse_power()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }
    fn parse_power(&mut self) -> Result<Expr, ParseError> {
        let lhs = self.parse_unary()?;
        if self.eat(&Tok::Power) {
            let rhs = self.parse_power()?;
            return Ok(Expr::Binary {
                op: BinOp::Pow,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            });
        }
        Ok(lhs)
    }
    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.eat(&Tok::Minus) {
            let e = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(e),
            });
        }
        if self.eat(&Tok::Plus) {
            return self.parse_unary();
        }
        self.parse_primary()
    }
    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            Tok::Number(n) => {
                self.bump();
                Ok(Expr::NumLit(n))
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Expr::StrLit(s))
            }
            Tok::LParen => {
                self.bump();
                let e = self.parse_expr()?;
                self.expect(&Tok::RParen, "parenthesized expression")?;
                Ok(e)
            }
            Tok::Ident(mut name) => {
                let start = self.current_span().start;
                self.bump();
                // Compose qualified names like `first.grp` / `last.grp` into
                // a single PDV identifier ("first.grp").
                while matches!(self.peek(), Tok::Dot) {
                    let saved = self.pos;
                    self.bump();
                    match self.peek().clone() {
                        Tok::Ident(rhs) => {
                            self.bump();
                            name.push('.');
                            name.push_str(&rhs);
                        }
                        _ => {
                            self.pos = saved;
                            break;
                        }
                    }
                }
                if self.eat(&Tok::LParen) {
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Tok::RParen) {
                        args.push(self.parse_expr()?);
                        while self.eat(&Tok::Comma) {
                            args.push(self.parse_expr()?);
                        }
                    }
                    self.expect(&Tok::RParen, "call args")?;
                    let end = self.toks[self.pos - 1].1.end;
                    Ok(Expr::Call {
                        name,
                        args,
                        span: Span::new(start, end),
                    })
                } else if self.eat(&Tok::LBrace) {
                    let idx = self.parse_expr()?;
                    self.expect(&Tok::RBrace, "array index")?;
                    let end = self.toks[self.pos - 1].1.end;
                    Ok(Expr::ArrayRef {
                        name,
                        index: Box::new(idx),
                        span: Span::new(start, end),
                    })
                } else if self.eat(&Tok::LBracket) {
                    let idx = self.parse_expr()?;
                    self.expect(&Tok::RBracket, "array index")?;
                    let end = self.toks[self.pos - 1].1.end;
                    Ok(Expr::ArrayRef {
                        name,
                        index: Box::new(idx),
                        span: Span::new(start, end),
                    })
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            other => Err(self.err(format!("unexpected token in expression: {:?}", other))),
        }
    }
}

/// Parse a raw informat string (e.g. `$char40.`, `date9.`, `dollar12.2`, `8.2`)
/// into a structured [`Informat`].
fn parse_informat_str(raw: &str) -> Result<Informat, String> {
    let s = raw.trim();
    let has_dollar = s.starts_with('$');
    let body = if has_dollar { &s[1..] } else { s };
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    let name = body[..i].to_ascii_lowercase();
    let wstart = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let width: usize = body[wstart..i].parse().unwrap_or(0);
    let mut decimals = 0usize;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let dstart = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if dstart < i {
            decimals = body[dstart..i].parse().unwrap_or(0);
        }
    }
    if i != bytes.len() {
        return Err(format!("unsupported informat {:?}", raw));
    }
    let kind = match (has_dollar, name.as_str()) {
        (true, "") => InformatKind::CharTrim,
        (true, "char") => InformatKind::CharPreserve,
        (false, "") | (false, "best") => InformatKind::Numeric,
        (false, "date") => InformatKind::Date,
        (false, "comma") | (false, "dollar") => InformatKind::NumericSymbol,
        _ => return Err(format!("unsupported informat {:?}", raw)),
    };
    let width = if width == 0 {
        match kind {
            InformatKind::CharPreserve | InformatKind::CharTrim => 8,
            _ => 12,
        }
    } else {
        width
    };
    Ok(Informat {
        kind,
        width,
        decimals,
    })
}

#[cfg(test)]
pub fn parse_expr_for_test(src: &str) -> Result<Expr, ParseError> {
    let toks = Lexer::new(src)
        .tokens_with_spans()
        .map_err(|m| ParseError {
            message: m,
            span: Span::point(0),
        })?;
    let mut p = Parser { toks, pos: 0, src };
    p.parse_expr()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_data_step() {
        let ds = parse_data_step("data out; set in; x = 1 + 2; run;").unwrap();
        assert_eq!(ds.outputs[0].name, "out");
        assert!(matches!(ds.input, Some(DataInput::Set(ref v)) if v.len() == 1));
    }

    #[test]
    fn parses_input_informats_and_readers() {
        let ds = parse_data_step(
            "data t; input emp_id name $char40. dept_id hire_date :date9. base_salary; run;",
        )
        .unwrap();
        let v = &ds.input_vars;
        assert_eq!(v.len(), 5);
        // emp_id: plain list numeric
        assert_eq!(v[0].reader, InputReader::List);
        assert!(v[0].informat.is_none() && !v[0].is_char);
        // name: formatted $char40.
        assert_eq!(v[1].reader, InputReader::Formatted);
        assert_eq!(
            v[1].informat,
            Some(Informat {
                kind: InformatKind::CharPreserve,
                width: 40,
                decimals: 0
            })
        );
        assert!(v[1].is_char);
        // hire_date: modified-list :date9.
        assert_eq!(v[3].reader, InputReader::Modified);
        assert_eq!(v[3].informat.map(|f| f.kind), Some(InformatKind::Date));
        assert!(!v[3].is_char);
    }

    #[test]
    fn parses_column_range_input() {
        let ds = parse_data_step("data t; input id 1-3 name $ 5-20 age 22-24; run;").unwrap();
        let v = &ds.input_vars;
        assert_eq!(v[0].reader, InputReader::Column { start: 1, end: 3 });
        assert!(!v[0].is_char);
        assert_eq!(v[1].reader, InputReader::Column { start: 5, end: 20 });
        assert!(v[1].is_char);
        assert_eq!(v[2].reader, InputReader::Column { start: 22, end: 24 });
    }

    #[test]
    fn column_range_does_not_swallow_informats() {
        // `8.2` must remain a numeric informat, not a column range.
        let ds = parse_data_step("data t; input x 8.2; run;").unwrap();
        assert_eq!(ds.input_vars[0].reader, InputReader::Formatted);
        assert_eq!(
            ds.input_vars[0].informat.map(|f| (f.width, f.decimals)),
            Some((8, 2))
        );
    }

    #[test]
    fn format_statement_is_accepted() {
        let ds = parse_data_step("data t; set s; format d date9. amt dollar12.2; run;").unwrap();
        assert_eq!(
            ds.formats,
            vec![
                FormatDecl {
                    name: "d".into(),
                    format: "date9.".into()
                },
                FormatDecl {
                    name: "amt".into(),
                    format: "dollar12.2".into()
                }
            ]
        );
        assert!(ds.body.is_empty());
    }

    #[test]
    fn parses_if_then_else() {
        let ds = parse_data_step("data o; set i; if x > 1 then y = 'big'; else y = 'small'; run;")
            .unwrap();
        assert_eq!(ds.body.len(), 1);
        assert!(matches!(
            ds.body[0],
            Stmt::IfThen {
                else_stmt: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn parses_subsetting_if() {
        let ds = parse_data_step("data o; set i; if x > 0; run;").unwrap();
        assert!(matches!(ds.body[0], Stmt::SubsetIf { .. }));
    }

    #[test]
    fn parses_keep_drop_length() {
        let ds = parse_data_step("data o; set i; keep a b c; drop z; length name $ 20 age 8; run;")
            .unwrap();
        assert_eq!(ds.keep.as_ref().unwrap().len(), 3);
        assert_eq!(ds.drop.as_ref().unwrap()[0], "z");
        assert_eq!(ds.lengths.len(), 2);
    }

    #[test]
    fn parses_merge_with_by() {
        let ds = parse_data_step("data o; merge a b; by id; run;").unwrap();
        assert!(matches!(ds.input, Some(DataInput::Merge(ref v)) if v.len() == 2));
        assert_eq!(ds.by, vec!["id".to_string()]);
    }

    #[test]
    fn parses_retain_with_initial() {
        let ds = parse_data_step("data o; set i; retain total 0; total = total + x; run;").unwrap();
        assert_eq!(ds.retain.len(), 1);
        assert_eq!(ds.retain[0].name, "total");
        assert_eq!(ds.retain[0].initial, Some(0.0));
    }

    #[test]
    fn parses_array_and_index() {
        let ds = parse_data_step("data o; set i; array a{3} a1 a2 a3; a{1} = 10; x = a[2]; run;")
            .unwrap();
        assert_eq!(ds.arrays.len(), 1);
        assert_eq!(ds.arrays[0].size, 3);
        assert!(matches!(
            ds.body[0],
            Stmt::Assign {
                target: AssignTarget::ArrayElem { .. },
                ..
            }
        ));
    }

    #[test]
    fn parses_iterative_do() {
        let ds =
            parse_data_step("data o; set i; do i = 1 to 5 by 2; y = y + i; end; run;").unwrap();
        assert!(matches!(ds.body[0], Stmt::DoLoop { .. }));
    }

    #[test]
    fn parses_do_while() {
        let ds = parse_data_step("data o; set i; do while (x < 10); x = x + 1; end; run;").unwrap();
        assert!(matches!(ds.body[0], Stmt::DoWhile { .. }));
    }

    #[test]
    fn parses_do_until() {
        let ds =
            parse_data_step("data o; set i; do until (x >= 10); x = x + 1; end; run;").unwrap();
        assert!(matches!(ds.body[0], Stmt::DoUntil { .. }));
    }
}
