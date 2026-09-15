//! Real semver range resolution for `deka.json` dependency constraints
//! (deka#1011).
//!
//! Previously `registry.rs::select_version` matched a requested version
//! string **literally** against the registry's published versions, so a
//! `deka.json` dependency written with the conventional `^`, `~` or `>=`
//! range syntax was silently reinterpreted as an exact pin (after a
//! separate stripping step removed the operator -- the now-deleted
//! `spec::strip_semver_range_prefix`, deka#971). That gave users either a
//! confusing "not found" or a pin to whatever version happened to share the
//! stripped literal -- never the range semantics every other package
//! manager gives `^`/`~`/`>=`.
//!
//! This module is the one place range strings are parsed and matched,
//! shared by version SELECTION (`registry::select_version`, picking the
//! highest published version satisfying a constraint) and version
//! SATISFACTION (`install::version_satisfies`, checking whether an
//! already-chosen version still satisfies every dependent's declared
//! constraint). Sharing one implementation is deliberate: two range
//! parsers that could drift is exactly the bug shape deka#971 already
//! produced once with the old prefix-stripping helper.
//!
//! Built on the `semver` crate (already a workspace dependency) rather than
//! hand-rolled comparison -- semver ordering, caret/tilde range math, and
//! prerelease matching are exactly the kind of logic that is easy to get
//! subtly wrong by hand, and the crate already implements the behavior we
//! want (see the prerelease policy on `constraint_matches` below).

use anyhow::Result;
use semver::{Version, VersionReq};

/// A parsed `deka.json` dependency constraint.
#[derive(Debug)]
enum Constraint {
    /// No constraint at all (`""`, `"latest"`, `"*"`): take the highest
    /// published stable version, falling back to the highest prerelease
    /// only if the package has never cut a stable release.
    Any,
    /// A bare version with no operator (e.g. `"1.2.3"`). This is an EXACT
    /// pin, not the cargo/npm convention of treating a bare version as an
    /// implicit caret range: `deka.json` and `deka.lock` already write bare
    /// versions to mean "this literal release" everywhere else in this
    /// codebase, and reinterpreting that under existing manifests would be
    /// its own silent behavior change. Only an explicit operator asks for
    /// range semantics.
    Exact(Version),
    /// An operator-prefixed range (`^`, `~`, `>=`, `>`, `<`, `<=`, `=`),
    /// delegated to `semver::VersionReq` for the comparison itself.
    Range(VersionReq),
}

/// Longest operators first so `">="` is never mistaken for `">"` and
/// `"<="` is never mistaken for `"<"`.
const OPERATORS: [&str; 7] = [">=", "<=", "^", "~", ">", "<", "="];

/// Parse a `deka.json` dependency version string into a constraint.
fn parse_constraint(raw: &str) -> Result<Constraint> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "latest" || trimmed == "*" {
        return Ok(Constraint::Any);
    }

    for op in OPERATORS {
        if let Some(rest) = trimmed.strip_prefix(op) {
            let version_text = rest.trim().trim_start_matches('v');
            if version_text.is_empty() {
                continue;
            }
            // semver::VersionReq wants the operator glued to the version,
            // no space in between.
            let req_text = format!("{op}{version_text}");
            let req = VersionReq::parse(&req_text)
                .map_err(|err| anyhow::anyhow!("`{raw}` is not a valid semver range: {err}"))?;
            return Ok(Constraint::Range(req));
        }
    }

    // No operator: an exact pin. Parsed through `semver::Version` (not raw
    // string equality) so `v1.2.3` and `1.2.3` are recognized as the same
    // pin regardless of the `v` prefix some registries and lockfiles carry.
    let version_text = trimmed.trim_start_matches('v');
    let version = Version::parse(version_text)
        .map_err(|err| anyhow::anyhow!("`{raw}` is not a valid semver version: {err}"))?;
    Ok(Constraint::Exact(version))
}

fn constraint_matches(constraint: &Constraint, version: &Version) -> bool {
    match constraint {
        Constraint::Any => version.pre.is_empty(),
        Constraint::Exact(exact) => version == exact,
        // `VersionReq::matches` already implements the semver.org
        // prerelease rule we want: a prerelease version is excluded from a
        // range unless the range's own comparator names a prerelease of
        // the *same* [major, minor, patch] triple (so `^1.2.3-alpha.1` can
        // match `1.2.3-alpha.2` or `1.2.3`, but `^1.2.0` never silently
        // pulls in `1.3.0-alpha.1`). We rely on the crate for this instead
        // of reimplementing the rule by hand.
        Constraint::Range(req) => req.matches(version),
    }
}

/// Parse a registry's published version list, skipping blank entries and
/// tolerating (and stripping) a leading `v`. Returns an error naming the
/// first entry that isn't valid semver -- a registry publishing garbage
/// versions is a registry bug, not something to silently paper over.
fn parse_catalog(versions: &[String]) -> Result<Vec<Version>> {
    let mut parsed = Vec::with_capacity(versions.len());
    for raw in versions {
        let trimmed = raw.trim().trim_start_matches('v');
        if trimmed.is_empty() {
            continue;
        }
        let version = Version::parse(trimmed)
            .map_err(|err| anyhow::anyhow!("registry version `{raw}` is not semver: {err}"))?;
        parsed.push(version);
    }
    Ok(parsed)
}

/// Resolve a `deka.json`-style constraint against a registry's published
/// version list, returning the **highest** version that satisfies it.
///
/// On no match, the error names both the constraint and the versions that
/// ARE available, so the fix is obvious from the message alone rather than
/// a bare "not found".
pub fn select_best(constraint_raw: &str, versions: &[String]) -> Result<String> {
    let constraint = parse_constraint(constraint_raw)?;
    let parsed = parse_catalog(versions)?;

    let mut candidates: Vec<&Version> = parsed
        .iter()
        .filter(|version| constraint_matches(&constraint, version))
        .collect();

    // `Any` prefers a stable release, but a package that has never cut one
    // (every published version is a pre-1.0 prerelease) must still resolve
    // to its highest prerelease rather than fail outright when versions
    // clearly exist.
    if matches!(constraint, Constraint::Any) && candidates.is_empty() {
        candidates = parsed.iter().collect();
    }

    candidates
        .into_iter()
        .max()
        .map(|version| version.to_string())
        .ok_or_else(|| describe_no_match(constraint_raw, &parsed))
}

/// Check whether an already-resolved version still satisfies a constraint.
/// Used to detect conflicts when two dependents declare different
/// constraints on the same package (deka.json + a transitive dependency).
/// An unparseable constraint or version is conservatively treated as
/// unsatisfied rather than silently accepted.
pub fn satisfies(constraint_raw: &str, version_raw: &str) -> bool {
    let Ok(constraint) = parse_constraint(constraint_raw) else {
        return false;
    };
    // `latest` / `*` / empty impose no real bound -- they are a preference
    // used when SELECTING a version (`select_best` prefers stable, falling
    // back to prerelease only when nothing stable was ever published), not
    // a constraint an already-resolved version must clear. A dependent
    // that wrote `latest` is satisfied by whatever version the package
    // actually resolved to, prerelease included. Matches every other
    // package manager's "no constraint" meaning of `latest`, and matches
    // this function's own pre-deka#1011 behavior, which returned `true`
    // unconditionally for these three spellings.
    if matches!(constraint, Constraint::Any) {
        return true;
    }
    let trimmed = version_raw.trim().trim_start_matches('v');
    let Ok(version) = Version::parse(trimmed) else {
        return false;
    };
    constraint_matches(&constraint, &version)
}

fn describe_no_match(constraint_raw: &str, catalog: &[Version]) -> anyhow::Error {
    let available = if catalog.is_empty() {
        "none".to_string()
    } else {
        let mut list: Vec<String> = catalog.iter().map(|version| version.to_string()).collect();
        list.sort();
        list.join(", ")
    };
    anyhow::anyhow!(
        "no published version satisfies `{constraint_raw}` -- available versions: {available}"
    )
}

#[cfg(test)]
mod tests {
    use super::{satisfies, select_best};

    fn versions(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn caret_picks_highest_compatible_minor_and_patch() {
        let catalog = versions(&["0.3.0", "0.3.1", "0.3.4", "0.4.0", "1.0.0"]);
        assert_eq!(select_best("^0.3.1", &catalog).unwrap(), "0.3.4");
    }

    #[test]
    fn caret_on_zero_major_is_locked_to_minor() {
        // Cargo/npm caret semantics: `^0.3.1` only floats the patch digit
        // (0.x is "still unstable" territory), it must never reach 0.4.0.
        let catalog = versions(&["0.3.1", "0.3.9", "0.4.0"]);
        assert_eq!(select_best("^0.3.1", &catalog).unwrap(), "0.3.9");
    }

    #[test]
    fn tilde_picks_highest_patch_only() {
        let catalog = versions(&["1.2.3", "1.2.9", "1.3.0"]);
        assert_eq!(select_best("~1.2.3", &catalog).unwrap(), "1.2.9");
    }

    #[test]
    fn gte_picks_highest_available() {
        let catalog = versions(&["1.2.3", "1.9.0", "2.0.0"]);
        assert_eq!(select_best(">=1.2.3", &catalog).unwrap(), "2.0.0");
    }

    #[test]
    fn gt_excludes_the_named_version() {
        let catalog = versions(&["1.2.3", "1.2.4"]);
        assert_eq!(select_best(">1.2.3", &catalog).unwrap(), "1.2.4");
        assert!(select_best(">1.2.4", &catalog).is_err());
    }

    #[test]
    fn lt_and_lte_pick_the_highest_below_the_bound() {
        let catalog = versions(&["1.0.0", "1.5.0", "2.0.0"]);
        assert_eq!(select_best("<2.0.0", &catalog).unwrap(), "1.5.0");
        assert_eq!(select_best("<=1.5.0", &catalog).unwrap(), "1.5.0");
        assert!(select_best("<1.0.0", &catalog).is_err());
    }

    #[test]
    fn explicit_eq_behaves_like_a_pin() {
        let catalog = versions(&["1.2.3", "1.2.4"]);
        assert_eq!(select_best("=1.2.3", &catalog).unwrap(), "1.2.3");
    }

    #[test]
    fn bare_version_is_an_exact_pin_not_an_implicit_caret() {
        // Unlike Cargo's own bare-version convention, a bare deka.json
        // version must NOT float to a higher patch -- only an explicit
        // operator asks for range semantics (see `Constraint::Exact`).
        let catalog = versions(&["1.2.3", "1.2.4"]);
        assert_eq!(select_best("1.2.3", &catalog).unwrap(), "1.2.3");
        assert!(select_best("1.2.5", &catalog).is_err());
    }

    #[test]
    fn latest_and_star_and_empty_all_mean_highest_stable() {
        let catalog = versions(&["0.1.0", "0.2.0"]);
        assert_eq!(select_best("latest", &catalog).unwrap(), "0.2.0");
        assert_eq!(select_best("*", &catalog).unwrap(), "0.2.0");
        assert_eq!(select_best("", &catalog).unwrap(), "0.2.0");
    }

    #[test]
    fn latest_skips_prereleases_when_a_stable_release_exists() {
        let catalog = versions(&["0.2.0", "0.3.0-beta.1"]);
        assert_eq!(select_best("latest", &catalog).unwrap(), "0.2.0");
    }

    #[test]
    fn latest_falls_back_to_prerelease_when_nothing_stable_exists() {
        let catalog = versions(&["0.1.0-alpha.1", "0.1.0-alpha.2"]);
        assert_eq!(select_best("latest", &catalog).unwrap(), "0.1.0-alpha.2");
    }

    #[test]
    fn range_excludes_prerelease_of_a_different_triple() {
        // semver.org rule: a prerelease only satisfies a range that names a
        // prerelease of the SAME [major, minor, patch]. `^1.2.0` must never
        // silently pull in a `1.3.0` prerelease even though it is
        // numerically "compatible" by the caret window.
        let catalog = versions(&["1.2.0", "1.3.0-alpha.1"]);
        assert_eq!(select_best("^1.2.0", &catalog).unwrap(), "1.2.0");
    }

    #[test]
    fn range_matches_prerelease_of_the_same_triple_when_named() {
        let catalog = versions(&["1.2.3-alpha.1", "1.2.3-alpha.2", "1.2.3-beta.1"]);
        assert_eq!(
            select_best("^1.2.3-alpha.1", &catalog).unwrap(),
            "1.2.3-beta.1"
        );
    }

    #[test]
    fn no_match_names_the_constraint_and_the_available_versions() {
        let catalog = versions(&["0.1.0", "0.2.0"]);
        let err = select_best("^1.0.0", &catalog).unwrap_err().to_string();
        assert!(
            err.contains("^1.0.0"),
            "error should name the constraint: {err}"
        );
        assert!(
            err.contains("0.1.0"),
            "error should list available versions: {err}"
        );
        assert!(
            err.contains("0.2.0"),
            "error should list available versions: {err}"
        );
    }

    #[test]
    fn no_match_reports_none_available_for_an_empty_catalog() {
        let err = select_best("^1.0.0", &[]).unwrap_err().to_string();
        assert!(
            err.contains("none"),
            "error should say none available: {err}"
        );
    }

    #[test]
    fn invalid_range_syntax_is_a_clear_error_not_a_panic() {
        let err = select_best("^not-a-version", &versions(&["1.0.0"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("^not-a-version"));
    }

    #[test]
    fn satisfies_checks_an_already_resolved_version_against_a_range() {
        assert!(satisfies("^1.2.0", "1.4.0"));
        assert!(!satisfies("^1.2.0", "2.0.0"));
        assert!(satisfies("~1.2.0", "1.2.9"));
        assert!(!satisfies("~1.2.0", "1.3.0"));
        assert!(satisfies("1.2.3", "1.2.3"));
        assert!(!satisfies("1.2.3", "1.2.4"));
        assert!(satisfies("latest", "9.9.9"));
    }

    /// deka#1011 regression: a `latest` requirement from one dependent
    /// (the common case is the implicit default when a package has no
    /// explicit version in `deka.json`) must still be satisfied by
    /// whatever concrete version the package actually resolved to, EVEN
    /// when that version is a prerelease (e.g. a fixture registry
    /// publishing `9.9.9-fixture`, or a real pre-1.0 stdlib release).
    /// `latest` is a preference used at SELECTION time, not a bound
    /// applied again at satisfaction-check time -- conflating the two
    /// made every prerelease-versioned package fail its own default
    /// `latest` requirement.
    #[test]
    fn latest_and_star_and_empty_satisfy_a_prerelease_resolved_version() {
        assert!(satisfies("latest", "9.9.9-fixture"));
        assert!(satisfies("*", "0.1.0-alpha.1"));
        assert!(satisfies("", "2.0.0-rc.1"));
    }
}
