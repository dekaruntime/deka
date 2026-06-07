pub fn is_bare_module_specifier(spec: &str) -> bool {
    !spec.is_empty()
        && !spec.starts_with("./")
        && !spec.starts_with("../")
        && !spec.starts_with('/')
        && !spec.starts_with("http://")
        && !spec.starts_with("https://")
        && !spec.starts_with("file://")
}

pub fn module_spec_aliases(spec: &str) -> Vec<String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(2);
    out.push(trimmed.to_string());

    if let Some(rest) = trimmed.strip_prefix("@deka/") {
        if !rest.is_empty() {
            out.push(rest.to_string());
        }
        return out;
    }

    if is_bare_module_specifier(trimmed) && !trimmed.starts_with('@') {
        out.push(format!("@deka/{}", trimmed));
    }

    out
}

pub fn canonical_php_package_spec(spec: &str) -> Option<String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('@') {
        return Some(trimmed.to_string());
    }
    // Package specs are @scope/name. For convenience, only simple unscoped
    // tokens map to @deka/<name>; nested import paths (e.g. component/router)
    // are import-time aliases, not package names.
    if is_bare_module_specifier(trimmed) && !trimmed.contains('/') {
        return Some(format!("@deka/{}", trimmed));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{canonical_php_package_spec, module_spec_aliases};

    #[test]
    fn bare_spec_includes_deka_alias() {
        assert_eq!(module_spec_aliases("json"), vec!["json", "@deka/json"]);
    }

    #[test]
    fn deka_scope_includes_bare_alias() {
        assert_eq!(
            module_spec_aliases("@deka/json"),
            vec!["@deka/json", "json"]
        );
    }

    #[test]
    fn scoped_non_deka_has_no_alias() {
        assert_eq!(module_spec_aliases("@sami/tool"), vec!["@sami/tool"]);
    }

    #[test]
    fn canonicalizes_bare_packages_to_deka_scope() {
        assert_eq!(
            canonical_php_package_spec("json"),
            Some("@deka/json".to_string())
        );
    }

    #[test]
    fn does_not_map_nested_import_paths_to_package_specs() {
        assert_eq!(canonical_php_package_spec("component/router"), None);
    }
}
