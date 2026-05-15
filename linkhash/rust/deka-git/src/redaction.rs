use regex::{Captures, Regex};
use std::sync::OnceLock;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redaction {
    pub token_type: &'static str,
    pub marker: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedText {
    pub text: String,
    pub redactions: Vec<Redaction>,
}

#[derive(Debug, Clone)]
pub struct RedactionAlertContext<'a> {
    pub repo_owner: &'a str,
    pub repo_name: &'a str,
    pub location: &'a str,
    pub subject: Option<String>,
    pub caller: &'a str,
}

#[derive(Clone, Copy)]
enum Replacement {
    WholeSecret,
    AssignmentValue,
}

struct SecretPattern {
    token_type: &'static str,
    regex: Regex,
    replacement: Replacement,
}

pub fn redact_secret_strings(input: &str) -> RedactedText {
    let mut text = input.to_string();
    let mut redactions = Vec::new();

    for pattern in secret_patterns() {
        let token_type = pattern.token_type;
        let replacement = pattern.replacement;
        text = pattern
            .regex
            .replace_all(&text, |captures: &Captures<'_>| match replacement {
                Replacement::WholeSecret => {
                    let marker = marker_for(token_type);
                    redactions.push(Redaction {
                        token_type,
                        marker: marker.clone(),
                    });
                    marker
                }
                Replacement::AssignmentValue => {
                    let key = captures.get(1).unwrap().as_str();
                    let sep = captures.get(2).unwrap().as_str();
                    let marker = marker_for(token_type);
                    redactions.push(Redaction {
                        token_type,
                        marker: marker.clone(),
                    });
                    format!("{key}{sep}{marker}")
                }
            })
            .into_owned();
    }

    RedactedText { text, redactions }
}

pub fn redact_optional_secret_strings(input: Option<String>) -> (Option<String>, Vec<Redaction>) {
    match input {
        Some(value) => {
            let redacted = redact_secret_strings(&value);
            (Some(redacted.text), redacted.redactions)
        }
        None => (None, Vec::new()),
    }
}

pub fn log_redactions(scope: &str, redactions: &[Redaction]) {
    if redactions.is_empty() {
        return;
    }

    let markers = redactions
        .iter()
        .map(|redaction| redaction.marker.as_str())
        .collect::<Vec<_>>()
        .join(",");
    tracing::warn!(
        target: "security.redaction",
        scope = scope,
        count = redactions.len(),
        markers = markers,
        "redacted secret-shaped string before persistence"
    );
}

pub async fn alert_redactions(context: RedactionAlertContext<'_>, redactions: &[Redaction]) {
    if redactions.is_empty() {
        return;
    }

    let secret = match std::env::var("TANA_INTERNAL_API_SECRET") {
        Ok(value) if !value.is_empty() => value,
        _ => {
            tracing::warn!(
                target: "security.redaction",
                repo = format!("{}/{}", context.repo_owner, context.repo_name),
                location = context.location,
                "TANA_INTERNAL_API_SECRET unset; skipping Telegram redaction alert"
            );
            return;
        }
    };

    let url = std::env::var("TANA_REDACTION_ALERT_URL")
        .unwrap_or_else(|_| "http://localhost:9420/internal/notify".to_string());
    let timestamp = chrono::Utc::now().to_rfc3339();
    let pattern_types = redactions
        .iter()
        .map(|redaction| redaction.token_type)
        .collect::<Vec<_>>();
    let pattern_text = pattern_types.join(", ");
    let subject = context.subject.as_deref().unwrap_or("unknown");
    let message = format!(
        "[security] Secret redaction triggered\nrepo: {}/{}\ncontext: {} {}\npatterns: {}\ncaller: {}\ntimestamp: {}",
        context.repo_owner,
        context.repo_name,
        context.location,
        subject,
        pattern_text,
        context.caller,
        timestamp
    );
    let body = serde_json::json!({
        "kind": "secret_redaction",
        "message": message,
        "repo": format!("{}/{}", context.repo_owner, context.repo_name),
        "context": {
            "location": context.location,
            "subject": subject,
        },
        "pattern_types": pattern_types,
        "caller": context.caller,
        "timestamp": timestamp,
    });

    let notify_result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        post_internal_notify(&url, &secret, &body.to_string()),
    )
    .await;
    if let Err(err) =
        notify_result.unwrap_or_else(|_| Err(anyhow::anyhow!("alert endpoint timed out")))
    {
        tracing::warn!(
            target: "security.redaction",
            repo = format!("{}/{}", context.repo_owner, context.repo_name),
            location = context.location,
            error = %err,
            "failed to send Telegram redaction alert"
        );
    }
}

async fn post_internal_notify(
    url: &str,
    secret: &str,
    body: &str,
) -> anyhow::Result<()> {
    let endpoint = parse_http_url(url).ok_or_else(|| anyhow::anyhow!("unsupported alert URL"))?;
    let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port)).await?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nX-Internal-Token: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.path,
        endpoint.host,
        secret,
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).await?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let status_ok = String::from_utf8_lossy(&response).starts_with("HTTP/1.1 2")
        || String::from_utf8_lossy(&response).starts_with("HTTP/1.0 2");
    if !status_ok {
        return Err(anyhow::anyhow!("alert endpoint returned non-2xx status"));
    }
    Ok(())
}

struct HttpEndpoint {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(url: &str) -> Option<HttpEndpoint> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().ok()?),
        None => (authority.to_string(), 80),
    };
    if host.is_empty() {
        return None;
    }
    Some(HttpEndpoint { host, port, path })
}

fn marker_for(token_type: &'static str) -> String {
    format!("[REDACTED:{token_type}]")
}

fn secret_patterns() -> &'static [SecretPattern] {
    static PATTERNS: OnceLock<Vec<SecretPattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            whole("private-key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----"),
            whole("anthropic-api-key", r"sk-ant-api03-[A-Za-z0-9_-]{93,}"),
            whole("anthropic-api-key", r"sk-ant-[A-Za-z0-9_-]{40,}"),
            whole("openai-project-key", r"sk-proj-[A-Za-z0-9_-]{120,}"),
            whole("openai-api-key", r"sk-[A-Za-z0-9]{40,}"),
            whole("stripe-secret-key", r"sk_live_[A-Za-z0-9]{24,}"),
            whole("stripe-secret-key", r"sk_test_[A-Za-z0-9]{24,}"),
            whole("stripe-restricted-key", r"rk_live_[A-Za-z0-9]{24,}"),
            whole("stripe-restricted-key", r"rk_test_[A-Za-z0-9]{24,}"),
            whole("tana-git-token", r"tg_(usr|agt)_[A-Za-z0-9]{40,}"),
            whole("jwt", r"eyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}"),
            whole("bcrypt-hash", r"\$2[abxy]\$[0-9]{2}\$[./A-Za-z0-9]{53}"),
            whole("github-token", r"(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{16,}"),
            whole("github-token", r"github_pat_[A-Za-z0-9_]{16,}"),
            assignment("env-secret-assignment", r#"\b([A-Z][A-Z0-9_]{2,}(?:KEY|SECRET|TOKEN|PASSWORD|PASSWD|HMAC|JWT|API_KEY|ACCESS_KEY|PRIVATE_KEY))\s*([:=])\s*([^\s`'"&]{8,})"#),
            assignment("secret-assignment", r#"(?i)\b(password|passwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key)\s*([:=])\s*([^\s`'"&]{8,})"#),
        ]
    })
}

fn whole(token_type: &'static str, regex: &str) -> SecretPattern {
    SecretPattern {
        token_type,
        regex: Regex::new(regex).expect("valid redaction regex"),
        replacement: Replacement::WholeSecret,
    }
}

fn assignment(token_type: &'static str, regex: &str) -> SecretPattern {
    SecretPattern {
        token_type,
        regex: Regex::new(regex).expect("valid redaction regex"),
        replacement: Replacement::AssignmentValue,
    }
}

#[cfg(test)]
mod tests {
    use super::redact_secret_strings;

    fn assert_redacts(input: String, token_type: &str) {
        let redacted = redact_secret_strings(&input);
        assert!(!redacted.text.contains(&input));
        assert!(
            redacted
                .text
                .contains(&format!("[REDACTED:{token_type}]")),
            "unexpected redacted text: {}",
            redacted.text
        );
        assert!(!redacted.text.contains(":sk_"));
        assert!(!redacted.text.contains(":sk-"));
        assert!(!redacted.text.contains(":tg_"));
    }

    #[test]
    fn redacts_anthropic_api03_key() {
        assert_redacts(
            format!("sk-ant-api03-{}", "A".repeat(93)),
            "anthropic-api-key",
        );
    }

    #[test]
    fn redacts_other_anthropic_key_shape() {
        assert_redacts(
            format!("sk-ant-{}", "b".repeat(50)),
            "anthropic-api-key",
        );
    }

    #[test]
    fn redacts_openai_project_key() {
        assert_redacts(
            format!("sk-proj-{}", "C".repeat(120)),
            "openai-project-key",
        );
    }

    #[test]
    fn redacts_openai_api_key() {
        assert_redacts(
            format!("sk-{}", "D".repeat(40)),
            "openai-api-key",
        );
    }

    #[test]
    fn redacts_stripe_secret_and_restricted_keys() {
        for (key, token_type) in [
            (format!("sk_live_{}", "E".repeat(24)), "stripe-secret-key"),
            (format!("sk_test_{}", "F".repeat(24)), "stripe-secret-key"),
            (format!("rk_live_{}", "G".repeat(24)), "stripe-restricted-key"),
            (format!("rk_test_{}", "H".repeat(24)), "stripe-restricted-key"),
        ] {
            assert_redacts(key, token_type);
        }
    }

    #[test]
    fn redacts_tana_git_tokens() {
        assert_redacts(
            format!("tg_usr_{}", "I".repeat(40)),
            "tana-git-token",
        );
        assert_redacts(
            format!("tg_agt_{}", "J".repeat(40)),
            "tana-git-token",
        );
    }

    #[test]
    fn redacts_jwt_tokens() {
        assert_redacts(
            "eyJabcdefgh.eyJijklmnop.qrstuvwxyz".to_string(),
            "jwt",
        );
    }

    #[test]
    fn redacts_bcrypt_hashes() {
        assert_redacts(
            format!("$2b$12${}", "K".repeat(53)),
            "bcrypt-hash",
        );
    }

    #[test]
    fn redacts_private_keys() {
        assert_redacts(
            "-----BEGIN PRIVATE KEY-----\nabc123\n-----END PRIVATE KEY-----".to_string(),
            "private-key",
        );
    }

    #[test]
    fn redacts_github_tokens() {
        assert_redacts(
            format!("ghp_{}", "L".repeat(36)),
            "github-token",
        );
        assert_redacts(
            format!("github_pat_{}", "M".repeat(40)),
            "github-token",
        );
    }

    #[test]
    fn redacts_secret_assignments_without_dropping_key_name() {
        let redacted = redact_secret_strings("password=correcthorsebatterystaple");
        assert_eq!(
            redacted.text,
            "password=[REDACTED:secret-assignment]"
        );

        let redacted = redact_secret_strings("AGENT_DISPATCHER_HMAC_KEY=abcdef1234567890");
        assert_eq!(
            redacted.text,
            "AGENT_DISPATCHER_HMAC_KEY=[REDACTED:env-secret-assignment]"
        );
    }

    #[test]
    fn does_not_redact_generic_hex_strings() {
        let input = "0123456789abcdef0123456789abcdef";
        let redacted = redact_secret_strings(input);
        assert_eq!(redacted.text, input);
        assert!(redacted.redactions.is_empty());
    }
}
