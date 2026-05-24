//! AST for the v0.4 DATA step.

#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    pub libref: Option<String>,
    pub name: String,
}

impl TableRef {
    pub fn qualified(&self) -> String {
        match &self.libref {
            Some(l) => format!("{}.{}", l, self.name),
            None => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataStep {
    pub outputs: Vec<TableRef>,
    pub input: Option<DataInput>,
    /// `by` variables in effect — drives first./last. and merge matching.
    pub by: Vec<String>,
    pub where_expr: Option<Expr>,
    pub keep: Option<Vec<String>>,
    pub drop: Option<Vec<String>>,
    pub lengths: Vec<LengthDecl>,
    pub retain: Vec<RetainDecl>,
    pub arrays: Vec<ArrayDecl>,
    /// Free-form `input <name> [$] ...;` definitions.
    pub input_vars: Vec<InputVar>,
    /// Inline data attached to a `datalines;` block. Filled in by the
    /// session after parsing (the parser doesn't see the raw lines).
    pub datalines: Vec<String>,
    /// `infile 'path' [dsd] [dlm=','] [firstobs=N];`
    pub infile: Option<InfileSpec>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InfileSpec {
    pub path: String,
    /// `None` means whitespace splitting.
    pub dlm: Option<String>,
    /// `dsd` — delimiter-sensitive: missing values between consecutive
    /// delimiters, quoted strings respected.
    pub dsd: bool,
    /// First line to read (1-based); lines before are skipped (header).
    pub firstobs: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputVar {
    pub name: String,
    pub is_char: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DataInput {
    /// `set ds [ds2 ...];` — concatenate rows
    Set(Vec<TableRef>),
    /// `merge ds1 ds2 ...; by var;` — match-merge by `by` variables
    Merge(Vec<TableRef>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LengthDecl {
    pub name: String,
    pub is_char: bool,
    pub width: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RetainDecl {
    pub name: String,
    /// Optional initial value (numeric only in v0.4).
    pub initial: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayDecl {
    pub name: String,
    pub size: usize,
    pub is_char: bool,
    /// Explicit element names — if empty, defaults to `name1`..`nameN`.
    pub elements: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Assign {
        target: AssignTarget,
        expr: Expr,
    },
    IfThen {
        cond: Expr,
        then_stmt: Box<Stmt>,
        else_stmt: Option<Box<Stmt>>,
    },
    SubsetIf {
        cond: Expr,
    },
    Output {
        dataset: Option<TableRef>,
    },
    Delete,
    Block(Vec<Stmt>),
    /// `do var = start to stop [by step]; … end;`
    DoLoop {
        var: String,
        start: Expr,
        stop: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },
    /// `select [(<switch>)]; when (<v1>, <v2>...) <stmt>; … otherwise <stmt>; end;`
    Select {
        /// `Some(expr)` if `select(expr);` form; `None` for boolean-when form.
        switch: Option<Expr>,
        branches: Vec<SelectBranch>,
        otherwise: Option<Box<Stmt>>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectBranch {
    /// Candidate values (`when (e1, e2, ...)`). For boolean-when form, just
    /// one expression is typical but multiple are allowed (OR'd together).
    pub values: Vec<Expr>,
    pub stmt: Box<Stmt>,
}

/// An assignment target — a plain variable or an array element.
#[derive(Debug, Clone, PartialEq)]
pub enum AssignTarget {
    Var(String),
    ArrayElem { name: String, index: Expr },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    NumLit(f64),
    StrLit(String),
    Ident(String),
    /// `name(args...)`. `span` covers the whole call (name through `)`).
    Call {
        name: String,
        args: Vec<Expr>,
        span: super::lex::Span,
    },
    /// `name{index}` or `name[index]`. `span` covers `name { … }`.
    ArrayRef {
        name: String,
        index: Box<Expr>,
        span: super::lex::Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Mod,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}
