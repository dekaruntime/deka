//! Single source of truth for the `deka-island` HTML-comment marker grammar
//! that delimits server-rendered islands in HMR snapshots.
//!
//! Every consumer derives from the tokens below: the server-side scanner in
//! `websocket.rs`, the mirrored resolver in `websocket_hmr_tests.rs`, and the
//! browser client in `hmr_client/` (which receives them through
//! [`JS_PRELUDE`], injected ahead of the client fragments by
//! `router::inject_hmr_client`). Do not re-introduce literal copies of these
//! tokens anywhere; a grammar change is a change to the macro invocation
//! below and nowhere else, so comments cannot drift apart.
//!
//! Grammar (produced by the paused ui/server runtime, e.g. its
//! `wrapDeferred`):
//!
//! ```text
//! <!--deka-island start:<b64 name> directive:<b64> [props:<b64>] [id:<b64>] [cache:<b64>]-->
//! ...island body...
//! <!--deka-island end:<b64 name>-->
//! ```
//!
//! Fields after `start:` are space-separated `key:<base64>` pairs. `directive`
//! is present in practice; `props`, `id` (deferred-island identity), and
//! `cache` (deferred-island cache hint, e.g. `60s`) are optional and may
//! appear in any order. Consumers must match only the `start:`/`end:`
//! prefixes and must not depend on field presence or order. The island's
//! stable identity is the decoded component name from the start marker; the
//! body is the markup between the start and end markers.

macro_rules! define_island_markers {
    (
        comment_open = $comment_open:literal,
        tag = $tag:literal,
        start = $start:literal,
        end = $end:literal,
        field_directive = $field_directive:literal,
        field_props = $field_props:literal,
        field_id = $field_id:literal,
        field_cache = $field_cache:literal,
    ) => {
        /// Raw HTML comment open, before the marker tag.
        pub const COMMENT_OPEN: &str = $comment_open;
        /// Marker tag shared by the start and end markers.
        pub const TAG: &str = $tag;
        /// Keyword introducing a start marker.
        pub const START: &str = $start;
        /// Keyword introducing an end marker.
        pub const END: &str = $end;

        /// Header field name for the island's hydration directive.
        pub const FIELD_DIRECTIVE: &str = $field_directive;
        /// Header field name for the island's serialized props.
        pub const FIELD_PROPS: &str = $field_props;
        /// Header field name for the deferred-island identity.
        pub const FIELD_ID: &str = $field_id;
        /// Header field name for the deferred-island cache hint.
        pub const FIELD_CACHE: &str = $field_cache;

        /// Full start-marker needle: `<!--deka-island start:`
        pub const START_NEEDLE: &str = concat!($comment_open, $tag, " ", $start);
        /// Needle matching any island marker, for depth walks: `<!--deka-island `
        pub const ANY_NEEDLE: &str = concat!($comment_open, $tag, " ");

        /// JavaScript prelude declaring the marker grammar for the
        /// `hmr_client/` fragments. `router::inject_hmr_client` places this
        /// ahead of the fragments, so morph.js/patch.js read the exact same
        /// token values as the Rust scanner.
        pub const JS_PRELUDE: &str = concat!(
            "var DEKA_ISLAND_START_PREFIX = \"",
            $tag,
            " ",
            $start,
            "\";\n",
            "var DEKA_ISLAND_END_PREFIX = \"",
            $tag,
            " ",
            $end,
            "\";\n",
        );
    };
}

define_island_markers!(
    comment_open = "<!--",
    tag = "deka-island",
    start = "start:",
    end = "end:",
    field_directive = "directive:",
    field_props = "props:",
    field_id = "id:",
    field_cache = "cache:",
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_needles_derive_from_tokens() {
        assert!(ANY_NEEDLE.starts_with(COMMENT_OPEN));
        assert!(START_NEEDLE.starts_with(ANY_NEEDLE));
        assert!(START_NEEDLE[ANY_NEEDLE.len()..].starts_with(START));
    }

    #[test]
    fn js_preamble_carries_the_same_tokens() {
        // The browser client and the Rust scanner read one grammar: the
        // prelude is generated from the same tokens as the needles, so a
        // hand-edited prelude that drifts from them fails here.
        let start_decl = format!("DEKA_ISLAND_START_PREFIX = \"{} {}\"", TAG, START);
        let end_decl = format!("DEKA_ISLAND_END_PREFIX = \"{} {}\"", TAG, END);
        assert!(JS_PRELUDE.contains(&start_decl));
        assert!(JS_PRELUDE.contains(&end_decl));
    }
}
