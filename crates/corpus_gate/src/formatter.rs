//! Exercise the real formatter before evaluating the archived formatter cases.
use crate::{Case, RunResult, process};
use std::{fs, path::Path};

fn parses(source: &str) -> bool {
    let arena = bumpalo::Bump::new();
    let parsed = deka_syntax::parse(source, &arena);
    parsed.errors.is_empty() && parsed.program.is_some()
}

fn comments(source: &str) -> Vec<String> {
    use deka_syntax::lexer::TokenKind;
    let mut lexer = deka_syntax::Lexer::new(source);
    let mut comments = Vec::new();
    loop {
        let token = lexer.next_token();
        match token.kind {
            TokenKind::Comment => comments.push(token.text.to_owned()),
            TokenKind::Eof => break,
            _ => {}
        }
    }
    comments
}

fn invoke(deka: &Path, entry: &str, project: &Path, slug: &str) -> Result<(), String> {
    let output = process::execute(deka, "fmt", entry, project, slug)?;
    if !output.status.success() {
        return Err(format!(
            "{slug}: formatter failed for {entry}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

pub(super) fn format_project(
    deka: &Path,
    case: &Case,
    entry: &str,
    project: &Path,
) -> Result<(), String> {
    let mut entries = vec![entry.to_owned()];
    entries.extend(
        case.files
            .iter()
            .filter(|(name, _)| name.ends_with(".ds") || name.ends_with(".dsx"))
            .map(|(name, _)| format!("./{name}")),
    );
    for entry in entries {
        let path = project.join(&entry);
        let before = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        invoke(deka, &entry, project, &case.slug)?;
        let once = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        if parses(&before) {
            if !parses(&once) {
                return Err(format!("{}: formatted {entry} no longer parses", case.slug));
            }
            if comments(&before) != comments(&once) {
                return Err(format!(
                    "{}: formatting changed comments in {entry}",
                    case.slug
                ));
            }
        } else if once != before {
            return Err(format!(
                "{}: formatting changed invalid source {entry}",
                case.slug
            ));
        }
        invoke(deka, &entry, project, &case.slug)?;
        let twice = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        if once != twice {
            return Err(format!(
                "{}: formatter second pass changed {entry}",
                case.slug
            ));
        }
    }
    Ok(())
}

pub(super) fn preserves_execution(
    slug: &str,
    before: &RunResult,
    after: &RunResult,
) -> Result<(), String> {
    if before.ok != after.ok
        || before.transpile_failed != after.transpile_failed
        || before.stdout != after.stdout
        || before.diagnostics != after.diagnostics
    {
        return Err(format!(
            "{slug}: formatting changed observed check/run behavior: before {before:?}, after {after:?}"
        ));
    }
    Ok(())
}
