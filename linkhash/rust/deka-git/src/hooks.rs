//! Post-receive hooks — triggered after a successful git push.
//!
//! When code is pushed to a repo, determines which refs were updated and runs
//! the configured post-receive side effects.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;

type HmacSha256 = Hmac<Sha256>;

/// Information about a ref update from a push.
#[derive(Debug)]
pub struct RefUpdate {
    pub old_hash: String,
    pub new_hash: String,
    pub ref_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DeployWatcher {
    pub id: i64,
    pub repo_owner: String,
    pub repo_name: String,
    pub watcher_url: String,
    pub active: i64,
    pub created_by: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub last_branch: Option<String>,
    pub last_sha: Option<String>,
    pub last_delivery_status: Option<String>,
    pub last_delivery_code: Option<i64>,
    pub last_delivery_error: Option<String>,
    pub last_delivery_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct DeployWatcherPayload<'a> {
    repo: &'a str,
    branch: &'a str,
    sha: &'a str,
    pusher_account: &'a str,
    ts: i64,
}

#[derive(Debug)]
struct DeliveryOutcome {
    ok: bool,
    status_code: Option<u16>,
    error: Option<String>,
}

const DEPLOY_SIGNATURE_HEADER: &str = "X-Linkhash-Signature";
const DEPLOY_HMAC_ENV: &str = "LINKHASH_DEPLOY_HMAC_KEY";
const RETRY_DELAYS: [u64; 3] = [5, 15, 45];

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

pub fn branch_from_ref(ref_name: &str) -> Option<&str> {
    ref_name.strip_prefix("refs/heads/")
}

pub fn sign_deploy_payload(secret: &str, body: &[u8]) -> Result<String, String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("invalid HMAC key: {}", e))?;
    mac.update(body);
    Ok(format!(
        "sha256={}",
        hex::encode(mac.finalize().into_bytes())
    ))
}

fn now_unix_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub async fn list_deploy_watchers() -> Result<Vec<DeployWatcher>, sqlx::Error> {
    sqlx::query_as::<_, DeployWatcher>(
        r#"
        SELECT id, repo_owner, repo_name, watcher_url, active, created_by, created_at, updated_at,
               last_branch, last_sha, last_delivery_status, last_delivery_code,
               last_delivery_error, last_delivery_at
        FROM deploy_watchers
        ORDER BY repo_owner, repo_name
        "#,
    )
    .fetch_all(crate::db::pool())
    .await
}

pub async fn upsert_deploy_watcher(
    repo_owner: &str,
    repo_name: &str,
    watcher_url: &str,
    active: bool,
    created_by: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO deploy_watchers (repo_owner, repo_name, watcher_url, active, created_by)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(repo_owner, repo_name) DO UPDATE SET
            watcher_url = excluded.watcher_url,
            active = excluded.active,
            updated_at = datetime('now')
        "#,
    )
    .bind(repo_owner)
    .bind(repo_name)
    .bind(watcher_url)
    .bind(if active { 1 } else { 0 })
    .bind(created_by)
    .execute(crate::db::pool())
    .await?;
    Ok(())
}

async fn watcher_for_repo(owner: &str, repo: &str) -> Result<Option<DeployWatcher>, sqlx::Error> {
    sqlx::query_as::<_, DeployWatcher>(
        r#"
        SELECT id, repo_owner, repo_name, watcher_url, active, created_by, created_at, updated_at,
               last_branch, last_sha, last_delivery_status, last_delivery_code,
               last_delivery_error, last_delivery_at
        FROM deploy_watchers
        WHERE repo_owner = ? AND repo_name = ? AND active = 1
        "#,
    )
    .bind(owner)
    .bind(repo.strip_suffix(".git").unwrap_or(repo))
    .fetch_optional(crate::db::pool())
    .await
}

async fn update_watcher_status(
    watcher_id: i64,
    branch: &str,
    sha: &str,
    outcome: &DeliveryOutcome,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE deploy_watchers
        SET last_branch = ?,
            last_sha = ?,
            last_delivery_status = ?,
            last_delivery_code = ?,
            last_delivery_error = ?,
            last_delivery_at = datetime('now'),
            updated_at = datetime('now')
        WHERE id = ?
        "#,
    )
    .bind(branch)
    .bind(sha)
    .bind(if outcome.ok { "ok" } else { "failed" })
    .bind(outcome.status_code.map(i64::from))
    .bind(outcome.error.as_deref())
    .bind(watcher_id)
    .execute(crate::db::pool())
    .await?;
    Ok(())
}

async fn post_deploy_watcher_once(
    client: &reqwest::Client,
    url: &str,
    body: Vec<u8>,
    signature: String,
) -> DeliveryOutcome {
    match client
        .post(url)
        .header("Content-Type", "application/json")
        .header(DEPLOY_SIGNATURE_HEADER, signature)
        .body(body)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                DeliveryOutcome {
                    ok: true,
                    status_code: Some(status.as_u16()),
                    error: None,
                }
            } else {
                let text = response.text().await.unwrap_or_default();
                DeliveryOutcome {
                    ok: false,
                    status_code: Some(status.as_u16()),
                    error: Some(if text.is_empty() {
                        format!("watcher returned {}", status)
                    } else {
                        format!("watcher returned {}: {}", status, text)
                    }),
                }
            }
        }
        Err(e) => DeliveryOutcome {
            ok: false,
            status_code: None,
            error: Some(e.to_string()),
        },
    }
}

async fn append_deploy_failure_log(line: &str) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".linkhash");
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        tracing::error!("failed to create deploy failure log dir: {}", e);
        return;
    }
    let path = dir.join("deploy-failures.log");
    match tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
    {
        Ok(mut file) => {
            let _ = file.write_all(line.as_bytes()).await;
            let _ = file.write_all(b"\n").await;
        }
        Err(e) => tracing::error!("failed to open {}: {}", path.display(), e),
    }
}

async fn comment_on_infra_issue(owner: &str, repo: &str, body: String) {
    let row: Result<Option<(i64,)>, sqlx::Error> = sqlx::query_as(
        r#"
        SELECT number FROM issues
        WHERE repo_owner = ? AND repo_name = ? AND state != 'closed'
          AND (upper(title) LIKE '%#INFRA%' OR upper(title) LIKE '%INFRA%')
        ORDER BY number ASC
        LIMIT 1
        "#,
    )
    .bind(owner)
    .bind(repo.strip_suffix(".git").unwrap_or(repo))
    .fetch_optional(crate::db::pool())
    .await;

    if let Ok(Some((number,))) = row {
        let req = crate::issues::CreateCommentRequest { body };
        if let Err(e) = crate::issues::add_comment(
            owner,
            repo.strip_suffix(".git").unwrap_or(repo),
            number,
            "linkhash-deploy",
            req,
        )
        .await
        {
            tracing::warn!(
                "failed to comment on #INFRA issue for {}/{}: {}",
                owner,
                repo,
                e
            );
        }
    }
}

async fn deliver_deploy_watcher_with_delays(
    watcher: &DeployWatcher,
    branch: &str,
    sha: &str,
    pusher_account: &str,
    retry_delays: &[u64],
) -> DeliveryOutcome {
    let secret = match std::env::var(DEPLOY_HMAC_ENV) {
        Ok(value) if !value.is_empty() => value,
        _ => {
            return DeliveryOutcome {
                ok: false,
                status_code: None,
                error: Some(format!("{} is not set", DEPLOY_HMAC_ENV)),
            }
        }
    };

    let repo = format!("{}/{}", watcher.repo_owner, watcher.repo_name);
    let payload = DeployWatcherPayload {
        repo: &repo,
        branch,
        sha,
        pusher_account,
        ts: now_unix_ts(),
    };
    let body = match serde_json::to_vec(&payload) {
        Ok(body) => body,
        Err(e) => {
            return DeliveryOutcome {
                ok: false,
                status_code: None,
                error: Some(e.to_string()),
            }
        }
    };
    let signature = match sign_deploy_payload(&secret, &body) {
        Ok(signature) => signature,
        Err(e) => {
            return DeliveryOutcome {
                ok: false,
                status_code: None,
                error: Some(e),
            }
        }
    };

    let client = reqwest::Client::new();
    let attempts = retry_delays.len() + 1;
    let mut last = DeliveryOutcome {
        ok: false,
        status_code: None,
        error: Some("not attempted".to_string()),
    };
    for attempt in 0..attempts {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(retry_delays[attempt - 1])).await;
        }
        last = post_deploy_watcher_once(
            &client,
            &watcher.watcher_url,
            body.clone(),
            signature.clone(),
        )
        .await;
        if last.ok {
            return last;
        }
    }
    last
}

async fn deliver_deploy_watcher(owner: &str, repo: &str, update: &RefUpdate, pusher_account: &str) {
    let branch = match branch_from_ref(&update.ref_name) {
        Some(branch) => branch,
        None => return,
    };
    let watcher = match watcher_for_repo(owner, repo).await {
        Ok(Some(watcher)) => watcher,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!("deploy watcher lookup failed for {}/{}: {}", owner, repo, e);
            return;
        }
    };

    let outcome = deliver_deploy_watcher_with_delays(
        &watcher,
        branch,
        &update.new_hash,
        pusher_account,
        &RETRY_DELAYS,
    )
    .await;
    if let Err(e) = update_watcher_status(watcher.id, branch, &update.new_hash, &outcome).await {
        tracing::warn!("failed to update deploy watcher status: {}", e);
    }
    if !outcome.ok {
        let error = outcome.error.as_deref().unwrap_or("delivery failed");
        let line = format!(
            "repo={}/{} branch={} sha={} pusher={} error={}",
            owner, repo, branch, update.new_hash, pusher_account, error
        );
        append_deploy_failure_log(&line).await;
        comment_on_infra_issue(
            owner,
            repo,
            format!(
                "Deploy watcher delivery failed for `{}/{}` `{}` at `{}`.\n\nError: {}",
                owner, repo, branch, update.new_hash, error
            ),
        )
        .await;
    }
}

/// Trigger a rebuild of a tenant's bundle on the Deka platform.
/// Sends POST to the platform's admin rebuild endpoint.
/// If `git_ref` is provided, the bundle is cached as `shop_id:ref` (preview build).
pub async fn trigger_rebuild(shop_id: &str, git_ref: Option<&str>) -> Result<String, String> {
    let mut url = format!("http://localhost:8530/__admin/rebuild?shop_id={}", shop_id);
    if let Some(r) = git_ref {
        url.push_str(&format!("&ref={}", r));
    }

    let label = if git_ref.is_some() {
        "preview rebuild"
    } else {
        "rebuild"
    };
    tracing::info!(
        "Triggering {} for shop_id={} ref={:?}",
        label,
        shop_id,
        git_ref
    );

    let output = tokio::process::Command::new("curl")
        .args([
            "-s",
            "-X",
            "POST",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            &url,
        ])
        .output()
        .await
        .map_err(|e| format!("failed to spawn curl: {}", e))?;

    let status = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if status == "200" {
        tracing::info!(
            "{} succeeded for shop_id={} ref={:?}",
            label,
            shop_id,
            git_ref
        );
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

        deliver_deploy_watcher(owner, repo, update, auth_owner).await;

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
                        owner,
                        shop_id,
                        update.ref_name,
                        hash,
                        shop_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Preview rebuild failed for {}/{} ref {}: {}",
                        owner,
                        shop_id,
                        update.ref_name,
                        e
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
        assert_eq!(
            short_hash("abcdef1234567890abcdef1234567890abcdef12"),
            "abcdef1"
        );
    }

    #[test]
    fn short_hash_short_input() {
        assert_eq!(short_hash("abc"), "abc");
    }

    #[test]
    fn deploy_hmac_signature_header_format() {
        let signature = sign_deploy_payload("topsecret", br#"{"repo":"tana/deka"}"#).unwrap();
        assert!(signature.starts_with("sha256="));
        assert_eq!(signature.len(), "sha256=".len() + 64);
        assert!(signature["sha256=".len()..]
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn deploy_watcher_delivery_posts_signed_payload() {
        std::env::set_var(DEPLOY_HMAC_ENV, "integration-secret");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let received = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 8192];
            let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                .await
                .unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            tokio::io::AsyncWriteExt::write_all(
                &mut stream,
                b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n",
            )
            .await
            .unwrap();
            request
        });

        let watcher = DeployWatcher {
            id: 1,
            repo_owner: "tana".to_string(),
            repo_name: "deka".to_string(),
            watcher_url: format!("http://{}/deploy", addr),
            active: 1,
            created_by: "test".to_string(),
            created_at: None,
            updated_at: None,
            last_branch: None,
            last_sha: None,
            last_delivery_status: None,
            last_delivery_code: None,
            last_delivery_error: None,
            last_delivery_at: None,
        };
        let outcome = deliver_deploy_watcher_with_delays(
            &watcher,
            "main",
            "abcdef1234567890abcdef1234567890abcdef12",
            "samira",
            &[],
        )
        .await;
        assert!(outcome.ok);

        let request = received.await.unwrap();
        assert!(request.starts_with("POST /deploy HTTP/1.1"));
        assert!(request.contains("x-linkhash-signature: sha256="));
        assert!(request.contains(r#""repo":"tana/deka""#));
        assert!(request.contains(r#""branch":"main""#));
        assert!(request.contains(r#""sha":"abcdef1234567890abcdef1234567890abcdef12""#));
        assert!(request.contains(r#""pusher_account":"samira""#));
    }
}
