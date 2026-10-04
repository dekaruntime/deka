//! Shared signatures for the language's existing number math receiver methods.
//! Partial methods map NaN to None, while infinities remain valid Some values.
pub const NUMBER_MATH_TOTAL: &[&str] = &[
    "abs", "ceil", "floor", "round", "trunc", "sign", "cbrt", "exp", "atan", "sinh", "cosh", "tanh",
];
pub const NUMBER_MATH_PARTIAL: &[&str] = &[
    "sqrt", "log", "log2", "log10", "asin", "acos", "acosh", "atanh", "sin", "cos", "tan",
];
#[derive(Clone, Copy, Debug)]
pub struct MathMethod {
    pub name: &'static str,
    /// Explicit arguments beyond the receiver.
    pub arguments: usize,
    pub partial: bool,
}
pub fn methods() -> impl Iterator<Item = MathMethod> {
    NUMBER_MATH_TOTAL
        .iter()
        .map(|&name| MathMethod {
            name,
            arguments: 0,
            partial: false,
        })
        .chain(NUMBER_MATH_PARTIAL.iter().map(|&name| MathMethod {
            name,
            arguments: 0,
            partial: true,
        }))
        .chain([
            MathMethod {
                name: "max",
                arguments: 1,
                partial: false,
            },
            MathMethod {
                name: "min",
                arguments: 1,
                partial: false,
            },
            MathMethod {
                name: "pow",
                arguments: 1,
                partial: true,
            },
        ])
}
pub fn method(name: &str) -> Option<MathMethod> {
    methods().find(|method| method.name == name)
}
