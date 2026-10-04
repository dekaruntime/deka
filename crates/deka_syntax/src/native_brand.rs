//! Native declaration identity is private; reflected names remain public.
const PREFIX: &str = "__DekaHost_";
pub fn brand(name: &str) -> String {
    format!("{PREFIX}{name}")
}
pub fn public_name(name: &str) -> &str {
    name.strip_prefix(PREFIX).unwrap_or(name)
}
