/// Strip a leading semver-range operator (`^`, `~`, `>=`) from a
/// `deka.json` dependency version string.
///
/// The installer has no range resolver -- every version string it accepts
/// is matched literally against the registry's published versions. A
/// `deka.json` written with the conventional `^0.4.1` / `~0.4.1` / `>=0.4.1`
/// range syntax must still resolve to the pinned base version rather than
/// failing to find a release literally named `^0.4.1` (deka#971). Both
/// `deka.json`-dependency readers share this so the stripping rule can't
/// drift between them (`collect_deka_json_deps` for `deka update`,
/// `collect_deka_json_deps_in` for `deka install`).
pub fn strip_semver_range_prefix(version: &str) -> &str {
    version
        .trim_start_matches('^')
        .trim_start_matches('~')
        .trim_start_matches(">=")
}

pub fn parse_package_spec(spec: &str) -> (String, Option<String>) {
    if spec.starts_with('@') {
        if let Some(pos) = spec[1..].find('@') {
            let split = pos + 1;
            let name = spec[..split].to_string();
            let version = spec[split + 1..].to_string();
            return (name, Some(version));
        }
        return (spec.to_string(), None);
    }

    if let Some(pos) = spec.rfind('@') {
        if pos > 0 {
            let name = spec[..pos].to_string();
            let version = spec[pos + 1..].to_string();
            return (name, Some(version));
        }
    }

    (spec.to_string(), None)
}
