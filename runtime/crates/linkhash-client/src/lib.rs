//! First-party Linkhash API client plus the PHPX package install surface used by deka.

mod download;
mod resolve;

use std::{error::Error, fmt, net::IpAddr, path::Path};

use reqwest::{header, Client as HttpClient, Method, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub use resolve::ResolvedPackage;

const TRUSTED_FIRST_PARTY_API_HOSTS: &[&str] = &[
    "linkha.sh",
    "linkhash.tana.gg",
    "packages.tana.gg",
    "git.tana.gg",
    "linkhash.test",
];

#[derive(Clone)]
pub struct LinkhashClient {
    base_url: Url,
    token: Option<String>,
    http: HttpClient,
}

impl fmt::Debug for LinkhashClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LinkhashClient")
            .field("base_url", &self.base_url)
            .field("token", &"[REDACTED]")
            .field("http", &self.http)
            .finish()
    }
}

impl LinkhashClient {
    pub fn new(base_url: impl AsRef<str>, token: Option<&str>) -> Self {
        Self::build_unchecked(base_url, token.map(ToOwned::to_owned), HttpClient::new())
    }

    pub fn try_new(
        base_url: impl AsRef<str>,
        idp_token: impl Into<String>,
    ) -> Result<Self, ClientError> {
        Self::build(base_url, idp_token, HttpClient::new(), false)
    }

    pub fn with_http_client(
        base_url: impl AsRef<str>,
        idp_token: impl Into<String>,
        http: HttpClient,
    ) -> Result<Self, ClientError> {
        Self::build(base_url, idp_token, http, true)
    }

    fn build(
        base_url: impl AsRef<str>,
        idp_token: impl Into<String>,
        http: HttpClient,
        allow_loopback_http: bool,
    ) -> Result<Self, ClientError> {
        let mut base_url = Url::parse(base_url.as_ref())
            .map_err(|error| ClientError::InvalidBaseUrl(error.to_string()))?;
        validate_base_url(&base_url, allow_loopback_http)?;
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }
        Ok(Self {
            base_url,
            token: Some(idp_token.into()),
            http,
        })
    }

    fn build_unchecked(base_url: impl AsRef<str>, token: Option<String>, http: HttpClient) -> Self {
        let mut base_url = Url::parse(base_url.as_ref()).unwrap_or_else(|error| {
            panic!(
                "invalid Linkhash registry URL '{}': {error}",
                base_url.as_ref()
            )
        });
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }
        Self {
            base_url,
            token,
            http,
        }
    }

    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        self.get(["healthz"], Vec::new()).await
    }

    pub async fn list_repos(&self) -> Result<ListReposResponse, ClientError> {
        self.get(["api", "v1", "repos"], Vec::new()).await
    }

    pub async fn list_packages(
        &self,
        query: ListPackagesQuery<'_>,
    ) -> Result<ListPackagesResponse, ClientError> {
        self.get(["api", "v1", "packages"], query.into_pairs())
            .await
    }

    pub async fn list_ecosystem_versions(
        &self,
        ecosystem: &str,
        namespace: &str,
        name: &str,
    ) -> Result<ListVersionsResponse, ClientError> {
        self.get(
            [
                "api", "v1", "packages", ecosystem, namespace, name, "versions",
            ],
            Vec::new(),
        )
        .await
    }

    /// Resolve a scoped PHPX package to an exact version using deka's registry API.
    pub fn resolve(&self, name: &str, version_range: &str) -> anyhow::Result<ResolvedPackage> {
        let http = reqwest::blocking::Client::new();
        resolve::resolve(
            &http,
            self.base_url.as_str().trim_end_matches('/'),
            self.token.as_deref(),
            name,
            version_range,
        )
    }

    /// List all published versions for a scoped PHPX package.
    pub fn list_versions(&self, name: &str) -> anyhow::Result<Vec<String>> {
        let http = reqwest::blocking::Client::new();
        resolve::list_versions(
            &http,
            self.base_url.as_str().trim_end_matches('/'),
            self.token.as_deref(),
            name,
        )
    }

    /// Alias for callers that prefer an explicit package-install method name.
    pub fn list_package_versions(&self, name: &str) -> anyhow::Result<Vec<String>> {
        self.list_versions(name)
    }

    /// Download a scoped PHPX package release into a target directory.
    pub fn download(&self, name: &str, version: &str, target_dir: &Path) -> anyhow::Result<()> {
        let http = reqwest::blocking::Client::new();
        download::download(
            &http,
            self.base_url.as_str().trim_end_matches('/'),
            self.token.as_deref(),
            name,
            version,
            target_dir,
        )
    }

    /// Preflight a package publish without committing the release.
    pub fn preflight(&self, req: &PublishRequest) -> anyhow::Result<PreflightResult> {
        let token = self
            .token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("auth token required for preflight"))?;

        let url = self.endpoint("api/packages/preflight")?;
        let http = reqwest::blocking::Client::new();
        let response = http
            .post(url)
            .bearer_auth(token)
            .json(req)
            .send()
            .map_err(|e| anyhow::anyhow!("preflight request failed: {}", e))?;

        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .map_err(|e| anyhow::anyhow!("failed to parse preflight response: {}", e))?;

        if !status.is_success() {
            let err = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("preflight failed ({}): {}", status, err);
        }

        let preflight = body.get("preflight").unwrap_or(&body);
        Ok(PreflightResult {
            allowed: preflight
                .get("allowed")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            required_bump: preflight
                .get("required_bump")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
            minimum_allowed_version: preflight
                .get("minimum_allowed_version")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
            reasons: preflight
                .get("reasons")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                        .collect()
                }),
        })
    }

    /// Publish a package release.
    pub fn publish(&self, req: &PublishRequest) -> anyhow::Result<PublishResult> {
        let token = self
            .token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("auth token required for publish"))?;

        let url = self.endpoint("api/packages/publish")?;
        let http = reqwest::blocking::Client::new();
        let response = http
            .post(url)
            .bearer_auth(token)
            .json(req)
            .send()
            .map_err(|e| anyhow::anyhow!("publish request failed: {}", e))?;

        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .map_err(|e| anyhow::anyhow!("failed to parse publish response: {}", e))?;

        if !status.is_success() {
            let err = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("publish failed ({}): {}", status, err);
        }

        let release = body.get("release").unwrap_or(&body);

        Ok(PublishResult {
            package_name: release
                .get("package_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            version: release
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url, ClientError> {
        self.base_url
            .join(path)
            .map_err(|_| ClientError::InvalidUrlBase)
    }

    pub async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        query: ListIssuesQuery<'_>,
    ) -> Result<ListIssuesResponse, ClientError> {
        self.get(
            ["api", "v1", "repos", owner, repo, "issues"],
            query.into_pairs(),
        )
        .await
    }

    pub async fn get_issue(
        &self,
        owner: &str,
        repo: &str,
        number: i64,
    ) -> Result<GetIssueResponse, ClientError> {
        let number = number.to_string();
        self.get(
            ["api", "v1", "repos", owner, repo, "issues", &number],
            Vec::new(),
        )
        .await
    }

    pub async fn list_pulls(
        &self,
        owner: &str,
        repo: &str,
        query: ListPullsQuery<'_>,
    ) -> Result<ListPullsResponse, ClientError> {
        self.get(
            ["api", "v1", "repos", owner, repo, "pulls"],
            query.into_pairs(),
        )
        .await
    }

    pub async fn get_pull(
        &self,
        owner: &str,
        repo: &str,
        number: i64,
    ) -> Result<GetPullResponse, ClientError> {
        let number = number.to_string();
        self.get(
            ["api", "v1", "repos", owner, repo, "pulls", &number],
            Vec::new(),
        )
        .await
    }

    pub async fn list_runs(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<ListRunsResponse, ClientError> {
        self.list_runs_with_query(owner, repo, ListRunsQuery::default())
            .await
    }

    pub async fn list_runs_with_query(
        &self,
        owner: &str,
        repo: &str,
        query: ListRunsQuery,
    ) -> Result<ListRunsResponse, ClientError> {
        self.get(
            ["api", "v1", "repos", owner, repo, "runs"],
            query.into_pairs(),
        )
        .await
    }

    pub async fn get_action_run(&self, run_id: &str) -> Result<ActionRunDetail, ClientError> {
        self.get(["api", "v1", "actions", "runs", run_id], Vec::new())
            .await
    }

    pub async fn get_action_run_logs(
        &self,
        run_id: &str,
        query: ActionRunLogsQuery,
    ) -> Result<String, ClientError> {
        self.get_text(
            ["api", "v1", "actions", "runs", run_id, "logs"],
            query.into_pairs(),
        )
        .await
    }

    pub async fn list_activity(
        &self,
        owner: &str,
        repo: &str,
        query: ListActivityQuery<'_>,
    ) -> Result<ListActivityResponse, ClientError> {
        self.get(
            ["api", "v1", "repos", owner, repo, "activity"],
            query.into_pairs(),
        )
        .await
    }

    async fn get<const N: usize, T>(
        &self,
        path: [&str; N],
        query: Vec<(&'static str, String)>,
    ) -> Result<T, ClientError>
    where
        T: DeserializeOwned,
    {
        let mut url = self.base_url.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| ClientError::InvalidUrlBase)?;
            segments.pop_if_empty();
            segments.extend(path);
        }
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, &value);
            }
        }

        let mut request = self.http.request(Method::GET, url);
        if let Some(token) = &self.token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {}", token));
        }
        let response = request.send().await.map_err(ClientError::Transport)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(ClientError::Transport)?;
        if !status.is_success() {
            let error = serde_json::from_slice::<ErrorResponse>(&bytes)
                .ok()
                .and_then(|body| body.error)
                .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
            return Err(ClientError::Api { status, error });
        }
        serde_json::from_slice(&bytes).map_err(ClientError::Decode)
    }

    async fn get_text<const N: usize>(
        &self,
        path: [&str; N],
        query: Vec<(&'static str, String)>,
    ) -> Result<String, ClientError> {
        let mut url = self.base_url.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| ClientError::InvalidUrlBase)?;
            segments.pop_if_empty();
            segments.extend(path);
        }
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, &value);
            }
        }

        let mut request = self.http.request(Method::GET, url);
        if let Some(token) = &self.token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {}", token));
        }
        let response = request.send().await.map_err(ClientError::Transport)?;
        let status = response.status();
        let text = response.text().await.map_err(ClientError::Transport)?;
        if !status.is_success() {
            return Err(ClientError::Api {
                status,
                error: text,
            });
        }
        Ok(text)
    }
}

/// Request body for publish/preflight operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishRequest {
    pub name: String,
    pub version: String,
    pub repo: String,
    pub git_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<serde_json::Value>,
}

/// Result of a preflight check.
#[derive(Debug, Clone, Deserialize)]
pub struct PreflightResult {
    pub allowed: bool,
    pub required_bump: Option<String>,
    pub minimum_allowed_version: Option<String>,
    pub reasons: Option<Vec<String>>,
}

/// Result of a publish operation.
#[derive(Debug, Clone, Deserialize)]
pub struct PublishResult {
    pub package_name: String,
    pub version: String,
}

/// Parse a scoped package name like `@scope/name` into `("scope", "name")`.
pub fn parse_scoped_name(name: &str) -> anyhow::Result<(String, String)> {
    if !name.starts_with('@') {
        anyhow::bail!("package name must start with @: {}", name);
    }
    let without_at = &name[1..];
    let base = if let Some(idx) = without_at.find('@') {
        &without_at[..idx]
    } else {
        without_at
    };
    let mut parts = base.split('/');
    let scope = parts.next().unwrap_or("").to_string();
    let pkg = parts.next().unwrap_or("").to_string();
    if scope.is_empty() || pkg.is_empty() || parts.next().is_some() {
        anyhow::bail!(
            "invalid scoped package name: {} (expected @scope/name)",
            name
        );
    }
    Ok((scope, pkg))
}

/// Check if a package spec looks like a scoped PHPX package.
pub fn is_phpx_package(name: &str) -> bool {
    name.starts_with('@')
}

#[derive(Debug)]
pub enum ClientError {
    InvalidBaseUrl(String),
    UntrustedBaseUrl(String),
    InvalidUrlBase,
    Transport(reqwest::Error),
    Decode(serde_json::Error),
    Api { status: StatusCode, error: String },
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBaseUrl(error) => write!(formatter, "invalid base URL: {error}"),
            Self::UntrustedBaseUrl(error) => write!(formatter, "untrusted base URL: {error}"),
            Self::InvalidUrlBase => {
                formatter.write_str("base URL cannot be a cannot-be-a-base URL")
            }
            Self::Transport(error) => write!(formatter, "request failed: {error}"),
            Self::Decode(error) => write!(formatter, "failed to decode response: {error}"),
            Self::Api { status, error } => {
                write!(formatter, "linkhash API returned {status}: {error}")
            }
        }
    }
}

impl Error for ClientError {}

fn validate_base_url(base_url: &Url, allow_loopback_http: bool) -> Result<(), ClientError> {
    let scheme = base_url.scheme();
    let host = base_url
        .host_str()
        .ok_or_else(|| ClientError::UntrustedBaseUrl("missing host".into()))?;
    if scheme == "https" && is_trusted_first_party_api_host(host) {
        return Ok(());
    }
    if allow_loopback_http && scheme == "http" && is_loopback_host(host) {
        return Ok(());
    }
    Err(ClientError::UntrustedBaseUrl(
        "credentials are only sent to HTTPS first-party Linkhash API hosts".into(),
    ))
}

fn is_trusted_first_party_api_host(host: &str) -> bool {
    TRUSTED_FIRST_PARTY_API_HOSTS
        .iter()
        .any(|trusted| host.eq_ignore_ascii_case(trusted))
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ListPackagesQuery<'a> {
    pub ecosystem: Option<&'a str>,
    pub namespace: Option<&'a str>,
}

impl ListPackagesQuery<'_> {
    fn into_pairs(self) -> Vec<(&'static str, String)> {
        optional_pairs([("ecosystem", self.ecosystem), ("namespace", self.namespace)])
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ListIssuesQuery<'a> {
    pub state: Option<&'a str>,
    pub assignee: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub repo: Option<&'a str>,
}

impl ListIssuesQuery<'_> {
    fn into_pairs(self) -> Vec<(&'static str, String)> {
        optional_pairs([
            ("state", self.state),
            ("assignee", self.assignee),
            ("priority", self.priority),
            ("repo", self.repo),
        ])
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ListPullsQuery<'a> {
    pub state: Option<&'a str>,
}

impl ListPullsQuery<'_> {
    fn into_pairs(self) -> Vec<(&'static str, String)> {
        optional_pairs([("state", self.state)])
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ListRunsQuery {
    pub limit: Option<usize>,
}

impl ListRunsQuery {
    fn into_pairs(self) -> Vec<(&'static str, String)> {
        self.limit
            .map(|limit| vec![("limit", limit.to_string())])
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ActionRunLogsQuery {
    pub limit: Option<usize>,
}

impl ActionRunLogsQuery {
    fn into_pairs(self) -> Vec<(&'static str, String)> {
        self.limit
            .map(|limit| vec![("limit", limit.to_string())])
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ListActivityQuery<'a> {
    pub event_type: Option<&'a str>,
    pub actor: Option<&'a str>,
    pub subject_type: Option<&'a str>,
    pub after_number: Option<i64>,
    pub limit: Option<usize>,
}

impl ListActivityQuery<'_> {
    fn into_pairs(self) -> Vec<(&'static str, String)> {
        let mut pairs = optional_pairs([
            ("event_type", self.event_type),
            ("actor", self.actor),
            ("subject_type", self.subject_type),
        ]);
        if let Some(after_number) = self.after_number {
            pairs.push(("after_number", after_number.to_string()));
        }
        if let Some(limit) = self.limit {
            pairs.push(("limit", limit.to_string()));
        }
        pairs
    }
}

fn optional_pairs<const N: usize>(
    pairs: [(&'static str, Option<&str>); N],
) -> Vec<(&'static str, String)> {
    pairs
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value.to_owned())))
        .collect()
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub service: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ListReposResponse {
    pub repos: Vec<Repo>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Repo {
    pub id: String,
    pub slug: String,
    pub owner: String,
    pub name: String,
    pub description: Option<String>,
    pub last_activity: Option<i64>,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ListPackagesResponse {
    pub packages: Vec<Package>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Package {
    pub id: String,
    pub ecosystem: String,
    pub namespace: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ListVersionsResponse {
    pub package: Package,
    pub versions: Vec<Version>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Version {
    pub id: String,
    pub version: String,
    pub published_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ListIssuesResponse {
    pub issues: Vec<Issue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GetIssueResponse {
    pub issue: Issue,
    pub comments: Vec<Comment>,
    pub labels: Vec<Label>,
    pub commits: Vec<Commit>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Issue {
    pub id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub number: i64,
    pub title: String,
    pub body: String,
    pub state: String,
    pub author: String,
    pub assignee: String,
    pub priority: String,
    pub repo: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ListPullsResponse {
    pub pulls: Vec<Pull>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GetPullResponse {
    pub pull: Pull,
    pub comments: Vec<Comment>,
    pub review_threads: Vec<ReviewThread>,
    pub reviews: Vec<Review>,
    pub status_checks: Vec<StatusCheck>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Pull {
    pub id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub number: i64,
    pub title: String,
    pub body: String,
    pub state: String,
    pub author: String,
    pub source_ref: String,
    pub target_ref: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: String,
    pub merge_commit_sha: String,
    pub merged_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Comment {
    pub id: String,
    pub body: String,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Label {
    pub id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub name: String,
    pub color: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Commit {
    pub id: String,
    pub commit_hash: String,
    pub repo: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReviewThread {
    pub id: String,
    pub pull_id: String,
    pub path: String,
    pub line: String,
    pub status: String,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Review {
    pub id: String,
    pub pull_id: String,
    pub author: String,
    pub state: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StatusCheck {
    pub id: String,
    pub pull_id: String,
    pub name: String,
    pub status: String,
    pub conclusion: String,
    pub commit_sha: String,
    pub target_url: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ListRunsResponse {
    pub runs: Vec<ActionRun>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionRun {
    pub run_id: String,
    pub workflow_name: String,
    pub commit_sha: String,
    pub trigger: String,
    pub finished_at: Option<i64>,
    pub id: String,
    pub repo_id: String,
    pub action_name: String,
    pub event: String,
    pub git_ref: String,
    pub sha: String,
    pub status: String,
    pub started_at: i64,
    pub completed_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionRunDetail {
    pub run_id: String,
    pub repo: String,
    pub workflow: String,
    pub commit: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub artifacts: Vec<ActionArtifact>,
    pub run: ActionRun,
    pub jobs: Vec<ActionJobRun>,
    pub steps: Vec<ActionStepRun>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionJobRun {
    pub id: String,
    pub run_id: String,
    pub job_id: String,
    pub name: String,
    pub status: String,
    pub started_at: i64,
    pub completed_at: i64,
    pub steps: Vec<ActionStepRun>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionStepRun {
    #[serde(default)]
    pub job_id: Option<String>,
    pub id: String,
    pub job_run_id: String,
    pub step_index: i64,
    pub name: String,
    pub status: String,
    pub exit_code: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub completed_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionArtifact {
    pub path: String,
    pub sha256: String,
    pub cas_ref: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ListActivityResponse {
    pub activity: Vec<Event>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Event {
    pub id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub number: i64,
    pub event_type: String,
    pub actor: String,
    pub subject_type: String,
    pub subject_id: String,
    pub subject_number: i64,
    pub subject_title: String,
    pub timestamp: String,
    pub payload_json: String,
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::{
        extract::State,
        http::{header, HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
        Json, Router,
    };
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Clone)]
    struct StubState {
        token: &'static str,
    }

    #[tokio::test]
    async fn sends_bearer_auth_and_decodes_success_response() {
        let address = spawn_stub().await;
        let client = LinkhashClient::with_http_client(
            format!("http://{address}"),
            "idp-token",
            HttpClient::new(),
        )
        .unwrap();

        let response = client
            .list_packages(ListPackagesQuery {
                ecosystem: Some("phpx"),
                namespace: Some("tana"),
            })
            .await
            .unwrap();

        assert_eq!(response.packages[0].name, "store");
    }

    #[tokio::test]
    async fn surfaces_api_errors() {
        let address = spawn_stub().await;
        let client = LinkhashClient::with_http_client(
            format!("http://{address}"),
            "wrong-token",
            HttpClient::new(),
        )
        .unwrap();

        let error = client.list_repos().await.unwrap_err();

        assert!(matches!(
            error,
            ClientError::Api {
                status: StatusCode::UNAUTHORIZED,
                ..
            }
        ));
    }

    #[test]
    fn debug_redacts_bearer_token() {
        let token = "idp-token-secret-value";
        let client = LinkhashClient::new("https://linkhash.test", Some(token));

        let debug = format!("{client:?}");

        assert!(debug.contains("LinkhashClient"));
        assert!(debug.contains("base_url"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(token));
    }

    #[test]
    fn rejects_untrusted_base_urls_before_requests_can_be_sent() {
        let error = LinkhashClient::with_http_client(
            "https://attacker.example",
            "idp-token",
            HttpClient::new(),
        )
        .unwrap_err();

        assert!(matches!(error, ClientError::UntrustedBaseUrl(_)));
    }

    #[test]
    fn strict_constructor_rejects_cleartext_even_for_loopback() {
        let error = LinkhashClient::try_new("http://127.0.0.1:8080", "idp-token").unwrap_err();

        assert!(matches!(error, ClientError::UntrustedBaseUrl(_)));
    }

    #[test]
    fn accepts_https_first_party_api_hosts() {
        for host in TRUSTED_FIRST_PARTY_API_HOSTS {
            let client = LinkhashClient::try_new(format!("https://{host}"), "idp-token").unwrap();

            assert_eq!(client.base_url.host_str(), Some(*host));
        }
    }

    async fn spawn_stub() -> SocketAddr {
        let app = Router::new()
            .route("/api/v1/repos", get(repos))
            .route("/api/v1/packages", get(packages))
            .with_state(StubState { token: "idp-token" });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        address
    }

    fn require_auth(headers: &HeaderMap, state: &StubState) -> Result<(), StatusCode> {
        match headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        {
            Some(value) if value == format!("Bearer {}", state.token) => Ok(()),
            _ => Err(StatusCode::UNAUTHORIZED),
        }
    }

    async fn repos(State(state): State<StubState>, headers: HeaderMap) -> impl IntoResponse {
        if let Err(status) = require_auth(&headers, &state) {
            return (status, Json(serde_json::json!({ "error": "unauthorized" }))).into_response();
        }
        Json(serde_json::json!({
            "repos": [{
                "id": "tana/linkhash",
                "slug": "tana/linkhash",
                "owner": "tana",
                "name": "linkhash",
                "description": null,
                "last_activity": 10,
                "url": "https://git.linkha.sh/tana/linkhash"
            }]
        }))
        .into_response()
    }

    async fn packages(
        State(state): State<StubState>,
        headers: HeaderMap,
        uri: axum::http::Uri,
    ) -> impl IntoResponse {
        if let Err(status) = require_auth(&headers, &state) {
            return (status, Json(serde_json::json!({ "error": "unauthorized" }))).into_response();
        }
        assert_eq!(uri.query(), Some("ecosystem=phpx&namespace=tana"));
        Json(serde_json::json!({
            "packages": [{
                "id": "package:phpx:tana/store",
                "ecosystem": "phpx",
                "namespace": "tana",
                "name": "store"
            }]
        }))
        .into_response()
    }
}
