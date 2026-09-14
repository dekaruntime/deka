//! The `index.html` document contract: hole markers, the client import map
//! placeholder, and the fragment/static Accept types.
//!
//! The three holes below are an OPTIONAL override, never a requirement.
//! `deka` never generates them (`DEFAULT_INDEX_HARNESS` and the `deka init`
//! scaffold are clean, Vite-shaped documents), and it infers the split
//! points structurally when a document doesn't carry them: `</head>` for
//! the head hole, the `<div id="app">` element for the app hole, and
//! immediately before `</body>` for scripts. A hand-written document that
//! still wants a marker — say, to force scripts somewhere other than just
//! before `</body>` — keeps working: each marker is honored independently
//! wherever it appears. See `split_document` in `codegen/serve.rs`, the
//! only place that reads these constants for splitting.
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
/// head/mid/tail parts. Clean and Vite-shaped — no hole markers, no explicit
/// entry `<script>` (deka injects the bundle; the app-router convention is
/// the entry). The split points are inferred structurally: `</head>`,
/// `<div id="app">`, and immediately before `</body>`.
pub const DEFAULT_INDEX_HARNESS: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
  </head>
  <body>
    <div id="app"></div>
  </body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_harness_is_clean_and_marker_free() {
        assert!(!DEFAULT_INDEX_HARNESS.contains(DEKA_HEAD_HOLE));
        assert!(!DEFAULT_INDEX_HARNESS.contains(DEKA_APP_HOLE));
        assert!(!DEFAULT_INDEX_HARNESS.contains(DEKA_SCRIPTS_HOLE));
        assert!(!DEFAULT_INDEX_HARNESS.contains("<script"));
        assert!(DEFAULT_INDEX_HARNESS.contains("</head>"));
        assert!(DEFAULT_INDEX_HARNESS.contains(r#"<div id="app">"#));
        assert!(DEFAULT_INDEX_HARNESS.contains("</body>"));
    }
}
