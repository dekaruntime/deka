use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameworkEntryKind {
    Page,
    Layout,
    Api,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkEntry {
    pub kind: FrameworkEntryKind,
    pub route: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FrameworkManifest {
    pub root: String,
    pub entries: Vec<FrameworkEntry>,
}

pub fn route_from_relative_path(kind: FrameworkEntryKind, relative_path: &str) -> Option<String> {
    let normalized = relative_path.replace('\\', "/");
    let trimmed = normalized.trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let suffix = match kind {
        FrameworkEntryKind::Page => "/page.phpx",
        FrameworkEntryKind::Layout => "/layout.phpx",
        FrameworkEntryKind::Api => ".phpx",
    };

    let route_source = match kind {
        FrameworkEntryKind::Page | FrameworkEntryKind::Layout => {
            if trimmed == suffix.trim_start_matches('/') {
                ""
            } else {
                trimmed.strip_suffix(suffix)?
            }
        }
        FrameworkEntryKind::Api => trimmed.strip_suffix(suffix)?,
    };

    if route_source.is_empty() {
        return Some("/".to_string());
    }

    Some(format!("/{}", route_source.trim_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::{FrameworkEntryKind, route_from_relative_path};

    #[test]
    fn derives_root_page_route() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Page, "page.phpx"),
            Some("/".to_string())
        );
    }

    #[test]
    fn derives_nested_page_route() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Page, "users/page.phpx"),
            Some("/users".to_string())
        );
    }

    #[test]
    fn derives_layout_scope_route() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Layout, "dashboard/layout.phpx"),
            Some("/dashboard".to_string())
        );
    }

    #[test]
    fn derives_api_route() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Api, "api/packages.phpx"),
            Some("/api/packages".to_string())
        );
    }

    #[test]
    fn preserves_dynamic_segments() {
        assert_eq!(
            route_from_relative_path(FrameworkEntryKind::Page, "blog/[slug]/page.phpx"),
            Some("/blog/[slug]".to_string())
        );
    }
}
