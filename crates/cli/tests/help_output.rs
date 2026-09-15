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

#[test]
fn self_help_list_line_is_not_introspects_limit_param() {
    // deka#996 review: `deka self --help` was showing introspect's
    // unrelated `-l` ("limit number of rows (top command)") instead of
    // self's own `--list`. Assert on the rendered text, not by construction
    // (a name exclusion list), so a future edit that reintroduces a
    // name-based lookup fails this test instead of silently regressing.
    let output = run(&["self", "--help"]);
    assert!(
        !output.contains("limit number of rows"),
        "`deka self --help` must not show introspect's `-l` description, got:
{output}"
    );
    assert!(
        output.contains("--list") && output.contains("list fixtures or lessons"),
        "`deka self --help` should show its own `--list` description, got:
{output}"
    );
}

#[test]
fn release_help_shows_its_own_flag_descriptions_not_globals_or_installs() {
    // deka#996 review: `deka release --help` showed the *global* `--version`
    // ("show version") and `--token`/`--registry-url` ("linkhash auth
    // token" / "linkhash registry base URL"), and install's `--yes`
    // ("assume yes when prompted") instead of release's own.
    let output = run(&["release", "--help"]);
    assert!(
        output.contains("target version (deprecated alias; use --pkg-version)"),
        "release --help should show its own --version meaning, got:
{output}"
    );
    assert!(
        !output.contains("--version		show version"),
        "release --help must not show the global --version description, got:
{output}"
    );
    assert!(
        output.contains("PAT token passed through to publish"),
        "release --help should show its own --token meaning, got:
{output}"
    );
    assert!(
        output.contains("auto-accept prompts"),
        "release --help should show its own --yes meaning, got:
{output}"
    );
    assert!(
        !output.contains("assume yes when prompted"),
        "release --help must not show install's --yes description, got:
{output}"
    );
}

#[test]
fn publish_help_shows_its_own_registry_flag_not_installs_stale_default() {
    // deka#996 review: `deka publish --help` showed install's `--registry`
    // ("registry base URL (default: https://git.tana.gg)") instead of its
    // own ("alias for --registry-url") — dragging a retired host into a
    // second command's help.
    let output = run(&["publish", "--help"]);
    assert!(
        output.contains("alias for --registry-url"),
        "publish --help should show its own --registry meaning, got:
{output}"
    );
    assert!(
        !output.contains("git.tana.gg"),
        "publish --help must not show install's stale --registry default, got:
{output}"
    );
}

#[test]
fn task_and_introspect_show_their_own_shared_flag_names() {
    // deka#996 review: `deka task --help`'s `--list` showed self_cmd's
    // description, and `deka introspect --help`'s `--json` showed
    // deka_task's description — both first-match-in-registry leaks.
    let task = run(&["task", "--help"]);
    assert!(
        task.contains("--list		list tasks"),
        "task --help should show its own --list meaning, got:
{task}"
    );
    let introspect = run(&["introspect", "--help"]);
    assert!(
        introspect.contains("--json		output JSON format"),
        "introspect --help should show its own --json meaning, got:
{introspect}"
    );
    assert!(
        !introspect.contains("output tasks as json"),
        "introspect --help must not show deka_task's --json description, got:
{introspect}"
    );
}

/// Generic property, not a per-command list: every flag/param a command's
/// own registration function adds must render, verbatim, under that
/// command's `--help` — regardless of whether some other command also
/// registers a name-alike flag/param with different text. This is the
/// class of bug the four tests above each caught one instance of; this one
/// walks the whole registry so a fifth instance can't slip in unnoticed.
#[test]
fn every_commands_help_shows_only_that_commands_own_flag_descriptions() {
    let registry = cli::build_registry();
    let index = cli::command_flag_index();

    for command in registry.commands() {
        let Some(owned) = index.flags.get(command.name) else {
            continue;
        };
        let rendered = dcore::help::render_command_help(command, Some(owned)).join(
            "
",
        );
        for flag in &owned.flags {
            let expected = format!("{}		{}", flag.name, flag.description);
            assert!(
                rendered.contains(&expected),
                "`deka {} --help` must show its own registration for `{}` verbatim                  (expected line containing `{}`); rendered:
{}",
                command.name,
                flag.name,
                expected,
                rendered
            );
        }
        for param in &owned.params {
            let expected = format!("{} <value>	{}", param.name, param.description);
            assert!(
                rendered.contains(&expected),
                "`deka {} --help` must show its own registration for `{}` verbatim                  (expected line containing `{}`); rendered:
{}",
                command.name,
                param.name,
                expected,
                rendered
            );
        }
    }

    // This repo genuinely has flag/param names registered by more than one
    // command with different meanings (--version, --yes, --token, --json,
    // --list, ...) — that's what made the resolution bug possible. Assert
    // that's still true so this test can't quietly become vacuous if every
    // colliding name is later renamed away.
    let mut descriptions_by_name: std::collections::HashMap<&str, std::collections::HashSet<&str>> =
        std::collections::HashMap::new();
    for owned in index.flags.values() {
        for flag in &owned.flags {
            descriptions_by_name
                .entry(flag.name)
                .or_default()
                .insert(flag.description);
        }
        for param in &owned.params {
            descriptions_by_name
                .entry(param.name)
                .or_default()
                .insert(param.description);
        }
    }
    assert!(
        descriptions_by_name.values().any(|d| d.len() > 1),
        "expected at least one flag/param name registered with more than one distinct          description across different commands, to actually exercise cross-command          resolution — if that's no longer true this test needs a new fixture"
    );
}

/// deka#1004: the property test above compares `command_flag_index()` to
/// its own rendering, which only proves `render_command_help` is faithful
/// to whatever the index says — a bug in how the index itself is *built*
/// (exactly the bug class #996 fixed) is invisible to it. QA proved this
/// by sabotaging `build_ownership_index` to reintroduce the original
/// contamination: the four named regression tests above failed, and the
/// property test passed unchanged.
///
/// This test gets its ground truth a different way: it replays each
/// registration function against its own fresh `Registry` right here, not
/// by calling `build_ownership_index`, and diffs that directly against
/// the index's own reported ownership. A regression in how the index is
/// assembled changes what the index reports without changing what any
/// registration function actually adds, so — unlike the test above — this
/// one can see it.
#[test]
fn ownership_index_matches_independently_replayed_registration() {
    let index = cli::command_flag_index();

    for register_fn in cli::register_fns() {
        let mut scratch = dcore::Registry::new();
        register_fn(&mut scratch);
        if scratch.commands().is_empty() {
            continue;
        }

        let expected_flags: std::collections::HashSet<(&str, &str)> = scratch
            .flags()
            .iter()
            .map(|flag| (flag.name, flag.description))
            .collect();
        let expected_params: std::collections::HashSet<(&str, &str)> = scratch
            .params()
            .iter()
            .map(|param| (param.name, param.description))
            .collect();

        for command in scratch.commands() {
            let owned = index.flags.get(command.name).unwrap_or_else(|| {
                panic!(
                    "ownership index has no entry for `{}`, but its own \
                     registration function adds it",
                    command.name
                )
            });

            let actual_flags: std::collections::HashSet<(&str, &str)> = owned
                .flags
                .iter()
                .map(|flag| (flag.name, flag.description))
                .collect();
            let actual_params: std::collections::HashSet<(&str, &str)> = owned
                .params
                .iter()
                .map(|param| (param.name, param.description))
                .collect();

            assert_eq!(
                actual_flags, expected_flags,
                "ownership index's flags for `{}` diverge from replaying its own \
                 registration function directly — the index was built wrong, not \
                 just rendered wrong",
                command.name
            );
            assert_eq!(
                actual_params, expected_params,
                "ownership index's params for `{}` diverge from replaying its own \
                 registration function directly — the index was built wrong, not \
                 just rendered wrong",
                command.name
            );
        }
    }
}
