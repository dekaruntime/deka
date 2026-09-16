//! Version-skew detection between the running `deka` binary and the deka a
//! project installed via npm (deka#1101).
//!
//! A stale globally-installed deka can shadow the project's npm-installed one
//! (`./node_modules/.bin/deka`) and then fail on a scaffold the newer version
//! generated — while naming only the user's source file. This module compares
//! `node_modules/@dekaruntime/deka/package.json`'s version against the running
//! binary's own version and says so plainly before the command does work.

use std::fs;
use std::path::Path;

/// npm package directory that carries the project-local deka version.
const PACKAGE_JSON: &str = "node_modules/@dekaruntime/deka/package.json";

/// Declared `version` of the nearest `node_modules/@dekaruntime/deka` at or
/// above `start`, walking parent directories the way Node resolution does.
pub fn project_deka_version(start: &Path) -> Option<String> {
    for dir in start.ancestors() {
        let manifest = dir.join(PACKAGE_JSON);
        if let Ok(text) = fs::read_to_string(&manifest) {
            if let Some(version) = package_version(&text) {
                return Some(version);
            }
        }
    }
    None
}

/// Extract the `"version"` field from package.json text without a JSON
/// dependency: scan for the key, then read its quoted string value.
fn package_version(json: &str) -> Option<String> {
    let bytes = json.as_bytes();
    let mut cursor = 0;
    while let Some(found) = json[cursor..].find("\"version\"") {
        let mut i = cursor + found + "\"version\"".len();
        while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b':') {
            cursor += found + 1;
            continue;
        }
        i += 1;
        while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'"') {
            cursor += found + 1;
            continue;
        }
        let mut end = i + 1;
        while end < bytes.len() && bytes[end] != b'"' {
            if bytes[end] == b'\\' {
                end += 1;
            }
            end += 1;
        }
        let value = json.get(i + 1..end)?.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
        cursor += found + 1;
    }
    None
}

/// Path of the running deka binary, for "you are running deka X from Y".
pub fn running_binary_path() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string())
}

/// Emit one `[warn]` line when the project pins a different deka than the
/// binary running now. Silent when there is no npm-installed deka above the
/// cwd or when the versions agree.
pub fn warn_on_version_skew() {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let Some(expected) = project_deka_version(&cwd) else {
        return;
    };
    let running = env!("CARGO_PKG_VERSION");
    if expected == running {
        return;
    }
    stdio::warn(
        "version",
        &format!(
            "this project expects deka {expected}, you are running deka {running} from {}; \
             the project's own ./node_modules/.bin/deka may serve it better",
            running_binary_path()
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_package(root: &Path, version: &str) {
        let dir = root.join("node_modules/@dekaruntime/deka");
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = std::fs::File::create(dir.join("package.json")).unwrap();
        write!(
            file,
            r#"{{"name":"@dekaruntime/deka","version":"{version}","bin":{{"deka":"bin.js"}}}}"#
        )
        .unwrap();
    }

    #[test]
    fn package_version_reads_quoted_field() {
        assert_eq!(
            package_version(r#"{"name":"x","version":"0.53.4"}"#),
            Some("0.53.4".to_string())
        );
        assert_eq!(package_version(r#"{"name":"x"}"#), None);
        assert_eq!(package_version(""), None);
    }

    #[test]
    fn project_version_walks_up_to_node_modules() {
        let temp = std::env::temp_dir().join(format!("skew-walk-{}", std::process::id()));
        let nested = temp.join("app/src/pages");
        std::fs::create_dir_all(&nested).unwrap();
        write_package(&temp, "0.53.4");
        assert_eq!(
            project_deka_version(&nested).as_deref(),
            Some("0.53.4")
        );
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn project_version_absent_without_install() {
        let temp = std::env::temp_dir().join(format!("skew-none-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        assert_eq!(project_deka_version(&temp), None);
        let _ = std::fs::remove_dir_all(&temp);
    }
}
