//! Per-route CSS: utility classes + imported stylesheets, as `<link>` assets.

use std::fs;
use std::path::Path;

use runtime_core::framework::{
    collect_route_styles, css_plan_from_styles, route_css_slug, scan_app_dir, CssPlan, RouteStyle,
};

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

    if plan.common {
        let mut css = deka_http::utility_css::utility_css_for_classes(
            &plan.common_classes.iter().cloned().collect::<Vec<_>>(),
            true,
        );
        for file in &plan.common_files {
            css.push_str(&read_css(file));
        }
        fs::write(css_dir.join("common.css"), css.as_bytes())
            .map_err(|err| format!("failed to write common.css: {err}"))?;
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
        let name = format!("route-{}.css", route_css_slug(&style.route));
        fs::write(css_dir.join(&name), css.as_bytes())
            .map_err(|err| format!("failed to write {name}: {err}"))?;
    }
    Ok(())
}

fn read_css(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default()
}
