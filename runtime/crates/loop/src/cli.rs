use chrono::Utc;
use core::{CommandSpec, Context, ParamSpec, Registry, SubcommandSpec};
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

const AGENT_TOOLS: [&str; 3] = ["claude", "codex", "gemini"];
const DEFAULT_AGENT_TOOL: &str = AGENT_TOOLS[0];
const DEFAULT_CATEGORY: &str = "project";

const COMMAND: CommandSpec = CommandSpec {
    name: "loop",
    category: "workflow",
    summary: "manage looping AI agent tasks",
    aliases: &[],
    subcommands: &[
        SubcommandSpec {
            name: "init",
            summary: "initialize loop workspace",
            aliases: &[],
            handler: cmd_init,
        },
        SubcommandSpec {
            name: "list",
            summary: "list tasks",
            aliases: &[],
            handler: cmd_list,
        },
        SubcommandSpec {
            name: "add",
            summary: "add a new task",
            aliases: &[],
            handler: cmd_add,
        },
        SubcommandSpec {
            name: "progress",
            summary: "view the progress file",
            aliases: &[],
            handler: cmd_progress,
        },
        SubcommandSpec {
            name: "run",
            summary: "run the task loop TUI",
            aliases: &[],
            handler: cmd_run,
        },
        SubcommandSpec {
            name: "view",
            summary: "view the loop TUI without triggering tasks",
            aliases: &[],
            handler: cmd_view,
        },
    ],
    handler: cmd_default,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_param(ParamSpec {
        name: "--category",
        description: "category when adding a task",
    });
    registry.add_param(ParamSpec {
        name: "--description",
        description: "description when adding a task",
    });
    registry.add_param(ParamSpec {
        name: "-c",
        description: "category when adding a task (alias for --category)",
    });
    registry.add_param(ParamSpec {
        name: "--steps",
        description: "steps for the task (semicolon-separated)",
    });
    registry.add_param(ParamSpec {
        name: "--status",
        description: "status for the new task (waiting/running/done/failed)",
    });
    registry.add_param(ParamSpec {
        name: "--date",
        description: "filter progress output by YYYY-MM-DD",
    });
    registry.add_param(ParamSpec {
        name: "--iterations",
        description: "limit iterations when running the loop TUI",
    });
}

fn cmd_default(_context: &Context) {
    stdio::log("loop", "use one of: init, list, add, progress, run");
}

fn cmd_init(context: &Context) {
    maybe_suggest_git(&context.env.cwd);
    let dir = loop_dir(&context.env.cwd);
    if let Err(err) = fs::create_dir_all(&dir) {
        stdio::error(
            "loop",
            &format!("failed to prepare .deka directory: {}", err),
        );
        return;
    }

    let path = dir.join("loop.json");
    if path.exists() {
        stdio::log(
            "loop",
            ".deka/loop.json already exists, leaving it untouched",
        );
    } else if let Err(err) = run_init_prompt(&path) {
        stdio::error("loop", &format!("failed to write loop.json: {}", err));
        return;
    } else {
        stdio::log("loop", "created .deka/loop.json");
    }

    let progress_path = dir.join("progress.md");
    if progress_path.exists() {
        stdio::log("loop", ".deka/progress.md already exists");
    } else if let Err(err) = fs::write(
        &progress_path,
        format!(
            "## {} loop initialized\n- waiting for tasks\n",
            current_date()
        ),
    ) {
        stdio::error("loop", &format!("failed to write progress.md: {}", err));
    } else {
        stdio::log("loop", "created .deka/progress.md");
    }
}

fn cmd_list(context: &Context) {
    maybe_suggest_git(&context.env.cwd);
    let path = match ensure_loop_file(&context.env.cwd) {
        Ok(value) => value,
        Err(err) => {
            stdio::error("loop", &err);
            return;
        }
    };

    let state = match load_loop_state(&path) {
        Ok(state) => state,
        Err(err) => {
            stdio::error("loop", &err);
            return;
        }
    };

    stdio::header("loop tasks");
    if state.tasks.is_empty() {
        stdio::log(
            "loop",
            "no tasks defined yet; run `deka loop add` or `deka loop run`",
        );
        return;
    }

    let counts = task_counts(&state.tasks);
    stdio::info("total", &state.tasks.len().to_string());
    stdio::info("waiting", &counts.waiting.to_string());
    stdio::info("running", &counts.running.to_string());
    stdio::info("done", &counts.done.to_string());
    stdio::info("failed", &counts.failed.to_string());

    for (idx, task) in state.tasks.iter().enumerate() {
        stdio::log(
            "loop",
            &format!(
                "#{} [{}] {} - {} ({} steps)",
                idx + 1,
                task.status.as_ref(),
                task.category,
                task.description,
                task.steps.len()
            ),
        );
    }
}

fn cmd_add(context: &Context) {
    maybe_suggest_git(&context.env.cwd);
    let params = &context.args.params;
    let category_value = params
        .get("--category")
        .or_else(|| params.get("-c"))
        .map(|value| value.trim().to_string());
    let category = category_value.unwrap_or_else(|| DEFAULT_CATEGORY.to_string());
    let description = params
        .get("--description")
        .map(|value| value.trim().to_string())
        .or_else(|| context.args.positionals.first().cloned())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            stdio::error("loop", "--description is required when adding tasks");
            String::new()
        });
    if description.is_empty() {
        return;
    }
    let steps = params
        .get("--steps")
        .map(|value| {
            value
                .split(';')
                .map(|step| step.trim().to_string())
                .filter(|step| !step.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let status = params
        .get("--status")
        .and_then(|value| TaskStatus::from_str(value).ok())
        .unwrap_or(TaskStatus::Waiting);

    let path = match ensure_loop_file(&context.env.cwd) {
        Ok(value) => value,
        Err(err) => {
            stdio::error("loop", &err);
            return;
        }
    };

    let mut state = match load_loop_state(&path) {
        Ok(state) => state,
        Err(err) => {
            stdio::error("loop", &err);
            return;
        }
    };

    let task = LoopTask {
        id: nanoid::nanoid!(10),
        category: category.clone(),
        description,
        steps,
        status,
    };
    state.tasks.push(task);

    if let Err(err) = save_loop_state(&path, &state) {
        stdio::error("loop", &format!("failed to persist task: {}", err));
        return;
    }

    stdio::log(
        "loop",
        &format!("new task added, using category '{}'", category),
    );
}

fn cmd_progress(context: &Context) {
    maybe_suggest_git(&context.env.cwd);
    let path = progress_path(&context.env.cwd);
    if !path.exists() {
        stdio::warn("loop", "progress file not found (create .deka/progress.md)");
        return;
    }

    let content = match fs::read_to_string(&path) {
        Ok(value) => value,
        Err(err) => {
            stdio::error("loop", &format!("failed to read progress file: {}", err));
            return;
        }
    };

    stdio::header("loop progress");
    if let Some(date) = context.args.params.get("--date") {
        print_progress_for_date(&content, date);
    } else {
        print_progress(&content);
    }
}

fn cmd_run(context: &Context) {
    launch_tui(context, "run");
}

fn cmd_view(context: &Context) {
    launch_tui(context, "view");
}

fn launch_tui(context: &Context, mode: &str) {
    maybe_suggest_git(&context.env.cwd);
    if let Err(err) = ensure_loop_file(&context.env.cwd) {
        stdio::error("loop", &err);
        return;
    }

    let iterations = collect_iterations(context);
    let ui_path = get_ui_path("loop-ui.tsx");
    if std::env::var("DEKA_DEBUG").is_ok() {
        stdio::log(
            "loop",
            &format!("loop-ui handler path: {}", ui_path.display()),
        );
    }
    if !ui_path.exists() {
        stdio::error(
            "loop",
            &format!("TUI handler missing: {}", ui_path.display()),
        );
        std::process::exit(1);
    }

    let args = build_tui_args(iterations);

    unsafe {
        let args_json = serde_json::to_string(&args).unwrap_or_else(|_| "[]".to_string());
        std::env::set_var("DEKA_ARGS", args_json);
        std::env::set_var("HANDLER_PATH", ui_path.to_string_lossy().to_string());
        std::env::set_var("DEKA_JSX_IMPORT_SOURCE", "react");
    }

    if std::env::var("DEKA_DEBUG").is_ok() {
        stdio::log("loop", &format!("launching loop {}", mode));
    }
    runtime::run(context);
}

fn collect_iterations(context: &Context) -> Option<String> {
    if let Some(value) = context.args.params.get("--iterations") {
        if value.parse::<u64>().is_err() {
            stdio::warn("loop", "--iterations expects a number");
            None
        } else {
            Some(value.clone())
        }
    } else {
        None
    }
}

fn build_tui_args(iterations: Option<String>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(value) = iterations {
        args.push("--iterations".to_string());
        args.push(value);
    }
    args
}

fn maybe_suggest_git(cwd: &Path) {
    if !cwd.join(".git").exists() {
        stdio::warn_simple("no git repository found; consider running 'git init'");
    }
}

fn loop_dir(cwd: &Path) -> PathBuf {
    cwd.join(".deka")
}

fn progress_path(cwd: &Path) -> PathBuf {
    loop_dir(cwd).join("progress.md")
}

fn ensure_loop_file(cwd: &Path) -> Result<PathBuf, String> {
    let dir = loop_dir(cwd);
    fs::create_dir_all(&dir).map_err(|err| format!("unable to create .deka directory: {}", err))?;
    let path = dir.join("loop.json");
    if !path.exists() {
        save_loop_state(&path, &LoopState::default())
            .map_err(|err| format!("unable to bootstrap loop.json: {}", err))?;
    }
    Ok(path)
}

fn load_loop_state(path: &Path) -> Result<LoopState, String> {
    let content = fs::read_to_string(path).map_err(|err| err.to_string())?;
    serde_json::from_str(&content).map_err(|err| format!("invalid loop.json: {}", err))
}

fn save_loop_state(path: &Path, state: &LoopState) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|err| err.to_string())?;
    fs::write(path, text).map_err(|err| err.to_string())
}

fn run_init_prompt(path: &Path) -> Result<(), String> {
    let coding_tool = choose_agent_tool()?;
    let workflow_notes = prompt("Workflow notes (optional)", None)?;
    let notes = workflow_notes.trim();
    let meta = LoopMeta {
        coding_tool: Some(coding_tool),
        notes: (!notes.is_empty()).then(|| notes.to_string()),
    };
    save_loop_state(
        path,
        &LoopState {
            meta: Some(meta),
            tasks: Vec::new(),
        },
    )
}

fn prompt(question: &str, default: Option<&str>) -> Result<String, String> {
    if let Some(value) = default {
        print!("{} [{}]: ", question, value);
    } else {
        print!("{}: ", question);
    }
    io::stdout().flush().map_err(|err| err.to_string())?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|err| err.to_string())?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        if let Some(value) = default {
            return Ok(value.to_string());
        }
    }
    Ok(trimmed.to_string())
}

fn choose_agent_tool() -> Result<String, String> {
    let options = AGENT_TOOLS.join(", ");
    loop {
        print!(
            "Preferred CLI agent tool ({}) [{}]: ",
            options, DEFAULT_AGENT_TOOL
        );
        io::stdout().flush().map_err(|err| err.to_string())?;
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|err| err.to_string())?;
        let trimmed = input.trim();
        let normalized = if trimmed.is_empty() {
            DEFAULT_AGENT_TOOL.to_string()
        } else {
            trimmed.to_lowercase()
        };

        if AGENT_TOOLS.iter().any(|tool| tool == &normalized) {
            return Ok(normalized);
        }

        stdio::warn_simple("please choose one of the available agent tools");
        println!("available tools: {}", options);
    }
}

fn current_date() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

fn print_progress(content: &str) {
    for line in content.lines() {
        println!("{}", line);
    }
}

fn print_progress_for_date(content: &str, date: &str) {
    let mut printing = false;
    let mut found = false;
    for line in content.lines() {
        if let Some(stripped) = line.strip_prefix("## ") {
            printing = stripped.trim() == date;
            found |= printing;
            if printing {
                println!("{}", line);
            }
            continue;
        }
        if printing {
            println!("{}", line);
        }
    }
    if !found {
        stdio::warn_simple(&format!("no progress entries for {}", date));
    }
}

fn get_ui_path(filename: &str) -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("src")
        .join("ui")
        .join(filename)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LoopState {
    meta: Option<LoopMeta>,
    #[serde(default)]
    tasks: Vec<LoopTask>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LoopMeta {
    coding_tool: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LoopTask {
    id: String,
    category: String,
    description: String,
    #[serde(default)]
    steps: Vec<String>,
    #[serde(default)]
    status: TaskStatus,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum TaskStatus {
    #[default]
    Waiting,
    Running,
    Done,
    Failed,
}

impl TaskStatus {
    fn as_ref(&self) -> &str {
        match self {
            TaskStatus::Waiting => "waiting",
            TaskStatus::Running => "running",
            TaskStatus::Done => "done",
            TaskStatus::Failed => "failed",
        }
    }
}

impl FromStr for TaskStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "waiting" => Ok(TaskStatus::Waiting),
            "running" => Ok(TaskStatus::Running),
            "done" => Ok(TaskStatus::Done),
            "failed" => Ok(TaskStatus::Failed),
            _ => Err(()),
        }
    }
}

struct TaskCounts {
    waiting: usize,
    running: usize,
    done: usize,
    failed: usize,
}

fn task_counts(tasks: &[LoopTask]) -> TaskCounts {
    let mut counts = TaskCounts {
        waiting: 0,
        running: 0,
        done: 0,
        failed: 0,
    };
    for task in tasks {
        match task.status {
            TaskStatus::Waiting => counts.waiting += 1,
            TaskStatus::Running => counts.running += 1,
            TaskStatus::Done => counts.done += 1,
            TaskStatus::Failed => counts.failed += 1,
        }
    }
    counts
}
