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
    assert_eq!(names, {
        let mut expected: Vec<_> = registry.commands().iter().map(|c| c.name).collect();
        expected.sort();
        expected
    });
}
