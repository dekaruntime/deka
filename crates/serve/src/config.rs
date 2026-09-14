use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServeMode {
    Js,
    Static,
    Php,
}

/// Legacy in-process mirror of `engine::config::ServeConfig` (deka#1038).
/// `run::handler::ResolvedHandler.config` still carries this type — see
/// `as_legacy_serve_config` — but nothing in this crate loads it from disk
/// any more. `serve.json` (the file this struct used to be parsed from) is
/// no longer a supported config surface: `deka.json`'s `serve` object is
/// the only place these fields are read from now
/// (`engine::config::ServeConfig::load`). The struct and its `Default`
/// derive stay because `ambient_environment.rs`, `self_cmd/src/monitor.rs`
/// and `self_cmd/src/update/pipeline.rs` construct fixture values with
/// `::default()`.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ServeConfig {
    pub mode: Option<ServeMode>,
    pub entry: Option<String>,
    pub directory_listing: Option<bool>,
}
