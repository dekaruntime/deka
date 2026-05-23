use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateJobRequest {
    pub subdomain: String,
    pub address: String,
    pub git_repo: String,
    pub install_command: Option<String>,
    pub build_command: Option<String>,
    pub output_dir: Option<String>,
    /// Delegate JWT for auto-deploy (rapid deploy mode)
    /// When present, tana-deploy can sign transactions on behalf of the user
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegate_jwt: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct JobResponse {
    pub id: String,
    pub subdomain: String,
    #[allow(dead_code)]
    pub status: String,
}

/// Trigger a build in tana-deploy service
///
/// If `delegate_jwt` is provided (rapid deploy mode), it will be passed to tana-deploy
/// which can use it to sign transactions on behalf of the user.
pub async fn trigger_build(
    _network: &str,
    address: &str,
    repo: &str,
    repo_path: &str,
    delegate_jwt: Option<&str>,
) -> Result<JobResponse, anyhow::Error> {
    let deploy_url =
        std::env::var("DEPLOY_URL").unwrap_or_else(|_| "http://localhost:8509".to_string());

    // Extract subdomain from repo name (remove .git suffix)
    let subdomain = repo.strip_suffix(".git").unwrap_or(repo);

    let request = CreateJobRequest {
        subdomain: format!("{}-{}", address, subdomain),
        address: address.to_string(),
        git_repo: format!("file://{}", repo_path),
        install_command: Some("npm install".to_string()),
        build_command: Some("npm run build".to_string()),
        output_dir: Some("dist".to_string()),
        delegate_jwt: delegate_jwt.map(|s| s.to_string()),
    };

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/jobs", deploy_url))
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        anyhow::bail!("Deploy service returned error: {}", error_text);
    }

    let job: JobResponse = response.json().await?;
    Ok(job)
}
