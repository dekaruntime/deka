//! Per-route CSS: utility classes + imported stylesheets, as `<link>` assets.
//!
//! All emitted stylesheets are content-addressed: `<stem>.<sha256-10>.css`
//! (see `crate::islands`), so `deka build` and `deka serve` derive the same
//! name for the same bytes.
//!
//! Component-authored CSS (side-effect `import "./x.css"` in a component
//! module) is scoped at write time per RFD 24 §10.6: every selector is
//! rewritten to require the importing module's stamp
//! (`[data-deka-cid-<hash>]`), which the compiler emits on that module's host
//! elements. Utility classes stay global by design.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use lightningcss::rules::{CssRule, CssRuleList};
use lightningcss::selector::{Component, Selector, SelectorList};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::traits::ToCss;
use runtime_core::framework::{
    CssPlan, RouteStyle, ScopedStyleFile, collect_route_styles, css_plan_from_styles,
    route_css_slug, scan_app_dir,
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
    if styles
        .iter()
        .all(|s| s.classes.is_empty() && s.files.is_empty())
    {
        return Ok(());
    }
    write_route_css_assets(
        &project_root
            .join(".cache")
            .join("dekascript")
            .join("assets"),
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
        for scoped in common_scoped_files(styles, plan) {
            css.push_str(&scope_css_file(scoped)?);
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
        let unique_files: Vec<&ScopedStyleFile> = style
            .files
            .iter()
            .filter(|file| !plan.common_files.contains(&file.path))
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
        for scoped in unique_files {
            css.push_str(&scope_css_file(scoped)?);
        }
        let name = hashed_css_name(&format!("route-{}", route_css_slug(&style.route)), &css);
        fs::write(css_dir.join(&name), css.as_bytes())
            .map_err(|err| format!("failed to write {name}: {err}"))?;
        keep.insert(name);
    }
    clean_stale_css(&css_dir, &keep)?;
    Ok(())
}

/// Scoped CSS files whose path is hoisted into common.css, deduplicated by
/// (path, cid): a file imported by several modules appears once per importing
/// module so every importer's stamp matches.
fn common_scoped_files<'a>(styles: &'a [RouteStyle], plan: &CssPlan) -> Vec<&'a ScopedStyleFile> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for style in styles {
        for file in &style.files {
            if plan.common_files.contains(&file.path)
                && seen.insert((file.path.clone(), file.cid.clone()))
            {
                out.push(file);
            }
        }
    }
    out
}

fn scope_css_file(scoped: &ScopedStyleFile) -> Result<String, String> {
    let source = fs::read_to_string(&scoped.path)
        .map_err(|err| format!("failed to read {}: {err}", scoped.path))?;
    scope_css_selectors(&source, &scoped.path, &scoped.cid)
}

fn print_options() -> PrinterOptions<'static> {
    PrinterOptions {
        minify: false,
        ..Default::default()
    }
}

/// Rewrite every selector in a component CSS file to require the module's
/// stamp (`[data-deka-cid-<cid>]`). Selectors that cannot be scoped safely —
/// anything addressing `:root`, `html`, or `body` — are a hard build error:
/// global escapes are rejected by RFD 24 §10.6 (Shadow DOM was rejected for
/// the same theming reason, so document-level styling has no back door here).
fn scope_css_selectors(source: &str, filename: &str, cid: &str) -> Result<String, String> {
    let attr = format!("[data-deka-cid-{cid}]");
    let stylesheet = StyleSheet::parse(
        source,
        ParserOptions {
            filename: filename.to_string(),
            ..Default::default()
        },
    )
    .map_err(|err| format!("failed to parse component CSS {filename}: {err}"))?;
    let mut out = String::new();
    for rule in &stylesheet.rules.0 {
        scope_rule(rule, &mut out, &attr, filename)?;
    }
    // The rewrite above is textual surgery on serialized selectors; reparse
    // the result so a bad rewrite fails the build here, not in the browser.
    StyleSheet::parse(
        &out,
        ParserOptions {
            filename: filename.to_string(),
            ..Default::default()
        },
    )
    .map_err(|err| format!("scoped CSS for {filename} is invalid: {err}\n{out}"))?;
    Ok(out)
}

fn scope_rule(
    rule: &CssRule<'_>,
    out: &mut String,
    attr: &str,
    filename: &str,
) -> Result<(), String> {
    match rule {
        CssRule::Style(style) => scope_style_rule(style, out, attr, filename),
        CssRule::Nesting(nesting) => scope_style_rule(&nesting.style, out, attr, filename),
        CssRule::Media(media) => {
            out.push_str("@media ");
            out.push_str(
                &media
                    .query
                    .to_css_string(print_options())
                    .map_err(|err| format!("failed to print @media in {filename}: {err}"))?,
            );
            scope_rule_block(&media.rules, out, attr, filename)
        }
        CssRule::Supports(supports) => {
            out.push_str("@supports ");
            out.push_str(
                &supports
                    .condition
                    .to_css_string(print_options())
                    .map_err(|err| format!("failed to print @supports in {filename}: {err}"))?,
            );
            scope_rule_block(&supports.rules, out, attr, filename)
        }
        CssRule::LayerBlock(layer) => {
            out.push_str("@layer");
            if let Some(name) = &layer.name {
                out.push(' ');
                out.push_str(
                    &name
                        .to_css_string(print_options())
                        .map_err(|err| format!("failed to print @layer in {filename}: {err}"))?,
                );
            }
            scope_rule_block(&layer.rules, out, attr, filename)
        }
        CssRule::Container(container) => {
            let mut prelude = String::from("@container");
            if let Some(name) = &container.name {
                prelude.push(' ');
                prelude.push_str(
                    &name.to_css_string(print_options()).map_err(|err| {
                        format!("failed to print @container in {filename}: {err}")
                    })?,
                );
            }
            if let Some(condition) = &container.condition {
                prelude.push(' ');
                prelude.push_str(
                    &condition.to_css_string(print_options()).map_err(|err| {
                        format!("failed to print @container in {filename}: {err}")
                    })?,
                );
            }
            out.push_str(&prelude);
            scope_rule_block(&container.rules, out, attr, filename)
        }
        CssRule::StartingStyle(starting) => {
            out.push_str("@starting-style");
            scope_rule_block(&starting.rules, out, attr, filename)
        }
        CssRule::Scope(scope) => {
            // `@scope` picks its root from the surrounding document, which is
            // exactly the escape hatch component scoping rejects.
            return Err(format!(
                "@scope is not supported in component-scoped CSS ({filename}): its prelude selects outside the component"
            ));
        }
        CssRule::Import(import) => {
            // An imported stylesheet's selectors cannot be rewritten here, so
            // letting it through would silently leak global rules.
            return Err(format!(
                "@import is not supported in component-scoped CSS ({filename}): import \"{}\" from the component module instead",
                import.url
            ));
        }
        // @font-face, @keyframes, @page, @custom-media, … carry no element
        // selectors. @keyframes names stay global (pre-existing behavior).
        // Unknown at-rules pass through unrewritten.
        _ => {
            out.push_str(
                &rule
                    .to_css_string(print_options())
                    .map_err(|err| format!("failed to print a rule in {filename}: {err}"))?,
            );
            Ok(())
        }
    }
}

fn scope_rule_block(
    rules: &CssRuleList<'_>,
    out: &mut String,
    attr: &str,
    filename: &str,
) -> Result<(), String> {
    out.push_str(" {\n");
    for rule in &rules.0 {
        scope_rule(rule, out, attr, filename)?;
    }
    out.push_str("}\n");
    Ok(())
}

fn scope_style_rule(
    style: &lightningcss::rules::style::StyleRule<'_>,
    out: &mut String,
    attr: &str,
    filename: &str,
) -> Result<(), String> {
    out.push_str(&scope_selector_list(&style.selectors, attr, filename)?);
    out.push_str(" {\n");
    out.push_str(
        &style
            .declarations
            .to_css_string(print_options())
            .map_err(|err| format!("failed to print declarations in {filename}: {err}"))?,
    );
    // The declaration block serialization carries no trailing separator; a
    // nested rule after it would otherwise fuse into the last declaration.
    if !style.rules.0.is_empty() && !out.trim_end().is_empty() && !out.trim_end().ends_with(';') {
        out.push(';');
    }
    out.push('\n');
    for nested in &style.rules.0 {
        scope_rule(nested, out, attr, filename)?;
    }
    out.push_str("}\n");
    Ok(())
}

fn scope_selector_list(
    selectors: &SelectorList<'_>,
    attr: &str,
    filename: &str,
) -> Result<String, String> {
    let mut out = Vec::new();
    for selector in selectors.0.iter() {
        reject_unsafe_selector(selector, filename)?;
        let text = selector
            .to_css_string(print_options())
            .map_err(|err| format!("failed to print a selector in {filename}: {err}"))?;
        out.push(insert_scope_attr(&text, attr));
    }
    Ok(out.join(", "))
}

fn reject_unsafe_selector(selector: &Selector<'_>, filename: &str) -> Result<(), String> {
    let mut iter = selector.iter();
    loop {
        for component in &mut iter {
            match component {
                Component::Root => {
                    return Err(format!(
                        ":root cannot be scoped in component CSS ({filename}): component styles must not target the document root"
                    ));
                }
                Component::LocalName(local)
                    if local.lower_name.as_ref() == "html"
                        || local.lower_name.as_ref() == "body" =>
                {
                    return Err(format!(
                        "{} cannot be scoped in component CSS ({filename}): component styles must not target the document element",
                        local.lower_name.as_ref()
                    ));
                }
                _ => {}
            }
        }
        if iter.next_sequence().is_none() {
            return Ok(());
        }
    }
}

/// Append the scope attribute to a serialized selector. A pseudo-element, if
/// present, must stay last, so the attribute is inserted before it:
/// lightningcss serializes modern pseudo-elements with `::` but legacy ones
/// (`:before`, `:after`, `:first-line`, `:first-letter`) with a single colon.
/// Markers inside strings and attribute selectors (`[href="a::b"]`) are
/// skipped, as are backslash escapes so `\:` in a class name is not mistaken
/// for a pseudo-element marker.
fn insert_scope_attr(selector: &str, attr: &str) -> String {
    const LEGACY_PSEUDO_ELEMENTS: [&str; 4] = ["before", "after", "first-line", "first-letter"];
    let bytes = selector.as_bytes();
    let mut i = 0;
    let mut in_brackets = false;
    let mut quote: Option<u8> = None;
    while i + 1 < bytes.len() {
        let byte = bytes[i];
        if let Some(q) = quote {
            if byte == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'\\' => {
                i += 2;
                continue;
            }
            b'[' => in_brackets = true,
            b']' => in_brackets = false,
            b':' if !in_brackets => {
                if bytes[i + 1] == b':' {
                    return format!("{}{}{}", &selector[..i], attr, &selector[i..]);
                }
                let rest = &selector[i + 1..];
                let ident_len = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                    .unwrap_or(rest.len());
                if ident_len > 0 && LEGACY_PSEUDO_ELEMENTS.contains(&&rest[..ident_len]) {
                    return format!("{}{}{}", &selector[..i], attr, &selector[i..]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    format!("{selector}{attr}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CID: &str = "e1c9193cd172";
    const ATTR: &str = "[data-deka-cid-e1c9193cd172]";

    fn scope(css: &str) -> Result<String, String> {
        scope_css_selectors(css, "test.css", CID)
    }

    #[test]
    fn scopes_a_class_selector() {
        let out = scope(".greeting { color: red; }").expect("scope");
        assert!(out.contains(&format!(".greeting{ATTR}")), "got: {out}");
    }

    #[test]
    fn scopes_each_selector_in_a_group() {
        let out = scope(".a, .b > .c { color: red; }").expect("scope");
        assert!(out.contains(&format!(".a{ATTR}")), "got: {out}");
        assert!(out.contains(&format!(".b > .c{ATTR}")), "got: {out}");
    }

    #[test]
    fn scopes_inside_media_queries() {
        // lightningcss modernizes media features (`min-width: 40em` becomes
        // `width >= 40em`), so assert the structure, not the query spelling.
        let out = scope("@media (min-width: 40em) { .greeting { color: red; } }").expect("scope");
        assert!(out.contains("@media"), "got: {out}");
        assert!(out.contains(&format!(".greeting{ATTR}")), "got: {out}");
    }

    #[test]
    fn keeps_pseudo_classes_and_inserts_before_pseudo_elements() {
        let out =
            scope(".btn:hover { color: red; } .btn::before { content: \"\"; }").expect("scope");
        // A pseudo-class takes the attribute at the end of the compound…
        assert!(out.contains(&format!(".btn:hover{ATTR}")), "got: {out}");
        // …but lightningcss serializes legacy pseudo-elements with a single
        // colon, and the attribute must precede them.
        assert!(out.contains(&format!(".btn{ATTR}:before")), "got: {out}");
    }

    #[test]
    fn does_not_mistake_escaped_colons_or_attribute_values() {
        let out = scope(".a\\:b { color: red; } [href=\"x::y\"] { color: blue; }").expect("scope");
        assert!(out.contains(&format!(".a\\:b{ATTR}")), "got: {out}");
        assert!(
            out.contains(&format!("[href=\"x::y\"]{ATTR}")),
            "got: {out}"
        );
    }

    #[test]
    fn scopes_nested_style_rules() {
        let out = scope(".card { color: red; &:hover { color: blue; } }").expect("scope");
        assert!(out.contains(&format!(".card{ATTR}")), "got: {out}");
        assert!(out.contains(&format!("&:hover{ATTR}")), "got: {out}");
    }

    #[test]
    fn leaves_keyframes_untouched() {
        let out = scope("@keyframes spin { from { transform: rotate(0deg); } }").expect("scope");
        assert!(out.contains("@keyframes spin"), "got: {out}");
        assert!(!out.contains(ATTR), "got: {out}");
    }

    #[test]
    fn rejects_root_html_and_body() {
        for bad in [
            ":root { margin: 0; }",
            "html { margin: 0; }",
            "body { margin: 0; }",
        ] {
            assert!(scope(bad).is_err(), "{bad} must be rejected");
        }
    }

    #[test]
    fn rejects_import_and_scope_at_rules() {
        assert!(scope("@import \"./other.css\";").is_err());
        assert!(scope("@scope (.card) { .greeting { color: red; } }").is_err());
    }

    #[test]
    fn scoped_output_reparses() {
        // Exercised implicitly by every scope() call (the validation reparse),
        // asserted here explicitly for a compound document.
        let out = scope(
            "@media (min-width: 1em) { .a { color: red; } }\n.b::after { content: \"::\"; }\n",
        )
        .expect("scope");
        assert!(StyleSheet::parse(&out, ParserOptions::default()).is_ok());
    }

    #[test]
    fn css_names_are_content_addressed() {
        assert_eq!(hashed_css_name("common", "a"), hashed_css_name("common", "a"));
        assert_ne!(hashed_css_name("common", "a"), hashed_css_name("common", "b"));
        let name = hashed_css_name("route-root", ".p-4{}");
        assert!(name.starts_with("route-root."), "{name}");
        assert!(name.ends_with(".css"), "{name}");
    }
}
