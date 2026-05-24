//! Complete SAS macro preprocessor (v2.0).
//!
//! Fully supported:
//! - `%let name = value;`
//! - `%put text;`
//! - `%local name1 name2;` and `%global name1 name2;` declarations.
//! - `%macro name(args); ... %mend name;` definitions with positional and keyword parameters.
//! - `%name(args)` and `%name` macro invocations.
//! - `%if condition %then action; %else action;` conditionals.
//! - `%do; ... %end;` blocks.
//! - `%do var = start %to end %by step; ... %end;` iterative loops.
//! - `%do %while(condition); ... %end;` and `%do %until(condition); ... %end;` loops.
//! - Built-in functions: `%eval`, `%sysevalf`, `%upcase`, `%lowcase`, `%substr`, `%length`, `%index`, `%scan`, `%str`, `%quote`.
//! - Lexical environment scopes stack.
//! - Automatic system variables like `&sysdate`, `&systime`, `&sysday`, etc.

#![allow(
    clippy::map_entry,
    clippy::inherent_to_string,
    clippy::manual_ignore_case_cmp,
    clippy::collapsible_if,
    clippy::unnecessary_unwrap,
    clippy::get_first
)]

use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct MacroDef {
    pub _name: String,
    pub params: Vec<MacroParam>,
    pub body: Vec<Node>,
}

#[derive(Clone, Debug)]
pub struct MacroParam {
    pub name: String,
    pub default_value: Option<Vec<Node>>,
}

#[derive(Clone, Debug)]
pub enum Node {
    Text(String),
    VarRef {
        name: String,
        _has_dot: bool,
    },
    Let {
        name: String,
        value: Vec<Node>,
    },
    Put {
        value: Vec<Node>,
    },
    Local(Vec<String>),
    Global(Vec<String>),
    MacroDef {
        name: String,
        params: Vec<MacroParam>,
        body: Vec<Node>,
    },
    MacroCall {
        name: String,
        args: Vec<Vec<Node>>,
    },
    If {
        cond: Vec<Node>,
        then_branch: Vec<Node>,
        else_branch: Option<Vec<Node>>,
    },
    Do(Vec<Node>),
    DoLoop {
        var: String,
        start: Vec<Node>,
        end: Vec<Node>,
        by: Option<Vec<Node>>,
        body: Vec<Node>,
    },
    DoWhile {
        cond: Vec<Node>,
        body: Vec<Node>,
    },
    DoUntil {
        cond: Vec<Node>,
        body: Vec<Node>,
    },
    FuncCall {
        name: String,
        args: Vec<Vec<Node>>,
    },
}

// ── Environment Scoping ───────────────────────────────────────────────────

pub struct Env {
    pub global: HashMap<String, String>,
    pub local_stack: Vec<HashMap<String, String>>,
}

impl Env {
    pub fn get(&self, name: &str) -> Option<String> {
        let key = name.to_ascii_lowercase();
        for scope in self.local_stack.iter().rev() {
            if let Some(val) = scope.get(&key) {
                return Some(val.clone());
            }
        }
        self.global.get(&key).cloned()
    }

    pub fn set(&mut self, name: &str, value: String) {
        let key = name.to_ascii_lowercase();
        for scope in self.local_stack.iter_mut().rev() {
            if scope.contains_key(&key) {
                scope.insert(key, value);
                return;
            }
        }
        self.global.insert(key, value);
    }

    pub fn declare_local(&mut self, name: &str) {
        let key = name.to_ascii_lowercase();
        if let Some(scope) = self.local_stack.last_mut() {
            scope.insert(key, "".to_string());
        } else {
            self.global.insert(key, "".to_string());
        }
    }

    pub fn declare_global(&mut self, name: &str) {
        let key = name.to_ascii_lowercase();
        if !self.global.contains_key(&key) {
            self.global.insert(key, "".to_string());
        }
    }
}

// ── Expression Evaluator ──────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum EvalValue {
    Num(f64),
    Str(String),
}

impl EvalValue {
    fn to_string(&self) -> String {
        match self {
            EvalValue::Num(n) => {
                if n.fract() == 0.0 {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            EvalValue::Str(s) => s.clone(),
        }
    }

    fn to_num(&self) -> Result<f64, String> {
        match self {
            EvalValue::Num(n) => Ok(*n),
            EvalValue::Str(s) => s
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("Cannot convert string '{}' to number", s)),
        }
    }

    fn truthy(&self) -> bool {
        match self {
            EvalValue::Num(n) => *n != 0.0,
            EvalValue::Str(s) => {
                if let Ok(n) = s.trim().parse::<f64>() {
                    n != 0.0
                } else {
                    !s.is_empty()
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum ExprTok {
    Num(f64),
    Str(String),
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    LParen,
    RParen,
}

fn tokenize_expr(s: &str) -> Vec<ExprTok> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut toks = Vec::new();

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        if c == '(' {
            toks.push(ExprTok::LParen);
            i += 1;
            continue;
        }
        if c == ')' {
            toks.push(ExprTok::RParen);
            i += 1;
            continue;
        }
        if c == '+' {
            toks.push(ExprTok::Add);
            i += 1;
            continue;
        }
        if c == '-' {
            toks.push(ExprTok::Sub);
            i += 1;
            continue;
        }
        if c == '*' {
            toks.push(ExprTok::Mul);
            i += 1;
            continue;
        }
        if c == '/' {
            toks.push(ExprTok::Div);
            i += 1;
            continue;
        }

        if c == '=' {
            toks.push(ExprTok::Eq);
            i += 1;
            continue;
        }
        if c == '!' && i + 1 < chars.len() && chars[i + 1] == '=' {
            toks.push(ExprTok::Ne);
            i += 2;
            continue;
        }
        if c == '<' {
            if i + 1 < chars.len() && chars[i + 1] == '=' {
                toks.push(ExprTok::Le);
                i += 2;
            } else if i + 1 < chars.len() && chars[i + 1] == '>' {
                toks.push(ExprTok::Ne);
                i += 2;
            } else {
                toks.push(ExprTok::Lt);
                i += 1;
            }
            continue;
        }
        if c == '>' {
            if i + 1 < chars.len() && chars[i + 1] == '=' {
                toks.push(ExprTok::Ge);
                i += 2;
            } else {
                toks.push(ExprTok::Gt);
                i += 1;
            }
            continue;
        }

        if c == '\'' || c == '"' {
            let quote_char = c;
            let mut val = String::new();
            i += 1;
            while i < chars.len() {
                let d = chars[i];
                if d == quote_char {
                    i += 1;
                    if i < chars.len() && chars[i] == quote_char {
                        val.push(quote_char);
                        i += 1;
                        continue;
                    }
                    break;
                }
                val.push(d);
                i += 1;
            }
            toks.push(ExprTok::Str(val));
            continue;
        }

        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            let mut val = String::new();
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                val.push(chars[i]);
                i += 1;
            }

            let lower = val.to_ascii_lowercase();
            match lower.as_str() {
                "and" => toks.push(ExprTok::And),
                "or" => toks.push(ExprTok::Or),
                "not" => toks.push(ExprTok::Not),
                "eq" => toks.push(ExprTok::Eq),
                "ne" => toks.push(ExprTok::Ne),
                "lt" => toks.push(ExprTok::Lt),
                "le" => toks.push(ExprTok::Le),
                "gt" => toks.push(ExprTok::Gt),
                "ge" => toks.push(ExprTok::Ge),
                _ => {
                    if let Ok(n) = val.parse::<f64>() {
                        toks.push(ExprTok::Num(n));
                    } else {
                        toks.push(ExprTok::Str(val));
                    }
                }
            }
            continue;
        }

        toks.push(ExprTok::Str(c.to_string()));
        i += 1;
    }

    toks
}

struct ExprParser {
    toks: Vec<ExprTok>,
    pos: usize,
}

impl ExprParser {
    fn new(toks: Vec<ExprTok>) -> Self {
        Self { toks, pos: 0 }
    }

    fn peek(&self) -> Option<&ExprTok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<ExprTok> {
        if self.pos < self.toks.len() {
            let t = self.toks[self.pos].clone();
            self.pos += 1;
            Some(t)
        } else {
            None
        }
    }

    fn parse(&mut self) -> Result<EvalValue, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<EvalValue, String> {
        let mut left = self.parse_and()?;
        while let Some(ExprTok::Or) = self.peek() {
            self.bump();
            let right = self.parse_and()?;
            left = EvalValue::Num(if left.truthy() || right.truthy() {
                1.0
            } else {
                0.0
            });
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<EvalValue, String> {
        let mut left = self.parse_eq()?;
        while let Some(ExprTok::And) = self.peek() {
            self.bump();
            let right = self.parse_eq()?;
            left = EvalValue::Num(if left.truthy() && right.truthy() {
                1.0
            } else {
                0.0
            });
        }
        Ok(left)
    }

    fn parse_eq(&mut self) -> Result<EvalValue, String> {
        let mut left = self.parse_add()?;
        while let Some(t) = self.peek() {
            match t {
                ExprTok::Eq
                | ExprTok::Ne
                | ExprTok::Lt
                | ExprTok::Le
                | ExprTok::Gt
                | ExprTok::Ge => {
                    let op = self.bump().unwrap();
                    let right = self.parse_add()?;
                    left = match op {
                        ExprTok::Eq => {
                            let matches = match (&left, &right) {
                                (EvalValue::Num(a), EvalValue::Num(b)) => a == b,
                                _ => left.to_string() == right.to_string(),
                            };
                            EvalValue::Num(if matches { 1.0 } else { 0.0 })
                        }
                        ExprTok::Ne => {
                            let matches = match (&left, &right) {
                                (EvalValue::Num(a), EvalValue::Num(b)) => a != b,
                                _ => left.to_string() != right.to_string(),
                            };
                            EvalValue::Num(if matches { 1.0 } else { 0.0 })
                        }
                        ExprTok::Lt => {
                            let is_lt = match (left.to_num(), right.to_num()) {
                                (Ok(a), Ok(b)) => a < b,
                                _ => left.to_string() < right.to_string(),
                            };
                            EvalValue::Num(if is_lt { 1.0 } else { 0.0 })
                        }
                        ExprTok::Le => {
                            let is_le = match (left.to_num(), right.to_num()) {
                                (Ok(a), Ok(b)) => a <= b,
                                _ => left.to_string() <= right.to_string(),
                            };
                            EvalValue::Num(if is_le { 1.0 } else { 0.0 })
                        }
                        ExprTok::Gt => {
                            let is_gt = match (left.to_num(), right.to_num()) {
                                (Ok(a), Ok(b)) => a > b,
                                _ => left.to_string() > right.to_string(),
                            };
                            EvalValue::Num(if is_gt { 1.0 } else { 0.0 })
                        }
                        ExprTok::Ge => {
                            let is_ge = match (left.to_num(), right.to_num()) {
                                (Ok(a), Ok(b)) => a >= b,
                                _ => left.to_string() >= right.to_string(),
                            };
                            EvalValue::Num(if is_ge { 1.0 } else { 0.0 })
                        }
                        _ => unreachable!(),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<EvalValue, String> {
        let mut left = self.parse_mul()?;
        while let Some(t) = self.peek() {
            match t {
                ExprTok::Add | ExprTok::Sub => {
                    let op = self.bump().unwrap();
                    let right = self.parse_mul()?;
                    let a = left.to_num()?;
                    let b = right.to_num()?;
                    left = match op {
                        ExprTok::Add => EvalValue::Num(a + b),
                        ExprTok::Sub => EvalValue::Num(a - b),
                        _ => unreachable!(),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<EvalValue, String> {
        let mut left = self.parse_unary()?;
        while let Some(t) = self.peek() {
            match t {
                ExprTok::Mul | ExprTok::Div => {
                    let op = self.bump().unwrap();
                    let right = self.parse_unary()?;
                    let a = left.to_num()?;
                    let b = right.to_num()?;
                    left = match op {
                        ExprTok::Mul => EvalValue::Num(a * b),
                        ExprTok::Div => {
                            if b == 0.0 {
                                return Err("Division by zero in macro expression".to_string());
                            }
                            EvalValue::Num(a / b)
                        }
                        _ => unreachable!(),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<EvalValue, String> {
        if let Some(ExprTok::Not) = self.peek() {
            self.bump();
            let val = self.parse_unary()?;
            Ok(EvalValue::Num(if val.truthy() { 0.0 } else { 1.0 }))
        } else if let Some(ExprTok::Sub) = self.peek() {
            self.bump();
            let val = self.parse_unary()?;
            let n = val.to_num()?;
            Ok(EvalValue::Num(-n))
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<EvalValue, String> {
        match self.bump() {
            Some(ExprTok::Num(n)) => Ok(EvalValue::Num(n)),
            Some(ExprTok::Str(s)) => Ok(EvalValue::Str(s)),
            Some(ExprTok::LParen) => {
                let val = self.parse()?;
                if let Some(ExprTok::RParen) = self.bump() {
                    Ok(val)
                } else {
                    Err("Expected ')'".to_string())
                }
            }
            other => Err(format!("Expected operand, got {:?}", other)),
        }
    }
}

pub fn eval_expression(expr_str: &str) -> Result<EvalValue, String> {
    let toks = tokenize_expr(expr_str);
    let mut p = ExprParser::new(toks);
    p.parse()
}

// ── Recursive Parser ──────────────────────────────────────────────────────

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(chars: &'a [char]) -> Self {
        Self { chars, pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn bump(&mut self) -> Option<char> {
        if self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            self.pos += 1;
            Some(c)
        } else {
            None
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn read_identifier(&mut self) -> Option<String> {
        self.skip_whitespace();
        self.read_identifier_no_skip()
    }

    fn read_identifier_no_skip(&mut self) -> Option<String> {
        let mut name = String::new();
        if let Some(c) = self.peek() {
            if c.is_ascii_alphabetic() || c == '_' {
                name.push(self.bump().unwrap());
                while let Some(d) = self.peek() {
                    if d.is_ascii_alphanumeric() || d == '_' {
                        name.push(self.bump().unwrap());
                    } else {
                        break;
                    }
                }
                return Some(name);
            }
        }
        None
    }

    fn starts_with_word(&self, word: &str) -> bool {
        if self.pos + word.len() > self.chars.len() {
            return false;
        }
        for (i, c) in word.chars().enumerate() {
            if self.chars[self.pos + i].to_ascii_lowercase() != c.to_ascii_lowercase() {
                return false;
            }
        }
        // Word-boundary check: only apply when the stop word ends with an
        // alphanumeric or underscore character (i.e. a keyword like %then,
        // %end).  For punctuation stop words (`;`, `,`, `)`) the match is
        // exact and no boundary lookahead is needed.
        if let Some(&last) = word.as_bytes().last() {
            if last.is_ascii_alphanumeric() || last == b'_' {
                if let Some(next_char) = self.chars.get(self.pos + word.len()) {
                    if next_char.is_ascii_alphanumeric() || *next_char == '_' {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn parse_nodes(&mut self, stop_words: &[&str]) -> Result<Vec<Node>, String> {
        let mut nodes = Vec::new();
        let mut current_text = String::new();

        let flush_text = |nodes: &mut Vec<Node>, current_text: &mut String| {
            if !current_text.is_empty() {
                nodes.push(Node::Text(current_text.clone()));
                current_text.clear();
            }
        };

        while !self.is_eof() {
            let mut hit_stop = false;
            for sw in stop_words {
                if self.starts_with_word(sw) {
                    hit_stop = true;
                    break;
                }
            }
            if hit_stop {
                break;
            }

            let c = self.peek().unwrap();

            if c == '\'' {
                flush_text(&mut nodes, &mut current_text);
                let mut quoted = String::new();
                quoted.push(self.bump().unwrap());
                while let Some(d) = self.peek() {
                    quoted.push(self.bump().unwrap());
                    if d == '\'' {
                        if self.peek() == Some('\'') {
                            quoted.push(self.bump().unwrap());
                            continue;
                        }
                        break;
                    }
                }
                nodes.push(Node::Text(quoted));
                continue;
            }

            if c == '"' {
                flush_text(&mut nodes, &mut current_text);
                self.bump();
                let mut d_nodes = Vec::new();
                d_nodes.push(Node::Text("\"".to_string()));

                let mut inner_text = String::new();
                while let Some(d) = self.peek() {
                    if d == '"' {
                        self.bump();
                        if self.peek() == Some('"') {
                            inner_text.push('"');
                            self.bump();
                            continue;
                        }
                        break;
                    }

                    if d == '&' {
                        if !inner_text.is_empty() {
                            d_nodes.push(Node::Text(inner_text.clone()));
                            inner_text.clear();
                        }
                        let var_node = self.parse_amp()?;
                        d_nodes.push(var_node);
                        continue;
                    }

                    if d == '%' {
                        if let Some(next_c) = self.peek_next() {
                            if next_c.is_ascii_alphabetic() || next_c == '_' {
                                if !inner_text.is_empty() {
                                    d_nodes.push(Node::Text(inner_text.clone()));
                                    inner_text.clear();
                                }
                                self.bump();
                                let name = self
                                    .read_identifier()
                                    .ok_or("Expected identifier after % in double quotes")?;
                                if is_builtin_func(&name) {
                                    let func_node = self.parse_func_call(name)?;
                                    d_nodes.push(func_node);
                                } else {
                                    let macro_node = self.parse_macro_call(name)?;
                                    d_nodes.push(macro_node);
                                }
                                continue;
                            }
                        }
                    }

                    inner_text.push(self.bump().unwrap());
                }

                if !inner_text.is_empty() {
                    d_nodes.push(Node::Text(inner_text));
                }
                d_nodes.push(Node::Text("\"".to_string()));
                nodes.extend(d_nodes);
                continue;
            }

            if c == '&' {
                flush_text(&mut nodes, &mut current_text);
                let var_node = self.parse_amp()?;
                nodes.push(var_node);
                continue;
            }

            if c == '%' {
                if let Some(next_c) = self.peek_next() {
                    if next_c.is_ascii_alphabetic() || next_c == '_' {
                        flush_text(&mut nodes, &mut current_text);
                        self.bump();
                        let keyword_or_name = self
                            .read_identifier()
                            .ok_or("Expected identifier after %")?;
                        let kw_lower = keyword_or_name.to_ascii_lowercase();

                        match kw_lower.as_str() {
                            "let" => {
                                let node = self.parse_let()?;
                                nodes.push(node);
                            }
                            "put" => {
                                let node = self.parse_put()?;
                                nodes.push(node);
                            }
                            "local" => {
                                let node = self.parse_local()?;
                                nodes.push(node);
                            }
                            "global" => {
                                let node = self.parse_global()?;
                                nodes.push(node);
                            }
                            "macro" => {
                                let node = self.parse_macro_def()?;
                                nodes.push(node);
                            }
                            "if" => {
                                let node = self.parse_if()?;
                                nodes.push(node);
                            }
                            "do" => {
                                let node = self.parse_do()?;
                                nodes.push(node);
                            }
                            _ => {
                                if is_builtin_func(&kw_lower) {
                                    let node = self.parse_func_call(kw_lower)?;
                                    nodes.push(node);
                                } else {
                                    let node = self.parse_macro_call(kw_lower)?;
                                    nodes.push(node);
                                }
                            }
                        }
                        continue;
                    }
                }
            }

            current_text.push(self.bump().unwrap());
        }

        flush_text(&mut nodes, &mut current_text);
        Ok(nodes)
    }

    fn parse_amp(&mut self) -> Result<Node, String> {
        self.bump();
        if let Some(name) = self.read_identifier_no_skip() {
            let has_dot = if self.peek() == Some('.') {
                self.bump();
                true
            } else {
                false
            };
            Ok(Node::VarRef {
                name,
                _has_dot: has_dot,
            })
        } else {
            Ok(Node::Text("&".to_string()))
        }
    }

    fn parse_let(&mut self) -> Result<Node, String> {
        let name = self
            .read_identifier()
            .ok_or("Expected identifier for %let")?;
        self.skip_whitespace();
        if self.peek() == Some('=') {
            self.bump();
        } else {
            return Err("Expected '=' in %let statement".to_string());
        }
        let value_nodes = self.parse_nodes(&[";"])?;
        if self.peek() == Some(';') {
            self.bump();
        }
        Ok(Node::Let {
            name: name.to_ascii_lowercase(),
            value: value_nodes,
        })
    }

    fn parse_put(&mut self) -> Result<Node, String> {
        let value_nodes = self.parse_nodes(&[";"])?;
        if self.peek() == Some(';') {
            self.bump();
        }
        Ok(Node::Put { value: value_nodes })
    }

    fn parse_local(&mut self) -> Result<Node, String> {
        let mut names = Vec::new();
        while let Some(name) = self.read_identifier() {
            names.push(name.to_ascii_lowercase());
            self.skip_whitespace();
            if self.peek() == Some(',') {
                self.bump();
            }
        }
        if self.peek() == Some(';') {
            self.bump();
        }
        Ok(Node::Local(names))
    }

    fn parse_global(&mut self) -> Result<Node, String> {
        let mut names = Vec::new();
        while let Some(name) = self.read_identifier() {
            names.push(name.to_ascii_lowercase());
            self.skip_whitespace();
            if self.peek() == Some(',') {
                self.bump();
            }
        }
        if self.peek() == Some(';') {
            self.bump();
        }
        Ok(Node::Global(names))
    }

    fn parse_macro_def(&mut self) -> Result<Node, String> {
        let name = self
            .read_identifier()
            .ok_or("Expected macro name")?
            .to_ascii_lowercase();
        self.skip_whitespace();
        let mut params = Vec::new();
        if self.peek() == Some('(') {
            self.bump();
            self.skip_whitespace();
            while self.peek() != Some(')') && !self.is_eof() {
                let p_name = self
                    .read_identifier()
                    .ok_or("Expected parameter name")?
                    .to_ascii_lowercase();
                self.skip_whitespace();
                let mut default_value = None;
                if self.peek() == Some('=') {
                    self.bump();
                    let val_nodes = self.parse_nodes(&[",", ")"])?;
                    default_value = Some(val_nodes);
                }
                params.push(MacroParam {
                    name: p_name,
                    default_value,
                });
                self.skip_whitespace();
                if self.peek() == Some(',') {
                    self.bump();
                }
            }
            if self.peek() == Some(')') {
                self.bump();
            } else {
                return Err("Expected ')' to close parameter list".to_string());
            }
        }
        self.skip_whitespace();
        if self.peek() == Some(';') {
            self.bump();
        } else {
            return Err("Expected ';' after macro header".to_string());
        }

        let mut body_chars = Vec::new();
        let mut nesting = 1;
        while !self.is_eof() {
            if self.starts_with_word("%macro") {
                nesting += 1;
            } else if self.starts_with_word("%mend") {
                nesting -= 1;
                if nesting == 0 {
                    self.pos += 5;
                    self.skip_whitespace();
                    if let Some(_mend_name) = self.read_identifier() {
                        // ignored
                    }
                    self.skip_whitespace();
                    if self.peek() == Some(';') {
                        self.bump();
                    }
                    break;
                }
            }
            body_chars.push(self.bump().unwrap());
        }

        let mut body_parser = Parser::new(&body_chars);
        let body_nodes = body_parser.parse_nodes(&[])?;

        Ok(Node::MacroDef {
            name,
            params,
            body: body_nodes,
        })
    }

    fn parse_macro_call(&mut self, name: String) -> Result<Node, String> {
        let mut args = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some('(') {
            self.bump();
            while self.peek() != Some(')') && !self.is_eof() {
                let arg_nodes = self.parse_argument_nodes()?;
                args.push(arg_nodes);
                self.skip_whitespace();
                if self.peek() == Some(',') {
                    self.bump();
                }
            }
            if self.peek() == Some(')') {
                self.bump();
            }
        }
        Ok(Node::MacroCall {
            name: name.to_ascii_lowercase(),
            args,
        })
    }

    fn parse_argument_nodes(&mut self) -> Result<Vec<Node>, String> {
        let mut arg_chars = Vec::new();
        let mut p_nesting = 0;
        let mut in_single_quote = false;
        let mut in_double_quote = false;

        while !self.is_eof() {
            let c = self.peek().unwrap();
            if in_single_quote {
                arg_chars.push(self.bump().unwrap());
                if c == '\'' {
                    if self.peek() == Some('\'') {
                        arg_chars.push(self.bump().unwrap());
                        continue;
                    }
                    in_single_quote = false;
                }
                continue;
            }
            if in_double_quote {
                arg_chars.push(self.bump().unwrap());
                if c == '"' {
                    if self.peek() == Some('"') {
                        arg_chars.push(self.bump().unwrap());
                        continue;
                    }
                    in_double_quote = false;
                }
                continue;
            }

            if c == '\'' {
                in_single_quote = true;
                arg_chars.push(self.bump().unwrap());
                continue;
            }
            if c == '"' {
                in_double_quote = true;
                arg_chars.push(self.bump().unwrap());
                continue;
            }

            if c == '(' {
                p_nesting += 1;
            } else if c == ')' {
                if p_nesting == 0 {
                    break;
                }
                p_nesting -= 1;
            } else if c == ',' {
                if p_nesting == 0 {
                    break;
                }
            }

            arg_chars.push(self.bump().unwrap());
        }

        let mut arg_parser = Parser::new(&arg_chars);
        arg_parser.parse_nodes(&[])
    }

    fn parse_func_call(&mut self, name: String) -> Result<Node, String> {
        self.skip_whitespace();
        let mut args = Vec::new();
        if self.peek() == Some('(') {
            self.bump();
            while self.peek() != Some(')') && !self.is_eof() {
                let arg_nodes = self.parse_argument_nodes()?;
                args.push(arg_nodes);
                self.skip_whitespace();
                if self.peek() == Some(',') {
                    self.bump();
                }
            }
            if self.peek() == Some(')') {
                self.bump();
            }
        }
        Ok(Node::FuncCall {
            name: name.to_ascii_lowercase(),
            args,
        })
    }

    fn parse_if(&mut self) -> Result<Node, String> {
        let cond_nodes = self.parse_nodes(&["%then"])?;
        if self.starts_with_word("%then") {
            self.pos += 5;
        } else {
            return Err("Expected %then after %if condition".to_string());
        }

        let then_branch = self.parse_action_nodes()?;

        let mut else_branch = None;
        self.skip_whitespace();
        if self.starts_with_word("%else") {
            self.pos += 5;
            else_branch = Some(self.parse_action_nodes()?);
        }

        Ok(Node::If {
            cond: cond_nodes,
            then_branch,
            else_branch,
        })
    }

    fn parse_action_nodes(&mut self) -> Result<Vec<Node>, String> {
        self.skip_whitespace();
        if self.starts_with_word("%do") {
            self.pos += 3;
            let node = self.parse_do_body_after_header()?;
            Ok(vec![node])
        } else {
            let nodes = self.parse_nodes(&[";"])?;
            if self.peek() == Some(';') {
                self.bump();
            }
            Ok(nodes)
        }
    }

    fn parse_do(&mut self) -> Result<Node, String> {
        self.skip_whitespace();
        if self.peek() == Some(';') {
            self.bump();
            return self.parse_do_body_after_header();
        }

        if self.starts_with_word("%while") || self.starts_with_word("%until") {
            let is_while = self.starts_with_word("%while");
            self.pos += 6;
            self.skip_whitespace();
            if self.peek() == Some('(') {
                self.bump();
            } else {
                return Err("Expected '(' after %while/%until".to_string());
            }
            let mut cond_chars = Vec::new();
            let mut nesting = 0;
            while !self.is_eof() {
                let c = self.peek().unwrap();
                if c == '(' {
                    nesting += 1;
                } else if c == ')' {
                    if nesting == 0 {
                        self.bump();
                        break;
                    }
                    nesting -= 1;
                }
                cond_chars.push(self.bump().unwrap());
            }
            self.skip_whitespace();
            if self.peek() == Some(';') {
                self.bump();
            }
            let mut cond_parser = Parser::new(&cond_chars);
            let cond_nodes = cond_parser.parse_nodes(&[])?;

            let body_nodes = self.parse_nodes(&["%end"])?;
            if self.starts_with_word("%end") {
                self.pos += 4;
                self.skip_whitespace();
                if self.peek() == Some(';') {
                    self.bump();
                }
            }

            if is_while {
                Ok(Node::DoWhile {
                    cond: cond_nodes,
                    body: body_nodes,
                })
            } else {
                Ok(Node::DoUntil {
                    cond: cond_nodes,
                    body: body_nodes,
                })
            }
        } else {
            let var = self
                .read_identifier()
                .ok_or("Expected loop variable in %do")?;
            self.skip_whitespace();
            if self.peek() == Some('=') {
                self.bump();
            } else {
                return Err("Expected '=' in iterative %do".to_string());
            }

            let start_nodes = self.parse_nodes(&["%to"])?;
            if self.starts_with_word("%to") {
                self.pos += 3;
            } else {
                return Err("Expected %to in iterative %do".to_string());
            }

            let mut by_nodes = None;
            let end_nodes = self.parse_nodes(&["%by", ";"])?;
            if self.starts_with_word("%by") {
                self.pos += 3;
                let step_nodes = self.parse_nodes(&[";"])?;
                by_nodes = Some(step_nodes);
            }

            if self.peek() == Some(';') {
                self.bump();
            }

            let body_nodes = self.parse_nodes(&["%end"])?;
            if self.starts_with_word("%end") {
                self.pos += 4;
                self.skip_whitespace();
                if self.peek() == Some(';') {
                    self.bump();
                }
            }

            Ok(Node::DoLoop {
                var: var.to_ascii_lowercase(),
                start: start_nodes,
                end: end_nodes,
                by: by_nodes,
                body: body_nodes,
            })
        }
    }

    fn parse_do_body_after_header(&mut self) -> Result<Node, String> {
        let body_nodes = self.parse_nodes(&["%end"])?;
        if self.starts_with_word("%end") {
            self.pos += 4;
            self.skip_whitespace();
            if self.peek() == Some(';') {
                self.bump();
            }
        }
        Ok(Node::Do(body_nodes))
    }
}

fn is_builtin_func(name: &str) -> bool {
    matches!(
        name,
        "eval"
            | "sysevalf"
            | "upcase"
            | "lowcase"
            | "substr"
            | "length"
            | "index"
            | "scan"
            | "str"
            | "quote"
            | "bquote"
            | "superq"
    )
}

// ── Recursive Interpreter Context ──────────────────────────────────────────

pub struct Context<'a> {
    pub env: Env,
    pub defs: &'a mut HashMap<String, MacroDef>,
    pub puts: Vec<String>,
}

impl<'a> Context<'a> {
    pub fn new(vars: HashMap<String, String>, defs: &'a mut HashMap<String, MacroDef>) -> Self {
        let mut global = vars;
        if !global.contains_key("syscc") {
            global.insert("syscc".into(), "0".into());
        }
        if !global.contains_key("syserr") {
            global.insert("syserr".into(), "0".into());
        }
        if !global.contains_key("sysdate") {
            let now = chrono::Local::now();
            let sysdate = now.format("%d%b%y").to_string().to_uppercase();
            global.insert("sysdate".into(), sysdate);
        }
        if !global.contains_key("sysday") {
            let now = chrono::Local::now();
            let sysday = now.format("%A").to_string();
            global.insert("sysday".into(), sysday);
        }
        if !global.contains_key("systime") {
            let now = chrono::Local::now();
            let systime = now.format("%H:%M").to_string();
            global.insert("systime".into(), systime);
        }
        if !global.contains_key("sysuserid") {
            let user = std::env::var("USER").unwrap_or_else(|_| "gui".to_string());
            global.insert("sysuserid".into(), user);
        }

        Self {
            env: Env {
                global,
                local_stack: Vec::new(),
            },
            defs,
            puts: Vec::new(),
        }
    }

    pub fn eval_nodes(&mut self, nodes: &[Node]) -> Result<String, String> {
        let mut out = String::new();
        for node in nodes {
            let res = self.eval_node(node)?;
            out.push_str(&res);
        }
        Ok(out)
    }

    fn eval_node(&mut self, node: &Node) -> Result<String, String> {
        match node {
            Node::Text(t) => Ok(t.clone()),
            Node::VarRef { name, _has_dot: _ } => match self.env.get(name) {
                Some(v) => Ok(v),
                None => Ok(format!("&{}", name)),
            },
            Node::Let { name, value } => {
                let expanded_val = self.eval_nodes(value)?;
                self.env.set(name, expanded_val.trim().to_string());
                Ok(String::new())
            }
            Node::Put { value } => {
                let expanded_val = self.eval_nodes(value)?;
                self.puts.push(expanded_val.trim().to_string());
                Ok(String::new())
            }
            Node::Local(names) => {
                for name in names {
                    self.env.declare_local(name);
                }
                Ok(String::new())
            }
            Node::Global(names) => {
                for name in names {
                    self.env.declare_global(name);
                }
                Ok(String::new())
            }
            Node::MacroDef { name, params, body } => {
                self.defs.insert(
                    name.clone(),
                    MacroDef {
                        _name: name.clone(),
                        params: params.clone(),
                        body: body.clone(),
                    },
                );
                Ok(String::new())
            }
            Node::MacroCall { name, args } => {
                tracing::debug!(
                    macro_name = %name,
                    defs = ?self.defs.keys().collect::<Vec<_>>(),
                    "expanding macro call",
                );
                let mac_def = match self.defs.get(name).cloned() {
                    Some(d) => d,
                    None => {
                        return Err(format!("Macro %{} is not defined", name));
                    }
                };

                let mut local_scope = HashMap::new();
                let mut positional_idx = 0;

                for param in &mac_def.params {
                    let mut bound_value = None;

                    for arg in args {
                        let arg_str = self.eval_nodes(arg)?;
                        if let Some((k, v)) = arg_str.split_once('=') {
                            if k.trim().eq_ignore_ascii_case(&param.name) {
                                bound_value = Some(v.trim().to_string());
                                break;
                            }
                        }
                    }

                    if bound_value.is_none() {
                        if param.default_value.is_none() {
                            if let Some(arg) = args.get(positional_idx) {
                                let arg_str = self.eval_nodes(arg)?;
                                if !arg_str.contains('=') {
                                    bound_value = Some(arg_str);
                                    positional_idx += 1;
                                } else {
                                    let (k, _) = arg_str.split_once('=').unwrap();
                                    let matches_any_param = mac_def
                                        .params
                                        .iter()
                                        .any(|p| p.name.eq_ignore_ascii_case(k.trim()));
                                    if matches_any_param {
                                        positional_idx += 1;
                                        if let Some(next_arg) = args.get(positional_idx) {
                                            bound_value = Some(self.eval_nodes(next_arg)?);
                                            positional_idx += 1;
                                        }
                                    } else {
                                        bound_value = Some(arg_str);
                                        positional_idx += 1;
                                    }
                                }
                            } else {
                                bound_value = Some("".to_string());
                                positional_idx += 1;
                            }
                        } else {
                            let mut passed_val = None;
                            for arg in args {
                                let arg_str = self.eval_nodes(arg)?;
                                if let Some((k, v)) = arg_str.split_once('=') {
                                    if k.trim().eq_ignore_ascii_case(&param.name) {
                                        passed_val = Some(v.trim().to_string());
                                        break;
                                    }
                                }
                            }
                            if let Some(val) = passed_val {
                                bound_value = Some(val);
                            } else {
                                let def_nodes = param.default_value.as_ref().unwrap();
                                bound_value = Some(self.eval_nodes(def_nodes)?);
                            }
                        }
                    }

                    let final_val = bound_value.unwrap_or_else(|| "".to_string());
                    local_scope.insert(param.name.clone(), final_val);
                }

                tracing::debug!(local_scope = ?local_scope, body = ?mac_def.body, "macro scope bound");
                self.env.local_stack.push(local_scope);
                let body_expanded = self.eval_nodes(&mac_def.body)?;
                self.env.local_stack.pop();

                tracing::debug!(expanded = %body_expanded, "macro body expanded");
                Ok(body_expanded)
            }
            Node::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond_str = self.eval_nodes(cond)?;
                let eval_res = eval_expression(&cond_str)?;
                if eval_res.truthy() {
                    self.eval_nodes(then_branch)
                } else if let Some(else_b) = else_branch {
                    self.eval_nodes(else_b)
                } else {
                    Ok(String::new())
                }
            }
            Node::Do(body) => self.eval_nodes(body),
            Node::DoLoop {
                var,
                start,
                end,
                by,
                body,
            } => {
                let start_val = self.eval_nodes(start)?;
                let start_num = eval_expression(&start_val)?.to_num()?;
                let end_val = self.eval_nodes(end)?;
                let end_num = eval_expression(&end_val)?.to_num()?;
                let step_num = if let Some(by_nodes) = by {
                    let by_val = self.eval_nodes(by_nodes)?;
                    eval_expression(&by_val)?.to_num()?
                } else {
                    1.0
                };

                let mut out = String::new();
                let mut current = start_num;
                if step_num > 0.0 {
                    while current <= end_num {
                        self.env.set(var, EvalValue::Num(current).to_string());
                        let iteration_out = self.eval_nodes(body)?;
                        out.push_str(&iteration_out);
                        current += step_num;
                    }
                } else if step_num < 0.0 {
                    while current >= end_num {
                        self.env.set(var, EvalValue::Num(current).to_string());
                        let iteration_out = self.eval_nodes(body)?;
                        out.push_str(&iteration_out);
                        current += step_num;
                    }
                }
                self.env.set(var, EvalValue::Num(current).to_string());

                Ok(out)
            }
            Node::DoWhile { cond, body } => {
                let mut out = String::new();
                loop {
                    let cond_str = self.eval_nodes(cond)?;
                    let eval_res = eval_expression(&cond_str)?;
                    if !eval_res.truthy() {
                        break;
                    }
                    let iter_out = self.eval_nodes(body)?;
                    out.push_str(&iter_out);
                }
                Ok(out)
            }
            Node::DoUntil { cond, body } => {
                let mut out = String::new();
                loop {
                    let iter_out = self.eval_nodes(body)?;
                    out.push_str(&iter_out);
                    let cond_str = self.eval_nodes(cond)?;
                    let eval_res = eval_expression(&cond_str)?;
                    if eval_res.truthy() {
                        break;
                    }
                }
                Ok(out)
            }
            Node::FuncCall { name, args } => {
                let mut evaluated_args = Vec::new();
                for arg in args {
                    evaluated_args.push(self.eval_nodes(arg)?.trim().to_string());
                }

                match name.as_str() {
                    "eval" | "sysevalf" => {
                        let expr = evaluated_args.get(0).cloned().unwrap_or_default();
                        let val = eval_expression(&expr)?;
                        Ok(val.to_string())
                    }
                    "upcase" => {
                        let text = evaluated_args.get(0).cloned().unwrap_or_default();
                        Ok(text.to_uppercase())
                    }
                    "lowcase" => {
                        let text = evaluated_args.get(0).cloned().unwrap_or_default();
                        Ok(text.to_lowercase())
                    }
                    "length" => {
                        let text = evaluated_args.get(0).cloned().unwrap_or_default();
                        Ok(text.len().to_string())
                    }
                    "substr" => {
                        let text = evaluated_args.get(0).cloned().unwrap_or_default();
                        let pos = evaluated_args
                            .get(1)
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(1);
                        let length = evaluated_args.get(2).and_then(|s| s.parse::<usize>().ok());

                        if pos == 0 || pos > text.len() {
                            return Ok(String::new());
                        }
                        let start = pos - 1;
                        if let Some(len) = length {
                            let end = (start + len).min(text.len());
                            Ok(text[start..end].to_string())
                        } else {
                            Ok(text[start..].to_string())
                        }
                    }
                    "index" => {
                        let source = evaluated_args.get(0).cloned().unwrap_or_default();
                        let target = evaluated_args.get(1).cloned().unwrap_or_default();
                        match source.find(&target) {
                            Some(idx) => Ok((idx + 1).to_string()),
                            None => Ok("0".to_string()),
                        }
                    }
                    "scan" => {
                        let text = evaluated_args.get(0).cloned().unwrap_or_default();
                        let n = evaluated_args
                            .get(1)
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(1);
                        let delimiters_str = evaluated_args
                            .get(2)
                            .cloned()
                            .unwrap_or_else(|| " \t,".to_string());
                        let delimiters: Vec<char> = delimiters_str.chars().collect();

                        let words: Vec<&str> = text
                            .split(|c| delimiters.contains(&c))
                            .filter(|s| !s.is_empty())
                            .collect();

                        if n == 0 || n > words.len() {
                            Ok(String::new())
                        } else {
                            Ok(words[n - 1].to_string())
                        }
                    }
                    "str" | "quote" | "bquote" | "superq" => {
                        Ok(evaluated_args.get(0).cloned().unwrap_or_default())
                    }
                    _ => Err(format!("Unknown built-in function %{}", name)),
                }
            }
        }
    }
}

// ── Public Interface ──────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct PreprocessOutput {
    pub expanded: String,
    pub puts: Vec<String>,
}

pub fn preprocess(
    src: &str,
    vars: &mut HashMap<String, String>,
    defs: &mut HashMap<String, MacroDef>,
) -> PreprocessOutput {
    let chars: Vec<char> = src.chars().collect();
    let mut parser = Parser::new(&chars);

    let nodes = match parser.parse_nodes(&[]) {
        Ok(ns) => ns,
        Err(e) => {
            // Fallback: if there was a syntax error, return the original source
            // with a log event indicating the macro parsing failure
            return PreprocessOutput {
                expanded: src.to_string(),
                puts: vec![format!("MACRO ERROR: {}", e)],
            };
        }
    };

    let mut ctx = Context::new(vars.clone(), defs);
    let expanded = match ctx.eval_nodes(&nodes) {
        Ok(exp) => exp,
        Err(e) => {
            return PreprocessOutput {
                expanded: src.to_string(),
                puts: vec![format!("MACRO RUNTIME ERROR: {}", e)],
            };
        }
    };

    *vars = ctx.env.global;

    PreprocessOutput {
        expanded,
        puts: ctx.puts,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn let_and_amp() {
        let mut vars = HashMap::new();
        let mut defs = HashMap::new();
        let out = preprocess("%let x = 42; y = &x;", &mut vars, &mut defs);
        assert_eq!(vars.get("x"), Some(&"42".to_string()));
        assert_eq!(out.expanded.trim(), "y = 42;");
    }

    #[test]
    fn amp_with_dot_terminator() {
        let mut vars = HashMap::new();
        let mut defs = HashMap::new();
        let out = preprocess(
            "%let lib = work; data &lib..out; run;",
            &mut vars,
            &mut defs,
        );
        assert!(out.expanded.contains("data work.out"));
    }

    #[test]
    fn single_quotes_disable_expansion() {
        let mut vars = HashMap::from([("x".into(), "42".into())]);
        let mut defs = HashMap::new();
        let out = preprocess("a = '&x'; b = \"&x\";", &mut vars, &mut defs);
        assert!(out.expanded.contains("a = '&x'"));
        assert!(out.expanded.contains("b = \"42\""));
    }

    #[test]
    fn put_captures_text() {
        let mut vars = HashMap::from([("name".into(), "Ada".into())]);
        let mut defs = HashMap::new();
        let out = preprocess("%put hello &name;", &mut vars, &mut defs);
        assert_eq!(out.puts, vec!["hello Ada".to_string()]);
        assert!(out.expanded.trim().is_empty());
    }

    #[test]
    fn let_value_can_reference_other_var() {
        let mut vars = HashMap::new();
        let mut defs = HashMap::new();
        preprocess("%let a = 1; %let b = a is &a;", &mut vars, &mut defs);
        assert_eq!(vars.get("b"), Some(&"a is 1".to_string()));
    }

    #[test]
    fn expression_evaluator() {
        assert_eq!(eval_expression("1 + 2 * 3"), Ok(EvalValue::Num(7.0)));
        assert_eq!(eval_expression("(1 + 2) * 3"), Ok(EvalValue::Num(9.0)));
        assert_eq!(eval_expression("10 > 5 and 2 < 4"), Ok(EvalValue::Num(1.0)));
        assert_eq!(eval_expression("'Ada' = 'Ada'"), Ok(EvalValue::Num(1.0)));
        assert_eq!(eval_expression("'Ada' = 'ada'"), Ok(EvalValue::Num(0.0)));
    }

    #[test]
    fn built_in_functions() {
        let mut vars = HashMap::new();
        let mut defs = HashMap::new();
        let out = preprocess("up = %upcase(ada); low = %lowcase(ADA); len = %length(hello); sub = %substr(abcdef, 2, 3); idx = %index(abcdef, cd); sc = %scan(a b c, 2);", &mut vars, &mut defs);
        assert!(out.expanded.contains("up = ADA"));
        assert!(out.expanded.contains("low = ada"));
        assert!(out.expanded.contains("len = 5"));
        assert!(out.expanded.contains("sub = bcd"));
        assert!(out.expanded.contains("idx = 3"));
        assert!(out.expanded.contains("sc = b"));
    }

    #[test]
    fn macro_definition_and_call() {
        let mut vars = HashMap::new();
        let mut defs = HashMap::new();
        let out = preprocess(
            r#"
            %macro my_macro(a, b=default);
                %put Param a is &a and b is &b;
                x = &a;
                y = &b;
            %mend;
            %my_macro(42, b=custom)
            "#,
            &mut vars,
            &mut defs,
        );
        assert!(out
            .puts
            .contains(&"Param a is 42 and b is custom".to_string()));
        assert!(out.expanded.contains("x = 42;"));
        assert!(out.expanded.contains("y = custom;"));
    }

    #[test]
    fn macro_conditional_and_loop() {
        let mut vars = HashMap::new();
        let mut defs = HashMap::new();
        let out = preprocess(
            r#"
            %macro loop_test;
                %do i = 1 %to 3;
                    val_&i = &i;
                %end;
            %mend;
            %loop_test;
            "#,
            &mut vars,
            &mut defs,
        );
        assert!(out.expanded.contains("val_1 = 1;"));
        assert!(out.expanded.contains("val_2 = 2;"));
        assert!(out.expanded.contains("val_3 = 3;"));
    }

    #[test]
    fn debug_macro_and_symput_integration() {
        let mut vars = HashMap::from([("my_prefix".to_string(), "user".to_string())]);
        let mut defs = HashMap::new();
        let input = r#"
            %macro test_macro(val);
                %put --- Executing test_macro ---;
                %let processed_val = %upcase(&val);
                
                data work.&my_prefix._data;
                    length name $15 val_str $15;
                    name = "symput_test";
                    val_str = "&processed_val";
                    output;
                run;
            %mend;
            %test_macro(active);
        "#;
        let out = preprocess(input, &mut vars, &mut defs);
        assert!(out.expanded.contains("data work.user_data;"));
        let sub_blocks = crate::split::split_blocks(&out.expanded);
        assert!(!sub_blocks.is_empty());
    }

    #[test]
    fn debug_multi_block_macro() {
        let s = crate::Session::new_in_memory().unwrap();
        let _evs1 = s.submit(
            r#"
            %let my_prefix = user;

            %macro test_macro(val);
                %put --- Executing test_macro ---;
                %let processed_val = %upcase(&val);
                
                data work.&my_prefix._data;
                    length name $15 val_str $15;
                    name = "symput_test";
                    val_str = "&processed_val";
                    output;
                run;
            %mend;

            %test_macro(active);
            "#,
        );
    }
}
