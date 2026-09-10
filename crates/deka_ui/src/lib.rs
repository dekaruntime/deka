//! Compiler-provided UI runtime. One copy of each file; hosts load these
//! strings (or write them to disk) under the `ui/jsx` … `ui/server` specifiers.
//!
//! `jsx`, `router`, `form`, and `suspense` are DekaScript: `ds/*.ds` is the
//! authoritative source and `emit/*.js` is the pinned dsc output (regenerated
//! with the pinned compiler; `cargo test -p deka_ui` verifies it byte-for-byte
//! and typechecks the sources). The remaining modules are still hand-written
//! JavaScript pending their own ports (deka#771).

pub const JSX: &str = include_str!("../emit/jsx.js");
pub const REACTIVE: &str = include_str!("../js/reactive.js");
pub const CLIENT: &str = include_str!("../js/client.js");
pub const SERVER: &str = include_str!("../js/server.js");
pub const ISLAND_MARKER: &str = include_str!("../js/island-marker.js");
pub const FORM: &str = include_str!("../emit/form.js");
pub const SUSPENSE: &str = include_str!("../emit/suspense.js");
pub const ROUTER: &str = include_str!("../emit/router.js");

/// DekaScript sources for the ported modules, kept for checks that operate on
/// source rather than emitted JavaScript.
pub const JSX_DS: &str = include_str!("../ds/jsx.ds");
pub const FORM_DS: &str = include_str!("../ds/form.ds");
pub const SUSPENSE_DS: &str = include_str!("../ds/suspense.ds");
pub const ROUTER_DS: &str = include_str!("../ds/router.ds");

#[cfg(test)]
mod tests {
    use super::JSX_DS;

    #[test]
    fn jsx_factory_does_not_freeze_ephemeral_nodes() {
        // The emitted prelude freezes the Option/Result helpers; the
        // invariant that matters is that the DekaScript source never freezes
        // a node (hidden/frozen fields push nodes into dictionary mode,
        // ~12us/render slower on a 49-node grid, deka#580).
        assert!(!JSX_DS.contains("Object.freeze"));
    }
}

pub const SPECIFIERS: &[&str] = &[
    "ui/jsx",
    "ui/reactive",
    "ui/client",
    "ui/server",
    "ui/island-marker",
    "ui/form",
    "ui/suspense",
    "ui/router",
];

pub fn source_for(specifier: &str) -> Option<&'static str> {
    match specifier.trim_end_matches(".js").trim_end_matches(".mjs") {
        "ui/jsx" => Some(JSX),
        "ui/reactive" => Some(REACTIVE),
        "ui/client" => Some(CLIENT),
        "ui/server" => Some(SERVER),
        "ui/island-marker" => Some(ISLAND_MARKER),
        "ui/form" => Some(FORM),
        "ui/suspense" => Some(SUSPENSE),
        "ui/router" => Some(ROUTER),
        _ => None,
    }
}

pub fn file_name_for(specifier: &str) -> Option<&'static str> {
    match specifier.trim_end_matches(".js").trim_end_matches(".mjs") {
        "ui/jsx" => Some("jsx.js"),
        "ui/reactive" => Some("reactive.js"),
        "ui/client" => Some("client.js"),
        "ui/server" => Some("server.js"),
        "ui/island-marker" => Some("island-marker.js"),
        "ui/form" => Some("form.js"),
        "ui/suspense" => Some("suspense.js"),
        "ui/router" => Some("router.js"),
        _ => None,
    }
}
