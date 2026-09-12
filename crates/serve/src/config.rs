use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServeMode {
    Js,
    Static,
    Php,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct ServeConfig {
    pub mode: Option<ServeMode>,
    pub entry: Option<String>,
    pub directory_listing: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StaticServeConfig {
    #[serde(default)]
    pub public: Option<String>,

    #[serde(default = "default_clean_urls")]
    pub clean_urls: CleanUrls,

    #[serde(default)]
    pub rewrites: Vec<Rewrite>,

    #[serde(default)]
    pub redirects: Vec<Redirect>,

    #[serde(default)]
    pub headers: Vec<Header>,

    #[serde(default = "default_directory_listing")]
    pub directory_listing: DirectoryListing,

    #[serde(default = "default_unlisted")]
    pub unlisted: Vec<String>,

    #[serde(default)]
    pub trailing_slash: Option<bool>,

    #[serde(default)]
    pub render_single: bool,

    #[serde(default)]
    pub symlinks: bool,

    #[serde(default)]
    pub server_routes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CleanUrls {
    Bool(bool),
    Patterns(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DirectoryListing {
    Bool(bool),
    Patterns(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rewrite {
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Redirect {
    pub source: String,
    pub destination: String,
    #[serde(default = "default_redirect_type")]
    pub r#type: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub source: String,
    pub headers: Vec<HeaderEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderEntry {
    pub key: String,
    pub value: Option<String>,
}

impl ServeConfig {
    pub fn load(directory: &Path) -> Self {
        let config_path = directory.join("serve.json");
        if !config_path.exists() {
            return Self::default();
        }

        let contents = match std::fs::read_to_string(&config_path) {
            Ok(contents) => contents,
            Err(_) => return Self::default(),
        };

        serde_json::from_str::<ServeConfig>(&contents).unwrap_or_default()
    }
}

impl Default for StaticServeConfig {
    fn default() -> Self {
        Self {
            public: None,
            clean_urls: default_clean_urls(),
            rewrites: Vec::new(),
            redirects: Vec::new(),
            headers: Vec::new(),
            directory_listing: default_directory_listing(),
            unlisted: default_unlisted(),
            trailing_slash: None,
            render_single: false,
            symlinks: false,
            server_routes: Vec::new(),
        }
    }
}

impl StaticServeConfig {
    pub fn load(directory: &Path) -> Self {
        let config_path = directory.join("serve.json");
        if !config_path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(&config_path) {
            Ok(content) => serde_json::from_str::<StaticServeConfig>(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}

fn default_clean_urls() -> CleanUrls {
    CleanUrls::Bool(true)
}

fn default_directory_listing() -> DirectoryListing {
    DirectoryListing::Bool(true)
}

fn default_unlisted() -> Vec<String> {
    vec![".DS_Store".to_string(), ".git".to_string()]
}

fn default_redirect_type() -> u16 {
    301
}
