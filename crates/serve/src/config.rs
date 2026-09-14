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
    /// Load `serve.json`'s static-serve fields (`headers`, `rewrites`,
    /// `redirects`, ...). A missing file is a normal default configuration.
    /// A present-but-malformed file is a hard error, not a silent default
    /// (deka#1034, same class as #1017/#1020): a typo in a `headers` block
    /// used to fall back to `StaticServeConfig::default()` with no
    /// diagnostic, so a project's declared headers/rewrites/redirects were
    /// simply never applied and nobody was told why.
    pub fn load(directory: &Path) -> Result<Self, String> {
        let config_path = directory.join("serve.json");
        if !config_path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(&config_path).map_err(|err| {
            format!("failed to read {}: {err}", config_path.display())
        })?;

        serde_json::from_str::<StaticServeConfig>(&contents).map_err(|err| {
            format!(
                "{}: invalid serve config: {}. Check `headers`, `rewrites`, `redirects`, \
                 `clean_urls`, and `directory_listing` against the serve.json schema.",
                config_path.display(),
                err
            )
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{}_{}", prefix, nonce));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_serve_json_loads_defaults() {
        let dir = temp_dir("serve_config_missing");
        let config = StaticServeConfig::load(&dir).expect("no serve.json is not an error");
        assert!(config.headers.is_empty());
        assert!(config.rewrites.is_empty());
        assert!(config.redirects.is_empty());
    }

    #[test]
    fn malformed_headers_block_is_a_hard_error_not_a_silent_default() {
        // deka#1034: a `headers` entry of the wrong shape used to be caught
        // by `unwrap_or_default()` and silently discarded — the project's
        // declared headers, rewrites and redirects all reverted to empty
        // with no diagnostic. That must now be a hard, named error.
        let dir = temp_dir("serve_config_malformed_headers");
        std::fs::write(
            dir.join("serve.json"),
            r#"{"headers":[{"source":"**/*.html","headers":"oops-not-an-array"}]}"#,
        )
        .unwrap();

        let err = StaticServeConfig::load(&dir)
            .expect_err("malformed headers block must fail to load, not silently default");
        assert!(err.contains("invalid serve config"), "{err}");
        assert!(err.contains("serve.json"), "{err}");
    }

    #[test]
    fn malformed_json_syntax_is_a_hard_error() {
        let dir = temp_dir("serve_config_malformed_syntax");
        std::fs::write(dir.join("serve.json"), r#"{"headers": [}"#).unwrap();

        let err = StaticServeConfig::load(&dir)
            .expect_err("syntactically invalid JSON must fail to load");
        assert!(err.contains("invalid serve config"), "{err}");
    }

    #[test]
    fn valid_headers_rewrites_and_redirects_round_trip() {
        let dir = temp_dir("serve_config_valid");
        std::fs::write(
            dir.join("serve.json"),
            r#"{
                "headers": [
                    {"source": "**/*.html", "headers": [{"key": "X-Test", "value": "1"}]}
                ],
                "rewrites": [
                    {"source": "/foo", "destination": "/index.html"}
                ],
                "redirects": [
                    {"source": "/old", "destination": "/index.html"}
                ]
            }"#,
        )
        .unwrap();

        let config = StaticServeConfig::load(&dir).expect("valid serve.json must load");
        assert_eq!(config.headers.len(), 1);
        assert_eq!(config.headers[0].source, "**/*.html");
        assert_eq!(config.headers[0].headers[0].key, "X-Test");
        assert_eq!(config.rewrites.len(), 1);
        assert_eq!(config.rewrites[0].destination, "/index.html");
        assert_eq!(config.redirects.len(), 1);
        assert_eq!(config.redirects[0].r#type, 301);
    }
}
