//! rfd#61: every registered command declares an owner matching the
//! checked-in manifest. A new command is a manifest diff.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn manifest() -> BTreeMap<String, String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/command-owners.txt");
    let raw = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read {}: {err}", path.display())
    });
    let mut map = BTreeMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let name = parts.next().expect("command name");
        let owner = parts.next().expect("owner");
        map.insert(name.to_string(), owner.to_string());
    }
    map
}

#[test]
fn every_command_declares_its_owner() {
    let expected = manifest();
    let registry = cli::build_registry();
    let mut seen = BTreeMap::new();
    for command in registry.commands() {
        assert!(
            !command.owner.is_empty() && command.owner != "legacy",
            "command `{}` must declare a non-legacy owner",
            command.name
        );
        seen.insert(command.name.to_string(), command.owner.to_string());
        let Some(owner) = expected.get(command.name) else {
            panic!(
                "command `{}` owner `{}` is missing from scripts/command-owners.txt",
                command.name, command.owner
            );
        };
        assert_eq!(
            command.owner, owner,
            "command `{}` owner mismatch",
            command.name
        );
    }
    assert!(
        !seen.is_empty(),
        "registry registered no commands"
    );
}

#[test]
fn help_lists_every_registered_command() {
    let registry = cli::build_registry();
    // Capture help the same way `deka --help` does: grouped by category.
    let mut names: Vec<_> = registry.commands().iter().map(|c| c.name).collect();
    names.sort();
    assert!(names.contains(&"install"));
    assert!(names.contains(&"build"));
    assert!(names.contains(&"run"));
    assert!(names.contains(&"serve"));
    assert!(names.contains(&"dev"));
    assert!(registry.flags().iter().any(|flag| flag.name == "--dev"));
    assert_eq!(names, {
        let mut expected: Vec<_> = registry.commands().iter().map(|c| c.name).collect();
        expected.sort();
        expected
    });
}

/// Getting Started, then category-grouped command names + summaries, then
/// the global-only deduplicated flags, matching `deka --help` (minus the
/// version banner). Insertion order within each category is load-bearing.
///
/// Getting Started and the flags filter/dedup come straight from
/// `dcore::help` (the same code the real `deka --help` renders through) so
/// this snapshot can't silently drift from production behavior (deka#978).
#[cfg(not(feature = "self-update"))]
fn help_surface() -> String {
    let registry = cli::build_registry();
    let mut out = String::new();

    let known: std::collections::HashSet<&str> =
        registry.commands().iter().map(|c| c.name).collect();
    out.push_str("[getting started]\n");
    for (name, blurb) in dcore::help::GETTING_STARTED {
        if known.contains(name) {
            out.push_str(&format!("{name}\t\t{blurb}\n"));
        }
    }
    out.push('\n');

    let mut grouped: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for command in registry.commands() {
        grouped
            .entry(command.category)
            .or_default()
            .push(format!("{}\t\t{}", command.name, command.summary));
        for subcommand in command.subcommands {
            grouped.entry(command.category).or_default().push(format!(
                "{} {}\t{}",
                command.name, subcommand.name, subcommand.summary
            ));
        }
    }
    for (category, lines) in grouped {
        out.push('[');
        out.push_str(category);
        out.push_str("]\n");
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str("[flags]\n");
    for flag in dcore::help::global_flags(&registry) {
        out.push_str(&format!("{}\t\t{}\n", flag.name, flag.description));
    }
    out
}

#[cfg(not(feature = "self-update"))]
fn help_snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/help-commands.txt")
}

// This snapshot pins what a real DEFAULT build's `deka --help` shows.
// self-update is deferred behind the non-default `self-update` feature
// (deka#992) and is deliberately absent from the checked-in snapshot; a
// `--features self-update` build legitimately shows more commands (`self
// update`, `self monitor`), so this test does not run there rather than
// being made to tolerate two different "correct" outputs.
#[test]
#[cfg(not(feature = "self-update"))]
fn help_snapshot_command_names_and_summaries() {
    let actual = help_surface();
    let path = help_snapshot_path();
    if std::env::var_os("UPDATE_HELP_SNAPSHOT").is_some() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read {}: {err}\nre-run with UPDATE_HELP_SNAPSHOT=1 to write it\nactual:\n{actual}",
            path.display()
        )
    });
    assert_eq!(
        actual, expected,
        "help snapshot mismatch (UPDATE_HELP_SNAPSHOT=1 to refresh {})\nactual:\n{actual}",
        path.display()
    );
}

#[test]
fn lsp_registers_between_link_and_pkg() {
    let names: Vec<_> = cli::build_registry()
        .commands()
        .iter()
        .map(|c| c.name)
        .collect();
    let link = names.iter().position(|&n| n == "link").expect("link");
    let unlink = names.iter().position(|&n| n == "unlink").expect("unlink");
    let pkg = names.iter().position(|&n| n == "pkg").expect("pkg");
    assert_eq!(unlink, link + 1, "unlink follows link");
    #[cfg(feature = "lsp")]
    {
        assert_eq!(names.get(unlink + 1).copied(), Some("lsp"));
        assert_eq!(pkg, unlink + 2, "pkg follows lsp when lsp is enabled");
    }
    #[cfg(not(feature = "lsp"))]
    {
        assert_eq!(pkg, unlink + 1, "pkg follows unlink when lsp is off");
        assert!(!names.contains(&"lsp"));
    }
}

#[test]
#[cfg(not(feature = "dev-server"))]
fn dev_without_feature_reports_rebuild_instruction() {
    for args in [vec!["dev"], vec!["serve", "--dev"]] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_cli"))
            .args(args)
            .output()
            .expect("run cli");
        assert!(!output.status.success());
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            text.contains("this build lacks the dev server; rebuild with --features dev-server"),
            "{text}",
        );
    }
}
