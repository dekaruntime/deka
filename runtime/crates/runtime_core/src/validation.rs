pub fn validate_phpx_handler_with<E, ReadSource, Validate, FormatError>(
    handler_path: &str,
    read_source: &ReadSource,
    validate: &Validate,
    format_error: &FormatError,
) -> Result<(), String>
where
    ReadSource: Fn(&str) -> Result<String, String>,
    Validate: Fn(&str, &str) -> Vec<E>,
    FormatError: Fn(&str, &str, &E) -> String,
{
    let is_deka_source = matches!(
        handler_path.rsplit_once('.').map(|(_, extension)| extension),
        Some(extension) if extension.eq_ignore_ascii_case("ds")
    );
    if !is_deka_source {
        return Ok(());
    }

    let source = read_source(handler_path)?;
    let errors = validate(&source, handler_path);
    if errors.is_empty() {
        return Ok(());
    }

    let mut out = String::new();
    for error in errors.iter().take(3) {
        out.push_str(&format_error(&source, handler_path, error));
    }
    if errors.len() > 3 {
        out.push_str(&format!(
            "\n... plus {} additional module validation error(s)\n",
            errors.len() - 3
        ));
    }

    Err(format!(
        "DekaScript module graph validation failed for {}:\n{}",
        handler_path, out
    ))
}

#[cfg(test)]
mod tests {
    use super::validate_phpx_handler_with;

    #[test]
    fn skips_non_dekascript_paths() {
        let result = validate_phpx_handler_with(
            "index.php",
            &|_| Ok(String::new()),
            &|_, _| vec![1],
            &|_, _, _| "err".to_string(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn returns_error_with_limited_output() {
        let result = validate_phpx_handler_with(
            "index.ds",
            &|_| Ok("source".to_string()),
            &|_, _| vec![1, 2, 3, 4],
            &|_, _, e| format!("error:{}\n", e),
        );
        let err = result.expect_err("expected validation error");
        assert!(err.contains("DekaScript module graph validation failed"));
        assert!(err.contains("error:1"));
        assert!(err.contains("error:3"));
        assert!(err.contains("plus 1 additional"));
    }

    #[test]
    fn validates_dekascript_paths() {
        let result = validate_phpx_handler_with(
            "index.ds",
            &|_| Ok("source".to_string()),
            &|_, _| vec![1],
            &|_, _, _| "err".to_string(),
        );
        assert!(result.is_err());
    }
}
