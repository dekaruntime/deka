#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;

async fn run(source: &str) -> Result<HostValue> {
    let hosts = Hosts::default();
    let program = compiler::compile(source, &hosts)?;
    Vm::new(program, hosts)?.run().await
}

#[tokio::test]
async fn let_list_element_assignment() {
    let value = run(r#"
        fn main() number {
            let xs = [1, 2, 3];
            if (xs.has(0)) { xs[0] = 9 }
            return xs.has(0) ? xs[0] : 0;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(9.));
}

#[tokio::test]
async fn let_record_field_assignment() {
    let value = run(r#"
        fn main() number {
            let obj = {a: 1};
            obj.a = 2;
            return obj.a;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(2.));
}

#[tokio::test]
async fn struct_field_assignment_through_let() {
    let value = run(r#"
        struct Point {
            x: number;
            y: number;
        }
        fn main() number {
            let p = Point { x: 1, y: 2 };
            p.x = 5;
            return p.x + p.y;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(7.));
}

/// The sharing rule from the old toolchain: two names, one value. A `const`
/// alias cannot be written through (the typechecker guards that), but it
/// reads the same heap value, so a write through the `let` name shows.
#[tokio::test]
async fn const_alias_shares_the_let_value() {
    let value = run(r#"
        fn main() number {
            let a = [1];
            const b = a;
            if (a.has(0)) { a[0] = 42 }
            return b.has(0) ? b[0] : 0;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(42.));
}

#[tokio::test]
async fn push_appends_and_returns_the_new_length() {
    let value = run(r#"
        fn main() number {
            let xs = [1, 2];
            const n = xs.push(3);
            return n * 10 + (xs.has(2) ? xs[2] : 0);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(33.));
}

#[tokio::test]
async fn pop_and_shift_remove_ends() {
    let value = run(r#"
        fn main() number {
            let xs = [1, 2, 3];
            xs.pop();
            xs.shift();
            return xs.has(0) ? xs[0] : 0;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(2.));
}

#[tokio::test]
async fn pop_on_an_empty_list_yields_unit() {
    let value = run(r#"
        fn main() number {
            let xs = [1];
            xs.pop();
            xs.pop();
            return xs.has(0) ? 1 : 0;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(0.));
}

#[tokio::test]
async fn unshift_prepends_and_returns_the_new_length() {
    let value = run(r#"
        fn main() number {
            let xs = [2, 3];
            const n = xs.unshift(1);
            return n * 10 + (xs.has(0) ? xs[0] : 0);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(31.));
}

#[tokio::test]
async fn splice_removes_a_range_and_returns_it() {
    let value = run(r#"
        fn main() number {
            let xs = [1, 2, 3, 4];
            const removed = xs.splice(1, 2);
            const first = removed.has(0) ? removed[0] : 0;
            const second = removed.has(1) ? removed[1] : 0;
            const kept = xs.has(1) ? xs[1] : 0;
            return first * 100 + second * 10 + kept;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(234.));
}

#[tokio::test]
async fn sort_and_reverse_mutate_in_place() {
    let value = run(r#"
        fn main() number {
            let xs = [3, 1, 2];
            const other = xs;
            xs.sort();
            const sorted = other.has(0) ? other[0] : 0;
            xs.reverse();
            return sorted * 10 + (other.has(0) ? other[0] : 0);
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(13.));
}

#[tokio::test]
async fn fill_and_copy_within() {
    let value = run(r#"
        fn main() number {
            let xs = [1, 2, 3];
            xs.fill(4, 1);
            const a = xs.has(1) ? xs[1] : 0;
            const b = xs.has(2) ? xs[2] : 0;
            xs.copyWithin(0, 1);
            const c = xs.has(0) ? xs[0] : 0;
            return a * 100 + b * 10 + c;
        }"#)
    .await
    .unwrap();
    assert_eq!(value, HostValue::Number(444.));
}

#[tokio::test]
async fn string_index_assignment_is_a_run_error() {
    let err = run(r#"
        fn main() string {
            const s = "hello";
            s[0] = "H";
            return s;
        }"#)
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "Cannot assign to read only property");
}
