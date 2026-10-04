#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::time::{Duration, UNIX_EPOCH};

async fn run(source: &str, clock: impl Fn() -> std::time::SystemTime + 'static) -> HostValue {
    let mut hosts = Hosts::default();
    builtin_time::register_with_clock(&mut hosts, clock).unwrap();
    let program = compiler::compile(source, &hosts).unwrap();
    let program: Program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let value = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    value
}
#[tokio::test(start_paused = true)]
async fn now_and_sleep_share_real_declared_types_and_tokio_clock() {
    let start = tokio::time::Instant::now();
    assert_eq!(
        run(
            r#"import {now,sleep} from "time";
async fn main() Promise<number> {const before=now();await sleep(25);return now()-before;}"#,
            move || UNIX_EPOCH + Duration::from_secs(1000) + (tokio::time::Instant::now() - start)
        )
        .await,
        HostValue::Number(25.)
    );
}
#[tokio::test(start_paused = true)]
async fn zero_timer_completes_and_imported_aliases_work() {
    assert_eq!(
        run(
            r#"import {now as clock,sleep as wait} from "time";
async fn main() Promise<number> {await wait(0);return clock();}"#,
            || UNIX_EPOCH + Duration::from_millis(123)
        )
        .await,
        HostValue::Number(123.)
    );
}
#[tokio::test]
async fn wall_clock_can_move_backwards_and_is_not_elapsed_time() {
    let times = std::cell::Cell::new(0);
    assert_eq!(
        run(
            r#"import {now} from "time";
fn main() number {const first=now();const second=now();return second-first;}"#,
            move || {
                let n = times.get();
                times.set(n + 1);
                UNIX_EPOCH + Duration::from_millis(if n == 0 { 100 } else { 90 })
            }
        )
        .await,
        HostValue::Number(-10.)
    );
}
#[test]
fn wrong_arity_types_and_unawaited_timer_values_are_compile_errors() {
    let mut hosts = Hosts::default();
    builtin_time::register(&mut hosts).unwrap();
    for source in [
        r#"import {now} from "time";fn main(){now(1);}"#,
        r#"import {sleep} from "time";fn main(){sleep();}"#,
        r#"import {sleep} from "time";fn main(){sleep("1");}"#,
        r#"import {sleep} from "time";fn main() number{return sleep(1);}"#,
        r#"import {now} from "io";fn main(){now();}"#,
    ] {
        assert!(compiler::compile(source, &hosts).is_err(), "{source}");
    }
}
#[tokio::test]
async fn invalid_duration_is_an_argument_fault_not_an_invented_success() {
    let mut hosts = Hosts::default();
    builtin_time::register(&mut hosts).unwrap();
    let program = compiler::compile(
        r#"import {sleep} from "time";async fn main() Promise<void>{await sleep(-1);}"#,
        &hosts,
    )
    .unwrap();
    let error = Vm::new(program, hosts).unwrap().run().await.unwrap_err();
    assert!(
        error.contains("finite, nonnegative milliseconds"),
        "{error}"
    );
}
#[tokio::test(start_paused = true)]
async fn source_deleted_modules_retain_clock_and_timer_imports() {
    let dir = tempfile::tempdir().unwrap();
    let module = dir.path().join("clock.ds");
    let entry = dir.path().join("main.ds");
    std::fs::write(&module,r#"import {now,sleep} from "time";export async fn elapsed() Promise<number>{const before=now();await sleep(7);return now()-before;}"#).unwrap();
    std::fs::write(&entry,r#"import {elapsed} from "./clock.ds";async fn main() Promise<number>{return await elapsed();}"#).unwrap();
    let start = tokio::time::Instant::now();
    let mut hosts = Hosts::default();
    builtin_time::register_with_clock(&mut hosts, move || {
        UNIX_EPOCH + Duration::from_secs(1000) + (tokio::time::Instant::now() - start)
    })
    .unwrap();
    let program = compiler::compile_file(&entry, &hosts, Some("main")).unwrap();
    let program: Program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    std::fs::remove_file(module).unwrap();
    std::fs::remove_file(entry).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    assert_eq!(vm.run().await.unwrap(), HostValue::Number(7.));
    assert_eq!(vm.stats().live, 0);
}
