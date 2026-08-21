use core::Context;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use stdio;

const COLOR_GREEN: &str = "\x1b[32m";
const COLOR_RED: &str = "\x1b[31m";
const COLOR_YELLOW: &str = "\x1b[33m";
const COLOR_RESET: &str = "\x1b[0m";

pub fn cmd(context: &Context) {
    let suite = context.args.positionals.get(0).map(|s| s.as_str());
    let result = match suite {
        Some("php") => run_php_suite(context),
        Some(other) => Err(format!("unknown self test suite '{}'", other)),
        None => Err("missing suite name (php)".to_string()),
    };

    if let Err(message) = result {
        stdio::error("self test", &message);
        std::process::exit(1);
    }
}

fn run_php_suite(context: &Context) -> Result<(), String> {
    let cwd = context.env.cwd.clone();
    let suite_dir = cwd.join("tests").join("php");
    if !suite_dir.exists() {
        return Err(format!("php suite not found at {}", suite_dir.display()));
    }

    let mut pending = Vec::new();
    let mut files = Vec::new();
    collect_php_files(&suite_dir, &mut files, &mut pending)?;
    files.sort();
    pending.sort();

    if files.is_empty() {
        print_pending(&cwd, &pending);
        stdio::log("self test", "no php tests found");
        return Ok(());
    }

    let php_official = std::env::var("PHP_BIN").unwrap_or_else(|_| "php".to_string());
    let deka_bin = resolve_deka_bin()?;

    let mut failures = 0usize;
    for file in &files {
        let relative = file.strip_prefix(&cwd).unwrap_or(file);
        print!("{} ... ", relative.to_string_lossy());

        let directives = load_directives(file);
        let official = run_php_binary(&php_official, file)?;
        let native = run_deka_php(&deka_bin, file)?;

        let matches = official.stdout == native.stdout
            && official.stderr == native.stderr
            && official.code == native.code;

        if matches {
            println!("{COLOR_GREEN}ok{COLOR_RESET}");
            continue;
        }

        if let Some(shapes) = directives.shapes.as_ref() {
            let official_shape_ok = matches_shapes(shapes, &official);
            let native_shape_ok = matches_shapes(shapes, &native);
            let stderr_match = shapes.contains_key("stderr") || official.stderr == native.stderr;
            if official_shape_ok && native_shape_ok && official.code == native.code && stderr_match
            {
                println!("{COLOR_YELLOW}shape ok{COLOR_RESET}");
                continue;
            }
        }

        if directives.nondeterministic
            && official.code == native.code
            && (!native.stdout.is_empty() || !native.stderr.is_empty())
        {
            println!("{COLOR_YELLOW}soft ok{COLOR_RESET}");
            continue;
        }

        failures += 1;
        println!("{COLOR_RED}FAILED{COLOR_RESET}");
        if official.code != native.code {
            println!(
                "  exit codes differ: official={} php={}",
                official.code, native.code
            );
        }
        print_block("stdout", &official.stdout, &native.stdout);
        print_block("stderr", &official.stderr, &native.stderr);
    }

    print_pending(&cwd, &pending);
    let matched = files.len().saturating_sub(failures);
    let total = files.len() + pending.len();
    let percent = if total == 0 {
        0
    } else {
        ((matched as f64 / total as f64) * 100.0).round() as i32
    };
    let summary_color = if failures > 0 { COLOR_RED } else { COLOR_GREEN };
    println!(
        "\n{summary_color}Summary: {matched}/{total} passed. {percent}% overall. {pending_count} tests pending language implementation.{COLOR_RESET}",
        matched = matched,
        total = total,
        percent = percent,
        pending_count = pending.len()
    );
    println!();

    if failures > 0 {
        return Err("php self test failed".to_string());
    }
    Ok(())
}

fn resolve_deka_bin() -> Result<String, String> {
    let exe =
        std::env::current_exe().map_err(|err| format!("failed to resolve deka binary: {}", err))?;
    Ok(exe.to_string_lossy().to_string())
}

#[derive(Debug)]
struct RunResult {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run_php_binary(binary: &str, script: &Path) -> Result<RunResult, String> {
    let output = Command::new(binary)
        .arg(script)
        .current_dir(script.parent().unwrap_or_else(|| Path::new(".")))
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("failed to run {}: {}", binary, err))?;
    let code = output.status.code().unwrap_or(0);
    Ok(RunResult {
        stdout: sanitize_stream(&String::from_utf8_lossy(&output.stdout)),
        stderr: sanitize_stream(&String::from_utf8_lossy(&output.stderr)),
        code,
    })
}

fn run_deka_php(binary: &str, script: &Path) -> Result<RunResult, String> {
    let output = Command::new(binary)
        .arg("run")
        .arg(script)
        .current_dir(script.parent().unwrap_or_else(|| Path::new(".")))
        .env("LOG_LEVEL", "error")
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("failed to run deka: {}", err))?;
    let code = output.status.code().unwrap_or(0);
    Ok(RunResult {
        stdout: sanitize_stream(&String::from_utf8_lossy(&output.stdout)),
        stderr: sanitize_stream(&String::from_utf8_lossy(&output.stderr)),
        code,
    })
}

fn sanitize_stream(text: &str) -> String {
    let mut sanitized = text
        .lines()
        .filter(|line| !line.starts_with("[PthreadsExtension]"))
        .collect::<Vec<_>>()
        .join("\n");
    while sanitized.contains("\n\n\n") {
        sanitized = sanitized.replace("\n\n\n", "\n\n");
    }
    let trimmed_start = sanitized.trim_start_matches(|ch| ch == '\n' || ch == '\r');
    if text.ends_with('\n') {
        format!("{}\n", trimmed_start.trim_end())
    } else {
        trimmed_start.trim_end().to_string()
    }
}

#[derive(Default)]
struct Directives {
    nondeterministic: bool,
    shapes: Option<BTreeMap<String, String>>,
}

fn load_directives(path: &Path) -> Directives {
    let content = fs::read_to_string(path).unwrap_or_default();
    let mut shapes = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.contains("@nondeterministic") {
            // keep scanning for shapes
        }
        if let Some((key, value)) = parse_shape_line(line) {
            shapes.entry(key).or_insert(value);
        }
    }
    Directives {
        nondeterministic: content.contains("@nondeterministic"),
        shapes: if shapes.is_empty() {
            None
        } else {
            Some(shapes)
        },
    }
}

fn parse_shape_line(line: &str) -> Option<(String, String)> {
    let idx = line.find("@shape")?;
    let mut rest = line[idx + "@shape".len()..].trim();
    if rest.is_empty() {
        return None;
    }
    if let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim();
        rest = rest[eq + 1..].trim();
        let value = rest.split_whitespace().next().unwrap_or("").trim();
        if !key.is_empty() && !value.is_empty() {
            return Some((key.to_string(), value.to_string()));
        }
    }
    None
}

fn matches_shapes(shapes: &BTreeMap<String, String>, result: &RunResult) -> bool {
    for (target, shape) in shapes {
        let output = if target == "stderr" {
            &result.stderr
        } else {
            &result.stdout
        };
        if !matches_shape(shape, output) {
            return false;
        }
    }
    true
}

fn matches_shape(shape: &str, output: &str) -> bool {
    let trimmed = output.trim();
    match shape {
        "int" => trimmed.parse::<i64>().is_ok(),
        "float" => {
            let ok = trimmed.parse::<f64>().is_ok();
            ok && (trimmed.contains('.') || trimmed.contains('e') || trimmed.contains('E'))
        }
        "number" => trimmed.parse::<f64>().is_ok(),
        "string" => !trimmed.is_empty(),
        _ => {
            if let Some(inner) = shape
                .strip_prefix("array<")
                .and_then(|s| s.strip_suffix('>'))
            {
                if let Some(values) = parse_print_r_array(output) {
                    return values.iter().all(|value| matches_shape(inner, value));
                }
                return false;
            }
            if let Some(inner) = shape
                .strip_prefix("lines<")
                .and_then(|s| s.strip_suffix('>'))
            {
                let lines: Vec<&str> = output
                    .lines()
                    .map(|line| line.trim())
                    .filter(|line| !line.is_empty())
                    .collect();
                if lines.is_empty() {
                    return false;
                }
                return lines.iter().all(|line| matches_shape(inner, line));
            }
            false
        }
    }
}

fn parse_print_r_array(output: &str) -> Option<Vec<String>> {
    let start = output.find("Array")?;
    let slice = &output[start..];
    let open = slice.find('(')?;
    let close = slice.rfind(')')?;
    let body = &slice[open + 1..close];
    let mut values = Vec::new();
    for line in body.lines() {
        if let Some(pos) = line.find("=>") {
            let value = line[pos + 2..].trim();
            if !value.is_empty() {
                values.push(value.to_string());
            }
        }
    }
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

fn print_block(label: &str, official: &str, native: &str) {
    if official == native {
        return;
    }
    println!("  {label}:");
    println!("    official:");
    println!("{}", indent(official));
    println!("    deka:");
    println!("{}", indent(native));
}

fn indent(text: &str) -> String {
    if text.is_empty() {
        return "    (empty)".to_string();
    }
    text.lines()
        .map(|line| format!("    {}", line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn collect_php_files(
    dir: &Path,
    files: &mut Vec<PathBuf>,
    pending: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|err| format!("failed to read {}: {}", dir.display(), err))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('_') {
                collect_all_php_files(&path, pending)?;
                continue;
            }
            collect_php_files(&path, files, pending)?;
        } else if file_type.is_file() {
            if path.extension().and_then(|e| e.to_str()) == Some("php") {
                files.push(path);
            }
        }
    }
    Ok(())
}

fn collect_all_php_files(dir: &Path, pending: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|err| format!("failed to read {}: {}", dir.display(), err))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            collect_all_php_files(&path, pending)?;
        } else if file_type.is_file() {
            if path.extension().and_then(|e| e.to_str()) == Some("php") {
                pending.push(path);
            }
        }
    }
    Ok(())
}

fn print_pending(cwd: &Path, pending: &[PathBuf]) {
    for path in pending {
        let relative = path.strip_prefix(cwd).unwrap_or(path);
        println!(
            "{} ... {COLOR_YELLOW}pending{COLOR_RESET}",
            relative.to_string_lossy()
        );
    }
}
