use std::process::{Child, Command, Output, Stdio};

use anyhow::{Context, Result, bail};

pub fn spawn_agent(host: &str, cmd: &str) -> Result<Child> {
    validate_host(host)?;
    let mut ssh = Command::new("ssh");
    // cmd is a single shell-safe token sequence; quoting unsupported in Part 1;
    // revisit if Part 2 scenarios need it.
    ssh.arg("--")
        .arg(host)
        .arg("sudo")
        .arg("gild-storm-agent")
        .args(cmd.split_whitespace())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    ssh.spawn()
        .with_context(|| format!("spawn ssh to {host} for gild-storm-agent {cmd}"))
}

pub fn run_systemd_is_active(host: &str, service: &str) -> Result<Output> {
    validate_host(host)?;
    Command::new("ssh")
        .arg("--")
        .arg(host)
        .arg("systemctl")
        .arg("is-active")
        .arg(service)
        .output()
        .with_context(|| format!("ssh {host} systemctl is-active {service}"))
}

pub fn abort_agents(hosts: &[String]) {
    for host in hosts {
        if validate_host(host).is_err() {
            continue;
        }
        let _ = Command::new("ssh")
            .arg("--")
            .arg(host)
            .arg("sudo")
            .arg("pkill")
            .arg("-TERM")
            .arg("-f")
            .arg("gild-storm-agent")
            .status();
    }
}

pub fn validate_host(host: &str) -> Result<()> {
    if host.starts_with('-') {
        bail!("ssh host must not start with '-'");
    }
    let mut chars = host.chars();
    let Some(first) = chars.next() else {
        bail!("ssh host is required");
    };
    if !first.is_ascii_alphanumeric() {
        bail!("ssh host must start with an ASCII alphanumeric character");
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')) {
        bail!("ssh host contains unsupported characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_ssh_hosts() {
        for host in ["-oProxyCommand=sh", "bad host", "bad;host", ".hidden", ""] {
            assert!(validate_host(host).is_err(), "{host:?} should be rejected");
        }
    }

    #[test]
    fn accepts_safe_ssh_hosts() {
        for host in ["demon", "phobos.local", "host_1", "host-1.example"] {
            validate_host(host).unwrap();
        }
    }
}
