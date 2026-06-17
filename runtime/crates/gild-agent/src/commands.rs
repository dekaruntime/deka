use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::Serialize;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

use crate::audit::open_audit_log;
use crate::auth::PeerCredentials;
use crate::config::{AGENT_RUNTIME_GROUP, AppState, CommandOutput};
use crate::protocol::ServiceReply;
use crate::requests::{CreateAgentRequest, RemoveAgentRequest};

#[derive(Debug, Serialize)]
struct OperationReply {
    ok: bool,
    operation: &'static str,
    message: String,
    exit: Option<i32>,
    stdout: String,
    stderr: String,
    error: Option<String>,
    pending: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct AuditRecord<'a> {
    ts: u64,
    op: &'a str,
    slug: &'a str,
    exit: Option<i32>,
    by_uid: u32,
    by_pid: u32,
}

struct CommandReplySpec<'a> {
    audit_op: &'static str,
    operation: &'static str,
    slug: &'a str,
    success_message: String,
    pending: Vec<&'static str>,
}

pub(crate) fn create_agent(
    state: &AppState,
    peer: PeerCredentials,
    request: &CreateAgentRequest,
) -> ServiceReply {
    if let Err(err) = validate_slug(&request.slug) {
        let _ = audit(state, "useradd", &request.slug, None, peer);
        return ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }));
    }
    if request.persona_ref.trim().is_empty() {
        let _ = audit(state, "useradd", &request.slug, None, peer);
        return ServiceReply::json_value(
            400,
            serde_json::json!({ "error": "persona_ref is required" }),
        );
    }
    match user_exists(&request.slug, &state.passwd_path) {
        Ok(true) => {
            let _ = audit(state, "useradd", &request.slug, None, peer);
            return ServiceReply::json_value(
                409,
                serde_json::json!({ "error": format!("user {} already exists", request.slug) }),
            );
        }
        Ok(false) => {}
        Err(err) => {
            let _ = audit(state, "useradd", &request.slug, None, peer);
            return ServiceReply::json_value(500, serde_json::json!({ "error": err.to_string() }));
        }
    }

    let group_output = state.command_runner.output(
        "/usr/sbin/groupadd",
        &["--system", "--force", AGENT_RUNTIME_GROUP],
    );
    match group_output {
        Ok(output) if output.success => {}
        Ok(output) => {
            let _ = audit(state, "groupadd", AGENT_RUNTIME_GROUP, output.exit, peer);
            return ServiceReply::json_value(
                500,
                serde_json::json!({
                    "ok": false,
                    "operation": "agent.create",
                    "message": format!("groupadd failed for {AGENT_RUNTIME_GROUP}"),
                    "exit": output.exit,
                    "stdout": output.stdout,
                    "stderr": output.stderr,
                    "error": format!("groupadd failed with exit {:?}", output.exit),
                    "pending": ["useradd", "systemd unit write", "sudoers setup"]
                }),
            );
        }
        Err(err) => {
            let _ = audit(state, "groupadd", AGENT_RUNTIME_GROUP, None, peer);
            return ServiceReply::json_value(
                500,
                serde_json::json!({
                    "ok": false,
                    "operation": "agent.create",
                    "message": format!("groupadd failed for {AGENT_RUNTIME_GROUP}"),
                    "exit": null,
                    "stdout": "",
                    "stderr": "",
                    "error": err.to_string(),
                    "pending": ["useradd", "systemd unit write", "sudoers setup"]
                }),
            );
        }
    }

    let output = state.command_runner.output(
        "/usr/sbin/useradd",
        &[
            "--create-home",
            "--shell",
            "/bin/bash",
            "--gid",
            AGENT_RUNTIME_GROUP,
            &request.slug,
        ],
    );
    command_reply(
        state,
        peer,
        output,
        CommandReplySpec {
            audit_op: "useradd",
            operation: "agent.create",
            slug: &request.slug,
            success_message: format!("created Linux user {}", request.slug),
            pending: vec!["systemd unit write", "sudoers setup"],
        },
    )
}

pub(crate) fn remove_agent(
    state: &AppState,
    peer: PeerCredentials,
    request: &RemoveAgentRequest,
) -> ServiceReply {
    if let Err(err) = validate_slug(&request.slug) {
        let _ = audit(state, "userdel", &request.slug, None, peer);
        return ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }));
    }

    match running_processes(state, &request.slug) {
        Ok(Some(processes)) => {
            let _ = audit(state, "userdel", &request.slug, None, peer);
            return ServiceReply::json_value(
                409,
                serde_json::json!({
                    "ok": false,
                    "operation": "agent.remove",
                    "error": format!("user {} still has running processes", request.slug),
                    "processes": processes,
                }),
            );
        }
        Ok(None) => {}
        Err(err) => {
            let _ = audit(state, "userdel", &request.slug, None, peer);
            return ServiceReply::json_value(500, serde_json::json!({ "error": err.to_string() }));
        }
    }

    let output = state
        .command_runner
        .output("/usr/sbin/userdel", &["--remove", &request.slug]);
    command_reply(
        state,
        peer,
        output,
        CommandReplySpec {
            audit_op: "userdel",
            operation: "agent.remove",
            slug: &request.slug,
            success_message: format!("removed Linux user {}", request.slug),
            pending: vec!["systemd disable"],
        },
    )
}

pub(crate) fn not_implemented(operation: &str, target: &str) -> ServiceReply {
    ServiceReply::json_value(
        501,
        serde_json::json!({
            "ok": false,
            "operation": operation,
            "target": target,
            "error": "not yet implemented"
        }),
    )
}

pub(crate) fn validate_slug(slug: &str) -> Result<()> {
    static SLUG_RE: OnceLock<Regex> = OnceLock::new();
    let re = SLUG_RE.get_or_init(|| {
        Regex::new(r"^agent-[a-z][a-z0-9-]{1,30}$").expect("agent slug regex compiles")
    });
    if !re.is_match(slug) {
        bail!("slug must match ^agent-[a-z][a-z0-9-]{{1,30}}$");
    }
    Ok(())
}
fn command_reply(
    state: &AppState,
    peer: PeerCredentials,
    output: io::Result<CommandOutput>,
    spec: CommandReplySpec<'_>,
) -> ServiceReply {
    match output {
        Ok(output) => {
            let _ = audit(state, spec.audit_op, spec.slug, output.exit, peer);
            let error = (!output.success).then(|| {
                format!(
                    "{} failed with exit {:?}: {}",
                    spec.audit_op,
                    output.exit,
                    output.stderr.trim()
                )
            });
            let status = if output.success { 200 } else { 500 };
            ServiceReply::json_value(
                status,
                serde_json::json!(OperationReply {
                    ok: output.success,
                    operation: spec.operation,
                    message: if output.success {
                        spec.success_message
                    } else {
                        format!("{} failed for {}", spec.audit_op, spec.slug)
                    },
                    exit: output.exit,
                    stdout: output.stdout,
                    stderr: output.stderr,
                    error,
                    pending: spec.pending,
                }),
            )
        }
        Err(err) => {
            let _ = audit(state, spec.audit_op, spec.slug, None, peer);
            ServiceReply::json_value(
                500,
                serde_json::json!(OperationReply {
                    ok: false,
                    operation: spec.operation,
                    message: format!("{} failed for {}", spec.audit_op, spec.slug),
                    exit: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    error: Some(err.to_string()),
                    pending: spec.pending,
                }),
            )
        }
    }
}

pub(crate) fn user_exists(username: &str, passwd_path: &Path) -> Result<bool> {
    let raw = match fs::read_to_string(passwd_path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("read {}", passwd_path.display())),
    };
    Ok(raw.lines().any(|line| {
        line.split_once(':')
            .map(|(name, _)| name == username)
            .unwrap_or(false)
    }))
}

pub(crate) fn running_processes(state: &AppState, username: &str) -> Result<Option<Vec<u32>>> {
    let output = state
        .command_runner
        .output("/usr/bin/pgrep", &["-u", username])
        .context("run /usr/bin/pgrep")?;
    match output.exit {
        Some(0) => {
            let processes = output
                .stdout
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect::<Vec<_>>();
            Ok(Some(processes))
        }
        Some(1) => Ok(None),
        _ => bail!(
            "pgrep failed with exit {:?}: {}",
            output.exit,
            output.stderr.trim()
        ),
    }
}

pub(crate) fn audit(
    state: &AppState,
    op: &'static str,
    slug: &str,
    exit: Option<i32>,
    peer: PeerCredentials,
) -> Result<()> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock is before UNIX epoch")?
        .as_secs();
    let line = serde_json::to_string(&AuditRecord {
        ts,
        op,
        slug,
        exit,
        by_uid: peer.uid,
        by_pid: peer.pid,
    })?;
    let mut file = open_audit_log(&state.audit_log_path)?;
    writeln!(file, "{line}").context("write audit log")?;
    eprintln!("{line}");
    Ok(())
}
