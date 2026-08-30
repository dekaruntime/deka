//! Compiler-provided UI runtime. One copy of each file; hosts load these
//! strings (or write them to disk) under the `ui/jsx` … `ui/server` specifiers.

pub const JSX: &str = include_str!("../js/jsx.js");
pub const REACTIVE: &str = include_str!("../js/reactive.js");
pub const CLIENT: &str = include_str!("../js/client.js");
pub const SERVER: &str = include_str!("../js/server.js");
pub const FORM: &str = include_str!("../js/form.js");

pub const SPECIFIERS: &[&str] = &["ui/jsx", "ui/reactive", "ui/client", "ui/server", "ui/form"];

pub fn source_for(specifier: &str) -> Option<&'static str> {
    match specifier.trim_end_matches(".js").trim_end_matches(".mjs") {
        "ui/jsx" => Some(JSX),
        "ui/reactive" => Some(REACTIVE),
        "ui/client" => Some(CLIENT),
        "ui/server" => Some(SERVER),
        "ui/form" => Some(FORM),
        _ => None,
    }
}

pub fn file_name_for(specifier: &str) -> Option<&'static str> {
    match specifier.trim_end_matches(".js").trim_end_matches(".mjs") {
        "ui/jsx" => Some("jsx.js"),
        "ui/reactive" => Some("reactive.js"),
        "ui/client" => Some("client.js"),
        "ui/server" => Some("server.js"),
        "ui/form" => Some("form.js"),
        _ => None,
    }
}
