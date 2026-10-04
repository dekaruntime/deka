pub(crate) const SIGN_HOST: &str = "__jwt_sign";
pub(crate) const VERIFY_HOST: &str = "__jwt_verify";
pub(crate) const PAYLOAD_ENCODING_ERROR: &str = "failed to encode jwt payload";
pub(crate) const OPTIONS_ENCODING_ERROR: &str = "invalid jwt options";

/// The same closed field contracts seed the compiler's public record types and
/// validate external token JSON. Unknown payload fields remain signed data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Number,
    String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Claim {
    Iat,
    Exp,
    Nbf,
    Iss,
    Aud,
    Sub,
}
impl Claim {
    pub(crate) const ALL: [Self; 6] = [
        Self::Iat,
        Self::Exp,
        Self::Nbf,
        Self::Iss,
        Self::Aud,
        Self::Sub,
    ];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Iat => "iat",
            Self::Exp => "exp",
            Self::Nbf => "nbf",
            Self::Iss => "iss",
            Self::Aud => "aud",
            Self::Sub => "sub",
        }
    }
    pub(crate) fn kind(self) -> Kind {
        match self {
            Self::Iat | Self::Exp | Self::Nbf => Kind::Number,
            _ => Kind::String,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum OptionField {
    Alg,
    Iss,
    Aud,
    Sub,
    ExpIn,
    Exp,
    Leeway,
}
impl OptionField {
    pub(crate) const ALL: [Self; 7] = [
        Self::Alg,
        Self::Iss,
        Self::Aud,
        Self::Sub,
        Self::ExpIn,
        Self::Exp,
        Self::Leeway,
    ];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Alg => "alg",
            Self::Iss => "iss",
            Self::Aud => "aud",
            Self::Sub => "sub",
            Self::ExpIn => "exp_in",
            Self::Exp => "exp",
            Self::Leeway => "leeway",
        }
    }
    pub(crate) fn kind(self) -> Kind {
        match self {
            Self::ExpIn | Self::Exp | Self::Leeway => Kind::Number,
            _ => Kind::String,
        }
    }
}
