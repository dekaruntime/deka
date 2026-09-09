// deka#738 (F1): unrecognised arguments are usage errors — the CLI must exit
// non-zero (2, per the GNU usage-error convention). Previously every
// parse-error path printed `[cli] unknown argument ...` and fell through to
// exit 0, so a typo'd flag (e.g. inside `deka build ...`) reported success.
//
// These tests assert exit codes via the real binary, not message text.

use std::process::Command;

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cli")
}

fn exit_code(args: &[&str]) -> Option<i32> {
    Command::new(cli_bin())
        .args(args)
        .output()
        .expect("run deka")
        .status
        .code()
}

#[test]
fn unknown_flag_exits_non_zero() {
    assert_eq!(exit_code(&["--bogus-flag-xyz"]), Some(2));
}

#[test]
fn build_unknown_flag_exits_non_zero() {
    assert_eq!(exit_code(&["build", "--bogus-xyz"]), Some(2));
}

#[test]
fn missing_param_value_exits_non_zero() {
    assert_eq!(exit_code(&["--port"]), Some(2));
}

#[test]
fn help_exits_zero() {
    assert_eq!(exit_code(&["--help"]), Some(0));
}
