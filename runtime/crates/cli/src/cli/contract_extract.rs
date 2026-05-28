use core::{CommandSpec, Context, Registry};

const COMMAND: CommandSpec = CommandSpec {
    name: "contract-extract",
    category: "project",
    summary: "extract a seam contract from a PHPX handler",
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
        Err(err) => stdio::error("contract-extract", &err),
    }
}

fn run(context: &Context) -> Result<String, String> {
    let input = context
        .args
        .positionals
        .first()
        .ok_or_else(|| "usage: deka contract-extract <file.phpx>".to_string())?;
    let contract = modules_php::seam_contract::extract_contract_from_file(input)?;
    serde_json::to_string_pretty(&contract)
        .map_err(|err| format!("failed to serialize seam contract: {}", err))
}
