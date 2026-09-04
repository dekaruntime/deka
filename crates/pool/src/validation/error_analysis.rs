//! Smart Error Analysis
//!
//! Analyzes runtime errors and provides helpful suggestions for common issues.

/// Analyzes runtime errors and provides helpful suggestions
pub fn analyze_runtime_error(error_msg: &str, source_code: &str) -> String {
    if error_msg.contains("❌") {
        return error_msg.to_string();
    }
    // Pattern: "X is not defined"
    if error_msg.contains("is not defined") {
        if let Some(var_name) = extract_undefined_variable(error_msg) {
            return suggest_import_for_variable(&var_name, source_code);
        }
    }

    // Pattern: "Cannot read property 'X' of undefined"
    if error_msg.contains("Cannot read property")
        || error_msg.contains("Cannot read properties of undefined")
    {
        return format!(
            "{}\n\nHint: Check that you've initialized all variables before use.\n\
            Common causes:\n\
            - Missing await on async functions\n\
            - Accessing properties before object is created\n\
            - Using undefined function parameters",
            error_msg
        );
    }

    // Pattern: Module not found
    if error_msg.contains("unknown deka module") {
        return format!(
            "{}\n\nHint: Available deka modules:\n\
            - deka/router - HTTP routing and middleware\n\
            - deka/postgres - PostgreSQL database access\n\
            - deka/docker - Docker container management\n\
            - deka/t4 - File storage (S3-compatible)\n\n\
            Example:\n  import {{ Router }} from 'deka/router'",
            error_msg
        );
    }

    // Default: return original error
    error_msg.to_string()
}

/// Extract variable name from "X is not defined" error
fn extract_undefined_variable(error_msg: &str) -> Option<String> {
    // Pattern: "ReferenceError: Router is not defined"
    if let Some(start) = error_msg.find("ReferenceError: ") {
        let after = &error_msg[start + "ReferenceError: ".len()..];
        if let Some(end) = after.find(" is not defined") {
            return Some(after[..end].to_string());
        }
    }
    None
}

/// Suggest import for commonly used variables
fn suggest_import_for_variable(var_name: &str, source_code: &str) -> String {
    let known_imports = [
        (
            "Router",
            "deka/router",
            "import { Router } from 'deka/router'\nconst app = new Router()\napp.get('/', (c) => c.json({ ok: true }))",
        ),
        (
            "Context",
            "deka/router",
            "import { Context } from 'deka/router'\n// Context is passed to route handlers automatically",
        ),
        (
            "cors",
            "deka/router",
            "import { Router, cors } from 'deka/router'\nconst app = new Router()\napp.use(cors())",
        ),
        (
            "logger",
            "deka/router",
            "import { Router, logger } from 'deka/router'\nconst app = new Router()\napp.use(logger())",
        ),
        (
            "query",
            "deka/postgres",
            "import { query } from 'deka/postgres'\nconst rows = await query('SELECT * FROM users')",
        ),
        (
            "execute",
            "deka/postgres",
            "import { execute } from 'deka/postgres'\nawait execute('INSERT INTO users (name) VALUES ($1)', ['Alice'])",
        ),
        (
            "t4",
            "deka/t4",
            "import { t4 } from 'deka/t4'\nconst file = t4.file('data.json')\nconst data = await file.json()",
        ),
        (
            "T4Client",
            "deka/t4",
            "import { T4Client } from 'deka/t4'\nconst client = new T4Client({ bucket: 'my-bucket' })",
        ),
        (
            "createContainer",
            "deka/docker",
            "import { createContainer } from 'deka/docker'\nawait createContainer({ image: 'nginx' })",
        ),
    ];

    for (export_name, module_name, example) in &known_imports {
        if var_name == *export_name {
            // Check if import already exists in source
            let import_pattern = format!("from '{}'", module_name);
            if source_code.contains(&import_pattern) {
                return format!(
                    "ReferenceError: {} is not defined\n\n\
                    Hint: You imported from '{}' but didn't destructure '{}'.\n\n\
                    Update your import:\n  import {{ {} }} from '{}'\n\n\
                    Example:\n  {}",
                    var_name, module_name, export_name, export_name, module_name, example
                );
            } else {
                return format!(
                    "ReferenceError: {} is not defined\n\n\
                    Hint: Missing import.\n\n\
                    Add this import:\n  import {{ {} }} from '{}'\n\n\
                    Example:\n  {}",
                    var_name, export_name, module_name, example
                );
            }
        }
    }

    // Unknown variable
    format!(
        "ReferenceError: {} is not defined\n\n\
        Hint: Make sure you've:\n\
        - Imported all required deka modules\n\
        - Declared the variable before use\n\
        - Spelled the variable name correctly\n\
        \n\
        Available deka modules:\n\
        - deka/router - HTTP routing\n\
        - deka/postgres - Database access\n\
        - deka/docker - Container management\n\
        - deka/t4 - File storage",
        var_name
    )
}
