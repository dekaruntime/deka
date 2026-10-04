//! Native `deka fmt`: the migrated DS printer, without executing source.
use deka_cli_core::registry::{CommandError, Context, ExitStatus, HandlerResult};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(crate) fn cmd(context: &Context) -> HandlerResult {
    if !context.args.params.is_empty()
        || context
            .args
            .flags
            .keys()
            .any(|flag| !matches!(flag.as_str(), "--check" | "--stdin"))
    {
        return Err(CommandError::usage("fmt accepts only --check and --stdin"));
    }
    let check = context.args.flags.get("--check").copied().unwrap_or(false);
    let stdin = context.args.flags.get("--stdin").copied().unwrap_or(false);
    if stdin {
        if !context.args.positionals.is_empty() {
            return Err(CommandError::usage("--stdin does not accept a file path"));
        }
        let mut source = String::new();
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| CommandError::runtime(format!("failed to read stdin: {error}")))?;
        let formatted = deka_fmt::format_ds(&source).map_err(CommandError::Runtime)?;
        if check {
            return Ok(ExitStatus::from_code(u8::from(formatted != source)));
        }
        context.out().print(format_args!("{formatted}"));
        return Ok(ExitStatus::SUCCESS);
    }
    let [input] = context.args.positionals.as_slice() else {
        return Err(CommandError::usage(
            "usage: deka fmt <file-or-directory> [--check] | deka fmt --stdin [--check]",
        ));
    };
    let path = Path::new(input);
    let directory = path.is_dir();
    let entries = if directory {
        collect(path).map_err(CommandError::Runtime)?
    } else if path.is_file() {
        vec![path.to_owned()]
    } else {
        return Err(CommandError::runtime(format!(
            "input path does not exist: {}",
            path.display()
        )));
    };
    let mut changed = false;
    let mut reformatted = 0;
    for entry in &entries {
        let source = fs::read_to_string(entry).map_err(|error| {
            CommandError::runtime(format!("failed to read {}: {error}", entry.display()))
        })?;
        let formatted = deka_fmt::format_ds(&source).map_err(CommandError::Runtime)?;
        if formatted != source {
            changed = true;
            if check {
                context
                    .err()
                    .print(format_args!("{} would be reformatted\n", entry.display()));
                continue;
            }
            fs::write(entry, formatted).map_err(|error| {
                CommandError::runtime(format!("failed to write {}: {error}", entry.display()))
            })?;
            reformatted += 1;
            context
                .out()
                .print(format_args!("formatted {}\n", entry.display()));
        } else if !directory && !check {
            context
                .out()
                .print(format_args!("formatted {}\n", entry.display()));
        }
    }
    if directory {
        context.out().print(format_args!(
            "visited {} DekaScript file(s) under {} ({} reformatted)\n",
            entries.len(),
            path.display(),
            reformatted
        ));
    }
    Ok(ExitStatus::from_code(u8::from(check && changed)))
}

fn collect(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| {
                format!("failed to read entry in {}: {error}", directory.display())
            })?;
            if !crate::source_entry::discoverable(&entry)
                .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?
            {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "ds" | "dsx"))
            {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}
