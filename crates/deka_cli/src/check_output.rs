//! One stderr renderer for native check findings and progress (APS 55).
use deka_cli_core::registry::Output;
use std::path::Path;

pub(super) fn success(output: &Output, source: &Path) {
    output.print(format_args!("[check] {} - ok\n", source.display()));
}

pub(super) fn failure(output: &Output, source: Option<&Path>, error: &str) {
    let mut active = source.map(|path| path.canonicalize().unwrap_or_else(|_| path.to_owned()));
    if let Some(source) = source {
        output.print(format_args!("[check] {}\n", source.display()));
    }
    for line in error.lines() {
        if let Some((path, row, column, message)) = deka_vm::compiler::diagnostic_location(line) {
            if active.as_deref() != Some(path) {
                output.print(format_args!("[check] {}\n", path.display()));
                active = Some(path.to_owned());
            }
            output.print(format_args!("[check] {row}:{column}: {message}\n"));
        } else if let Some((row, column, message)) = deka_vm::compiler::diagnostic_position(line) {
            output.print(format_args!("[check] {row}:{column}: {message}\n"));
        } else {
            output.print(format_args!("[check] {line}\n"));
        }
    }
}
