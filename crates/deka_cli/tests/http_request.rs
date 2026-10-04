#[path = "support/check.rs"]
mod check;
use std::{
    path::Path,
    process::{Command, Output},
};
fn ok(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn cli(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_deka"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn request_runs_and_survives_source_deletion_relocation_and_empty_environment() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("request.ds"),r#"
fn describe(request:Request) string {
    const headers=request.headers;
    const updated=headers.set("X-Project","Deka");
    const value=match request.headers.get("x-project") { Ok(value)=>string(value),Err(error)=>error };
    return request.method+" "+request.url+" "+value;
}
console.log(match Request("https://EXAMPLE.com:443/a/../tour#project") { Ok(request)=>describe(request),Err(error)=>error });
console.log(match Request("https://user:pass@example.com") { Ok(request)=>"wrong",Err(error)=>"credentials rejected" });
"#).unwrap();
    let expected = "GET https://example.com/tour#project Some(\"Deka\")\ncredentials rejected\n";
    check::checked(cli(project.path(), &["check", "request.ds"]), "request.ds");
    assert_eq!(ok(cli(project.path(), &["run", "request.ds"])), expected);
    let output = tempfile::tempdir().unwrap();
    let binary = output.path().join("request-app");
    ok(cli(
        project.path(),
        &["build", "request.ds", "--outfile", binary.to_str().unwrap()],
    ));
    drop(project);
    let elsewhere = tempfile::tempdir().unwrap();
    let relocated = elsewhere.path().join("app");
    std::fs::rename(binary, &relocated).unwrap();
    assert_eq!(
        ok(Command::new(&relocated)
            .current_dir(elsewhere.path())
            .env_clear()
            .output()
            .unwrap()),
        expected
    );
}
