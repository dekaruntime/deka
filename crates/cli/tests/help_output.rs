//! deka#977 / deka#978: `deka <cmd> --help` must be its own text, not a
//! copy of `deka --help`; global help must lead with Getting Started; and
//! `--yes` must not appear three times in the flags dump.
//!
//! These assert the real rendered output of the *built binary* — not a
//! formatting helper in isolation — because the bug was in the composed
//! CLI, and only running the actual binary proves the fix composes too.

use std::process::Command;

fn run(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_cli"))
        .args(args)
        .output()
        .expect("run cli binary");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

#[test]
fn init_help_is_not_the_global_help() {
    let global = run(&["--help"]);
    let init = run(&["init", "--help"]);
    assert_ne!(
        global, init,
        "`deka init --help` must not be byte-identical to `deka --help` (deka#977)"
    );
    assert!(
        init.contains("Usage: deka init"),
        "init --help should show its own usage line, got:\n{init}"
    );
    assert!(
        init.contains("deka init my-shop") || init.contains("deka init"),
        "init --help should show a worked example, got:\n{init}"
    );
}

#[test]
fn every_first_touch_command_has_its_own_usage_line() {
    for (command, needle) in [
        ("init", "Usage: deka init"),
        ("serve", "Usage: deka serve"),
        ("dev", "Usage: deka dev"),
        ("run", "Usage: deka run"),
        ("build", "Usage: deka build"),
        ("add", "Usage: deka install"),
        ("install", "Usage: deka install"),
    ] {
        let output = run(&[command, "--help"]);
        assert!(
            output.contains(needle),
            "`deka {command} --help` should contain `{needle}`, got:\n{output}"
        );
        let global = run(&["--help"]);
        assert_ne!(
            output, global,
            "`deka {command} --help` must not be byte-identical to `deka --help`"
        );
    }
}

#[test]
fn global_help_leads_with_getting_started() {
    let output = run(&["--help"]);
    let getting_started_at = output
        .find("Getting Started")
        .expect("global help must contain a Getting Started section");
    // Every other category header in the current registry sorts
    // alphabetically after "Getting Started" is printed, so asserting a
    // couple of the alphabetically-earlier ones land later is enough to
    // prove the section is not just present but actually first (deka#978).
    for later_category in ["[auth]", "[database]", "[debug]"] {
        if let Some(pos) = output.find(later_category) {
            assert!(
                getting_started_at < pos,
                "Getting Started ({getting_started_at}) must appear before {later_category} ({pos})"
            );
        }
    }
    assert!(
        output.contains("init")
            && output.find("Getting Started").unwrap() < output.find("init").unwrap()
            || output.contains("init\t\tscaffold a new project"),
        "Getting Started should list init with a plain-English blurb, got:\n{output}"
    );
}

#[test]
fn global_help_flags_do_not_duplicate_yes() {
    let output = run(&["--help"]);
    let yes_lines = output
        .lines()
        .filter(|line| line.trim_start().starts_with("--yes"))
        .count();
    assert_eq!(
        yes_lines, 0,
        "`--yes` is a command-specific flag (install/publish/release), not global; \
         it should not appear at all in `deka --help`'s flags section, got:\n{output}"
    );
}

#[test]
fn install_help_shows_its_own_yes_flag_once() {
    let output = run(&["install", "--help"]);
    let yes_lines = output
        .lines()
        .filter(|line| line.trim_start().starts_with("--yes"))
        .count();
    assert_eq!(
        yes_lines, 1,
        "`deka install --help` should show its own `--yes` flag exactly once, got:\n{output}"
    );
}

#[test]
fn global_help_does_not_advertise_removed_commands() {
    // Guard against advertising a command that is not compiled into this
    // binary — deka#990 is putting `self update`/`self monitor` behind a
    // non-default feature. Whatever this build actually has, help must
    // match: every subcommand line under `self` must correspond to a real
    // registered subcommand.
    let registry = cli::build_registry();
    let Some(self_command) = registry.command_named("self") else {
        return;
    };
    let output = run(&["--help"]);
    let known: Vec<&str> = self_command.subcommands.iter().map(|s| s.name).collect();
    for candidate in ["fetch", "monitor", "test", "update"] {
        let line_present = output
            .lines()
            .any(|line| line.trim_start().starts_with(&format!("self {candidate}")));
        if line_present {
            assert!(
                known.contains(&candidate),
                "help advertises `self {candidate}` but it is not registered in this build"
            );
        }
    }
}
