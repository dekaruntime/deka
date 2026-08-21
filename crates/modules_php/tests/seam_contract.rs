use std::fs;
use std::path::PathBuf;

use modules_php::seam_contract::extract_contract_from_file;
use serde_json::json;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn extracts_storefront_handler_contract() {
    let path = fixtures_root().join("seams/storefront_handler.phpx");
    let contract = extract_contract_from_file(&path).expect("contract extraction failed");
    let actual = serde_json::to_value(&contract).unwrap();

    assert_eq!(
        actual,
        json!({
            "format": "seam.contract@1",
            "name": "handle",
            "version": 1,
            "boundaries": [
                {
                    "function": "handle",
                    "request": "Request",
                    "response": "Response"
                }
            ],
            "definitions": [
                {
                    "kind": "record",
                    "name": "Request",
                    "fields": {
                        "body": {
                            "kind": "option",
                            "item": { "kind": "primitive", "name": "String" }
                        },
                        "flags": {
                            "kind": "list",
                            "item": { "kind": "primitive", "name": "String" }
                        },
                        "method": { "kind": "primitive", "name": "String" },
                        "path": { "kind": "primitive", "name": "String" },
                        "shop_id": { "kind": "primitive", "name": "String" }
                    }
                },
                {
                    "kind": "record",
                    "name": "Response",
                    "fields": {
                        "body": { "kind": "primitive", "name": "String" },
                        "state": { "kind": "named", "name": "HandlerState" },
                        "status": { "kind": "primitive", "name": "Int" }
                    }
                },
                {
                    "kind": "enum",
                    "name": "HandlerState",
                    "variants": [
                        { "name": "Ready" },
                        {
                            "name": "Redirect",
                            "fields": {
                                "location": { "kind": "primitive", "name": "String" },
                                "permanent": { "kind": "primitive", "name": "Bool" }
                            }
                        }
                    ]
                }
            ]
        })
    );
}

#[test]
fn cli_fixture_stays_readable() {
    let path = fixtures_root().join("seams/storefront_handler.phpx");
    let source = fs::read_to_string(path).unwrap();
    assert!(source.contains("export function handle"));
}
