//! Audit an explicit corpus/tour snapshot without downloading during tests.
use std::{
    fs,
    path::{Path, PathBuf},
};
fn collect(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if root.is_file() {
        files.push(root.to_owned());
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect(&path, files)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "ds" | "dsx"))
        {
            files.push(path);
        }
    }
    Ok(())
}
fn comments(source: &str) -> Vec<String> {
    let mut lexer = deka_syntax::Lexer::new(source);
    let mut out = Vec::new();
    loop {
        let token = lexer.next_token();
        if token.kind == deka_syntax::lexer::TokenKind::Comment {
            out.push(token.text.to_owned());
        }
        if token.kind == deka_syntax::lexer::TokenKind::Eof {
            break;
        }
    }
    out
}
fn audit() -> Result<(), String> {
    let roots: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if roots.is_empty() {
        return Err(
            "usage: cargo run -p deka-fmt --example audit -- <corpus-or-tour-directory>...".into(),
        );
    }
    let mut total = 0;
    for root in roots {
        let mut files = Vec::new();
        collect(&root, &mut files)?;
        files.sort();
        if files.is_empty() {
            return Err(format!("{}: no DekaScript sources found", root.display()));
        }
        let mut parsed = 0;
        let mut unchanged_invalid = 0;
        let mut failures = Vec::new();
        for file in &files {
            let source =
                fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            let once = deka_fmt::format_ds(&source)?;
            let arena = bumpalo::Bump::new();
            let input = deka_syntax::parse::parse(&source, &arena);
            if !input.errors.is_empty() || input.program.is_none() {
                if once != source {
                    failures.push(format!("{}: incomplete source changed", file.display()));
                }
                unchanged_invalid += 1;
                continue;
            }
            parsed += 1;
            let output_arena = bumpalo::Bump::new();
            let output = deka_syntax::parse::parse(&once, &output_arena);
            if !output.errors.is_empty() || output.program.is_none() {
                failures.push(format!(
                    "{}: output does not parse: {:?}",
                    file.display(),
                    output.errors
                ));
            }
            if deka_fmt::format_ds(&once)? != once {
                failures.push(format!("{}: second pass differs", file.display()));
            }
            if comments(&source) != comments(&once) {
                failures.push(format!("{}: comments changed", file.display()));
            }
        }
        println!(
            "{}: {} source(s), {} parsed roundtrips, {} unparseable preserved, {} failures",
            root.display(),
            files.len(),
            parsed,
            unchanged_invalid,
            failures.len()
        );
        if !failures.is_empty() {
            return Err(failures.join("\n"));
        }
        total += files.len();
    }
    println!("audited {total} source(s)");
    Ok(())
}
fn main() {
    if let Err(error) = audit() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
