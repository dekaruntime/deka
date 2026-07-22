use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoGrant {
    pub repo: String,
    pub access: RepoAccess,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RepoAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretGrant {
    pub repo: String,
    pub pattern: String,
}

pub fn repo_matches(pattern: &str, repo: &str) -> bool {
    pattern == "*" || pattern == repo
}

pub fn secret_matches(pattern: &str, secret_name: &str) -> bool {
    pattern == "*"
        || pattern == secret_name
        || pattern
            .strip_suffix('*')
            .map(|prefix| secret_name.starts_with(prefix))
            .unwrap_or(false)
}

pub fn can_read_repo(legacy_repos: &[String], grants: &[RepoGrant], repo: &str) -> bool {
    legacy_repos.iter().any(|r| repo_matches(r, repo))
        || grants.iter().any(|grant| repo_matches(&grant.repo, repo))
}

pub fn can_write_repo(legacy_repos: &[String], grants: &[RepoGrant], repo: &str) -> bool {
    legacy_repos.iter().any(|r| repo_matches(r, repo))
        || grants
            .iter()
            .any(|grant| grant.access == RepoAccess::Write && repo_matches(&grant.repo, repo))
}

pub fn can_read_secret(grants: &[SecretGrant], repo: &str, secret_name: &str) -> bool {
    grants.iter().any(|grant| {
        repo_matches(&grant.repo, repo) && secret_matches(&grant.pattern, secret_name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_grant_allows_repo_read() {
        let grants = vec![RepoGrant {
            repo: "tana/deka".to_string(),
            access: RepoAccess::Read,
        }];

        assert!(can_read_repo(&[], &grants, "tana/deka"));
        assert!(!can_read_repo(&[], &grants, "tana/tana-website"));
    }

    #[test]
    fn read_grant_does_not_allow_repo_write() {
        let grants = vec![RepoGrant {
            repo: "tana/deka".to_string(),
            access: RepoAccess::Read,
        }];

        assert!(!can_write_repo(&[], &grants, "tana/deka"));
    }

    #[test]
    fn write_grant_allows_repo_write() {
        let grants = vec![RepoGrant {
            repo: "tana/deka".to_string(),
            access: RepoAccess::Write,
        }];

        assert!(can_write_repo(&[], &grants, "tana/deka"));
    }

    #[test]
    fn legacy_repo_wildcard_still_allows_access() {
        assert!(can_read_repo(&["*".to_string()], &[], "tana/deka"));
        assert!(can_write_repo(&["*".to_string()], &[], "tana/deka"));
    }

    #[test]
    fn secret_acl_supports_exact_and_prefix_patterns() {
        let grants = vec![
            SecretGrant {
                repo: "tana/tana-website".to_string(),
                pattern: "RESEND_API_KEY".to_string(),
            },
            SecretGrant {
                repo: "tana/tana-website".to_string(),
                pattern: "STRIPE_TEST_*".to_string(),
            },
        ];

        assert!(can_read_secret(
            &grants,
            "tana/tana-website",
            "RESEND_API_KEY"
        ));
        assert!(can_read_secret(
            &grants,
            "tana/tana-website",
            "STRIPE_TEST_SECRET_KEY"
        ));
        assert!(!can_read_secret(
            &grants,
            "tana/tana-website",
            "TANA_ADMIN_TOTP_SECRET"
        ));
    }
}
