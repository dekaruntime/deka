//! Post-receive hooks — triggered after a successful git push.
//!
//! When code is pushed to a store repo, determines which refs were updated
//! and triggers a rebuild of the tenant's live bundle if `main` was pushed.

use std::process::Command;

/// Information about a ref update from a push.
#[derive(Debug)]
pub struct RefUpdate {
    pub old_hash: String,
    pub new_hash: String,
    pub ref_name: String,
}

/// Parse the refs that were updated by inspecting the receive-pack body.
///
/// The git smart HTTP protocol sends ref updates as pkt-line encoded data.
/// Each pkt-line starts with a 4-hex-digit length (including the 4 bytes
/// themselves), followed by the payload. `0000` is the flush packet.
/// Each update payload has the format: `old-hash new-hash ref-name[\0capabilities]\n`.
pub fn parse_ref_updates(body: &[u8]) -> Vec<RefUpdate> {
    let mut updates = Vec::new();
    let mut pos = 0;

    while pos + 4 <= body.len() {
        // Read 4-byte hex length
        let len_str = match std::str::from_utf8(&body[pos..pos + 4]) {
            Ok(s) => s,
            Err(_) => break,
        };
        let pkt_len = match usize::from_str_radix(len_str, 16) {
            Ok(n) => n,
            Err(_) => break,
        };

        // Flush packet
        if pkt_len == 0 {
            pos += 4;
            continue;
        }

        if pkt_len < 4 || pos + pkt_len > body.len() {
            break;
        }

        let payload = &body[pos + 4..pos + pkt_len];
        pos += pkt_len;

        // Convert payload to string, strip trailing newline
        let text = String::from_utf8_lossy(payload);
        let text = text.trim_end_matches('\n');

        // Strip NUL + capabilities if present
        let text = text.split('\0').next().unwrap_or(text);

        // Parse "old_hash new_hash ref_name"
        let parts: Vec<&str> = text.splitn(3, ' ').collect();
        if parts.len() >= 3 {
            let old = parts[0];
            let new = parts[1];
            let ref_name = parts[2];

            if old.len() == 40
                && new.len() == 40
                && old.chars().all(|c| c.is_ascii_hexdigit())
                && new.chars().all(|c| c.is_ascii_hexdigit())
                && ref_name.starts_with("refs/")
            {
                updates.push(RefUpdate {
                    old_hash: old.to_string(),
                    new_hash: new.to_string(),
                    ref_name: ref_name.to_string(),
                });
            }
        }
    }

    updates
}

/// Extract shop_id from a repo name.
/// Repo names are like "shop_beta" (or "shop_beta.git").
/// Returns the repo name stripped of .git suffix.
pub fn shop_id_from_repo(repo: &str) -> String {
    repo.strip_suffix(".git").unwrap_or(repo).to_string()
}

/// Check if a ref update targets the main branch.
pub fn is_main_branch(ref_name: &str) -> bool {
    ref_name == "refs/heads/main"
}

/// Trigger a rebuild of a tenant's bundle on the Deka platform.
/// Sends POST to the platform's admin rebuild endpoint.
/// If `git_ref` is provided, the bundle is cached as `shop_id:ref` (preview build).
pub async fn trigger_rebuild(shop_id: &str, git_ref: Option<&str>) -> Result<String, String> {
    let mut url = format!(
        "http://localhost:8530/__admin/rebuild?shop_id={}",
        shop_id
    );
    if let Some(r) = git_ref {
        url.push_str(&format!("&ref={}", r));
    }

    let label = if git_ref.is_some() { "preview rebuild" } else { "rebuild" };
    tracing::info!("Triggering {} for shop_id={} ref={:?}", label, shop_id, git_ref);

    let output = tokio::process::Command::new("curl")
        .args([
            "-s",
            "-X", "POST",
            "-o", "/dev/null",
            "-w", "%{http_code}",
            &url,
        ])
        .output()
        .await
        .map_err(|e| format!("failed to spawn curl: {}", e))?;

    let status = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if status == "200" {
        tracing::info!("{} succeeded for shop_id={} ref={:?}", label, shop_id, git_ref);
        Ok(status)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = format!(
            "{} returned status {} for shop_id={} ref={:?}: {}",
            label, status, shop_id, git_ref, stderr
        );
        tracing::warn!("{}", msg);
        Err(msg)
    }
}

/// Extract a short (7-char) commit hash from a full 40-char hash.
pub fn short_hash(full_hash: &str) -> &str {
    &full_hash[..7.min(full_hash.len())]
}

/// Full post-receive hook: parse updates, trigger rebuild.
/// - Main branch pushes: rebuild the main bundle (shop_id cache key).
/// - Non-main branch pushes: build a preview bundle (shop_id:hash cache key).
pub async fn run_post_receive(
    owner: &str,
    repo: &str,
    body: &[u8],
    auth_token_id: Option<i64>,
    auth_key_type: &str,
    auth_owner: &str,
) {
    let updates = parse_ref_updates(body);
    let shop_id = shop_id_from_repo(repo);

    for update in &updates {
        tracing::debug!(
            "Ref update: {} {} -> {} {}",
            update.ref_name,
            &update.old_hash[..8],
            &update.new_hash[..8],
            shop_id
        );

        if is_main_branch(&update.ref_name) {
            // Main branch — production rebuild
            crate::auth::log_audit(
                auth_token_id,
                auth_key_type,
                auth_owner,
                "deploy",
                Some(&format!("{}/{}", owner, repo)),
                Some("refs/heads/main"),
                Some(&format!(
                    "push {} -> {}, triggering rebuild",
                    &update.old_hash[..8.min(update.old_hash.len())],
                    &update.new_hash[..8.min(update.new_hash.len())]
                )),
                None,
            )
            .await;

            match trigger_rebuild(&shop_id, None).await {
                Ok(_) => {
                    tracing::info!("Deploy complete: {}/{} -> main", owner, shop_id);
                }
                Err(e) => {
                    tracing::error!("Deploy rebuild failed for {}/{}: {}", owner, shop_id, e);
                }
            }
        } else if update.ref_name.starts_with("refs/heads/") {
            // Non-main branch — preview build keyed by short commit hash
            let hash = short_hash(&update.new_hash);

            crate::auth::log_audit(
                auth_token_id,
                auth_key_type,
                auth_owner,
                "preview",
                Some(&format!("{}/{}", owner, repo)),
                Some(&update.ref_name),
                Some(&format!(
                    "push {} -> {}, triggering preview build (preview-{}-{}.tana.gg)",
                    &update.old_hash[..8.min(update.old_hash.len())],
                    &update.new_hash[..8.min(update.new_hash.len())],
                    hash,
                    shop_id
                )),
                None,
            )
            .await;

            match trigger_rebuild(&shop_id, Some(hash)).await {
                Ok(_) => {
                    tracing::info!(
                        "Preview build complete: {}/{} -> {} (preview-{}-{}.tana.gg)",
                        owner, shop_id, update.ref_name, hash, shop_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Preview rebuild failed for {}/{} ref {}: {}",
                        owner, shop_id, update.ref_name, e
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pkt_line(payload: &[u8]) -> Vec<u8> {
        let len = payload.len() + 4;
        let mut pkt = format!("{:04x}", len).into_bytes();
        pkt.extend_from_slice(payload);
        pkt
    }

    fn make_flush() -> Vec<u8> {
        b"0000".to_vec()
    }

    #[test]
    fn parse_ref_updates_extracts_main_push() {
        let mut body = make_pkt_line(
            b"0000000000000000000000000000000000000000 abcdef1234567890abcdef1234567890abcdef12 refs/heads/main\0 report-status\n",
        );
        body.extend(make_flush());
        let updates = parse_ref_updates(&body);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].ref_name, "refs/heads/main");
        assert_eq!(
            updates[0].new_hash,
            "abcdef1234567890abcdef1234567890abcdef12"
        );
    }

    #[test]
    fn parse_ref_updates_handles_non_main_branch() {
        let mut body = make_pkt_line(
            b"0000000000000000000000000000000000000000 abcdef1234567890abcdef1234567890abcdef12 refs/heads/feature-x\n",
        );
        body.extend(make_flush());
        let updates = parse_ref_updates(&body);
        assert_eq!(updates.len(), 1);
        assert!(!is_main_branch(&updates[0].ref_name));
    }

    #[test]
    fn shop_id_strips_git_suffix() {
        assert_eq!(shop_id_from_repo("shop_beta.git"), "shop_beta");
        assert_eq!(shop_id_from_repo("shop_beta"), "shop_beta");
    }

    #[test]
    fn is_main_branch_checks_correctly() {
        assert!(is_main_branch("refs/heads/main"));
        assert!(!is_main_branch("refs/heads/feature"));
        assert!(!is_main_branch("refs/tags/v1.0"));
    }

    #[test]
    fn short_hash_extracts_7_chars() {
        assert_eq!(short_hash("abcdef1234567890abcdef1234567890abcdef12"), "abcdef1");
    }

    #[test]
    fn short_hash_short_input() {
        assert_eq!(short_hash("abc"), "abc");
    }
}
