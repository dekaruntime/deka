#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn nested_embeddings_promote_fields_and_keep_shared_mutations() {
    let value = run(r#"
        struct Position { x: number; }
        struct Body { Position; }
        struct Player { Body; name: string; }
        fn main() number {
            let position = Position { x: 1 };
            let player = Player { Body: Body { Position: position }, name: "Ada" };
            const shared = player;
            player.x = player.x + 4;
            return shared.x + position.x;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(10.));
}

#[tokio::test]
async fn promoted_literals_build_embeds_with_defaults_and_methods() {
    let value = run(r#"
        struct Position { x: number; y: number = 2; }
        fn (p mut Position) move() void { p.x = p.x + p.y; }
        struct Body { Position; }
        struct Player { Body; }
        fn main() number {
            let player = Player { x: 3 };
            player.move();
            return player.x + player.y;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(7.));
}

#[tokio::test]
async fn an_own_field_shadows_the_promoted_field_without_changing_the_embed() {
    let value = run(r#"
        struct Base { x: number; }
        struct Item { Base; x: number; }
        fn main() number {
            let base = Base { x: 1 };
            let item = Item { Base: base, x: 10 };
            item.x = item.x + 2;
            return item.x + base.x;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(13.));
}

#[tokio::test]
async fn empty_embeds_are_constructed() {
    let value = run(r#"
        struct Empty {}
        struct Base { Empty; }
        struct Item { Base; }
        fn (e Empty) answer() number { return 42; }
        fn main() number { return Item {}.answer(); }
    "#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(42.));
}

#[test]
fn const_promoted_field_assignment_remains_a_type_error() {
    let error = compiler::compile(
        r#"
        struct Base { x: number; }
        struct Item { Base; }
        const item = Item { Base: Base { x: 1 } };
        item.x = 2;
    "#,
        &Hosts::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("immutable") || error.contains("const"),
        "{error}"
    );
}

#[tokio::test]
async fn promoted_initializers_evaluate_once_in_source_order() {
    let value = run(r#"
        struct Base { x: number; y: number; }
        struct Item { Base; }
        fn main() number {
            let calls = 0;
            const next = fn() number { calls = calls + 1; return calls; };
            const item = Item { y: next(), x: next() };
            return item.x * 10 + item.y + calls;
        }
    "#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(23.));
}
