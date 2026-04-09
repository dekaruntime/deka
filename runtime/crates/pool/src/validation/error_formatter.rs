//! Beautiful error formatting for validation errors using deka-validation.

/// Format a validation error with the shared Deka formatting.
pub fn format_validation_error(
    source_code: &str,
    file_path: &str,
    error_type: &str,
    line_num: usize,
    col_num: usize,
    message: &str,
    hint: &str,
    underline_length: usize,
) -> String {
    deka_validation::format_validation_error(
        source_code,
        file_path,
        error_type,
        line_num,
        col_num,
        message,
        hint,
        underline_length,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_formatting() {
        let source = r#"import { Router } from 'deka/router'

import fs from 'fs'

const app = new Router()"#;

        let error = format_validation_error(
            source,
            "handler.js",
            "Invalid Import",
            3,
            8,
            "Node.js modules are not available in deka-runtime",
            "This is not Node.js! Use deka/* modules instead.",
            2,
        );

        assert!(error.contains("Validation Error"));
        assert!(error.contains("Invalid Import"));
        assert!(error.contains("handler.js:3:8"));
        assert!(error.contains("import fs from 'fs'"));
        assert!(error.contains("^"));
    }

    #[test]
    fn test_error_at_start_of_file() {
        let source = "import fs from 'fs'";

        let error = format_validation_error(
            source,
            "handler.js",
            "Invalid Import",
            1,
            8,
            "Node.js modules are not available",
            "Use deka/* modules",
            2,
        );

        assert!(error.contains("1 │ import fs from 'fs'"));
    }

    #[test]
    fn test_error_at_end_of_file() {
        let source = "line 1\nline 2\nline 3\nimport fs from 'fs'";

        let error = format_validation_error(
            source,
            "handler.js",
            "Invalid Import",
            4,
            8,
            "Node.js modules are not available",
            "Use deka/* modules",
            2,
        );

        assert!(error.contains("Validation Error"));
        assert!(error.contains("4 │ import fs from 'fs'"));
    }
}
