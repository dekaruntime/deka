//! dsc#82: a `client:load` island must prerender a live signal read.
//!
//! The compiler stamps `data-deka-id` on the host element (RFD 24 §10.6);
//! this test does not change that. It asserts the initial value is inside
//! the stamped tag, matching static island content.

use std::fs;
use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

const EMPTY_DEKA_LOCK: &str = r#"{"lockfileVersion":1,"packages":{}}"#;

#[test]
fn run_ssr_client_load_island_emits_live_read_inside_stamped_host() {
    let project = tempfile::tempdir().expect("project");
    fs::write(
        project.path().join("deka.json"),
        r#"{"name":"island-ssr-signal","security":{"allow":{},"prompt":false}}"#,
    )
    .expect("manifest");
    fs::write(project.path().join("deka.lock"), EMPTY_DEKA_LOCK).expect("lockfile");
    fs::write(
        project.path().join("counter.dsx"),
        r#"
fn seed() number { return 0 }

export fn Counter() ReactNode {
  return <button>{seed()}</button>
}

const tree = <Counter client:load />
const _ = unsafe {
  const rendered = deka.ui.renderToString(tree)
  console.log(rendered.html)
  return 1
}
"#,
    )
    .expect("entry");

    let output = Command::new(cli_bin())
        .args(["run", "counter.dsx"])
        .current_dir(project.path())
        .env("NO_COLOR", "1")
        .output()
        .expect("deka run");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "deka run failed: {combined}");
    assert!(
        combined.contains("data-deka-id=\"counter:Counter/i0\""),
        "RFD 24 host stamp missing:\n{combined}"
    );
    assert!(
        combined.contains(">0</button>"),
        "live island read must prerender the initial value:\n{combined}"
    );
    assert!(
        combined.contains("data-deka-island=\"Counter\""),
        "client:load must wrap the island:\n{combined}"
    );
    assert!(
        !combined.contains("<button data-deka-id=\"counter:Counter/i0\"></button>"),
        "empty island body is the dsc#82 defect:\n{combined}"
    );
}
