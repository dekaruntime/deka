//! Pinned tour content for legacy `deka self fetch/test tour`.
//! The standalone testsuite is retired; runtime tests live with their crates.

/// A content repository `deka self fetch` can check out and `deka self test`
/// can run.
pub struct ContentTarget {
    /// Name on the command line: `deka self fetch <name>`.
    pub name: &'static str,
    /// GitHub repository the archive is downloaded from.
    pub repo: &'static str,
    /// Two-line pin (`<git-ref>` + archive SHA-256) embedded at build time.
    /// Same file the old CI fetch scripts consumed, so the pin has one owner.
    pub pin: &'static str,
    /// File that must exist after extraction, relative to the checkout root.
    /// Doubles as the `self test` presence check.
    pub marker_file: &'static str,
    /// Test runner invoked by `deka self test <name>`, relative to the
    /// checkout root. Owned by the content repo; `self test` shells to it
    /// and never reimplements it (RFD 59 open question 3).
    pub runner: &'static str,
}

/// `deka self fetch tour` / `deka self test tour`.
pub const TOUR: ContentTarget = ContentTarget {
    name: "tour",
    repo: "dekaruntime/tour",
    pin: include_str!("../../../scripts/tour-version"),
    marker_file: "tests/tour/manifest.json",
    runner: "tests/tour/run.mjs",
};

const ALL: &[&ContentTarget] = &[&TOUR];

/// Look up a supported content target.
pub fn by_name(name: &str) -> Option<&'static ContentTarget> {
    ALL.iter().copied().find(|target| target.name == name)
}

/// Every target name accepted by `self fetch` (the alias is test-only).
pub fn fetch_names() -> &'static [&'static str] {
    &["tour"]
}

/// A parsed, validated pin: the git ref to download and the checksum the
/// downloaded archive must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub reference: String,
    pub sha256: String,
}

impl Pin {
    /// Parse and validate the two-line embedded pin for `target`.
    pub fn parse(target: &ContentTarget) -> Result<Pin, String> {
        let mut lines = target.pin.lines();
        let reference = lines
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .ok_or_else(|| format!("{} pin is missing its reference line", target.name))?
            .to_string();
        let sha256 = lines
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .ok_or_else(|| format!("{} pin is missing its SHA-256 line", target.name))?
            .to_string();
        validate_ref(target, &reference)?;
        if !is_sha256_hex(&sha256) {
            return Err(format!(
                "invalid {} archive SHA-256 in scripts/{}-version: {}",
                target.name, target.name, sha256
            ));
        }
        Ok(Pin { reference, sha256 })
    }
}

/// Require immutable content references.
fn validate_ref(target: &ContentTarget, reference: &str) -> Result<(), String> {
    let valid = reference.len() == 40 && reference.chars().all(|c| c.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid {} reference in scripts/{}-version: {}",
            target.name, target.name, reference
        ))
    }
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Download URL for the pinned ref. GitHub serves the same archive endpoint
/// for tags and commit SHAs.
pub fn archive_url(target: &ContentTarget, reference: &str) -> String {
    format!(
        "https://github.com/{}/archive/{}.tar.gz",
        target.repo, reference
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_tour_parses() {
        assert_eq!(Pin::parse(&TOUR).unwrap().reference.len(), 40);
    }
    #[test]
    fn rejects_mutable_refs_and_bad_digests() {
        for reference in ["main", "34722c8", "v0.60.0"] {
            assert!(validate_ref(&TOUR, reference).is_err());
        }
        let bad = ContentTarget {
            pin: "34722c8a75a15a627781b0011c285fd4f26474d3\nnot-hex\n",
            ..TOUR
        };
        assert!(Pin::parse(&bad).is_err());
    }
    #[test]
    fn retired_suite_is_not_a_fetch_or_test_target() {
        assert!(by_name("suite").is_none());
        assert!(by_name("testsuite").is_none());
        assert!(by_name("tour").is_some());
        assert_eq!(fetch_names(), &["tour"]);
    }
    #[test]
    fn archive_uses_pinned_repository() {
        assert_eq!(
            archive_url(&TOUR, "abc"),
            "https://github.com/dekaruntime/tour/archive/abc.tar.gz"
        );
    }
}
