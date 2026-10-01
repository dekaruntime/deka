use deka_vm::*;
use std::task::{Context, Poll, Waker};
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: dvm-core program.dvm.json (no host operations)")?;
    let program = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut vm = Vm::new(program, Hosts::default())?;
    let waker = Waker::noop().clone();
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(result) = vm.poll(&mut cx) {
            println!("{:?}", result?);
            println!("{:?}", vm.stats());
            break;
        }
    }
    Ok(())
}
