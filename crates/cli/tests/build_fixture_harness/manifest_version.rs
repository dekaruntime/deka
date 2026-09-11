//! deka#849: producer.deka version normalization for the blessed build
//! fixtures. Split out of `build_fixture_harness.rs` to stay under the
//! file-size gate (scripts/check-file-size.sh, deka#391).
//!
//! The blessed `build-manifest.json` mirror stores `producer.deka`
//! normalized to [`WORKSPACE_VERSION_PLACEHOLDER`], and
//! `build-manifest.sha256` anchors the normalized bytes — otherwise every
//! workspace version bump changes produced bytes and breaks the fixtures.
//! The field itself is still verified on every run: the published
//! manifest's `producer.deka` must equal the workspace version
//! ([`check_producer_version`]).

use super::{sha256_hex, unified_diff};

/// Stand-in for the workspace version in the blessed mirror (deka#849): the
/// mirror must not encode whichever version was current when it was blessed,
/// or every release bump silently breaks every PR until someone re-blesses.
pub(crate) const WORKSPACE_VERSION_PLACEHOLDER: &str = "<workspace-version>";

/// The version the built CLI stamps into `producer.deka`
/// (`crates/cli/src/cli/build_publish.rs` uses `env!("CARGO_PKG_VERSION")`,
/// and the crate is `version.workspace = true`, so the harness — compiled in
/// the same workspace — sees the same value).
pub(crate) fn workspace_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Replace `producer.deka`'s value with [`WORKSPACE_VERSION_PLACEHOLDER`].
/// The manifest is serialized compactly (`serde_json::to_string`), so the
/// version appears exactly once, as `"deka":"<version>"`, inside the
/// `producer` object; fail loudly if that stops holding rather than
/// normalizing the wrong bytes.
pub(crate) fn normalize_manifest_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let manifest: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|err| format!("manifest unparseable: {err}"))?;
    let version = manifest["producer"]["deka"]
        .as_str()
        .ok_or("manifest has no producer.deka string")?;
    let text =
        String::from_utf8(bytes.to_vec()).map_err(|_| "manifest is not utf-8".to_string())?;
    let needle = format!("\"deka\":\"{version}\"");
    if text.matches(&needle).count() != 1 {
        return Err(format!(
            "producer.deka value {version:?} appears other than exactly once as {needle}; refusing to normalize"
        ));
    }
    Ok(text
        .replacen(
            &needle,
            &format!("\"deka\":\"{WORKSPACE_VERSION_PLACEHOLDER}\""),
            1,
        )
        .into_bytes())
}

/// Byte-compare two build-manifest bodies with `producer.deka` normalized on
/// both sides (deka#849). Any other difference is a real contract break.
pub(crate) fn manifest_bytes_match(expected: &[u8], actual: &[u8]) -> Result<(), String> {
    let expected = normalize_manifest_bytes(expected).map_err(|err| format!("expected: {err}"))?;
    let actual = normalize_manifest_bytes(actual).map_err(|err| format!("actual: {err}"))?;
    if expected == actual {
        return Ok(());
    }
    match (std::str::from_utf8(&expected), std::str::from_utf8(&actual)) {
        (Ok(expected), Ok(actual)) => Err(format!(
            "manifest differs beyond producer.deka:\n{}",
            unified_diff(expected, actual)
        )),
        _ => Err(format!(
            "manifest differs beyond producer.deka: expected {} bytes, actual {} bytes",
            expected.len(),
            actual.len()
        )),
    }
}

/// The anchor the blessed `build-manifest.sha256` mirror holds: the hash of
/// the NORMALIZED manifest bytes (deka#849), so the sidecar is
/// version-agnostic like the manifest mirror.
pub(crate) fn normalized_manifest_anchor(manifest_bytes: &[u8]) -> Result<String, String> {
    let normalized = normalize_manifest_bytes(manifest_bytes)?;
    Ok(format!(
        "{}  build-manifest.json\n",
        sha256_hex(&normalized)
    ))
}

/// The published manifest must report THIS workspace as its producer
/// (deka#849). The blessed mirror cannot check the field (it is normalized
/// there), so a manifest built by a different deka version — or one that
/// lost the field — must fail here, separately from the byte comparison.
pub(crate) fn check_producer_version(manifest: &serde_json::Value) -> Result<(), String> {
    let deka = manifest["producer"]["deka"]
        .as_str()
        .ok_or("manifest has no producer.deka string")?;
    if deka != workspace_version() {
        return Err(format!(
            "producer.deka is {deka:?}, expected the workspace version {:?} — the manifest was produced by a different deka",
            workspace_version()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal but structurally complete v2 manifest body with the given
    /// producer.deka version, in the compact serialization the CLI writes.
    fn manifest_with_version(version: &str) -> Vec<u8> {
        format!(
            r#"{{"format":"deka.artifact@2","origin":"authored","producer":{{"deka":"{version}","dsc":"dsc [version 0.9.0]","plan_version":2}},"compat":{{"runtime_abi":1,"module_format":"esm2022","targets":["native","worker"],"host_imports":[]}},"client":{{"root":"client","index":"client/index.html","trailing_slash":false}},"server":{{"root":"server","entries":[]}},"routes":[],"slots":[],"payloads":[],"payload_root":"sha256:abc"}}"#
        )
        .into_bytes()
    }

    fn parse(bytes: &[u8]) -> serde_json::Value {
        serde_json::from_slice(bytes).expect("manifest parses")
    }

    /// deka#849: manifests differing ONLY in producer.deka must compare
    /// equal after normalization — this is exactly what a version bump
    /// produces, and it must not re-break the blessed fixtures.
    #[test]
    fn manifest_differing_only_in_producer_version_passes() {
        let current = manifest_with_version(workspace_version());
        let bumped = manifest_with_version("999.0.0");
        manifest_bytes_match(&current, &bumped)
            .expect("a version-only difference must pass the normalized comparison");
    }

    /// Normalization must not swallow real differences: a manifest that
    /// differs anywhere else still fails.
    #[test]
    fn manifest_differing_elsewhere_still_fails() {
        let current = manifest_with_version(workspace_version());
        let other = String::from_utf8(manifest_with_version("999.0.0"))
            .expect("manifest is utf-8")
            .replace(
                "\"payload_root\":\"sha256:abc\"",
                "\"payload_root\":\"sha256:def\"",
            );
        let err = manifest_bytes_match(&current, other.as_bytes())
            .expect_err("a non-version difference must still fail");
        assert!(
            err.contains("payload_root"),
            "the error must point at the real difference: {err}"
        );
    }

    /// The blessed mirror normalizes the field, so the separate check is
    /// what catches a manifest produced by a different deka version.
    #[test]
    fn non_workspace_producer_version_fails_the_separate_check() {
        let manifest = parse(&manifest_with_version("0.1.2"));
        let err =
            check_producer_version(&manifest).expect_err("a non-workspace producer.deka must fail");
        assert!(
            err.contains("0.1.2") && err.contains(workspace_version()),
            "the error must name both versions: {err}"
        );
    }

    #[test]
    fn workspace_producer_version_passes_the_separate_check() {
        let manifest = parse(&manifest_with_version(workspace_version()));
        check_producer_version(&manifest).expect("the workspace version must pass");
    }

    /// The blessed sha256 sidecar anchors the NORMALIZED manifest bytes, so
    /// bumping the version leaves the expected anchor unchanged.
    #[test]
    fn sidecar_anchor_is_bump_invariant() {
        let current = manifest_with_version(workspace_version());
        let bumped = manifest_with_version("999.0.0");
        let anchor_current = normalized_manifest_anchor(&current).expect("anchor current");
        let anchor_bumped = normalized_manifest_anchor(&bumped).expect("anchor bumped");
        assert_eq!(
            anchor_current, anchor_bumped,
            "the normalized anchor must not change across a version bump"
        );
    }
}
