//! One stderr renderer for native check findings and progress (APS 55).
use deka_cli_core::registry::Output;
use std::path::Path;

pub(super) fn success(output: &Output, source: &Path) {
    output.print(format_args!("[check] {} - ok\n", source.display()));
}

fn position(line: &str) -> Option<(u32, u32, &str)> {
    let mut fields = line.splitn(3, ':');
    let row = fields.next()?.trim().parse().ok()?;
    let column = fields.next()?.trim().parse().ok()?;
    let message = fields.next()?.trim_start();
    (row > 0 && column > 0).then_some((row, column, message))
}

fn located(line: &str) -> Option<(&Path, u32, u32, &str)> {
    line.match_indices(": ").find_map(|(split, _)| {
        let path = Path::new(&line[..split]);
        let (row, column, message) = position(&line[split + 2..])?;
        path.is_absolute().then_some((path, row, column, message))
    })
}

pub(super) fn failure(output: &Output, source: Option<&Path>, error: &str) {
    let mut active = source.map(|path| path.canonicalize().unwrap_or_else(|_| path.to_owned()));
    if let Some(source) = source {
        output.print(format_args!("[check] {}\n", source.display()));
    }
    for line in error.lines() {
        if let Some((path, row, column, message)) = located(line) {
            if active.as_deref() != Some(path) {
                output.print(format_args!("[check] {}\n", path.display()));
                active = Some(path.to_owned());
            }
            output.print(format_args!("[check] {row}:{column}: {message}\n"));
        } else if let Some((row, column, message)) = position(line) {
            output.print(format_args!("[check] {row}:{column}: {message}\n"));
        } else {
            output.print(format_args!("[check] {line}\n"));
        }
    }
}
