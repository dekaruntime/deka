use core::{CommandSpec, Context, FlagSpec, Registry};
use deno_task_shell::ExecutableCommand;
use deno_task_shell::KillSignal;
use deno_task_shell::ShellCommand;
use deno_task_shell::ShellPipeWriter;
use deno_task_shell::ShellState;
use deno_task_shell::execute_with_pipes;
use deno_task_shell::parser::parse;
use deno_task_shell::pipe;
use glob::Pattern;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::thread;
use stdio::{raw, warn_simple};

const COMMAND: CommandSpec = CommandSpec {
    name: "task",
    category: "runtime",
    summary: "run a task from deka.json",
    aliases: &[],
    subcommands: &[],
    handler: cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(FlagSpec {
        name: "--list",
        aliases: &[],
        description: "list tasks",
    });
    registry.add_flag(FlagSpec {
        name: "--json",
        aliases: &[],
        description: "output tasks as json",
    });
}

pub fn cmd(context: &Context) {
    let cwd = std::env::current_dir().ok();
    let Some(cwd) = cwd else {
        stdio::error("task", "failed to resolve current directory");
        return;
    };
    let Some((project_root, json)) = load_deka_json(&cwd) else {
        stdio::error("task", "deka.json not found (searched parent directories)");
        return;
    };

    let (tasks, used_scripts) = extract_tasks(&json);
    if used_scripts {
        warn_simple("scripts is deprecated for tasks; prefer tasks in deka.json");
    }

    let task_name = requested_task_name(context);
    if context.args.flags.contains_key("--json") {
        print_task_json(&tasks);
        return;
    }
    if task_name.is_none() || context.args.flags.contains_key("--list") {
        print_task_list(&tasks);
        return;
    }

    let task_name = task_name.unwrap();
    if let Err(message) = run_tasks(task_name, &tasks, &project_root, &cwd) {
        stdio::error("task", &message);
        print_task_list(&tasks);
    }
}

fn requested_task_name<'a>(context: &'a Context) -> Option<&'a str> {
    if let Some(first) = context.args.positionals.first() {
        return Some(first.as_str());
    }
    if context.args.commands.len() > 1 {
        return Some(context.args.commands[1].as_str());
    }
    None
}

#[derive(Debug, Clone)]
struct TaskDef {
    command: String,
    description: Option<String>,
    dependencies: Vec<String>,
}

fn load_deka_json(start: &Path) -> Option<(PathBuf, Value)> {
    for dir in start.ancestors() {
        let path = dir.join("deka.json");
        if !path.is_file() {
            continue;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        let json = serde_json::from_str(&content).ok()?;
        return Some((dir.to_path_buf(), json));
    }
    None
}

fn extract_tasks(json: &Value) -> (BTreeMap<String, TaskDef>, bool) {
    let tasks = json.get("tasks").and_then(|value| value.as_object());
    let scripts = json.get("scripts").and_then(|value| value.as_object());
    let used_scripts = scripts.is_some();

    let mut out = BTreeMap::new();
    if let Some(tasks) = tasks {
        ingest_task_map(&mut out, tasks, true);
    }
    if let Some(scripts) = scripts {
        ingest_task_map(&mut out, scripts, false);
    }

    (out, used_scripts)
}

fn ingest_task_map(
    out: &mut BTreeMap<String, TaskDef>,
    source: &serde_json::Map<String, Value>,
    allow_overwrite: bool,
) {
    for (name, value) in source {
        if !allow_overwrite && out.contains_key(name) {
            continue;
        }
        if let Some(command) = value.as_str() {
            out.insert(
                name.to_string(),
                TaskDef {
                    command: command.to_string(),
                    description: None,
                    dependencies: Vec::new(),
                },
            );
            continue;
        }

        let Some(obj) = value.as_object() else {
            warn_simple(&format!(
                "task `{}` ignored: expected string or object",
                name
            ));
            continue;
        };

        let command = obj
            .get("command")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        let Some(command) = command else {
            warn_simple(&format!(
                "task `{}` ignored: missing required `command`",
                name
            ));
            continue;
        };

        let description = obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());
        let dependencies = obj
            .get("dependencies")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(|value| value.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        out.insert(
            name.to_string(),
            TaskDef {
                command,
                description,
                dependencies,
            },
        );
    }
}

fn print_task_list(tasks: &BTreeMap<String, TaskDef>) {
    if tasks.is_empty() {
        stdio::error("task", "no tasks found in deka.json");
        return;
    }

    raw("Note: tasks run in the CLI shell and are not subject to runtime permission gates.");
    raw("Tasks:");
    for (name, task) in tasks {
        let mut line = String::new();
        line.push_str("  ");
        line.push_str(name);
        if let Some(desc) = task.description.as_deref() {
            line.push_str("  - ");
            line.push_str(desc);
        }
        if !task.dependencies.is_empty() {
            line.push_str("  (deps: ");
            line.push_str(&task.dependencies.join(", "));
            line.push(')');
        }
        if !task.command.is_empty() {
            line.push_str("  -> ");
            line.push_str(&task.command);
        }
        raw(&line);
    }
}

fn print_task_json(tasks: &BTreeMap<String, TaskDef>) {
    let json = tasks_to_json(tasks);
    match serde_json::to_string_pretty(&json) {
        Ok(text) => raw(&text),
        Err(err) => stdio::error("task", &format!("failed to format tasks: {}", err)),
    }
}

fn tasks_to_json(tasks: &BTreeMap<String, TaskDef>) -> Value {
    let mut out = serde_json::Map::new();
    for (name, task) in tasks {
        let mut obj = serde_json::Map::new();
        obj.insert("command".to_string(), Value::String(task.command.clone()));
        if let Some(desc) = &task.description {
            obj.insert("description".to_string(), Value::String(desc.clone()));
        }
        if !task.dependencies.is_empty() {
            obj.insert(
                "dependencies".to_string(),
                Value::Array(
                    task.dependencies
                        .iter()
                        .map(|dep| Value::String(dep.clone()))
                        .collect(),
                ),
            );
        }
        out.insert(name.clone(), Value::Object(obj));
    }
    Value::Object(out)
}

fn run_task(
    name: &str,
    task: &TaskDef,
    project_root: &Path,
    init_cwd: &Path,
) -> Result<(), String> {
    if task.command.trim().is_empty() {
        return Err(format!("task `{}` has empty command", name));
    }

    let list = parse(&task.command).map_err(|err| err.to_string())?;
    // Deliberate child-process env inheritance: task subprocesses need the
    // ambient OS environment (PATH, HOME, etc.) to run user commands.
    // Scoping the child env is a deka#801 follow-up decision.
    let mut env_vars = HashMap::new();
    for (key, value) in std::env::vars_os() {
        env_vars.insert(key, value);
    }
    env_vars.insert(OsString::from("INIT_CWD"), OsString::from(init_cwd));

    let deka_exe = std::env::current_exe()
        .map_err(|err| format!("failed to resolve deka executable: {}", err))?;
    let mut custom_commands: HashMap<String, Rc<dyn ShellCommand>> = HashMap::new();
    custom_commands.insert(
        "deka".to_string(),
        Rc::new(ExecutableCommand::new("deka".to_string(), deka_exe)),
    );

    let kill_signal = KillSignal::default();
    let (stdin, stdin_writer) = pipe();
    drop(stdin_writer);
    let stdout = ShellPipeWriter::stdout();
    let stderr = ShellPipeWriter::stderr();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("failed to start task runtime: {}", err))?;
    let exit_code = runtime.block_on(async move {
        let local = tokio::task::LocalSet::new();
        let state = ShellState::new(
            env_vars,
            project_root.to_path_buf(),
            custom_commands,
            kill_signal,
        );
        local
            .run_until(execute_with_pipes(list, state, stdin, stdout, stderr))
            .await
    });

    if exit_code == 0 {
        Ok(())
    } else {
        Err(format!(
            "task `{}` failed with exit code {}",
            name, exit_code
        ))
    }
}

fn run_tasks(
    name: &str,
    tasks: &BTreeMap<String, TaskDef>,
    project_root: &Path,
    init_cwd: &Path,
) -> Result<(), String> {
    let runner = TaskRunner::new(tasks, project_root, init_cwd);
    if tasks.contains_key(name) {
        return runner.run(name, Vec::new());
    }

    let is_pattern = name.contains('*') || name.contains('?') || name.contains('[');
    if !is_pattern {
        return Err(format!("unknown task `{}`", name));
    }

    let pattern = Pattern::new(name).map_err(|err| err.to_string())?;
    let matches = tasks
        .keys()
        .filter(|task_name| pattern.matches(task_name))
        .cloned()
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Err(format!("no tasks matched `{}`", name));
    }
    let mut handles = Vec::new();
    for task_name in matches {
        let runner = runner.clone();
        handles.push(thread::spawn(move || runner.run(&task_name, Vec::new())));
    }
    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(_) => return Err("task thread panicked".to_string()),
        }
    }
    Ok(())
}

#[derive(Clone)]
struct TaskRunner {
    tasks: Arc<BTreeMap<String, TaskDef>>,
    project_root: PathBuf,
    init_cwd: PathBuf,
    state: Arc<(Mutex<HashMap<String, TaskState>>, Condvar)>,
}

#[derive(Clone)]
enum TaskState {
    InProgress,
    Done(Result<(), String>),
}

impl TaskRunner {
    fn new(tasks: &BTreeMap<String, TaskDef>, project_root: &Path, init_cwd: &Path) -> Self {
        Self {
            tasks: Arc::new(tasks.clone()),
            project_root: project_root.to_path_buf(),
            init_cwd: init_cwd.to_path_buf(),
            state: Arc::new((Mutex::new(HashMap::new()), Condvar::new())),
        }
    }

    fn run(&self, name: &str, stack: Vec<String>) -> Result<(), String> {
        if stack.iter().any(|item| item == name) {
            let mut chain = stack;
            chain.push(name.to_string());
            return Err(format!("task dependency cycle: {}", chain.join(" -> ")));
        }
        let task = self
            .tasks
            .get(name)
            .ok_or_else(|| format!("unknown task `{}`", name))?;

        let (lock, cvar) = &*self.state;
        let mut guard = lock.lock().map_err(|_| "task state poisoned".to_string())?;
        loop {
            match guard.get(name) {
                Some(TaskState::Done(result)) => return result.clone(),
                Some(TaskState::InProgress) => {
                    guard = cvar
                        .wait(guard)
                        .map_err(|_| "task state poisoned".to_string())?;
                }
                None => {
                    guard.insert(name.to_string(), TaskState::InProgress);
                    break;
                }
            }
        }
        drop(guard);

        let mut deps = task.dependencies.clone();
        deps.sort();
        let mut handles = Vec::new();
        for dep in deps {
            let mut next_stack = stack.clone();
            next_stack.push(name.to_string());
            let runner = self.clone();
            handles.push(thread::spawn(move || runner.run(&dep, next_stack)));
        }
        for handle in handles {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    self.finish(name, Err(err.clone()));
                    return Err(err);
                }
                Err(_) => {
                    let message = "task thread panicked".to_string();
                    self.finish(name, Err(message.clone()));
                    return Err(message);
                }
            }
        }

        let result = run_task(name, task, &self.project_root, &self.init_cwd);
        self.finish(name, result.clone());
        result
    }

    fn finish(&self, name: &str, result: Result<(), String>) {
        let (lock, cvar) = &*self.state;
        if let Ok(mut guard) = lock.lock() {
            guard.insert(name.to_string(), TaskState::Done(result));
            cvar.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn merges_tasks_and_scripts_with_task_precedence() {
        let doc = json!({
            "tasks": {
                "dev": "echo task"
            },
            "scripts": {
                "dev": "echo script",
                "build": "echo build"
            }
        });
        let (tasks, used_scripts) = extract_tasks(&doc);
        assert!(used_scripts);
        assert_eq!(tasks.get("dev").unwrap().command, "echo task");
        assert_eq!(tasks.get("build").unwrap().command, "echo build");
    }

    #[test]
    fn runs_task_with_init_cwd() {
        let project_root = tempdir().unwrap();
        let init_cwd = tempdir().unwrap();
        let task = TaskDef {
            command: "echo $INIT_CWD > init.txt".to_string(),
            description: None,
            dependencies: Vec::new(),
        };
        run_task("init", &task, project_root.path(), init_cwd.path()).expect("task should succeed");
        let contents = fs::read_to_string(project_root.path().join("init.txt")).unwrap();
        assert_eq!(contents.trim(), init_cwd.path().to_string_lossy().as_ref());
    }

    #[test]
    fn runs_dependencies_before_task() {
        let project_root = tempdir().unwrap();
        let init_cwd = project_root.path();
        let mut tasks = BTreeMap::new();
        tasks.insert(
            "prepare".to_string(),
            TaskDef {
                command: "echo ready > ready.txt".to_string(),
                description: None,
                dependencies: Vec::new(),
            },
        );
        tasks.insert(
            "build".to_string(),
            TaskDef {
                command: "echo build > build.txt".to_string(),
                description: None,
                dependencies: vec!["prepare".to_string()],
            },
        );
        run_tasks("build", &tasks, project_root.path(), init_cwd).expect("task should succeed");
        assert!(project_root.path().join("ready.txt").is_file());
        assert!(project_root.path().join("build.txt").is_file());
    }

    #[test]
    fn dependencies_run_once_across_parallel_tasks() {
        let project_root = tempdir().unwrap();
        let init_cwd = project_root.path();
        let mut tasks = BTreeMap::new();
        tasks.insert(
            "prep".to_string(),
            TaskDef {
                command: "echo ready >> log.txt".to_string(),
                description: None,
                dependencies: Vec::new(),
            },
        );
        tasks.insert(
            "build-a".to_string(),
            TaskDef {
                command: "echo a > a.txt".to_string(),
                description: None,
                dependencies: vec!["prep".to_string()],
            },
        );
        tasks.insert(
            "build-b".to_string(),
            TaskDef {
                command: "echo b > b.txt".to_string(),
                description: None,
                dependencies: vec!["prep".to_string()],
            },
        );
        run_tasks("build-*", &tasks, project_root.path(), init_cwd).expect("tasks should succeed");
        let log = fs::read_to_string(project_root.path().join("log.txt")).unwrap();
        assert_eq!(log.lines().count(), 1);
    }

    #[test]
    fn runs_wildcard_tasks() {
        let project_root = tempdir().unwrap();
        let init_cwd = project_root.path();
        let mut tasks = BTreeMap::new();
        tasks.insert(
            "lint-a".to_string(),
            TaskDef {
                command: "echo a > a.txt".to_string(),
                description: None,
                dependencies: Vec::new(),
            },
        );
        tasks.insert(
            "lint-b".to_string(),
            TaskDef {
                command: "echo b > b.txt".to_string(),
                description: None,
                dependencies: Vec::new(),
            },
        );
        run_tasks("lint-*", &tasks, project_root.path(), init_cwd).expect("tasks should succeed");
        assert!(project_root.path().join("a.txt").is_file());
        assert!(project_root.path().join("b.txt").is_file());
    }

    #[test]
    fn json_output_includes_details() {
        let mut tasks = BTreeMap::new();
        tasks.insert(
            "build".to_string(),
            TaskDef {
                command: "deka build".to_string(),
                description: Some("Build the app".to_string()),
                dependencies: vec!["prep".to_string()],
            },
        );
        let json = tasks_to_json(&tasks);
        let build = json.pointer("/build").expect("build task");
        assert_eq!(
            build.get("command").and_then(|v| v.as_str()),
            Some("deka build")
        );
        assert_eq!(
            build.get("description").and_then(|v| v.as_str()),
            Some("Build the app")
        );
        let deps = build
            .get("dependencies")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].as_str(), Some("prep"));
    }

    #[test]
    fn dependency_failure_order_is_deterministic() {
        let project_root = tempdir().unwrap();
        let init_cwd = project_root.path();
        let mut tasks = BTreeMap::new();
        tasks.insert(
            "a".to_string(),
            TaskDef {
                command: "false".to_string(),
                description: None,
                dependencies: Vec::new(),
            },
        );
        tasks.insert(
            "b".to_string(),
            TaskDef {
                command: "false".to_string(),
                description: None,
                dependencies: Vec::new(),
            },
        );
        tasks.insert(
            "root".to_string(),
            TaskDef {
                command: "true".to_string(),
                description: None,
                dependencies: vec!["b".to_string(), "a".to_string()],
            },
        );
        let err =
            run_tasks("root", &tasks, project_root.path(), init_cwd).expect_err("should fail");
        assert!(err.contains("task `a` failed"));
    }
}
