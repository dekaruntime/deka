//! The `index.html` document contract: hole markers, fragment/static Accept
//! types, and hole filling.
pub const DEKA_HEAD_HOLE: &str = "<!--deka-head-->";
pub const DEKA_APP_HOLE: &str = "<!--deka-app-->";
pub const DEKA_SCRIPTS_HOLE: &str = "<!--deka-scripts-->";
/// Generation-time placeholder for the client import map, injected into the
/// app-router document when it loads client chunks. Browsers reject the `src`
/// form of this element (the attribute is disallowed on
/// `<script type="importmap">`), so this is never the delivery mechanism:
/// `runtime::islands::rewrite_serve_entry_asset_urls` (serve) and the dist-HTML
/// rewrite in `deka build` swap it for the inline map once the hashed assets
/// — and therefore the map — exist.
pub const CLIENT_IMPORTMAP_PLACEHOLDER_TAG: &str =
    r#"<script type="importmap" src="/assets/importmap.json"></script>"#;
pub const FRAGMENT_ACCEPT: &str = "text/x-deka-fragment";
pub const FRAGMENT_ACCEPT_LEGACY: &str = "text/x-phpx-fragment";
pub const STATIC_ACCEPT: &str = "text/x-deka-static";
/// Fill the three document holes. Missing holes are left unchanged.
pub fn fill_document(index_html: &str, head: &str, app: &str, scripts: &str) -> String {
    // Replace holes in template order, each once. App HTML is escaped by
    // renderToString so it cannot contain a raw `<!--deka-scripts-->` that
    // would steal the later pass.
    let mut out = index_html.to_string();
    if out.contains(DEKA_HEAD_HOLE) {
        out = out.replacen(DEKA_HEAD_HOLE, head, 1);
    }
    if out.contains(DEKA_APP_HOLE) {
        out = out.replacen(DEKA_APP_HOLE, app, 1);
    }
    if out.contains(DEKA_SCRIPTS_HOLE) {
        out = out.replacen(DEKA_SCRIPTS_HOLE, scripts, 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_document_replaces_the_three_holes() {
        let index = "<head><!--deka-head--></head><body><!--deka-app--><!--deka-scripts--></body>";
        let filled = fill_document(index, "<title>Hi</title>", "<p>app</p>", "");
        assert_eq!(
            filled,
            "<head><title>Hi</title></head><body><p>app</p></body>"
        );
    }
}
