//! Per-route CSS: utility classes + imported stylesheets, as `<link>` assets.
//!
//! All emitted stylesheets are content-addressed: `<stem>.<sha256-10>.css`
//! (see `crate::islands`), so `deka build` and `deka serve` derive the same
//! name for the same bytes.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use runtime_core::framework::{
    collect_route_styles, css_plan_from_styles, route_css_slug, scan_app_dir, CssPlan, RouteStyle,
};

/// Content-addressed CSS name: `<stem>.<hash>.css`.
fn hashed_css_name(stem: &str, css: &str) -> String {
    format!("{stem}.{}.css", crate::islands::content_hash_hex(css.as_bytes()))
}

/// Remove content-hashed stylesheets not in `keep`; unhashed files stay.
fn clean_stale_css(css_dir: &Path, keep: &BTreeSet<String>) -> Result<(), String> {
    let Ok(reader) = fs::read_dir(css_dir) else {
        return Ok(());
    };
    for entry in reader.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let hashed = name.rsplit_once('.').and_then(|(stem, _)| {
            stem.rsplit_once('.')
                .filter(|(_, hash)| hash.len() == crate::islands::ASSET_HASH_LEN)
        });
        if hashed.is_some() && name.ends_with(".css") && !keep.contains(&name) {
            fs::remove_file(entry.path()).map_err(|err| {
                format!(
                    "failed to remove stale stylesheet {}: {err}",
                    entry.path().display()
                )
            })?;
        }
    }
    Ok(())
}

pub fn write_route_css_assets_for_project(project_root: &Path) -> Result<(), String> {
    if !runtime_core::framework::is_app_router_project(project_root) {
        return Ok(());
    }
    let manifest = scan_app_dir(&project_root.join("app"));
    let styles = collect_route_styles(&manifest);
    if styles.iter().all(|s| s.classes.is_empty() && s.files.is_empty()) {
        return Ok(());
    }
    write_route_css_assets(
        &project_root.join(".cache").join("dekascript").join("assets"),
        &styles,
    )
}

pub fn write_route_css_assets(assets_dir: &Path, styles: &[RouteStyle]) -> Result<(), String> {
    let plan = css_plan_from_styles(styles);
    write_route_css_assets_with_plan(assets_dir, styles, &plan)
}

fn write_route_css_assets_with_plan(
    assets_dir: &Path,
    styles: &[RouteStyle],
    plan: &CssPlan,
) -> Result<(), String> {
    let css_dir = assets_dir.join("css");
    fs::create_dir_all(&css_dir)
        .map_err(|err| format!("failed to create {}: {err}", css_dir.display()))?;

    let mut keep: BTreeSet<String> = BTreeSet::new();

    if plan.common {
        let mut css = deka_http::utility_css::utility_css_for_classes(
            &plan.common_classes.iter().cloned().collect::<Vec<_>>(),
            true,
        );
        for file in &plan.common_files {
            css.push_str(&read_css(file));
        }
        let name = hashed_css_name("common", &css);
        fs::write(css_dir.join(&name), css.as_bytes())
            .map_err(|err| format!("failed to write {name}: {err}"))?;
        keep.insert(name);
    }

    for style in styles {
        let unique_classes: Vec<String> = style
            .classes
            .iter()
            .filter(|class| !plan.common_classes.contains(*class))
            .cloned()
            .collect();
        let unique_files: Vec<&String> = style
            .files
            .iter()
            .filter(|file| !plan.common_files.contains(*file))
            .collect();
        if unique_classes.is_empty() && unique_files.is_empty() {
            continue;
        }
        let mut css = String::new();
        if !unique_classes.is_empty() {
            css.push_str(&deka_http::utility_css::utility_css_for_classes(
                &unique_classes,
                false,
            ));
        }
        for file in unique_files {
            css.push_str(&read_css(file));
        }
        let name = hashed_css_name(&format!("route-{}", route_css_slug(&style.route)), &css);
        fs::write(css_dir.join(&name), css.as_bytes())
            .map_err(|err| format!("failed to write {name}: {err}"))?;
        keep.insert(name);
    }
    clean_stale_css(&css_dir, &keep)?;
    Ok(())
}

fn read_css(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_names_are_content_addressed() {
        assert_eq!(hashed_css_name("common", "a"), hashed_css_name("common", "a"));
        assert_ne!(hashed_css_name("common", "a"), hashed_css_name("common", "b"));
        let name = hashed_css_name("route-root", ".p-4{}");
        assert!(name.starts_with("route-root."), "{name}");
        assert!(name.ends_with(".css"), "{name}");
    }
}
