use php_rs::parser::ast::Program;

use super::{ValidationError, parse_errors_to_validation_errors};

const DEFAULT_HELP_TEXT: &str = "Check syntax near the highlighted code.";

pub fn validate_syntax(source: &str, program: &Program, file_path: &str) -> Vec<ValidationError> {
    let mut errors = parse_errors_to_validation_errors(source, program.errors);
    let is_phpx = file_path.ends_with(".phpx");

    for error in &mut errors {
        improve_help_text(error, is_phpx);
    }

    errors
}

fn improve_help_text(error: &mut ValidationError, is_phpx: bool) {
    if error.help_text != DEFAULT_HELP_TEXT {
        return;
    }

    let message = error.message.as_str();
    let mut updated = false;
    if message.contains("Missing semicolon") || message.contains("Expected ';'") {
        if is_phpx {
            error.message =
                "Statements must be separated by a newline or semicolon in PHPX.".to_string();
            error.help_text =
                "Put each statement on its own line or add ';' between statements.".to_string();
        } else {
            error.help_text = "Add a semicolon at the end of the statement.".to_string();
        }
        updated = true;
    } else if message.contains("Expected '}'") {
        error.help_text = "Add a closing '}' for the block.".to_string();
        updated = true;
    } else if message.contains("Expected ')'") {
        error.help_text = "Add a closing ')' for the expression.".to_string();
        updated = true;
    } else if message.contains("Expected ']'") {
        error.help_text = "Add a closing ']' for the array or index access.".to_string();
        updated = true;
    } else if message.contains("Unexpected end of file") || message.contains("end of file") {
        error.help_text = "Check for missing closing braces or parentheses.".to_string();
        updated = true;
    } else if message.contains("Unexpected token") {
        error.help_text = "Remove the unexpected token or fix the surrounding syntax.".to_string();
        updated = true;
    } else if message.contains("Invalid token") {
        error.help_text =
            "Remove the invalid token or replace it with valid PHPX syntax.".to_string();
        updated = true;
    }

    if updated && error.suggestion.is_some() {
        error.suggestion = Some(error.help_text.clone());
    }
}
