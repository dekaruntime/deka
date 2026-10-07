//! JWT calls specialize complete payloads through the shared JSON descriptor.
use super::{Checker, DescriptorTree, JsonDescriptor, Type};
use crate::ast;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JwtOperation {
    Sign,
    Verify,
}
impl JwtOperation {
    pub const MODULE_FUNCTIONS: &'static [(&'static str, Self)] =
        &[("sign", Self::Sign), ("verify", Self::Verify)];
    pub fn module_type<'a>(self, claims: Type<'a>, options: Type<'a>) -> Type<'a> {
        Type::Generic {
            base: match self {
                Self::Sign => "$NativeJwtSign",
                Self::Verify => "$NativeJwtVerify",
            },
            args: vec![claims, options],
        }
    }
    pub fn from_module_type(ty: &Type<'_>) -> Option<Self> {
        match ty {
            Type::Generic {
                base: "$NativeJwtSign",
                ..
            } => Some(Self::Sign),
            Type::Generic {
                base: "$NativeJwtVerify",
                ..
            } => Some(Self::Verify),
            _ => None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct JwtCall<'a> {
    pub operation: JwtOperation,
    pub payload: JsonDescriptor<'a>,
    pub options: Option<JsonDescriptor<'a>>,
}
impl<'a> Checker<'a> {
    pub(super) fn check_jwt_call(
        &mut self,
        expr: &ast::Expr<'a>,
        marker: &Type<'a>,
        type_args: &'a [ast::Type<'a>],
        args: &'a [ast::Expr<'a>],
        span: ast::Span,
    ) -> Type<'a> {
        let operation = JwtOperation::from_module_type(marker).expect("JWT marker");
        let Type::Generic { args: contract, .. } = marker else {
            unreachable!()
        };
        let [claims, options] = contract.as_slice() else {
            self.error_span(span, "invalid native JWT contract");
            return Type::Error;
        };
        if !type_args.is_empty() || !(2..=3).contains(&args.len()) {
            self.error_span(
                span,
                "JWT calls expect two or three arguments and no type arguments",
            );
            return Type::Error;
        }
        let input = self.check_expr(&args[0]);
        let key = self.check_expr(&args[1]);
        if !matches!(key, Type::Named { name: "bytes" } | Type::Error) {
            self.error_span(args[1].span(), "JWT secret must be bytes");
        }
        let payload = if operation == JwtOperation::Sign {
            self.checked_jwt_object(&input, claims, args[0].span())
        } else {
            if !matches!(input, Type::Named { name: "string" } | Type::Error) {
                self.error_span(args[0].span(), "JWT token must be a string");
            }
            self.json_descriptor(claims, span)
        };
        let options_shape = if let Some(argument) = args.get(2) {
            let ty = self.check_expr(argument);
            match self.checked_jwt_object(&ty, options, argument.span()) {
                Ok(shape) => Some(shape),
                Err(message) => {
                    self.error_span(argument.span(), message);
                    return Type::Error;
                }
            }
        } else {
            None
        };
        let payload = match payload {
            Ok(shape) => shape,
            Err(message) => {
                self.error_span(args[0].span(), message);
                return Type::Error;
            }
        };
        self.jwt_calls.insert(
            expr as *const ast::Expr<'a>,
            JwtCall {
                operation,
                payload,
                options: options_shape,
            },
        );
        Type::Generic {
            base: "Result",
            args: vec![
                if operation == JwtOperation::Sign {
                    Type::Named { name: "string" }
                } else {
                    claims.clone()
                },
                Type::Named { name: "string" },
            ],
        }
    }
    fn checked_jwt_object(
        &mut self,
        actual: &Type<'a>,
        contract: &Type<'a>,
        span: ast::Span,
    ) -> Result<JsonDescriptor<'a>, String> {
        let shape = self.json_descriptor(actual, span)?;
        let fields: Vec<_> = match &shape {
            JsonDescriptor::Record(fields) => fields.clone(),
            JsonDescriptor::Type(DescriptorTree::Struct { fields, .. }) => fields
                .iter()
                .map(|field| (field.name, JsonDescriptor::Type(field.ty.clone())))
                .collect(),
            _ => return Err("JWT claims and options must be a checked record or struct".into()),
        };
        let Type::Object {
            fields: expected, ..
        } = contract
        else {
            return Err("invalid native JWT record contract".into());
        };
        for (name, expected) in expected {
            let Some((_, actual)) = fields.iter().find(|(field, _)| field == name) else {
                continue;
            };
            let Type::Option { inner } = expected else {
                return Err("invalid native JWT optional field contract".into());
            };
            let Type::Named { name: kind } = inner.as_ref() else {
                return Err("invalid native JWT scalar field contract".into());
            };
            if !jwt_scalar(actual, kind) {
                return Err(format!(
                    "JWT field `{name}` must be {kind} or Option<{kind}>"
                ));
            }
        }
        Ok(shape)
    }
}
fn jwt_scalar(shape: &JsonDescriptor<'_>, expected: &str) -> bool {
    match shape {
        JsonDescriptor::Type(DescriptorTree::Leaf { kind, .. }) => {
            *kind == expected || *kind == "none"
        }
        JsonDescriptor::Type(DescriptorTree::Option { inner }) => {
            matches!(inner.as_ref(),DescriptorTree::Leaf{kind,..} if *kind==expected)
        }
        JsonDescriptor::Option(inner) => jwt_scalar(inner, expected),
        _ => false,
    }
}
