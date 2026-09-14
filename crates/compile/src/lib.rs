pub mod binary;
pub mod command;
pub mod config;
mod desktop;
pub mod vfs;

pub use command::register;
pub use config::{DekaConfig, WindowConfig};

use core::Context;
use std::{fs, path::PathBuf, process::Command};

pub fn run(context: &Context) {
    if let Err(error) = compile(context) {
        stdio::error("compile", &error);
        std::process::exit(1);
    }
}

fn compile(context: &Context) -> Result<(), String> {
    if context
        .args
        .flags
        .get("--desktop")
        .copied()
        .unwrap_or(false)
    {
        return desktop::compile_desktop(context);
    }
    compile_executable(context)
}

fn compile_executable(context: &Context) -> Result<(), String> {
    if context.args.positionals.len() != 1 {
        return Err(
            "usage: deka compile <entry.ds> [--outfile <executable>]  (or deka compile --desktop)"
                .into(),
        );
    }
    let entry = PathBuf::from(&context.args.positionals[0]);
    if entry.extension().and_then(|s| s.to_str()) != Some("ds") {
        return Err("deka compile requires a .ds entry".into());
    }
    let entry = fs::canonicalize(&entry)
        .map_err(|e| format!("cannot read entry {}: {e}", entry.display()))?;
    let output = PathBuf::from(
        context
            .args
            .params
            .get("--outfile")
            .map(String::as_str)
            .unwrap_or("deka-app"),
    );
    if output.exists() {
        return Err(format!("output already exists: {}", output.display()));
    }
    let dsc = compiler::dsc::find_cli_dsc()?.ok_or_else(|| {
        "dsc is required for deka compile; set DEKA_DSC or install dsc beside deka / on PATH"
            .to_string()
    })?;
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let bundle = temp.path().join("main.js");
    let result = Command::new(dsc)
        .arg("transpile")
        .arg(&entry)
        .arg("--bundle")
        .arg("--out")
        .arg(&bundle)
        .output()
        .map_err(|e| format!("failed to execute dsc: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "dsc failed ({}):\n{}{}",
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    let js = fs::read(bundle).map_err(|e| format!("failed to read dsc bundle: {e}"))?;
    let mut vfs = vfs::VFS::new("main.js".into(), vfs::RuntimeMode::Server);
    vfs.add_file("main.js".into(), js, "js".into(), false);
    // Assemble beside the destination, then publish only a complete executable.
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    binary::BinaryEmbedder::new(std::env::current_exe().map_err(|e| e.to_string())?).embed(
        &vfs.to_bytes()?,
        "main.js",
        staged.path(),
    )?;
    staged
        .persist_noclobber(&output)
        .map_err(|e| format!("cannot publish {}: {e}", output.display()))?;
    stdio::log("compile", &format!("Created {}", output.display()));
    Ok(())
}
