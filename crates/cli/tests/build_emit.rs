//! `deka build` compiles through dsc, then copies host static files.

use std::fs;
use std::path::Path;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn init_project(dir: &Path) {
    let output = Command::new(cli_bin())
        .args(["init", "."])
        .current_dir(dir)
        .output()
        .expect("run deka init");
    assert!(
        output.status.success(),
        "deka init failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_build(dir: &Path) -> (bool, String) {
    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(dir)
        .output()
        .expect("run deka build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

#[test]
fn build_help_does_not_build() {
    // deka#750: `deka build --help` used to fall through to the build handler
    // (which needs a project and would fail or write dist). Help must print
    // and exit 0 without touching the filesystem.
    let project = tempfile::tempdir().expect("create temp project dir");
    let output = Command::new(cli_bin())
        .args(["build", "--help"])
        .current_dir(project.path())
        .output()
        .expect("run deka build --help");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "deka build --help must exit 0: {combined}");
    assert!(combined.contains("build"), "help must mention build: {combined}");
    assert!(
        !project.path().join("dist").exists(),
        "deka build --help must not write dist"
    );
}

#[test]
fn build_emits_src_one_to_one() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    let src = project.path().join("src");
    fs::create_dir_all(src.join("nested")).expect("mkdir src/nested");
    fs::write(src.join("util.ds"), "export const n = 1;\n").expect("write src/util.ds");
    fs::write(src.join("notes.txt"), "keep me\n").expect("write src/notes.txt");
    fs::write(src.join("nested").join("readme.md"), "verbatim\n")
        .expect("write src/nested/readme.md");

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with a no-magic src/ tree: {combined}"
    );

    let dist_src = project.path().join("dist").join("server").join("src");
    let util_js = fs::read_to_string(dist_src.join("util.js")).expect("read dist/src/util.js");
    assert!(
        util_js.contains("export const n") || util_js.contains("n = 1"),
        "src/ .ds must compile via dsc into dist/src: {util_js}"
    );
    assert!(
        !dist_src.join("util.ds").exists(),
        "src/ .ds must not be copied raw into dist/src"
    );
    assert_eq!(
        fs::read_to_string(dist_src.join("notes.txt")).expect("read notes"),
        "keep me\n"
    );
    assert_eq!(
        fs::read_to_string(dist_src.join("nested").join("readme.md")).expect("read readme"),
        "verbatim\n"
    );
}

#[test]
fn build_missing_dsc_is_hard_error() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());

    let output = Command::new(cli_bin())
        .arg("build")
        .current_dir(project.path())
        .env("DEKA_NO_DSC", "1")
        .env_remove("DEKA_DSC")
        .output()
        .expect("run deka build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "missing dsc must fail the build: {combined}"
    );
    assert!(
        combined.contains("dsc is required"),
        "error should say dsc is required: {combined}"
    );
    assert!(
        combined.contains("DEKA_DSC"),
        "error should mention DEKA_DSC: {combined}"
    );
    assert!(
        combined.contains("https://deka.gg/install") || combined.contains("install"),
        "error should include install guidance: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "missing dsc must not write dist/"
    );
}

#[test]
fn build_materializes_build_block_values_before_prerendering() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app/types.dsx"),
        r#"struct User { name: string }

export { User }
"#,
    )
    .expect("write build types");
    fs::write(
        project.path().join("app/page.dsx"),
        r#"import { User } from "./types.dsx"

enum Shape { Empty, Rect(number) }
type Cents number

const user: User = build {
  return Ok(User { name: "Ada" })
}
const shape: Shape = build {
  return Ok(Shape.Rect(3))
}
const note: Option<string> = build {
  return Ok(Some("ready"))
}
const cents: Cents = build {
  return Ok(Cents(12))
}

export fn Page() {
  const kind = match (shape) {
    Shape.Rect(size) => "rect " + string(size),
    Shape.Empty => "empty",
  }
  const subtitle = match (note) {
    Some(value) => value,
    None => "missing",
  }
  return <section><h1>{user.name}</h1><p>{kind} {subtitle} {string(unboxNumber(cents))}</p></section>
}
"#,
    )
    .expect("write build page");

    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should execute build blocks: {combined}"
    );

    let html = fs::read_to_string(project.path().join("dist/client/index.html"))
        .expect("read prerendered HTML");
    for expected in ["Ada", "rect 3", "ready", "12"] {
        assert!(
            html.contains(expected),
            "build value `{expected}` was not prerendered: {html}"
        );
    }

    let values = project.path().join(".cache/dekascript/build-values");
    let materialized = fs::read_dir(values)
        .expect("read materialized values")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "js"))
        .count();
    assert_eq!(materialized, 4, "one virtual module per build binding");

    let runtime_js = fs::read_to_string(project.path().join("dist/server/app/page.js"))
        .expect("read emitted runtime module");
    assert!(
        !runtime_js.contains("Ada") && !runtime_js.contains("build {"),
        "runtime output must not retain the build body: {runtime_js}"
    );
    let generated_entries = fs::read_dir(project.path().join("dist/server/app"))
        .expect("read emitted app")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".__deka_build_")
        });
    assert!(
        !generated_entries,
        "generated build entries must not be promoted to dist"
    );
}

#[test]
fn build_stops_when_a_build_block_returns_err() {
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app/page.dsx"),
        r#"const title: string = build {
  return Err("content source is unavailable")
}

export fn Page() {
  return <section><h1>{title}</h1></section>
}
"#,
    )
    .expect("write failing build page");

    let (success, combined) = run_build(project.path());
    assert!(!success, "Result.Err must stop deka build: {combined}");
    assert!(
        combined.contains("build `title` (at ")
            && combined.contains("page.dsx:1:23) failed: content source is unavailable"),
        "build error must link the source location and preserve Result.Err text: {combined}"
    );
    assert!(
        !project.path().join("dist").exists(),
        "failed build entries must not promote dist output"
    );
}


#[test]
fn build_prerenders_island_props_into_initial_html() {
    // deka#746 F3: an island rendered during prerender receives its props
    // in-process, so the prop values must land in the initial HTML — blank
    // spans that only fill after hydration are the silent default this
    // guards against.
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("page.dsx"),
        "import { signal } from \"ui/reactive\"\n\nstruct User { name: string; age: number }\nstruct CardProps { user: User }\n\nexport fn Card(props: CardProps) {\n    const c = signal(0)\n    return <div><span id=\"nm\">{props.user.name}</span><span id=\"ag\">{props.user.age}</span><span id=\"c\">{c[0]()}</span></div>\n}\n\nexport fn Page() {\n    const u = User { name: \"Ada\", age: 36 }\n    return <section><Card user={u} client:load /></section>\n}\n",
    )
    .expect("write island props page");
    let (success, combined) = run_build(project.path());
    assert!(
        success,
        "deka build should succeed with the issue's Card fixture: {combined}"
    );
    let index = fs::read_to_string(
        project
            .path()
            .join("dist")
            .join("client")
            .join("index.html"),
    )
    .expect("read prerendered html");
    assert!(
        index.contains("<span data-deka-id=\"page:Card/i0/i0\" id=\"nm\">Ada</span>"),
        "user.name must prerender into the initial HTML: {index}"
    );
    assert!(
        index.contains("<span data-deka-id=\"page:Card/i0/i1\" id=\"ag\">36</span>"),
        "user.age must prerender into the initial HTML: {index}"
    );
    assert!(
        index.contains("id=\"c\">0</span>"),
        "the island's local signal must still render: {index}"
    );
}

#[test]
fn build_fails_loudly_when_island_prop_is_absent_at_prerender() {
    // deka#746 F3: reading a field of an absent prop throws inside the
    // island's live() expression. That must fail the build with the actual
    // error — not ship blank spans that only fill after hydration. (A
    // typed prop can be absent at runtime here because the out-of-bounds
    // index is typed Array<User> but yields undefined.)
    let project = tempfile::tempdir().expect("create temp project dir");
    init_project(project.path());
    fs::write(
        project.path().join("app").join("page.dsx"),
        "struct User { name: string; age: number }\nstruct CardProps { user: User }\n\nexport fn Card(props: CardProps) {\n    return <div><span id=\"nm\">{props.user.name}</span></div>\n}\n\nexport fn Page() {\n    const users: Array<User> = []\n    return <section><Card user={users[0]} client:load /></section>\n}\n",
    )
    .expect("write absent-prop island page");
    let (success, combined) = run_build(project.path());
    assert!(
        !success,
        "deka build must fail when an island prop is absent at prerender, got success. output: {combined}"
    );
    assert!(
        combined.contains("Cannot read properties of undefined"),
        "the build error must name the actual failure, not a generic message: {combined}"
    );
    assert!(
        !project
            .path()
            .join("dist")
            .join("client")
            .join("index.html")
            .exists(),
        "a failed prerender must not publish dist HTML: {combined}"
    );
}
