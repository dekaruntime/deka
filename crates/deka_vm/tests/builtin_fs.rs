#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{collections::BTreeMap, path::Path};
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    builtin_fs::register(&mut hosts).unwrap();
    bytes::register(&mut hosts).unwrap();
    hosts
}
fn path(path: &Path) -> String {
    serde_json::to_string(path.to_str().unwrap()).unwrap()
}
async fn run(source: &str) -> HostValue {
    let hosts = hosts();
    let program = compiler::compile(source, &hosts).unwrap();
    let program: Program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let result = vm.run().await.unwrap();
    assert_eq!(vm.stats().live, 0);
    result
}
#[tokio::test]
async fn async_binary_io_preserves_bytes_and_reports_written_count() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("nested");
    let file = directory.join("data.bin");
    let source = format!(
        r#"import {{mkdirs,write_file,read_file}} from "fs"
import {{from_hex,to_hex}} from "bytes"
async fn main() Promise<string> {{
const made=unwrap(await mkdirs({directory})) or {{return "mkdirs failed";}};
const data=unwrap(from_hex("00ffc3a900")) or {{return "hex failed";}};
const written=unwrap(await write_file({file},data)) or {{return "write failed";}};
const read=unwrap(await read_file({file})) or {{return "read failed";}};
return string(made)+":"+string(written)+":"+to_hex(read)+":"+to_hex(data);
}}"#,
        directory = path(&directory),
        file = path(&file)
    );
    assert_eq!(
        run(&source).await,
        HostValue::String("true:5:00ffc3a900:00ffc3a900".into())
    );
    assert_eq!(std::fs::read(file).unwrap(), [0, 255, 195, 169, 0]);
}
#[tokio::test]
async fn synchronous_twins_are_blocking_values_and_mkdirs_is_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("nested");
    let file = directory.join("empty");
    let source = format!(
        r#"import {{mkdirs_sync,write_file_sync,read_file_sync}} from "fs"
import {{from_string,len}} from "bytes"
fn main() string {{
const first=unwrap(mkdirs_sync({directory})) or {{return "mkdir failed";}};
const second=unwrap(mkdirs_sync({directory})) or {{return "again failed";}};
const count=unwrap(write_file_sync({file},from_string(""))) or {{return "write failed";}};
const data=unwrap(read_file_sync({file})) or {{return "read failed";}};
return string(first)+":"+string(second)+":"+string(count)+":"+string(len(data));
}}"#,
        directory = path(&directory),
        file = path(&file)
    );
    assert_eq!(
        run(&source).await,
        HostValue::String("true:true:0:0".into())
    );
    assert!(std::fs::read(file).unwrap().is_empty());
}
#[tokio::test]
async fn failed_io_is_a_named_enum_result_without_creating_parents() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("absent/file");
    for operation in ["write_file", "write_file_sync"] {
        let call = format!("{operation}({},from_string(\"data\"))", path(&file));
        let call = if operation.ends_with("_sync") {
            call
        } else {
            format!("await {call}")
        };
        let source = format!(
            r#"import {{{operation},FsError as Fault}} from "fs"
import {{from_string}} from "bytes"
async fn main() Promise<string> {{
try {{return match {call} {{Ok(v)=>"wrong",Err(e)=>match e {{Failed(s)=>e.getType().toString(),PermissionDenied(p)=>"permission",UnsupportedHost=>"unsupported",InvalidPayload=>"invalid"}}}};}}
catch(e) {{return "thrown";}}
}}"#
        );
        assert_eq!(run(&source).await, HostValue::String("FsError".into()));
        assert!(!file.parent().unwrap().exists());
    }
}
#[tokio::test]
async fn directory_entries_are_typed_records_in_both_modes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("é.bin"), [255]).unwrap();
    std::fs::create_dir(root.path().join("child")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("é.bin", root.path().join("link")).unwrap();
    for operation in ["read_dir", "read_dir_sync"] {
        let call = format!("{operation}({})", path(root.path()));
        let call = if operation.ends_with("_sync") {
            call
        } else {
            format!("await {call}")
        };
        let source = format!(
            r#"import {{{operation}}} from "fs"
interface Entry {{name:string;is_dir:boolean;is_file:boolean;}}
async fn main() Promise<Array<Entry>> {{const entries=unwrap({call}) or {{return [];}};return entries;}}"#
        );
        let HostValue::List(entries) = run(&source).await else {
            panic!("not a list")
        };
        let mut actual = BTreeMap::new();
        for entry in entries {
            let HostValue::Record(mut fields) = entry else {
                panic!("not a record")
            };
            let HostValue::String(name) = fields.remove("name").unwrap() else {
                panic!("not a name")
            };
            actual.insert(name, fields);
        }
        let flags = |dir, file| {
            BTreeMap::from([
                ("is_dir".into(), HostValue::Bool(dir)),
                ("is_file".into(), HostValue::Bool(file)),
            ])
        };
        let mut expected = BTreeMap::from([
            ("é.bin".into(), flags(false, true)),
            ("child".into(), flags(true, false)),
        ]);
        #[cfg(unix)]
        expected.insert("link".into(), flags(false, false));
        assert_eq!(actual, expected);
    }
}
// APFS rejects non-UTF-8 names at creation; Linux can exercise this real I/O path.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn directory_names_are_strict_utf8_not_lossy() {
    use std::os::unix::ffi::OsStringExt;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join(std::ffi::OsString::from_vec(vec![255])),
        [],
    )
    .unwrap();
    let source = format!(
        r#"import {{read_dir,read_dir_sync}} from "fs"
async fn main() Promise<string> {{
const first=match await read_dir({directory}) {{Ok(v)=>"wrong",Err(e)=>match e {{Failed(s)=>s,PermissionDenied(p)=>"permission",UnsupportedHost=>"unsupported",InvalidPayload=>"invalid"}}}};
const second=match read_dir_sync({directory}) {{Ok(v)=>"wrong",Err(e)=>match e {{Failed(s)=>s,PermissionDenied(p)=>"permission",UnsupportedHost=>"unsupported",InvalidPayload=>"invalid"}}}};
return first+":"+second;
}}"#,
        directory = path(root.path())
    );
    assert_eq!(
        run(&source).await,
        HostValue::String(
            "read_dir: entry name is not valid UTF-8:read_dir: entry name is not valid UTF-8"
                .into()
        )
    );
}
#[tokio::test]
async fn two_started_reads_and_one_failure_complete_without_throwing() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    let absent = root.path().join("absent");
    std::fs::write(&a, [0, 255]).unwrap();
    std::fs::write(&b, [128]).unwrap();
    let source = format!(
        r#"import {{read_file}} from "fs"
import {{to_hex}} from "bytes"
async fn main() Promise<string> {{
const a=read_file({a});const b=read_file({b});const bad=read_file({absent});
const first=unwrap(await a) or {{return "a failed";}};
const second=unwrap(await b) or {{return "b failed";}};
const failure=match await bad {{Ok(v)=>"wrong",Err(e)=>match e {{Failed(s)=>"failed",PermissionDenied(p)=>"permission",UnsupportedHost=>"unsupported",InvalidPayload=>"invalid"}}}};
return to_hex(first)+":"+to_hex(second)+":"+failure;
}}"#,
        a = path(&a),
        b = path(&b),
        absent = path(&absent)
    );
    assert_eq!(
        run(&source).await,
        HostValue::String("00ff:80:failed".into())
    );
}
#[test]
fn filesystem_arguments_and_async_contract_are_checked() {
    for source in [
        r#"import {read_file} from "fs";fn main() {read_file(7);}"#,
        r#"import {write_file} from "fs";fn main() {write_file("file","text");}"#,
        r#"import {read_file_sync,FsError} from "fs";fn main() {const value:Promise<Result<bytes,FsError>>=read_file_sync("file");}"#,
        r#"import {read_file} from "fs";import {len} from "bytes";fn main() {len(read_file("file"));}"#,
        r#"import {read_file} from "io";fn main() {read_file("file");}"#,
    ] {
        assert!(compiler::compile(source, &hosts()).is_err(), "{source}");
    }
}
