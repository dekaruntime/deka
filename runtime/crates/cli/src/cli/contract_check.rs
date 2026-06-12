//! `deka contract-check` — the seam gate (tana #454, component #e).
//!
//! Extracts a producer contract and a consumer contract, diffs the consumer
//! against the producer with `seam_diff::check_consumer`, and **exits non-zero**
//! on any shape drift — so it works as a pre-merge CI gate and a local pre-PR
//! check. Errors name both the producer and consumer `file:line`.
//!
//! Each side is a `kind:ref` spec:
//!   - `rust:storefront` / `rust:data_backend` — a Rust producer contract
//!   - `phpx:<path>` — extracted from a PHPX source file (struct/enum/boundary)
//!
//! Example (the storefront pilot):
//!   deka contract-check --producer rust:storefront \
//!       --consumer phpx:crates/modules_php/tests/fixtures/seams/storefront_consumer.phpx

use core::{CommandSpec, Context, ParamSpec, Registry};
use seam_ir::SeamContract;

const USAGE: &str = "usage: deka contract-check --producer <rust:NAME|phpx:PATH> --consumer <rust:NAME|phpx:PATH>\n       deka contract-check --manifest <seams.json>   (check every declared seam)";

const COMMAND: CommandSpec = CommandSpec {
    name: "contract-check",
    category: "project",
    summary: "check a consumer seam against its producer; non-zero exit on shape drift",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_param(ParamSpec {
        name: "--producer",
        description: "producer contract source: rust:NAME or phpx:PATH",
    });
    registry.add_param(ParamSpec {
        name: "--consumer",
        description: "consumer contract source: rust:NAME or phpx:PATH",
    });
    registry.add_param(ParamSpec {
        name: "--manifest",
        description: "check every seam declared in a seams.json manifest",
    });
}

pub fn cmd(context: &Context) {
    match run(context) {
        Outcome::Ok(report) => stdio::raw(&report),
        Outcome::Drift(report) => {
            stdio::raw(&report);
            std::process::exit(1);
        }
        Outcome::Usage(message) => {
            stdio::error("contract-check", &message);
            std::process::exit(2);
        }
    }
}

enum Outcome {
    Ok(String),
    Drift(String),
    Usage(String),
}

fn run(context: &Context) -> Outcome {
    if let Some(path) = context.args.params.get("--manifest") {
        return run_manifest(path);
    }
    let producer_spec = match context.args.params.get("--producer") {
        Some(spec) => spec,
        None => return Outcome::Usage(USAGE.to_string()),
    };
    let consumer_spec = match context.args.params.get("--consumer") {
        Some(spec) => spec,
        None => return Outcome::Usage(USAGE.to_string()),
    };

    let producer = match resolve(producer_spec) {
        Ok(contract) => contract,
        Err(err) => return Outcome::Usage(err),
    };
    let consumer = match resolve(consumer_spec) {
        Ok(contract) => contract,
        Err(err) => return Outcome::Usage(err),
    };

    let errors = seam_diff::check_consumer(&producer, &consumer);
    if errors.is_empty() {
        Outcome::Ok(format!(
            "✓ seam ok: consumer '{}' matches producer '{}' (no shape drift)",
            consumer.name, producer.name
        ))
    } else {
        let mut out = format!(
            "✗ seam drift: consumer '{}' does not match producer '{}' ({} issue(s))\n",
            consumer.name,
            producer.name,
            errors.len()
        );
        for error in &errors {
            out.push_str(&format!("\n  [{:?}] {}\n", error.kind, error.message));
            if let Some(loc) = &error.producer_location {
                out.push_str(&format!("    producer: {}:{}\n", loc.file, loc.line));
            }
            if let Some(loc) = &error.consumer_location {
                out.push_str(&format!("    consumer: {}:{}\n", loc.file, loc.line));
            }
        }
        Outcome::Drift(out)
    }
}

/// Check every seam declared in a JSON manifest:
/// `{ "seams": [ { "name", "producer", "consumer" }, ... ] }`.
/// One drifting seam fails the whole run (exit 1) — the multi-seam CI gate.
fn run_manifest(path: &str) -> Outcome {
    let raw = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => return Outcome::Usage(format!("cannot read manifest '{path}': {err}")),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => return Outcome::Usage(format!("invalid manifest JSON '{path}': {err}")),
    };
    let seams = match parsed.get("seams").and_then(serde_json::Value::as_array) {
        Some(array) => array,
        None => return Outcome::Usage(format!("manifest '{path}' has no 'seams' array")),
    };

    let mut report = String::new();
    let mut drift = false;
    for seam in seams {
        let name = seam.get("name").and_then(serde_json::Value::as_str).unwrap_or("<unnamed>");
        let producer_spec = seam.get("producer").and_then(serde_json::Value::as_str).unwrap_or("");
        let consumer_spec = seam.get("consumer").and_then(serde_json::Value::as_str).unwrap_or("");

        let producer = match resolve(producer_spec) {
            Ok(contract) => contract,
            Err(err) => {
                drift = true;
                report.push_str(&format!("✗ {name}: producer '{producer_spec}': {err}\n"));
                continue;
            }
        };
        let consumer = match resolve(consumer_spec) {
            Ok(contract) => contract,
            Err(err) => {
                drift = true;
                report.push_str(&format!("✗ {name}: consumer '{consumer_spec}': {err}\n"));
                continue;
            }
        };

        let errors = seam_diff::check_consumer(&producer, &consumer);
        if errors.is_empty() {
            report.push_str(&format!("✓ {name}\n"));
        } else {
            drift = true;
            report.push_str(&format!("✗ {name} ({} issue(s)):\n", errors.len()));
            for error in &errors {
                report.push_str(&format!("    [{:?}] {}\n", error.kind, error.message));
            }
        }
    }

    let summary = format!(
        "\nseam check: {} seam(s) — {}",
        seams.len(),
        if drift { "DRIFT (build should fail)" } else { "all ok" }
    );
    let full = format!("{report}{summary}");
    if drift { Outcome::Drift(full) } else { Outcome::Ok(full) }
}

/// Resolve a `kind:ref` spec into a contract. A bare path is treated as `phpx:`.
fn resolve(spec: &str) -> Result<SeamContract, String> {
    let (kind, rest) = spec.split_once(':').unwrap_or(("phpx", spec));
    match kind {
        "rust" => match rest {
            "storefront" | "storefront-envelope" => {
                Ok(runtime_core::storefront_envelope::storefront_contract())
            }
            "data_backend" | "data-backend" => {
                Ok(runtime_core::data_envelope::data_backend_contract())
            }
            other => Err(format!(
                "unknown rust contract target '{other}'; expected 'storefront' or 'data_backend'"
            )),
        },
        "phpx" => modules_php::seam_contract::extract_contract_from_file(rest),
        other => Err(format!(
            "unknown contract source kind '{other}'; use 'rust:' or 'phpx:'"
        )),
    }
}
