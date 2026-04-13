use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use std::fs;
use std::path::Path;

pub async fn advertise_refs(
    owner: &str,
    repo: &str,
    service: &str,
) -> Result<Response, anyhow::Error> {
    let repo_path = crate::repo::storage::get_repo_path(owner, repo);

    if !repo_path.exists() {
        return Ok((StatusCode::NOT_FOUND, "Repository not found").into_response());
    }

    let mut lines = Vec::new();
    lines.extend_from_slice(&pkt_line(&format!("# service={}\n", service)));
    lines.extend_from_slice(&pkt_flush());

    let refs = read_refs(&repo_path)?;

    if refs.is_empty() {
        lines.extend_from_slice(&pkt_line("0000000000000000000000000000000000000000 capabilities^{}\0report-status delete-refs side-band-64k quiet atomic ofs-delta agent=deka-git/0.7.0\n"));
    } else {
        let (ref_name, sha) = &refs[0];
        let line = format!(
            "{} {}\0report-status delete-refs side-band-64k quiet atomic ofs-delta agent=deka-git/0.7.0\n",
            sha, ref_name
        );
        lines.extend_from_slice(&pkt_line(&line));

        for (ref_name, sha) in refs.iter().skip(1) {
            let line = format!("{} {}\n", sha, ref_name);
            lines.extend_from_slice(&pkt_line(&line));
        }
    }

    lines.extend_from_slice(&pkt_flush());

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

fn read_refs(repo_path: &Path) -> Result<Vec<(String, String)>, anyhow::Error> {
    let mut refs = Vec::new();

    let heads_dir = repo_path.join("refs/heads");
    if heads_dir.exists() {
        for entry in fs::read_dir(heads_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let ref_name = format!("refs/heads/{}", entry.file_name().to_string_lossy());
                let sha = fs::read_to_string(entry.path())?.trim().to_string();
                refs.push((ref_name, sha));
            }
        }
    }

    let tags_dir = repo_path.join("refs/tags");
    if tags_dir.exists() {
        for entry in fs::read_dir(tags_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let ref_name = format!("refs/tags/{}", entry.file_name().to_string_lossy());
                let sha = fs::read_to_string(entry.path())?.trim().to_string();
                refs.push((ref_name, sha));
            }
        }
    }

    Ok(refs)
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
}
