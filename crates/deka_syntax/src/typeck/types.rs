//! Type representation used by the v2 typechecker.

use std::fmt;

/// Type used internally by the typechecker.
///
/// `Type::None` is the type of the literal `none`.  It is distinct from
/// `Option<T>` but assignable to any `Option<T>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type<'a> {
    /// Sentinel used for error recovery.
    Error,
    /// Placeholder used while a function's return type is being inferred.
    Infer,
    /// The bottom type (`never`). Not produced by the parser, but accepted in
    /// annotations for forward compatibility.
    Never,
    /// The type of the literal `none`.
    None,
    /// A named scalar or user-defined type.
    Named { name: &'a str },
    /// `Option<T>`.
    Option { inner: Box<Type<'a>> },
    /// Function type.
    Function {
        params: Vec<Type<'a>>,
        ret: Box<Type<'a>>,
        /// Number of trailing parameters that have default values and may be
        /// omitted at call sites.
        optional: usize,
    },
    /// Generic instantiation, e.g. `Result<number, string>`.
    Generic {
        base: &'a str,
        args: Vec<Type<'a>>,
    },
    /// A user-defined struct type.
    Struct { name: &'a str },
    /// An array type, e.g. `Array<number>` or `number[]`.
    Array { elem: Box<Type<'a>> },
    /// An object record type with known fields.
    Object { fields: Vec<(&'a str, Type<'a>)> },
    /// A declared interface type.
    Interface { name: &'a str },
    /// A boxed newtype over a primitive representation.
    Newtype { name: &'a str, repr: crate::ast::NewtypeRepr },
    /// A type parameter, e.g. `T` inside a generic function or type.
    Param { name: &'a str },
}

impl<'a> Type<'a> {
    pub fn is_error(&self) -> bool {
        matches!(self, Type::Error)
    }

    pub fn from_newtype_repr(repr: crate::ast::NewtypeRepr) -> Self {
        match repr {
            crate::ast::NewtypeRepr::Number => Type::Named { name: "number" },
            crate::ast::NewtypeRepr::String => Type::Named { name: "string" },
            crate::ast::NewtypeRepr::Bool => Type::Named { name: "boolean" },
        }
    }
}

pub fn newtype_repr_from_name(name: &str) -> Option<crate::ast::NewtypeRepr> {
    match name {
        "number" => Some(crate::ast::NewtypeRepr::Number),
        "string" => Some(crate::ast::NewtypeRepr::String),
        "bool" => Some(crate::ast::NewtypeRepr::Bool),
        _ => None,
    }
}

/// How a primitive conversion call (`number(x)`, `string(x)`, `bool(x)`) should
/// be lowered after typechecking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnwrapKind {
    /// The argument is already the primitive; erase the call.
    Identity,
    /// The argument is a newtype; access its payload via `__p`.
    Payload,
}

impl fmt::Display for Type<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Error => write!(f, "<error>"),
            Type::Infer => write!(f, "<infer>"),
            Type::Never => write!(f, "never"),
            Type::None => write!(f, "none"),
            Type::Named { name } => write!(f, "{name}"),
            Type::Option { inner } => write!(f, "Option<{inner}>"),
            Type::Function { params, ret, optional } => {
                write!(f, "fn(")?;
                let required = params.len().saturating_sub(*optional);
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if i == required && *optional > 0 {
                        write!(f, "optional ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ") {ret}")
            }
            Type::Generic { base, args } => {
                write!(f, "{base}<")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{arg}")?;
                }
                write!(f, ">")
            }
            Type::Struct { name } => write!(f, "{name}"),
            Type::Array { elem } => write!(f, "Array<{elem}>"),
            Type::Object { fields } => {
                write!(f, "{{")?;
                for (i, (name, ty)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{name}: {ty}")?;
                }
                write!(f, "}}")
            }
            Type::Interface { name } => write!(f, "{name}"),
            Type::Newtype { name, .. } => write!(f, "{name}"),
            Type::Param { name } => write!(f, "{name}"),
        }
    }
}


