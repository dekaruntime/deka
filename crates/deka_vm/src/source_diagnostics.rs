//! Shared decoding of native compiler findings for CLI and editor transports.
use std::path::{Path, PathBuf};
#[derive(Clone, Debug)]
pub struct SourceDiagnostic {
    pub path: PathBuf,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
}
pub fn diagnostic_position(line: &str) -> Option<(u32, u32, &str)> {
    let mut fields = line.splitn(3, ':');
    let row = fields.next()?.trim().parse().ok()?;
    let column = fields.next()?.trim().parse().ok()?;
    let message = fields.next()?.trim_start();
    (row > 0 && column > 0).then_some((row, column, message))
}
pub fn diagnostic_location(line: &str) -> Option<(&Path, u32, u32, &str)> {
    line.match_indices(": ").find_map(|(split, _)| {
        let path = Path::new(&line[..split]);
        let (row, column, message) = diagnostic_position(&line[split + 2..])?;
        path.is_absolute().then_some((path, row, column, message))
    })
}
pub fn source_diagnostics(entry: &Path, error: &str) -> Vec<SourceDiagnostic> {
    let mut active = super::source_path(entry);
    error
        .lines()
        .map(|line| {
            let (position, message) =
                if let Some((path, row, column, message)) = diagnostic_location(line) {
                    active = path.to_owned();
                    (Some((row, column)), message)
                } else if let Some((row, column, message)) = diagnostic_position(line) {
                    (Some((row, column)), message)
                } else {
                    (None, line)
                };
            SourceDiagnostic {
                path: active.clone(),
                line: position.map(|p| p.0),
                column: position.map(|p| p.1),
                message: message.to_owned(),
            }
        })
        .collect()
}
