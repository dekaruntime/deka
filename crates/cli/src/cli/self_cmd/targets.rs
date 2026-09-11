//! Content targets for `deka self fetch` / `deka self test` (RFD 59, deka#836).
//!
//! A target is a pinned checkout of a content repository (conformance corpus,
//! language tour). Every property lives here in the target definition — adding
//! a new content repo is a new [`ContentTarget`] entry, never a new code path
//! (deka#836 sequencing rule 2).
//!
//! ## Version resolution: the lockstep rule (RFD 59)
//!
//! `self fetch` resolves "the version matching the running CLI" from an
//! embedded pin: two lines, `<git-ref>` and `<sha256-of-the-archive-tarball>`,
//! included at build time from `scripts/<name>-version`. The pin is the
//! resolution — it ships inside the binary, so it always names the content
//! this CLI was verified against, and it moves in the same PR that changes
//! the code (RFD 59 workflow step 4). Until the RFD 59 lockstep tags exist
//! (deka#835), the pin records the latest published content ref — a
//! `corpus-v*` tag for testsuite, a full commit SHA for tour — validated by
//! [`validate_ref`]. Once lockstep lands, the refs become plain version tags
//! equal to `CARGO_PKG_VERSION` (deka 0.46.0 pairs with testsuite 0.46.0);
//! the pin file then holds that tag and this code path is unchanged. The
//! SHA-256 line survives lockstep: whatever the ref becomes, a moved tag or
//! an altered archive stays a hard failure (deka#836 hard constraint).

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

/// `deka self fetch testsuite` / `deka self test suite`.
pub const TESTSUITE: ContentTarget = ContentTarget {
    name: "testsuite",
    repo: "dekaruntime/testsuite",
    pin: include_str!("../../../../../scripts/testsuite-corpus-version"),
    marker_file: "corpus/expected-failures.txt",
    runner: "corpus/run.mjs",
};

/// `deka self fetch tour` / `deka self test tour`.
pub const TOUR: ContentTarget = ContentTarget {
    name: "tour",
    repo: "dekaruntime/tour",
    pin: include_str!("../../../../../scripts/tour-version"),
    marker_file: "tests/tour/manifest.json",
    runner: "tests/tour/run.mjs",
};

const ALL: &[&ContentTarget] = &[&TESTSUITE, &TOUR];

/// Look up a target by command-line name. Accepts the suite alias
/// (`self test suite` in RFD 59) alongside the fetch name (`testsuite`).
pub fn by_name(name: &str) -> Option<&'static ContentTarget> {
    ALL.iter()
        .copied()
        .find(|target| target.name == name || (name == "suite" && target.name == "testsuite"))
}

/// Every target name accepted by `self fetch` (the alias is test-only).
pub fn fetch_names() -> &'static [&'static str] {
    &["testsuite", "tour"]
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

/// Enforce the ref shape the lockstep rule expects (see module docs).
fn validate_ref(target: &ContentTarget, reference: &str) -> Result<(), String> {
    let valid = if target.name == "testsuite" {
        // Pre-lockstep the corpus publishes `corpus-v*` tags; under lockstep
        // (deka#835) this becomes a plain `vX.Y.Z` version tag and the shape
        // check moves with the pin — the resolution code is unchanged. Both
        // shapes stay valid during the transition (deka#835/#844).
        let rest = reference
            .strip_prefix("corpus-v")
            .or_else(|| reference.strip_prefix('v'))
            .unwrap_or("");
        let mut parts = rest.split('.');
        matches!(parts.next(), Some(major) if !major.is_empty() && major.chars().all(|c| c.is_ascii_digit()))
            && matches!(parts.next(), Some(minor) if !minor.is_empty() && minor.chars().all(|c| c.is_ascii_digit()))
            && matches!(parts.next(), Some(patch) if !patch.is_empty() && patch.chars().all(|c| c.is_ascii_digit()))
            && parts.next().is_none()
    } else {
        // Tour has no release tags yet (deka#832 pinned by commit SHA); a
        // full SHA is immutable by construction, unlike a tag.
        reference.len() == 40 && reference.chars().all(|c| c.is_ascii_hexdigit())
    };
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
    fn embedded_pins_parse() {
        for target in ALL {
            let pin = Pin::parse(target).expect("embedded pin parses");
            assert!(
                pin.reference.starts_with("corpus-v")
                    || pin.reference.starts_with('v')
                    || pin.reference.len() == 40
            );
        }
        assert_eq!(
            Pin::parse(&TESTSUITE).expect("testsuite pin").reference,
            "corpus-v0.1.1"
        );
    }

    #[test]
    fn rejects_bad_refs_and_digests() {
        let bad_ref = ContentTarget {
            pin: "corpus-v1\n",
            ..TESTSUITE
        };
        assert!(Pin::parse(&bad_ref).is_err());
        let bad_tag = ContentTarget {
            pin: "not-a-tag\n0000000000000000000000000000000000000000000000000000000000000000\n",
            ..TESTSUITE
        };
        assert!(Pin::parse(&bad_tag).is_err());
        let bad_sha = ContentTarget {
            pin: "34722c8a75a15a627781b0011c285fd4f26474d3\nnot-hex\n",
            ..TOUR
        };
        assert!(Pin::parse(&bad_sha).is_err());
        let short_sha = ContentTarget {
            pin: "34722c8\n0000000000000000000000000000000000000000000000000000000000000000\n",
            ..TOUR
        };
        assert!(Pin::parse(&short_sha).is_err());
    }

    #[test]
    fn lockstep_v_tag_is_valid_for_testsuite() {
        let lockstep = ContentTarget {
            pin: "v0.47.0\n0000000000000000000000000000000000000000000000000000000000000000\n",
            ..TESTSUITE
        };
        assert_eq!(
            Pin::parse(&lockstep).expect("lockstep v-tag parses").reference,
            "v0.47.0"
        );
    }

    #[test]
    fn archive_url_uses_repo_and_ref() {
        assert_eq!(
            archive_url(&TESTSUITE, "corpus-v0.1.1"),
            "https://github.com/dekaruntime/testsuite/archive/corpus-v0.1.1.tar.gz"
        );
        assert_eq!(
            archive_url(&TOUR, "34722c8a75a15a627781b0011c285fd4f26474d3"),
            "https://github.com/dekaruntime/tour/archive/34722c8a75a15a627781b0011c285fd4f26474d3.tar.gz"
        );
    }

    #[test]
    fn suite_alias_resolves_to_testsuite() {
        assert!(by_name("suite").is_some());
        assert!(by_name("testsuite").is_some());
        assert!(by_name("tour").is_some());
        assert!(by_name("php").is_none());
    }
}
