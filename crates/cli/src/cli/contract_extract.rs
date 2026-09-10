//! `deka contract-extract` — emit a `seam.contract@1` document for one side of a
//! boundary (tana #454, component #e).
//!
//! Recovered from the pre-migration checkout (deka #789). The `phpx:` extractor
//! that used to sit alongside `ts:` and `--rust` lived in `modules_php`, on top
//! of the `php_rs` typechecker; both crates were removed by the DekaScript
//! cutover, so that source kind is gone rather than stubbed. See the PR for
//! deka #789.

use core::{CommandSpec, Context, Registry};

const COMMAND: CommandSpec = CommandSpec {
    name: "contract-extract",
    category: "project",
    summary: "extract a seam contract from TypeScript or Rust targets",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
}

pub fn cmd(context: &Context) {
    match run(context) {
        Ok(json) => stdio::raw(&json),
        Err(err) => {
            stdio::error("contract-extract", &err);
            std::process::exit(1);
        }
    }
}

fn run(context: &Context) -> Result<String, String> {
    if let Some(target) = context.args.params.get("--rust") {
        let contract = super::contract_check::resolve_rust(target)?;
        return serde_json::to_string_pretty(&contract)
            .map_err(|err| format!("failed to serialize seam contract: {}", err));
    }

    let input = context.args.positionals.first().ok_or_else(|| {
        "usage: deka contract-extract <file.ts|ts:PATH> | --rust <storefront|data_backend|platform_env_policy>"
            .to_string()
    })?;
    let contract = if let Some(path) = input.strip_prefix("ts:") {
        seam_ts::extract_contract_from_file(path)?
    } else if input.ends_with(".ts") || input.ends_with(".tsx") {
        seam_ts::extract_contract_from_file(input)?
    } else {
        return Err(format!(
            "unsupported contract source '{input}'; use a .ts/.tsx path, 'ts:PATH', or --rust <target>"
        ));
    };
    serde_json::to_string_pretty(&contract)
        .map_err(|err| format!("failed to serialize seam contract: {}", err))
}
