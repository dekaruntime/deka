//! The producer≡consumer gate for the data-layer envelope (deka #88 / tana #558).
//!
//! The PHPX `neo4j`/`redis` stdlib modules consume the envelope shapes that
//! `zega_backend` produces. This test extracts each consumer's declared shape
//! from a PHPX seam fixture and checks it against the single producer contract
//! (`runtime_core::data_envelope::data_backend_contract`). If an agent (or a
//! rewrite) changes either side so the shapes diverge, `check_consumer` reports
//! it and this test fails — "wrong shape for the receiving end" becomes red CI,
//! not a runtime surprise.

use std::path::PathBuf;

use modules_php::seam_contract::extract_contract_from_file;
use runtime_core::data_envelope::data_backend_contract;
use seam_diff::check_consumer;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/seams/data")
        .join(name)
}

#[test]
fn storefront_data_consumers_match_producer_contract() {
    let producer = data_backend_contract();

    for file in [
        "cql_query.phpx",
        "cql_execute.phpx",
        "kv_get.phpx",
        "kv_set.phpx",
        "kv_del.phpx",
    ] {
        let consumer = extract_contract_from_file(fixture(file))
            .unwrap_or_else(|err| panic!("extract {file}: {err}"));
        let errors = check_consumer(&producer, &consumer);
        assert!(
            errors.is_empty(),
            "{file}: consumer drifts from data_backend contract: {errors:#?}"
        );
    }
}
