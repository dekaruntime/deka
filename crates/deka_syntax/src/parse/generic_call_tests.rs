use super::parse;
use crate::ast::{BinOp, Expr, Stmt, Type};
use bumpalo::Bump;

fn with_value(source: &str, inspect: impl FnOnce(&Expr<'_>)) {
    let arena = Bump::new();
    let parsed = parse(source, &arena);
    assert!(parsed.errors.is_empty(), "{source}: {:?}", parsed.errors);
    let program = parsed.program.unwrap();
    let Stmt::Const { value, .. } = &program.statements[0] else {
        panic!("expected binding");
    };
    inspect(value);
}
fn arrays(ty: &Type<'_>, depth: usize) {
    if depth == 0 {
        assert!(matches!(ty, Type::Named { name: "number", .. }), "{ty:?}");
    } else {
        let Type::Generic { base, args, .. } = ty else {
            panic!("lost generic type: {ty:?}");
        };
        assert_eq!(*base, "Array");
        assert_eq!(args.len(), 1);
        arrays(&args[0], depth - 1);
    }
}
#[test]
fn adjacent_nested_closers_preserve_the_complete_call_type() {
    for depth in 1..=5 {
        let ty = format!("{}number{}", "Array<".repeat(depth), ">".repeat(depth));
        let source = format!("const value = JSON.parse<{ty}>(\"[]\");");
        with_value(&source, |value| {
            let Expr::Call {
                callee,
                type_args,
                args,
                ..
            } = value
            else {
                panic!("not a generic call: {value:?}");
            };
            assert!(matches!(callee, Expr::FieldAccess { field: "parse", .. }));
            assert_eq!(type_args.len(), 1);
            arrays(&type_args[0], depth);
            assert_eq!(args.len(), 1);
            assert!(matches!(args[0], Expr::String { value: "[]", .. }));
        });
    }
}
#[test]
fn mixed_and_multiple_type_arguments_remain_distinct() {
    with_value(
        "const value = combine<Array<Array<number>>, Result<number, string>>(input);",
        |value| {
            let Expr::Call { type_args, .. } = value else {
                panic!("not a generic call: {value:?}");
            };
            assert_eq!(type_args.len(), 2);
            arrays(&type_args[0], 2);
            let Type::Generic { base, args, .. } = &type_args[1] else {
                panic!("lost Result type");
            };
            assert_eq!(*base, "Result");
            assert_eq!(args.len(), 2);
            assert!(matches!(args[0], Type::Named { name: "number", .. }));
            assert!(matches!(args[1], Type::Named { name: "string", .. }));
        },
    );
}
#[test]
fn comparisons_keep_shift_operators_even_before_parentheses() {
    with_value("const value = a < b >> (c);", |value| {
        let Expr::Binary {
            op: BinOp::Lt,
            right,
            ..
        } = value
        else {
            panic!("comparison changed: {value:?}");
        };
        assert!(
            matches!(right, Expr::Binary { op: BinOp::Shr, .. }),
            "shift changed: {right:?}"
        );
    });
    with_value("const value = a >> b < c;", |value| {
        let Expr::Binary {
            op: BinOp::Lt,
            left,
            ..
        } = value
        else {
            panic!("comparison changed: {value:?}");
        };
        assert!(
            matches!(left, Expr::Binary { op: BinOp::Shr, .. }),
            "shift changed: {left:?}"
        );
    });
}
