#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::{HostValue, Vm, compiler, demo};
use std::process::Command;

const SOURCE: &str = r#"import {delay} from "vm:host";
async fn main() Promise<string> {return await delay(1000,"available");}"#;

#[tokio::test(start_paused = true)]
async fn registered_timer_runs_without_a_permission_policy() {
    let (hosts, _) = demo::hosts().unwrap();
    let mut vm = Vm::new(compiler::compile(SOURCE, &hosts).unwrap(), hosts).unwrap();
    assert_eq!(
        vm.run().await.unwrap(),
        HostValue::String("available".into())
    );
    assert_eq!(vm.stats().live, 0);
    assert_eq!(vm.pending_tasks(), 0);
}

fn source_file(directory: &std::path::Path) -> std::path::PathBuf {
    let path = directory.join("available.ds");
    std::fs::write(&path, SOURCE.replace("delay(1000", "delay(0")).unwrap();
    path
}

#[test]
fn source_and_serialized_bytecode_execute_without_grant_flags() {
    let directory = tempfile::tempdir().unwrap();
    let source = source_file(directory.path());
    let (hosts, _) = demo::hosts().unwrap();
    let program = compiler::compile(&std::fs::read_to_string(&source).unwrap(), &hosts).unwrap();
    let bytecode = directory.path().join("available.dvm.json");
    std::fs::write(&bytecode, serde_json::to_vec(&program).unwrap()).unwrap();
    for path in [&source, &bytecode] {
        let output = Command::new(env!("CARGO_BIN_EXE_dvm"))
            .arg(path)
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("result: String(\"available\")"));
    }
}

#[test]
fn obsolete_grant_flags_are_rejected_before_execution() {
    let directory = tempfile::tempdir().unwrap();
    let source = source_file(directory.path());
    for flag in ["--grant-timer", "--grant-network"] {
        let output = Command::new(env!("CARGO_BIN_EXE_dvm"))
            .arg(&source)
            .arg(flag)
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(&format!("unknown argument: {flag}"))
        );
    }
}
