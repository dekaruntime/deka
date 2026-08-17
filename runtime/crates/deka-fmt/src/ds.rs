//! DekaScript source formatter.
//!
//! v1 is intentionally conservative: it validates the source and normalizes
//! whitespace (trailing blanks, EOF newline), but preserves the author's
//! layout. AST-aware pretty-printing will replace this once the surface is
//! stable enough to commit to canonical formatting rules (see RFD 23).

/// Format a DekaScript source string.
///
/// v1 normalizes trailing whitespace and ensures a single trailing newline.
/// Invalid source is returned unchanged so the formatter is safe to run on
/// code that is still being typed.
pub fn format_ds(source: &str) -> Result<String, String> {
    let mut out = source
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_trailing_whitespace_and_eof() {
        let input = "function add(): int {\n  return 1;  \n}\n\n";
        let output = format_ds(input).unwrap();
        assert_eq!(output, "function add(): int {\n  return 1;\n}\n");
    }

    #[test]
    fn adds_trailing_newline_when_missing() {
        let input = "const x = 1;";
        let output = format_ds(input).unwrap();
        assert_eq!(output, "const x = 1;\n");
    }

    #[test]
    fn empty_source_stays_empty() {
        assert_eq!(format_ds("").unwrap(), "");
    }
}
