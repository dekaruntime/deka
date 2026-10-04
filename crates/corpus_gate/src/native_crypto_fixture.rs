//! Narrow metadata adapter for unchanged, natively executable crypto corpus cases.
use crate::{Case, Stage, Status};
const FIXTURES: [(&str, &str, &str); 4] = [
    (
        "crypto-bcrypt-verify-ok",
        include_str!("crypto_fixtures/bcrypt_verify_ok.ds"),
        "true\n",
    ),
    (
        "crypto-bcrypt-verify-wrong",
        include_str!("crypto_fixtures/bcrypt_verify_wrong.ds"),
        "false\n",
    ),
    (
        "crypto-random-bytes-rejects-zero",
        include_str!("crypto_fixtures/random_bytes_rejects_zero.ds"),
        "err\n",
    ),
    (
        "crypto-random-hex-length",
        include_str!("crypto_fixtures/random_hex_length.ds"),
        "16\n",
    ),
];
pub(crate) fn accepts(case: &Case) -> bool {
    case.status == Status::Pass
        && case.stage == Stage::Run
        && case.packages == ["crypto"]
        && case.files.is_empty()
        && case.deka_json.is_none()
        && case.expected_diagnostic_contains.is_none()
        && FIXTURES.iter().any(|(slug, source, stdout)| {
            case.slug == *slug
                && case.source == *source
                && case.expected_stdout.as_deref() == Some(*stdout)
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Case {
        let (slug, source, stdout) = FIXTURES[0];
        Case {
            slug: slug.into(),
            status: Status::Pass,
            stage: Stage::Run,
            source: source.into(),
            entry_path: "main.ds".into(),
            files: vec![],
            expected_stdout: Some(stdout.into()),
            expected_diagnostic_contains: None,
            deka_json: None,
            packages: vec!["crypto".into()],
        }
    }
    #[test]
    fn pinned_sources_match_but_changed_contracts_fail_closed() {
        for (slug, source, stdout) in FIXTURES {
            let mut c = fixture();
            c.slug = slug.into();
            c.source = source.into();
            c.expected_stdout = Some(stdout.into());
            assert!(accepts(&c));
        }
        let mutations: [fn(&mut Case); 9] = [
            |c| c.source.push(' '),
            |c| c.slug.push('x'),
            |c| c.stage = Stage::Parse,
            |c| c.status = Status::Fail,
            |c| c.packages.push("other".into()),
            |c| c.files.push(("other.ds".into(), "".into())),
            |c| c.deka_json = Some(serde_json::json!({})),
            |c| c.expected_stdout = Some("wrong".into()),
            |c| c.expected_diagnostic_contains = Some("error".into()),
        ];
        for mutate in mutations {
            let mut c = fixture();
            mutate(&mut c);
            assert!(!accepts(&c));
        }
    }
}
