use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use std::process::Command;

pub async fn advertise_refs(
    owner: &str,
    repo: &str,
    service: &str,
    git_protocol: Option<&str>,
) -> Result<Response, anyhow::Error> {
    let repo_path = crate::repo::storage::get_repo_path(owner, repo);

    if !repo_path.exists() {
        return Ok((StatusCode::NOT_FOUND, "Repository not found").into_response());
    }

    let git_service = match service {
        "git-upload-pack" => "git-upload-pack",
        "git-receive-pack" => "git-receive-pack",
        _ => return Ok((StatusCode::BAD_REQUEST, "Unsupported service").into_response()),
    };

    let mut lines = Vec::new();
    if !is_protocol_v2(git_protocol) {
        lines.extend_from_slice(&pkt_line(&format!("# service={}\n", service)));
        lines.extend_from_slice(&pkt_flush());
    }

    let mut command = Command::new(git_service);
    command
        .arg("--stateless-rpc")
        .arg("--advertise-refs")
        .arg(&repo_path);
    if let Some(protocol) = git_protocol {
        command.env("GIT_PROTOCOL", protocol);
    }

    let output = command.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("{} --advertise-refs failed: {}", git_service, stderr);
        return Ok((StatusCode::INTERNAL_SERVER_ERROR, "Failed to advertise refs").into_response());
    }

    lines.extend_from_slice(&output.stdout);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            format!("application/x-{}-advertisement", service),
        )
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from(lines))
        .unwrap())
}

fn pkt_line(data: &str) -> Vec<u8> {
    let len = data.len() + 4;
    format!("{:04x}{}", len, data).into_bytes()
}

fn pkt_flush() -> Vec<u8> {
    b"0000".to_vec()
}

fn is_protocol_v2(git_protocol: Option<&str>) -> bool {
    git_protocol
        .map(|value| value.split(':').any(|part| part.trim() == "version=2"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkt_line_format() {
        let line = pkt_line("hello\n");
        assert_eq!(line, b"000ahello\n");
    }

    #[test]
    fn test_pkt_flush() {
        let flush = pkt_flush();
        assert_eq!(flush, b"0000");
    }

    #[test]
    fn test_protocol_v2_detection() {
        assert!(is_protocol_v2(Some("version=2")));
        assert!(is_protocol_v2(Some("foo=bar:version=2")));
        assert!(!is_protocol_v2(Some("version=1")));
        assert!(!is_protocol_v2(None));
    }
}
