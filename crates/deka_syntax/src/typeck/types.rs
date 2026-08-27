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
    // A concrete `T` is assignable to `Option<T>` (sugar for `Some(T)`).
    if let Type::Option { inner } = expected {
        if is_assignable(inner, actual) {
            return true;
        }
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
    // Arrays are covariant in their element type.
    if let (Type::Array { elem: expected_elem }, Type::Array { elem: actual_elem }) =
        (expected, actual)
    {
        return is_assignable(expected_elem, actual_elem);
    }
    // Object structural subtyping: actual must supply at least the expected fields.
    if let (Type::Object { fields: expected_fields }, Type::Object { fields: actual_fields }) =
        (expected, actual)
    {
        return expected_fields.iter().all(|(name, expected_ty)| {
            actual_fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, actual_ty)| is_assignable(expected_ty, actual_ty))
                .unwrap_or(false)
        });
    }
    // Function subtyping: parameters are contravariant, return type is covariant.
    if let (
        Type::Function {
            params: expected_params,
            ret: expected_ret,
            optional: expected_optional,
        },
        Type::Function {
            params: actual_params,
            ret: actual_ret,
            optional: actual_optional,
        },
    ) = (expected, actual)
    {
        if expected_params.len() == actual_params.len() && expected_optional == actual_optional {
            return actual_params
                .iter()
                .zip(expected_params.iter())
                .all(|(a, e)| is_assignable(a, e))
                && is_assignable(expected_ret, actual_ret);
        }
    }
    false
}
