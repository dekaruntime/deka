use std::process::Command;
#[test]
fn run_and_check_use_the_same_native_codec_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.ds");
    std::fs::write(&file,r#"import {echo} from "io";
fn main() {const e=TextEncoder();const d=unwrap(TextDecoder()) or {echo("constructor failed");return;};
const text=unwrap(d.decode(e.encode("Hello, café 🙂"))) or {echo("decode failed");return;};
echo(e.encoding+":"+text);
const error=match TextDecoder("not-a-codec") {Ok(value)=>"accepted",Err(message)=>"unsupported"};echo(error);}"#).unwrap();
    let check = Command::new(env!("CARGO_BIN_EXE_deka"))
        .arg("check")
        .arg(&file)
        .arg("--entry")
        .arg("main")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let output = Command::new(env!("CARGO_BIN_EXE_deka"))
        .arg("run")
        .arg(&file)
        .arg("--entry")
        .arg("main")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout,
        "utf-8:Hello, café 🙂\nunsupported\n".as_bytes()
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
