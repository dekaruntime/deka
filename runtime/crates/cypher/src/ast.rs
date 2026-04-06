/// Cypher AST — represents a parsed Cypher query.
/// Designed for validation and parameter extraction, not execution planning.

/// A complete Cypher statement (single query or UNION of queries).
#[derive(Debug, Clone, PartialEq)]
pub struct CypherStatement {
    pub query: Query,
    pub span: Span,
}

/// A query is one or more single queries joined by UNION.
#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    Single(SingleQuery),
    Union {
        left: Box<Query>,
        all: bool,
        right: SingleQuery,
    },
}

/// A single query is a sequence of clauses.
#[derive(Debug, Clone, PartialEq)]
pub struct SingleQuery {
    pub clauses: Vec<Clause>,
    pub span: Span,
}

/// Individual Cypher clauses.
#[derive(Debug, Clone, PartialEq)]
pub enum Clause {
    Match {
        optional: bool,
        pattern: Vec<PatternPart>,
        where_clause: Option<Expr>,
        span: Span,
    },
    Create {
        pattern: Vec<PatternPart>,
        span: Span,
    },
    Merge {
        pattern: PatternPart,
        on_create: Option<Vec<SetItem>>,
        on_match: Option<Vec<SetItem>>,
        span: Span,
    },
    Return {
        distinct: bool,
        items: Vec<ReturnItem>,
        order_by: Option<Vec<OrderItem>>,
        skip: Option<Expr>,
        limit: Option<Expr>,
        span: Span,
    },
    With {
        distinct: bool,
        items: Vec<ReturnItem>,
        where_clause: Option<Expr>,
        order_by: Option<Vec<OrderItem>>,
        skip: Option<Expr>,
        limit: Option<Expr>,
        span: Span,
    },
    Unwind {
        expr: Expr,
        alias: String,
        span: Span,
    },
    Delete {
        detach: bool,
        exprs: Vec<Expr>,
        span: Span,
    },
    Set {
        items: Vec<SetItem>,
        span: Span,
    },
    Remove {
        items: Vec<RemoveItem>,
        span: Span,
    },
    Call {
        procedure: String,
        args: Vec<Expr>,
        yields: Option<Vec<String>>,
        span: Span,
    },
}

/// A pattern part: optional variable binding + path pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternPart {
    pub variable: Option<String>,
    pub path: Vec<PatternElement>,
    pub span: Span,
}

/// Elements in a graph pattern: nodes and relationships.
#[derive(Debug, Clone, PartialEq)]
pub enum PatternElement {
    Node {
        variable: Option<String>,
        labels: Vec<String>,
        properties: Option<Expr>,
        span: Span,
    },
    Relationship {
        variable: Option<String>,
        rel_types: Vec<String>,
        direction: Direction,
        properties: Option<Expr>,
        length: Option<RangeLength>,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Right,    // -[]->(
    Left,     // <-[]-
    Both,     // <-[]->
    Undirected, // -[]-
}

/// Variable-length relationship: *min..max
#[derive(Debug, Clone, PartialEq)]
pub struct RangeLength {
    pub min: Option<u64>,
    pub max: Option<u64>,
}

/// RETURN / WITH items.
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnItem {
    pub expr: Expr,
    pub alias: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderItem {
    pub expr: Expr,
    pub ascending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SetItem {
    Property {
        target: Expr,
        value: Expr,
    },
    Label {
        variable: String,
        labels: Vec<String>,
    },
    AllProperties {
        variable: String,
        value: Expr,
        merge: bool, // += vs =
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RemoveItem {
    Property(Expr),
    Label { variable: String, labels: Vec<String> },
}

/// Expressions in Cypher.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// $param — external parameter reference
    Param {
        name: String,
        span: Span,
    },
    /// Bare identifier (query-internal variable)
    Ident {
        name: String,
        span: Span,
    },
    /// Property access: expr.property
    Property {
        expr: Box<Expr>,
        name: String,
        span: Span,
    },
    /// Integer literal
    Integer(i64, Span),
    /// Float literal
    Float(f64, Span),
    /// String literal
    StringLit(String, Span),
    /// Boolean literal
    Bool(bool, Span),
    /// NULL literal
    Null(Span),
    /// List literal [a, b, c]
    List(Vec<Expr>, Span),
    /// Map literal {key: val, ...}
    Map(Vec<(String, Expr)>, Span),
    /// Binary operation: left op right
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        span: Span,
    },
    /// Unary operation: NOT expr, -expr
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    /// Function call: name(args)
    FunctionCall {
        name: String,
        distinct: bool,
        args: Vec<Expr>,
        span: Span,
    },
    /// count(*)
    CountAll(Span),
    /// CASE expression
    Case {
        operand: Option<Box<Expr>>,
        whens: Vec<(Expr, Expr)>,
        else_expr: Option<Box<Expr>>,
        span: Span,
    },
    /// expr IS NULL / IS NOT NULL
    IsNull {
        expr: Box<Expr>,
        negated: bool,
        span: Span,
    },
    /// expr IN list
    In {
        expr: Box<Expr>,
        list: Box<Expr>,
        negated: bool,
        span: Span,
    },
    /// expr STARTS WITH / ENDS WITH / CONTAINS
    StringMatch {
        expr: Box<Expr>,
        kind: StringMatchKind,
        pattern: Box<Expr>,
        span: Span,
    },
    /// Index access: expr[index]
    Index {
        expr: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    /// Existential subquery: EXISTS { ... }
    Exists {
        pattern: Vec<PatternPart>,
        where_clause: Option<Box<Expr>>,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    And,
    Or,
    Xor,
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    RegexMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringMatchKind {
    StartsWith,
    EndsWith,
    Contains,
}

/// Byte offset span within the Cypher source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }
}
