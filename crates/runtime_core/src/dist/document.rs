//! The `index.html` document contract: hole markers, the client import map
//! placeholder, and the fragment/static Accept types.
pub const DEKA_HEAD_HOLE: &str = "<!--deka-head-->";
pub const DEKA_APP_HOLE: &str = "<!--deka-app-->";
pub const DEKA_SCRIPTS_HOLE: &str = "<!--deka-scripts-->";
/// Generation-time placeholder for the client import map, injected into the
/// app-router document when it loads client chunks. Browsers reject the `src`
/// form of this element (the attribute is disallowed on
/// `<script type="importmap">`), so this is never the delivery mechanism:
/// the dist-HTML rewrite in `deka build` (and the serve path) swap it for the
/// inline map once the hashed assets — and therefore the map — exist.
pub const CLIENT_IMPORTMAP_PLACEHOLDER_TAG: &str =
    r#"<script type="importmap" src="/assets/importmap.json"></script>"#;
pub const FRAGMENT_ACCEPT: &str = "text/x-deka-fragment";
pub const FRAGMENT_ACCEPT_LEGACY: &str = "text/x-phpx-fragment";

/// The document `deka build` emits when a source project has no root
/// `index.html`: `dist/client/index.html` is a build output (RFD 54
/// amendment 1), and the generated server entry splits this harness into its
/// head/mid/tail parts. Carries the three hole markers so head, app HTML,
/// and scripts each land in their slot.
pub const DEFAULT_INDEX_HARNESS: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <!--deka-head-->
  </head>
  <body>
    <div id="app"><!--deka-app--></div>
    <!--deka-scripts-->
  </body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_harness_carries_the_three_holes() {
        assert!(DEFAULT_INDEX_HARNESS.contains(DEKA_HEAD_HOLE));
        assert!(DEFAULT_INDEX_HARNESS.contains(DEKA_APP_HOLE));
        assert!(DEFAULT_INDEX_HARNESS.contains(DEKA_SCRIPTS_HOLE));
    }
}
