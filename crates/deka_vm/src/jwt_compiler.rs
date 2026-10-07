//! JWT intrinsic lowering uses the ordinary JSON schema and decoder.
use super::*;
use crate::jwt_contract::{OPTIONS_ENCODING_ERROR, PAYLOAD_ENCODING_ERROR, SIGN_HOST, VERIFY_HOST};
use deka_syntax::typeck::JwtOperation;
#[derive(Clone)]
pub(super) struct Call {
    pub operation: JwtOperation,
    pub payload: Result<crate::JsonShape>,
    pub options: Option<Result<crate::JsonShape>>,
}
impl<'a> Lower<'a> {
    pub(super) fn jwt_call(&mut self, e: &Expr<'a>, c: &mut Context) -> Result<bool> {
        let Some(call) = self.jwt_calls.get(&(e as *const Expr as usize)).cloned() else {
            return Ok(false);
        };
        let Expr::Call { args, .. } = e else {
            return Err("invalid checked JWT call".into());
        };
        let shape = call.payload?;
        // Match ordinary function calls: every caller argument evaluates once in
        // authored order before validation or serialization inside the operation.
        let input = c.bind("<JWT input>");
        self.expr(&args[0], c)?;
        c.emit(Op::Store(input));
        let key = c.bind("<JWT key>");
        self.expr(&args[1], c)?;
        c.emit(Op::Store(key));
        let options = c.bind("<JWT options>");
        if let Some(argument) = args.get(2) {
            self.expr(argument, c)?;
        } else {
            c.emit(Op::Const(Literal::String("{}".into())));
        }
        c.emit(Op::Store(options));
        let mut failed = Vec::new();
        if call.operation == JwtOperation::Sign {
            failed.push(jwt_json(input, shape.clone(), PAYLOAD_ENCODING_ERROR, c));
        }
        if let Some(shape) = call.options {
            failed.push(jwt_json(options, shape?, OPTIONS_ENCODING_ERROR, c));
        }
        c.emit(Op::Load(input));
        c.emit(Op::Load(key));
        c.emit(Op::Load(options));
        c.emit(Op::Host {
            operation: match call.operation {
                JwtOperation::Sign => SIGN_HOST,
                JwtOperation::Verify => VERIFY_HOST,
            }
            .into(),
            arguments: 3,
        });
        if call.operation == JwtOperation::Verify {
            self.json_factory_record(&shape, c)?;
            c.emit(Op::JsonParseResult(shape));
        }
        if !failed.is_empty() {
            let complete = c.emit(Op::Jump(0));
            for branch in failed {
                c.patch(branch);
            }
            c.patch(complete);
        }
        Ok(true)
    }
}

// Success replaces the argument with checked JSON; failure retains the nominal
// Err on the expression stack and jumps past host execution/claim decoding.
fn jwt_json(slot: usize, shape: crate::JsonShape, error: &str, c: &mut Context) -> usize {
    c.emit(Op::Load(slot));
    c.emit(Op::JwtStringify {
        shape,
        error: error.into(),
    });
    c.emit(Op::Dup);
    c.emit(Op::MatchEnum {
        name: Some("Result".into()),
        case: "Ok".into(),
    });
    let failed = c.emit(Op::JumpIfFalse(0));
    c.emit(Op::Field("value".into()));
    c.emit(Op::Store(slot));
    failed
}
