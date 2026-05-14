use regex::{Captures, Regex};
use std::sync::OnceLock;

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
                    let marker = marker_for(token_type, captures.get(0).unwrap().as_str());
                    redactions.push(Redaction {
                        token_type,
                        marker: marker.clone(),
                    });
                    marker
                }
                Replacement::AssignmentValue => {
                    let key = captures.get(1).unwrap().as_str();
                    let sep = captures.get(2).unwrap().as_str();
                    let value = captures.get(3).unwrap().as_str();
                    let marker = marker_for(token_type, value);
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

fn marker_for(token_type: &'static str, secret: &str) -> String {
    let prefix = secret.chars().take(4).collect::<String>();
    format!("[REDACTED:{token_type}:{prefix}...]")
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

    fn assert_redacts(input: String, token_type: &str, prefix: &str) {
        let redacted = redact_secret_strings(&input);
        assert!(!redacted.text.contains(&input));
        assert!(
            redacted
                .text
                .contains(&format!("[REDACTED:{token_type}:{prefix}...]")),
            "unexpected redacted text: {}",
            redacted.text
        );
    }

    #[test]
    fn redacts_anthropic_api03_key() {
        assert_redacts(
            format!("sk-ant-api03-{}", "A".repeat(93)),
            "anthropic-api-key",
            "sk-a",
        );
    }

    #[test]
    fn redacts_other_anthropic_key_shape() {
        assert_redacts(
            format!("sk-ant-{}", "b".repeat(50)),
            "anthropic-api-key",
            "sk-a",
        );
    }

    #[test]
    fn redacts_openai_project_key() {
        assert_redacts(
            format!("sk-proj-{}", "C".repeat(120)),
            "openai-project-key",
            "sk-p",
        );
    }

    #[test]
    fn redacts_openai_api_key() {
        assert_redacts(
            format!("sk-{}", "D".repeat(40)),
            "openai-api-key",
            "sk-D",
        );
    }

    #[test]
    fn redacts_stripe_secret_and_restricted_keys() {
        for (key, token_type, prefix) in [
            (format!("sk_live_{}", "E".repeat(24)), "stripe-secret-key", "sk_l"),
            (format!("sk_test_{}", "F".repeat(24)), "stripe-secret-key", "sk_t"),
            (format!("rk_live_{}", "G".repeat(24)), "stripe-restricted-key", "rk_l"),
            (format!("rk_test_{}", "H".repeat(24)), "stripe-restricted-key", "rk_t"),
        ] {
            assert_redacts(key, token_type, prefix);
        }
    }

    #[test]
    fn redacts_tana_git_tokens() {
        assert_redacts(
            format!("tg_usr_{}", "I".repeat(40)),
            "tana-git-token",
            "tg_u",
        );
        assert_redacts(
            format!("tg_agt_{}", "J".repeat(40)),
            "tana-git-token",
            "tg_a",
        );
    }

    #[test]
    fn redacts_jwt_tokens() {
        assert_redacts(
            "eyJabcdefgh.eyJijklmnop.qrstuvwxyz".to_string(),
            "jwt",
            "eyJa",
        );
    }

    #[test]
    fn redacts_bcrypt_hashes() {
        assert_redacts(
            format!("$2b$12${}", "K".repeat(53)),
            "bcrypt-hash",
            "$2b$",
        );
    }

    #[test]
    fn redacts_private_keys() {
        assert_redacts(
            "-----BEGIN PRIVATE KEY-----\nabc123\n-----END PRIVATE KEY-----".to_string(),
            "private-key",
            "----",
        );
    }

    #[test]
    fn redacts_github_tokens() {
        assert_redacts(
            format!("ghp_{}", "L".repeat(36)),
            "github-token",
            "ghp_",
        );
        assert_redacts(
            format!("github_pat_{}", "M".repeat(40)),
            "github-token",
            "gith",
        );
    }

    #[test]
    fn redacts_secret_assignments_without_dropping_key_name() {
        let redacted = redact_secret_strings("password=correcthorsebatterystaple");
        assert_eq!(
            redacted.text,
            "password=[REDACTED:secret-assignment:corr...]"
        );

        let redacted = redact_secret_strings("AGENT_DISPATCHER_HMAC_KEY=abcdef1234567890");
        assert_eq!(
            redacted.text,
            "AGENT_DISPATCHER_HMAC_KEY=[REDACTED:env-secret-assignment:abcd...]"
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
