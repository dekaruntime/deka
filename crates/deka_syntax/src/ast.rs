//! DekaScript AST (Compiler v2).
//!
//! This AST intentionally contains only DekaScript nodes. There is no PHPX
//! compatibility surface and no parser-mode switching.

use bumpalo::Bump;
use serde::Serialize;

/// A source span: line and column are 1-based; byte offsets index the original
/// source string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Span {
    pub start: Pos,
    pub end: Pos,
    pub byte_start: usize,
    pub byte_end: usize,
}

impl Span {
    pub fn dummy() -> Self {
        Self {
            start: Pos { line: 1, column: 1 },
            end: Pos { line: 1, column: 1 },
            byte_start: 0,
            byte_end: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Pos {
    pub line: usize,
    pub column: usize,
}

/// A whole `.ds` file.
#[derive(Clone, Debug, Serialize)]
pub struct Program<'a> {
    pub statements: &'a [Stmt<'a>],
    pub span: Span,
}

/// Top-level or block statement.
#[derive(Clone, Debug, Serialize)]
pub enum Stmt<'a> {
    /// `export const x = 1;` or `export function f() {}`
    Export { decl: ExportDecl<'a>, span: Span },
    /// `import { a, b } from "./mod.ds";`
    Import {
        specifiers: &'a [ImportSpec<'a>],
        source: &'a str,
        span: Span,
    },
    /// `const x: number = 1;` or `let mut y = 2;`
    Const {
        name: &'a str,
        ty: Option<Type<'a>>,
        value: Expr<'a>,
        span: Span,
    },
    Let {
        name: &'a str,
        ty: Option<Type<'a>>,
        value: Expr<'a>,
        span: Span,
    },
    /// `function name<T>(args): Ret { body }`
    Function {
        name: &'a str,
        type_params: &'a [TypeParam<'a>],
        params: &'a [Param<'a>],
        return_type: Option<Type<'a>>,
        body: &'a [Stmt<'a>],
        is_async: bool,
        span: Span,
    },
    /// Receiver method: `fn StructName.method<T>(args): Ret { body }`
    ReceiverMethod {
        receiver_type: &'a str,
        receiver_name: &'a str,
        name: &'a str,
        type_params: &'a [TypeParam<'a>],
        params: &'a [Param<'a>],
        return_type: Option<Type<'a>>,
        body: &'a [Stmt<'a>],
        is_async: bool,
        span: Span,
    },
    /// `struct Name<T> { field: Type, embed Other }`
    Struct {
        name: &'a str,
        type_params: &'a [TypeParam<'a>],
        fields: &'a [StructField<'a>],
        embeds: &'a [Embed<'a>],
        span: Span,
    },
    /// `enum Name<T> { A, B(number), C(T) }`
    Enum {
        name: &'a str,
        type_params: &'a [TypeParam<'a>],
        cases: &'a [EnumCase<'a>],
        span: Span,
    },
    /// `type Name<T> = SomeType;`
    TypeAlias {
        name: &'a str,
        type_params: &'a [TypeParam<'a>],
        value: Type<'a>,
        span: Span,
    },
    /// Expression statement, e.g. `console.log(x);`
    Expr { expr: Expr<'a>, span: Span },
    /// `return expr;`
    Return { value: Option<Expr<'a>>, span: Span },
    /// `if (cond) { ... } else { ... }`
    If {
        condition: Expr<'a>,
        then_body: &'a [Stmt<'a>],
        else_body: &'a [Stmt<'a>],
        span: Span,
    },
    /// `{ ... }` block statement introducing a new scope.
    Block {
        body: &'a [Stmt<'a>],
        span: Span,
    },
    /// An empty statement: just `;`.
    Empty { span: Span },
    /// `for (init; cond; step) { ... }`
    For {
        init: Option<ForInit<'a>>,
        condition: Option<Expr<'a>>,
        step: Option<Expr<'a>>,
        body: &'a [Stmt<'a>],
        span: Span,
    },
    /// `break`
    Break {
        span: Span,
    },
    /// `continue`
    Continue {
        span: Span,
    },
}

#[derive(Clone, Debug, Serialize)]
pub enum ExportDecl<'a> {
    Const {
        name: &'a str,
        ty: Option<Type<'a>>,
        value: Expr<'a>,
    },
    Function {
        name: &'a str,
        type_params: &'a [TypeParam<'a>],
        params: &'a [Param<'a>],
        return_type: Option<Type<'a>>,
        body: &'a [Stmt<'a>],
        is_async: bool,
    },
    /// `export { a, b as c }` — re-exports already-declared names.
    NamedGroup {
        names: &'a [ExportName<'a>],
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct ExportName<'a> {
    pub name: &'a str,
    pub alias: Option<&'a str>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImportSpec<'a> {
    pub imported: &'a str,
    pub local: &'a str,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct Param<'a> {
    pub name: &'a str,
    pub ty: Option<Type<'a>>,
    pub default_value: Option<Expr<'a>>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypeParam<'a> {
    pub name: &'a str,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct StructField<'a> {
    pub name: &'a str,
    pub ty: Type<'a>,
    pub default_value: Option<Expr<'a>>,
    /// True for `field?: T` syntax: the field may be omitted in literals.
    pub optional: bool,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct Embed<'a> {
    pub name: &'a str,
    pub span: Span,
}

/// Lowering target for a receiver-method call that has been resolved by the
/// typechecker. `embed_path` is empty for methods declared directly on the
/// receiver type; otherwise it lists the embedded struct types that must be
/// traversed to reach the method's owner.
#[derive(Clone, Debug)]
pub struct MethodTarget<'a> {
    pub mangled: String,
    pub embed_path: Vec<&'a str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnumCase<'a> {
    pub name: &'a str,
    pub payload: Option<Type<'a>>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub enum ForInit<'a> {
    Const { name: &'a str, value: Expr<'a> },
    Let { name: &'a str, value: Expr<'a> },
    Expr(Expr<'a>),
}

/// Type syntax nodes.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub enum Type<'a> {
    Named {
        name: &'a str,
        span: Span,
    },
    Generic {
        base: &'a str,
        args: &'a [Type<'a>],
        span: Span,
    },
    Function {
        params: &'a [Type<'a>],
        ret: &'a Type<'a>,
        span: Span,
    },
    Option {
        inner: &'a Type<'a>,
        span: Span,
    },
    Tuple {
        elements: &'a [Type<'a>],
        span: Span,
    },
    Record {
        fields: &'a [RecordField<'a>],
        span: Span,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RecordField<'a> {
    pub name: &'a str,
    pub ty: Type<'a>,
    pub span: Span,
}

/// Expressions.
#[derive(Clone, Debug, Serialize)]
pub enum Expr<'a> {
    Number {
        value: f64,
        span: Span,
    },
    BigInt {
        value: &'a str,
        span: Span,
    },
    String {
        value: &'a str,
        span: Span,
    },
    Boolean {
        value: bool,
        span: Span,
    },
    None {
        span: Span,
    },
    Identifier {
        name: &'a str,
        span: Span,
    },
    Binary {
        op: BinOp,
        left: &'a Expr<'a>,
        right: &'a Expr<'a>,
        span: Span,
    },
    Unary {
        op: UnOp,
        operand: &'a Expr<'a>,
        span: Span,
    },
    Call {
        callee: &'a Expr<'a>,
        type_args: &'a [Type<'a>],
        args: &'a [Expr<'a>],
        span: Span,
    },
    FieldAccess {
        object: &'a Expr<'a>,
        field: &'a str,
        span: Span,
    },
    IndexAccess {
        object: &'a Expr<'a>,
        index: &'a Expr<'a>,
        span: Span,
    },
    StructLiteral {
        name: &'a str,
        fields: &'a [StructLiteralField<'a>],
        span: Span,
    },
    EnumConstructor {
        enum_name: &'a str,
        case_name: &'a str,
        payload: Option<&'a Expr<'a>>,
        span: Span,
    },
    Match {
        scrutinee: &'a Expr<'a>,
        arms: &'a [MatchArm<'a>],
        span: Span,
    },
    Unsafe {
        source: &'a str,
        span: Span,
    },
    Ternary {
        condition: &'a Expr<'a>,
        then_branch: &'a Expr<'a>,
        else_branch: &'a Expr<'a>,
        span: Span,
    },
    Await {
        expr: &'a Expr<'a>,
        span: Span,
    },
    JsxElement {
        element: JsxElement<'a>,
        span: Span,
    },
    JsxFragment {
        children: &'a [Expr<'a>],
        span: Span,
    },
    JsxText {
        value: &'a str,
        span: Span,
    },
    Array {
        elements: &'a [Expr<'a>],
        span: Span,
    },
    Object {
        fields: &'a [ObjectField<'a>],
        span: Span,
    },
    Spread {
        expr: &'a Expr<'a>,
        span: Span,
    },
    Paren {
        expr: &'a Expr<'a>,
        span: Span,
    },
    TemplateLiteral {
        parts: &'a [TemplatePart<'a>],
        span: Span,
    },
    /// Anonymous function expression: `fn (x: number) number { return x * 2 }`.
    Function {
        params: &'a [Param<'a>],
        return_type: Option<Type<'a>>,
        body: &'a [Stmt<'a>],
        is_async: bool,
        span: Span,
    },
}

#[derive(Clone, Debug, Serialize)]
pub enum TemplatePart<'a> {
    Text(&'a str),
    Expr(&'a Expr<'a>),
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Pipe,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Plus,
}

#[derive(Clone, Debug, Serialize)]
pub struct StructLiteralField<'a> {
    pub name: &'a str,
    pub value: Expr<'a>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct MatchArm<'a> {
    pub pattern: Pattern<'a>,
    pub guard: Option<Expr<'a>>,
    pub body: Expr<'a>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub enum Pattern<'a> {
    Wildcard {
        span: Span,
    },
    Identifier {
        name: &'a str,
        span: Span,
    },
    Literal {
        expr: Expr<'a>,
        span: Span,
    },
    Constructor {
        name: &'a str,
        payload: Option<&'a Pattern<'a>>,
        span: Span,
    },
    Struct {
        name: &'a str,
        fields: &'a [PatternField<'a>],
        span: Span,
    },
    Tuple {
        elements: &'a [Pattern<'a>],
        span: Span,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct PatternField<'a> {
    pub name: &'a str,
    pub pattern: Pattern<'a>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct ObjectField<'a> {
    pub key: &'a str,
    pub value: Expr<'a>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct JsxElement<'a> {
    pub tag: &'a str,
    pub attributes: &'a [JsxAttribute<'a>],
    pub children: &'a [Expr<'a>],
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct JsxAttribute<'a> {
    pub name: &'a str,
    pub value: Option<Expr<'a>>,
    pub span: Span,
}

impl<'a> Type<'a> {
    pub fn span(&self) -> Span {
        match self {
            Type::Named { span, .. } => *span,
            Type::Generic { span, .. } => *span,
            Type::Function { span, .. } => *span,
            Type::Option { span, .. } => *span,
            Type::Tuple { span, .. } => *span,
            Type::Record { span, .. } => *span,
        }
    }
}

impl<'a> Expr<'a> {
    pub fn span(&self) -> Span {
        match self {
            Expr::Number { span, .. } => *span,
            Expr::BigInt { span, .. } => *span,
            Expr::String { span, .. } => *span,
            Expr::Boolean { span, .. } => *span,
            Expr::None { span, .. } => *span,
            Expr::Identifier { span, .. } => *span,
            Expr::Binary { span, .. } => *span,
            Expr::Unary { span, .. } => *span,
            Expr::Call { span, .. } => *span,
            Expr::FieldAccess { span, .. } => *span,
            Expr::IndexAccess { span, .. } => *span,
            Expr::StructLiteral { span, .. } => *span,
            Expr::EnumConstructor { span, .. } => *span,
            Expr::Match { span, .. } => *span,
            Expr::Unsafe { span, .. } => *span,
            Expr::Ternary { span, .. } => *span,
            Expr::Await { span, .. } => *span,
            Expr::JsxElement { span, .. } => *span,
            Expr::JsxFragment { span, .. } => *span,
            Expr::JsxText { span, .. } => *span,
            Expr::Array { span, .. } => *span,
            Expr::Object { span, .. } => *span,
            Expr::Spread { span, .. } => *span,
            Expr::Paren { span, .. } => *span,
            Expr::TemplateLiteral { span, .. } => *span,
            Expr::Function { span, .. } => *span,
        }
    }
}

/// Helper to allocate a slice in the bump arena.
pub fn alloc_slice<'a, T>(arena: &'a Bump, items: Vec<T>) -> &'a [T] {
    let mut vec = bumpalo::collections::Vec::with_capacity_in(items.len(), arena);
    vec.extend(items);
    vec.into_bump_slice()
}

/// Helper to allocate a single value in the bump arena.
pub fn alloc<'a, T>(arena: &'a Bump, item: T) -> &'a T {
    arena.alloc(item)
}

/// Helper to allocate a string slice in the bump arena.
pub fn alloc_str<'a>(arena: &'a Bump, s: &str) -> &'a str {
    arena.alloc_str(s)
}
