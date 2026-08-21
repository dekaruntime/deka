use core::{CommandSpec, Context, FlagSpec, ParamSpec, Registry, SubcommandSpec};
use std::collections::HashMap;

mod runner;

const RUN: SubcommandSpec = SubcommandSpec {
    name: "run",
    summary: "run a pipeline from a linkhash.yaml file in a gild VM",
    aliases: &[],
    handler: run_cmd,
};

const SUBCOMMANDS: &[SubcommandSpec] = &[RUN];

const COMMAND: CommandSpec = CommandSpec {
    name: "deploy",
    category: "pipeline",
    summary: "linkhash pipeline deployment and execution",
    aliases: &[],
    subcommands: SUBCOMMANDS,
    handler: deploy_cmd,
};

pub fn register(registry: &mut Registry) {
    registry.add_command(COMMAND);
    registry.add_flag(FlagSpec {
        name: "--gild-socket",
        aliases: &[],
        description: "gild Unix socket path (default: /run/gild/sock)",
    });
    registry.add_param(ParamSpec {
        name: "--gild-bearer",
        description: "gild bearer token (default: from env GILD_BEARER_TOKEN)",
    });
    registry.add_param(ParamSpec {
        name: "--run-id",
        description: "pipeline run ID (default: auto-generated)",
    });
}

fn deploy_cmd(_context: &Context) {
    stdio::raw("Usage: deka deploy run <pipeline-path>");
    stdio::raw("");
    stdio::raw("Subcommands:");
    stdio::raw("  run     run a pipeline from a linkhash.yaml file in a gild VM");
}

fn run_cmd(context: &Context) {
    let pipeline_path = if let Some(first) = context.args.positionals.first() {
        first.clone()
    } else if context.args.positionals.is_empty() && context.args.commands.len() > 2 {
        context.args.commands[2].clone()
    } else {
        stdio::error(
            "deploy",
            "missing pipeline file path. Usage: deka deploy run <linkhash.yaml>",
        );
        return;
    };

    let gild_socket = context
        .args
        .params
        .get("--gild-socket")
        .cloned()
        .or_else(|| std::env::var("GILD_SOCKET_PATH").ok())
        .unwrap_or_else(|| "/run/gild/sock".to_string());

    let bearer_token = context
        .args
        .params
        .get("--gild-bearer")
        .cloned()
        .or_else(|| std::env::var("GILD_BEARER_TOKEN").ok())
        .unwrap_or_default();

    let run_id = context
        .args
        .params
        .get("--run-id")
        .cloned()
        .unwrap_or_else(|| {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("deploy-{}", ts)
        });

    let pipeline = match crate::cli::pipeline_yaml::parse_pipeline_yaml(&pipeline_path) {
        Ok(p) => p,
        Err(e) => {
            stdio::error("deploy", &format!("failed to parse pipeline: {}", e));
            return;
        }
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            stdio::error("deploy", &format!("failed to start runtime: {}", e));
            return;
        }
    };

    runtime.block_on(run_pipeline(
        &pipeline,
        &run_id,
        &gild_socket,
        &bearer_token,
    ));
}

async fn run_pipeline(
    pipeline: &crate::cli::pipeline_yaml::Pipeline,
    run_id: &str,
    gild_socket: &str,
    bearer_token: &str,
) {
    let total_jobs = pipeline.jobs.len();
    let mut completed = 0u32;
    let mut any_failed = false;

    for (job_name, job) in &pipeline.jobs {
        if job.runs_on != "gild" {
            stdio::warn_simple(&format!(
                "job `{}` has runs-on: `{}`, only `gild` is supported; skipping",
                job_name, job.runs_on
            ));
            continue;
        }

        if job.steps.is_empty() {
            stdio::warn_simple(&format!("job `{}` has no steps; skipping", job_name));
            continue;
        }

        stdio::log("deploy", &format!("running job `{}` in gild...", job_name));

        let argv: Vec<String> = build_job_argv(job);
        let payload = runner::GildExecuteRequest {
            run_id: format!("{}-{}", run_id, job_name),
            agent_slug: "deka-deploy".to_string(),
            argv,
            cwd: "/workspace".to_string(),
            stdin: String::new(),
            env: HashMap::new(),
            mounts: vec![runner::Mount {
                host: std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                guest: "/workspace".to_string(),
                ro: Some(true),
            }],
            network_policy: runner::NetworkPolicy {
                allow_ipsets: vec![],
                deny_cidrs: vec![],
            },
            vsock_callbacks: vec![],
            timeout_ms: 3600000,
            memory: "2048M".to_string(),
        };

        match runner::gild_execute_unix(&payload, gild_socket, bearer_token).await {
            Ok(response) => {
                if !response.stdout.is_empty() {
                    println!("[{} stdout]", job_name);
                    println!("{}", response.stdout);
                }
                if !response.stderr.is_empty() {
                    eprintln!("[{} stderr]", job_name);
                    eprintln!("{}", response.stderr);
                }

                if response.exit_code == 0 {
                    stdio::success(&format!(
                        "job `{}` completed ({} ms)",
                        job_name, response.duration_ms
                    ));
                } else {
                    stdio::error(
                        "deploy",
                        &format!(
                            "job `{}` failed with exit code {}",
                            job_name, response.exit_code
                        ),
                    );
                    any_failed = true;
                }
            }
            Err(e) => {
                stdio::error("deploy", &format!("job `{}` gild error: {}", job_name, e));
                any_failed = true;
            }
        }

        completed += 1;
    }

    if total_jobs > 1 {
        let passed = total_jobs as u32 - if any_failed { 1 } else { 0 };
        stdio::log(
            "deploy",
            &format!("pipeline finished: {}/{} jobs passed", passed, total_jobs),
        );
    }

    if any_failed {
        std::process::exit(1);
    }
}

fn build_job_argv(job: &crate::cli::pipeline_yaml::Job) -> Vec<String> {
    let mut argv = Vec::new();
    for step in &job.steps {
        if let Some(run) = &step.run {
            argv.push(run.clone());
        }
    }
    argv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::pipeline_yaml::{Job, Step};

    #[test]
    fn build_job_argv_collects_run_commands() {
        let job = Job {
            runs_on: "gild".to_string(),
            target: None,
            steps: vec![
                Step {
                    run: Some("cargo build".to_string()),
                    uses: None,
                    with: HashMap::new(),
                },
                Step {
                    run: Some("cargo test".to_string()),
                    uses: None,
                    with: HashMap::new(),
                },
                Step {
                    run: None,
                    uses: Some("tana-actions/checkout@v1".to_string()),
                    with: HashMap::new(),
                },
            ],
        };
        let argv = build_job_argv(&job);
        assert_eq!(argv.len(), 2);
        assert_eq!(argv[0], "cargo build");
        assert_eq!(argv[1], "cargo test");
    }

    #[test]
    fn build_job_argv_handles_no_run_steps() {
        let job = Job {
            runs_on: "gild".to_string(),
            target: None,
            steps: vec![Step {
                run: None,
                uses: Some("tana-actions/checkout@v1".to_string()),
                with: HashMap::new(),
            }],
        };
        let argv = build_job_argv(&job);
        assert!(argv.is_empty());
    }

    #[test]
    fn build_job_argv_with_target_included() {
        let job = Job {
            runs_on: "gild".to_string(),
            target: Some("x86_64-unknown-linux-gnu".to_string()),
            steps: vec![Step {
                run: Some("cargo build --target ${{ job.target }}".to_string()),
                uses: None,
                with: HashMap::new(),
            }],
        };
        let argv = build_job_argv(&job);
        assert_eq!(argv.len(), 1);
        assert!(argv[0].contains("${{ job.target }}"));
    }

    #[test]
    fn gild_request_construction() {
        let job = Job {
            runs_on: "gild".to_string(),
            target: None,
            steps: vec![Step {
                run: Some("echo hello".to_string()),
                uses: None,
                with: HashMap::new(),
            }],
        };
        let argv = build_job_argv(&job);
        let payload = runner::GildExecuteRequest {
            run_id: "test-run".to_string(),
            agent_slug: "deka-deploy".to_string(),
            argv,
            cwd: "/workspace".to_string(),
            stdin: String::new(),
            env: HashMap::new(),
            mounts: vec![runner::Mount {
                host: "/host/path".to_string(),
                guest: "/workspace".to_string(),
                ro: Some(true),
            }],
            network_policy: runner::NetworkPolicy {
                allow_ipsets: vec![],
                deny_cidrs: vec![],
            },
            vsock_callbacks: vec![],
            timeout_ms: 3600000,
            memory: "2048M".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"run_id\":\"test-run\""));
        assert!(json.contains("\"argv\":[\"echo hello\"]"));
        assert!(json.contains("\"agent_slug\":\"deka-deploy\""));
        assert!(json.contains("\"cwd\":\"/workspace\""));
        assert!(json.contains("\"timeout_ms\":3600000"));
    }
}
