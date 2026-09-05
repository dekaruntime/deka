//! Compiler-provided UI runtime. One copy of each file; hosts load these
//! strings (or write them to disk) under the `ui/jsx` … `ui/server` specifiers.

pub const JSX: &str = include_str!("../js/jsx.js");
pub const REACTIVE: &str = include_str!("../js/reactive.js");
pub const CLIENT: &str = include_str!("../js/client.js");
pub const SERVER: &str = include_str!("../js/server.js");
pub const ISLAND_MARKER: &str = include_str!("../js/island-marker.js");
pub const FORM: &str = include_str!("../js/form.js");
pub const SUSPENSE: &str = include_str!("../js/suspense.js");
pub const ROUTER: &str = include_str!("../js/router.js");

#[cfg(test)]
mod tests {
    use super::JSX;

    #[test]
    fn jsx_factory_does_not_freeze_ephemeral_nodes() {
        assert!(!JSX.contains("Object.freeze"));
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
