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
    },
    /// Generic instantiation, e.g. `Result<number, string>`.
    Generic {
        base: &'a str,
        args: Vec<Type<'a>>,
    },
    /// A user-defined struct type.
    Struct { name: &'a str },
    /// A type parameter, e.g. `T` inside a generic function or type.
    Param { name: &'a str },
}

impl<'a> Type<'a> {
    pub fn is_error(&self) -> bool {
        matches!(self, Type::Error)
    }
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
            Type::Function { params, ret } => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
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
            Type::Param { name } => write!(f, "{name}"),
        }
    }
}

/// Assignment / subtyping check. `actual` must be assignable to `expected`.
pub fn is_assignable<'a>(expected: &Type<'a>, actual: &Type<'a>) -> bool {
    if expected.is_error() || actual.is_error() {
        return true;
    }
    // `Infer` is the unknown/externally-provided type. It is compatible with
    // any type until a concrete type is available.
    if matches!(expected, Type::Infer) || matches!(actual, Type::Infer) {
        return true;
    }
    if expected == actual {
        return true;
    }
    // `never` is the bottom type: assignable to anything.
    if matches!(actual, Type::Never) {
        return true;
    }
    // `none` is assignable to any Option<T>.
    if matches!(expected, Type::Option { .. }) && matches!(actual, Type::None) {
        return true;
    }
    // `none` is assignable to `void`, and `void` is assignable to `void`.
    if matches!(expected, Type::Named { name: "void" })
        && matches!(actual, Type::None | Type::Named { name: "void" })
    {
        return true;
    }
    // Structural subtyping for generic types like Result<T, E>.
    if let (
        Type::Generic { base: expected_base, args: expected_args },
        Type::Generic { base: actual_base, args: actual_args },
    ) = (expected, actual)
    {
        if expected_base == actual_base && expected_args.len() == actual_args.len() {
            return expected_args
                .iter()
                .zip(actual_args.iter())
                .all(|(e, a)| is_assignable(e, a));
        }
    }
    false
}
