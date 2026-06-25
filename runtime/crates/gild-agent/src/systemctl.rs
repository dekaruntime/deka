use anyhow::{Context, Result, bail};
use regex::Regex;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

use crate::audit::{open_audit_log, timestamp_millis};
use crate::auth::PeerCredentials;
use crate::config::AppState;
use crate::protocol::ServiceReply;
use crate::requests::SystemctlRequest;

pub(crate) fn handle_systemctl_request(
    state: &AppState,
    request: &SystemctlRequest,
    peer: PeerCredentials,
) -> ServiceReply {
    if request.op != "systemctl" {
        return ServiceReply::json_value(
            400,
            serde_json::json!({ "error": "op must be systemctl" }),
        );
    }
    if let Err(err) = validate_systemctl_action(&request.action) {
        return ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }));
    }
    if let Err(err) = validate_systemctl_unit(&request.unit) {
        return ServiceReply::json_value(400, serde_json::json!({ "error": err.to_string() }));
    }

    let args = systemctl_args(&request.action, &request.unit);
    let output = match state.command_runner.output("/usr/bin/systemctl", &args) {
        Ok(result) => result,
        Err(err) => {
            let audit = write_systemctl_audit(
                &state.audit_log_path,
                &request.action,
                &request.unit,
                None,
                peer,
            );
            let audit_error = audit.err().map(|err| err.to_string());
            return ServiceReply::json_value(
                500,
                serde_json::json!({
                    "ok": false,
                    "op": "systemctl",
                    "action": request.action,
                    "unit": request.unit,
                    "exit": null,
                    "stdout": "",
                    "stderr": "",
                    "error": err.to_string(),
                    "audit_error": audit_error
                }),
            );
        }
    };

    let audit_error = write_systemctl_audit(
        &state.audit_log_path,
        &request.action,
        &request.unit,
        output.exit,
        peer,
    )
    .err()
    .map(|err| err.to_string());
    let ok = output.success;
    ServiceReply::json_value(
        if ok { 200 } else { 500 },
        serde_json::json!({
            "ok": ok,
            "op": "systemctl",
            "action": request.action,
            "unit": request.unit,
            "exit": output.exit,
            "stdout": output.stdout,
            "stderr": output.stderr,
            "audit_error": audit_error
        }),
    )
}

pub(crate) fn validate_systemctl_action(action: &str) -> Result<()> {
    match action {
        "daemon-reload" | "start" | "stop" | "restart" | "reload" | "enable" | "disable"
        | "status" => Ok(()),
        _ => bail!("invalid systemctl action"),
    }
}

fn systemctl_args<'a>(action: &'a str, unit: &'a str) -> Vec<&'a str> {
    if action == "daemon-reload" {
        vec![action]
    } else {
        vec![action, unit]
    }
}

pub(crate) fn validate_systemctl_unit(unit: &str) -> Result<()> {
    static UNIT_RE: OnceLock<Regex> = OnceLock::new();
    let re = UNIT_RE.get_or_init(|| {
        Regex::new(r"^gg\.tana\.[a-z][a-z0-9.@-]+\.(service|timer)$")
            .expect("systemctl unit regex compiles")
    });
    if re.is_match(unit) {
        Ok(())
    } else {
        bail!("invalid systemctl unit")
    }
}

fn write_systemctl_audit(
    path: &Path,
    action: &str,
    unit: &str,
    exit: Option<i32>,
    peer: PeerCredentials,
) -> Result<()> {
    let mut file = open_audit_log(path)?;
    let line = serde_json::json!({
        "ts": timestamp_millis().to_string(),
        "op": "systemctl",
        "action": action,
        "unit": unit,
        "exit": exit,
        "by_uid": peer.uid,
        "by_pid": peer.pid
    });
    writeln!(file, "{line}").context("write audit log")
}
