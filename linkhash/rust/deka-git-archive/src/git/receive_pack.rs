use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use std::process::{Command, Stdio};

pub async fn handle(
    owner: &str,
    repo: &str,
    body: bytes::Bytes,
) -> Result<Response, anyhow::Error> {
    let repo_path = crate::repo::storage::get_repo_path(owner, repo);

    if !repo_path.exists() {
        return Ok((StatusCode::NOT_FOUND, "Repository not found").into_response());
    }

    tracing::debug!("Spawning git-receive-pack for {}", repo_path.display());

    let mut child = Command::new("git-receive-pack")
        .arg("--stateless-rpc")
        .arg(&repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let mut stdin_async =
            tokio::io::BufWriter::new(tokio::process::ChildStdin::from_std(stdin)?);
        stdin_async.write_all(&body).await?;
        stdin_async.flush().await?;
        drop(stdin_async);
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git-receive-pack failed: {}", stderr);
        return Ok((StatusCode::INTERNAL_SERVER_ERROR, "Push failed").into_response());
    }

    tracing::info!("Push successful for {}/{}", owner, repo);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            "application/x-git-receive-pack-result",
        )
        .body(axum::body::Body::from(output.stdout))
        .unwrap())
}
