#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Wake, Waker},
};
fn build_vm(source: &str, hosts: Hosts) -> Vm {
    Vm::new(compiler::compile(source, &hosts).unwrap(), hosts).unwrap()
}
#[tokio::test]
async fn namespace_aliases_and_empty_all_are_typed_native_values() {
    let mut vm = build_vm(
        r#"async fn value(n:number) Promise<number> { return n; }
async fn main() Promise<Array<number>> { const P=Promise; const all=P.all;
const empty=await Promise.all<number>([]); const xs=await all([value(7),value(8)]);
return [empty.length,(xs.has(0)?xs[0]:-1),(xs.has(1)?xs[1]:-1)]; }"#,
        Hosts::default(),
    );
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::List(vec![
            HostValue::Number(0.),
            HostValue::Number(7.),
            HostValue::Number(8.)
        ])
    );
    assert_eq!(vm.stats().live, 0);
    let mut vm = build_vm(
        r#"fn main() number { const Promise={all:fn(n:number) number {return n+1;}}; return Promise.all(8); }"#,
        Hosts::default(),
    );
    assert_eq!(vm.run().await.unwrap(), HostValue::Number(9.));
}
#[test]
fn combinators_reject_non_promises_and_incompatible_result_types() {
    for source in [
        "async fn main() {await Promise.all([7]);}",
        "async fn main() {await Promise.race([7]);}",
        "async fn value() Promise<number> {return 7;} async fn main() Promise<string> {return await Promise.race([value()]);}",
        "async fn value() Promise<number> {return 7;} async fn main() Promise<Array<string>> {return await Promise.all([value()]);}",
        "fn main() {Promise.all();}",
        "fn main() {Promise.race([],[]);}",
    ] {
        assert!(
            compiler::compile(source, &Hosts::default()).is_err(),
            "{source}"
        );
    }
}
fn gated(source: &str) -> (Vm, Vec<tokio::sync::oneshot::Sender<String>>) {
    let (s0, r0) = tokio::sync::oneshot::channel();
    let (s1, r1) = tokio::sync::oneshot::channel();
    let receivers = Rc::new(RefCell::new(vec![Some(r0), Some(r1)]));
    let mut hosts = Hosts::default();
    hosts
        .register(HostOp::new(
            "gate",
            vec![HostType::Number],
            HostType::String,
            true,
            move |args| {
                let HostValue::Number(index) = args[0] else {
                    unreachable!()
                };
                let receive = receivers.borrow_mut()[index as usize].take().unwrap();
                HostReply::Pending(Box::pin(async move {
                    receive
                        .await
                        .map(HostValue::String)
                        .map_err(|e| e.to_string())
                }))
            },
        ))
        .unwrap();
    (build_vm(source, hosts), vec![s0, s1])
}
fn idle(vm: &mut Vm) {
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..20000 {
        if !vm.has_ready_work() {
            return;
        }
        assert!(vm.run_turn(&mut cx, 8).unwrap().instructions <= 8);
    }
    panic!("VM did not park");
}
#[tokio::test]
async fn all_snapshots_the_list_keeps_values_alive_and_preserves_input_order() {
    let (mut vm, mut sends) = gated(
        r#"import {gate} from "vm:host";
async fn main() Promise<Array<string>> { let inputs=[gate(0),gate(1)]; const joined=Promise.all(inputs); inputs=[];
return await joined; }"#,
    );
    idle(&mut vm);
    sends.pop().unwrap().send("second".into()).unwrap();
    idle(&mut vm);
    vm.collect().unwrap();
    assert_eq!(vm.pending_tasks(), 4);
    sends.pop().unwrap().send("first".into()).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::List(vec![
            HostValue::String("first".into()),
            HostValue::String("second".into())
        ])
    );
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test]
async fn race_returns_the_fast_input_but_main_still_waits_for_the_loser() {
    let (mut vm, mut sends) = gated(
        r#"import {gate} from "vm:host";
async fn main() Promise<string> { return await Promise.race([gate(0),gate(1)]); }"#,
    );
    idle(&mut vm);
    sends.pop().unwrap().send("winner".into()).unwrap();
    idle(&mut vm);
    assert_eq!(vm.pending_tasks(), 1);
    assert!(
        vm.poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    sends.pop().unwrap().send("loser".into()).unwrap();
    assert_eq!(vm.run().await.unwrap(), HostValue::String("winner".into()));
}
#[tokio::test]
async fn all_forwards_checked_throws_and_keeps_io_results_as_data() {
    for expression in [
        "Promise.all([source()])",
        "all([source()])",
        "([source()] |> all)",
        "([source()] |> all())",
        "([source()] |> all(_))",
    ] {
        let source = format!(
            r#"async fn source() Promise<Exception<number,string>> {{return Throw("bad");}}
async fn main() Promise<number> {{const all=Promise.all; return match await {expression} {{Ok(xs)=>xs.has(0)?xs[0]:-1,Throw(e)=>99}};}}"#
        );
        let mut vm = build_vm(&source, Hosts::default());
        assert_eq!(
            vm.run().await.unwrap(),
            HostValue::Number(99.),
            "{expression}"
        );
    }
    for kind in ["all", "race"] {
        let expression = if kind == "all" {
            "xs.has(0)?string(xs[0]):\"empty\""
        } else {
            "string(xs)"
        };
        let source = format!(
            r#"async fn source() Promise<Exception<number,string>> {{return Throw("bad");}}
async fn main() Promise<string> {{try {{const xs=await Promise.{kind}([source()]); return {expression};}} catch(e) {{return e;}}}}"#
        );
        let mut vm = build_vm(&source, Hosts::default());
        assert_eq!(vm.run().await.unwrap(), HostValue::String("bad".into()));
    }
    let mut vm = build_vm(
        r#"async fn source() Promise<Result<number,string>> {return Err("io");}
async fn main() Promise<string> {const xs=await Promise.all([source()]);return xs.has(0) ? match xs[0] {Ok(v)=>string(v),Err(e)=>e} : "empty";}"#,
        Hosts::default(),
    );
    assert_eq!(vm.run().await.unwrap(), HostValue::String("io".into()));
    let mut vm = build_vm(
        r#"async fn source() Promise<Exception<number,string>> {return Ok(7);}
async fn main() Promise<number> {return match await Promise.all([source()]) {Ok(xs)=>xs.has(0)?xs[0]:-1,Throw(e)=>99};}"#,
        Hosts::default(),
    );
    assert_eq!(vm.run().await.unwrap(), HostValue::Number(7.));
}
#[test]
fn all_cannot_erase_a_checked_exception_through_aliases_or_generic_wrappers() {
    for expression in [
        "Promise.all([source()])",
        "all([source()])",
        "([source()] |> all)",
        "([source()] |> all())",
        "([source()] |> all(_))",
    ] {
        let source = format!(
            r#"async fn source() Promise<Exception<number,string>> {{return Throw("bad");}}
async fn main() Promise<number> {{const all=Promise.all;const xs=await {expression};return xs.has(0)?xs[0]:-1;}}"#
        );
        assert!(
            compiler::compile(&source, &Hosts::default()).is_err(),
            "{expression}"
        );
    }
    for source in [
        "fn joined<T>(ps:Array<Promise<T>>) Promise<Array<T>> {return Promise.all(ps);} fn main() {}",
        "fn main() {const all=Promise.all(_);}",
        "fn main() {const all:fn(Array<Promise<number>>) Promise<Array<number>>=Promise.all;}",
    ] {
        assert!(
            compiler::compile(source, &Hosts::default()).is_err(),
            "{source}"
        );
    }
}
#[tokio::test]
async fn imported_alias_and_barrel_preserve_all_exception_contract() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("join.ds"), "export const all=Promise.all;").unwrap();
    std::fs::write(
        dir.path().join("barrel.ds"),
        "export {all} from \"./join.ds\";",
    )
    .unwrap();
    let entry = dir.path().join("main.ds");
    let source = r#"import {all} from "./barrel.ds";
async fn source() Promise<Exception<number,string>> {return Throw("bad");}
async fn main() Promise<number> {return match await all([source()]) {Ok(xs)=>xs.has(0)?xs[0]:-1,Throw(e)=>99};}"#;
    std::fs::write(&entry, source).unwrap();
    let hosts = Hosts::default();
    let program = compiler::compile_file(&entry, &hosts, Some("main")).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    assert_eq!(vm.run().await.unwrap(), HostValue::Number(99.));
    std::fs::write(
        &entry,
        source.replace(
            "return match await all([source()]) {Ok(xs)=>xs.has(0)?xs[0]:-1,Throw(e)=>99};",
            "const xs=await all([source()]);return xs.has(0)?xs[0]:-1;",
        ),
    )
    .unwrap();
    assert!(compiler::compile_file(&entry, &Hosts::default(), Some("main")).is_err());
}
#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn empty_race_parks_without_idle_wakes_and_can_be_cancelled() {
    let mut vm = build_vm(
        "async fn main() {await Promise.race<number>([]);}",
        Hosts::default(),
    );
    let wakes = Arc::new(Wakes::default());
    vm.set_waker(&Waker::from(wakes.clone()));
    idle(&mut vm);
    assert_eq!(vm.pending_tasks(), 3);
    let before = wakes.0.load(Ordering::SeqCst);
    assert!(
        vm.poll(&mut Context::from_waker(&Waker::from(wakes.clone())))
            .is_pending()
    );
    assert_eq!(before, wakes.0.load(Ordering::SeqCst));
    vm.cancel().unwrap();
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test]
async fn cli_turns_are_bounded_and_long_running_programs_have_no_lifetime_quota() {
    let mut vm = build_vm(
        "async fn spin() {for(;;){}} fn main() {for(let i=0;i<100;i+=1){spin();}}",
        Hosts::default(),
    );
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..10 {
        let before = vm.instructions();
        assert!(vm.poll(&mut cx).is_pending());
        assert!(vm.instructions() - before <= 4096);
    }
    vm.cancel().unwrap();
    let mut vm = build_vm(
        "fn main() number {let n=0;for(let i=0;i<1100000;i+=1){n+=1;}return n;}",
        Hosts::default(),
    );
    assert_eq!(vm.run().await.unwrap(), HostValue::Number(1100000.));
    assert!(vm.instructions() > 10_000_000);
    assert_eq!(vm.stats().live, 0);
}

#[tokio::test]
async fn transporting_native_functions_preserves_their_own_type_parameter() {
    let mut machine = build_vm(
        r#"fn identity<R>(x:R) R {return x;}
async fn source() Promise<Exception<number,string>> {return Throw("bad");}
async fn main() Promise<number> {const all=identity(Promise.all); return match await all([source()]) {Ok(xs)=>xs.has(0)?xs[0]:-1,Throw(e)=>99};}"#,
        Hosts::default(),
    );
    assert_eq!(machine.run().await.unwrap(), HostValue::Number(99.));
    for source in [
        "async fn main() {await Promise.all<number,string>([]);}",
        "async fn main() {await Promise.race<number,string>([]);}",
        "fn identity<R>(x:R) R {return x;} async fn text() Promise<string> {return \"text\";} async fn main() Promise<number> {const race=identity(Promise.race);return await race([text()]);}",
    ] {
        assert!(
            compiler::compile(source, &Hosts::default()).is_err(),
            "{source}"
        );
    }
}
