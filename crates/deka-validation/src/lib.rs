//! Deka Validation Library
//!
//! Shared validation and error formatting logic for Deka runtimes.
//! Supports both native Rust and WebAssembly compilation.

#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::*;

/// Format a validation error with beautiful Rust/Gleam-style output
///
/// This function creates consistent error messages across all Deka systems:
/// - deka-runtime (native Rust)
/// - deka-edge (native Rust)
/// - playground (WASM in browser)
/// - CLI tools (WASM in Bun/Node)
///
/// # Arguments
///
/// * `code` - The source code containing the error
/// * `file_path` - Path to the file (e.g., "handler.ts")
/// * `error_kind` - Category of error (e.g., "Invalid Import", "Type Error")
/// * `line_num` - Line number (1-indexed)
/// * `col_num` - Column number (1-indexed)
/// * `message` - Error message
/// * `help` - Help text explaining how to fix
/// * `underline_length` - Number of characters to underline (for ^^^)
///
/// # Example
///
/// ```rust
/// use deka_validation::format_validation_error;
///
/// let error = format_validation_error(
///     "import { serve } from 'deka/invalid';",
///     "handler.ts",
///     "Invalid Import",
///     1,
///     26,
///     "Module 'deka/invalid' not found",
///     "Available modules: deka, deka/router, deka/sqlite",
///     12
/// );
///
/// // Produces:
/// // Validation Error
/// // ❌ Invalid Import
/// //
/// // ┌─ handler.ts:1:26
/// // │
/// //   1 │ import { serve } from 'deka/invalid';
/// //     │                          ^^^^^^^^^^^^ Module 'deka/invalid' not found
/// // │
/// // = help: Available modules: deka, deka/router, deka/sqlite
/// // │
/// // └─
/// ```
// Pre-existing WASM API; refactoring is out of scope for #231.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(feature = "wasm", wasm_bindgen)]
pub fn format_validation_error(
    code: &str,
    file_path: &str,
    error_kind: &str,
    line_num: usize,
    col_num: usize,
    message: &str,
    help: &str,
    underline_length: usize,
) -> String {
    format_error_impl(
        code,
        file_path,
        error_kind,
        line_num,
        col_num,
        message,
        help,
        underline_length,
        None,
    )
}

// Pre-existing WASM API; refactoring is out of scope for #231.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(feature = "wasm", wasm_bindgen)]
pub fn format_validation_error_extended(
    code: &str,
    file_path: &str,
    error_kind: &str,
    line_num: usize,
    col_num: usize,
    message: &str,
    help: &str,
    underline_length: usize,
    severity: &str,
    docs_link: Option<String>,
) -> String {
    format_error_impl(
        code,
        file_path,
        error_kind,
        line_num,
        col_num,
        message,
        help,
        underline_length,
        Some(ExtraFormatInfo {
            severity: severity.to_string(),
            docs_link,
            suggestion: None,
        }),
    )
}

// Pre-existing WASM API; refactoring is out of scope for #231.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(feature = "wasm", wasm_bindgen)]
pub fn format_validation_error_with_suggestion(
    code: &str,
    file_path: &str,
    error_kind: &str,
    line_num: usize,
    col_num: usize,
    message: &str,
    help: &str,
    underline_length: usize,
    severity: &str,
    docs_link: Option<String>,
    suggestion: Option<String>,
) -> String {
    format_error_impl(
        code,
        file_path,
        error_kind,
        line_num,
        col_num,
        message,
        help,
        underline_length,
        Some(ExtraFormatInfo {
            severity: severity.to_string(),
            docs_link,
            suggestion,
        }),
    )
}

#[derive(Debug, Clone)]
struct ExtraFormatInfo {
    severity: String,
    docs_link: Option<String>,
    suggestion: Option<String>,
}

// Pre-existing formatter signature; refactoring is out of scope for #231.
#[allow(clippy::too_many_arguments)]
fn format_error_impl(
    code: &str,
    file_path: &str,
    error_kind: &str,
    line_num: usize,
    col_num: usize,
    message: &str,
    help: &str,
    underline_length: usize,
    extra: Option<ExtraFormatInfo>,
) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let error_line = if line_num > 0 && line_num <= lines.len() {
        lines[line_num - 1]
    } else {
        ""
    };

    let underline_length = underline_length.max(1);

    let severity = extra
        .as_ref()
        .map(|extra| extra.severity.as_str())
        .unwrap_or("error");
    let (icon, label) = match severity {
        "warning" | "warn" => ("⚠️", "Validation Warning"),
        "info" => ("ℹ️", "Validation Info"),
        _ => ("❌", "Validation Error"),
    };

    let use_color = use_color_output();
    let severity_color = match severity {
        "warning" | "warn" => "\x1b[33m",
        "info" => "\x1b[34m",
        _ => "\x1b[31m",
    };
    let kind_color = color_for_kind(error_kind).unwrap_or(severity_color);
    let icon = colorize(icon, severity_color, use_color);
    let label = colorize(label, severity_color, use_color);
    let kind_label = colorize(error_kind, kind_color, use_color);

    let mut out = format!(
        "\n{}\n\
        {} {}\n\
        \n\
        ┌─ {}:{}:{}\n\
        │\n\
        {:>3} │ {}\n\
            │ {}{} {}\n\
        │\n",
        label,
        icon,
        kind_label,
        file_path,
        line_num,
        col_num,
        line_num,
        error_line,
        " ".repeat(col_num.saturating_sub(1)),
        "^".repeat(underline_length),
        message,
    );

    let help_trimmed = help.trim();
    let suggestion = extra.as_ref().and_then(|extra| {
        extra
            .suggestion
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
    });
    let mut show_help = !help_trimmed.is_empty();
    if let Some(suggestion_value) = &suggestion {
        if suggestion_value == help_trimmed {
            show_help = false;
        }
    }
    if show_help {
        out.push_str(&format!("= help: {}\n", help));
    }
    if let Some(suggestion_value) = suggestion {
        let suggestion_label = colorize("suggestion", "\x1b[36m", use_color);
        out.push_str(&format!("= {}: {}\n", suggestion_label, suggestion_value));
    }
    if let Some(link) = extra.as_ref().and_then(|extra| extra.docs_link.clone()) {
        let docs_label = colorize("docs", "\x1b[36m", use_color);
        out.push_str(&format!("= {}: {}\n", docs_label, link));
    }
    out.push_str("│\n└─\n");
    out
}

fn use_color_output() -> bool {
    if cfg!(test) {
        return false;
    }
    if std::env::var("NO_COLOR").is_ok() || std::env::var("DEKA_NO_COLOR").is_ok() {
        return false;
    }
    if let Ok(term) = std::env::var("TERM") {
        if term == "dumb" {
            return false;
        }
    }
    true
}

fn colorize(text: &str, color: &str, enabled: bool) -> String {
    if !enabled {
        return text.to_string();
    }
    format!("{color}{text}\x1b[0m")
}

fn color_for_kind(kind: &str) -> Option<&'static str> {
    let lower = kind.to_ascii_lowercase();
    if lower.contains("syntax") || lower.contains("token") {
        return Some("\x1b[31m");
    }
    if lower.contains("type") {
        return Some("\x1b[35m");
    }
    if lower.contains("import") || lower.contains("export") || lower.contains("module") {
        return Some("\x1b[36m");
    }
    if lower.contains("wasm") {
        return Some("\x1b[33m");
    }
    if lower.contains("jsx") {
        return Some("\x1b[32m");
    }
    if lower.contains("struct") || lower.contains("enum") || lower.contains("pattern") {
        return Some("\x1b[34m");
    }
    if lower.contains("null") || lower.contains("exception") || lower.contains("namespace") {
        return Some("\x1b[33m");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_error_formatting() {
        let code = "import { serve } from 'deka/invalid';";
        let error = format_validation_error(
            code,
            "test.ts",
            "Invalid Import",
            1,
            26,
            "Module 'deka/invalid' not found",
            "Available modules: deka, deka/router",
            12,
        );

        assert!(error.contains("❌ Invalid Import"));
        assert!(error.contains("test.ts:1:26"));
        assert!(error.contains("deka/invalid"));
        assert!(error.contains("^^^^^^^^^^^^"));
        assert!(error.contains("= help: Available modules"));
    }

    #[test]
    fn test_multiline_code() {
        let code = "line 1\nline 2 with error\nline 3";
        let error = format_validation_error(
            code,
            "multi.ts",
            "Type Error",
            2,
            7,
            "Something wrong here",
            "Fix it like this",
            4,
        );

        assert!(error.contains("❌ Type Error"));
        assert!(error.contains("multi.ts:2:7"));
        assert!(error.contains("line 2 with error"));
        assert!(error.contains("^^^^"));
    }

    #[test]
    fn test_underline_length_minimum() {
        let error = format_validation_error("test", "test.ts", "Error", 1, 1, "msg", "help", 0);

        assert!(error.contains("^"));
    }

    #[test]
    fn test_out_of_bounds_line() {
        let error = format_validation_error(
            "only one line",
            "test.ts",
            "Error",
            999,
            1,
            "msg",
            "help",
            5,
        );

        assert!(error.contains("❌ Error"));
        assert!(error.contains("999 │"));
    }
}
