use std::collections::{BTreeSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, anyhow, bail};

use crate::scenario::{Assertion, AssertionKind, Fault, Scenario};
use crate::ssh;

pub struct RunResult {
    pub passed: bool,
    pub run_id: String,
    pub log_path: PathBuf,
}

struct Logger {
    file: std::fs::File,
}

impl Logger {
    fn new(run_id: &str) -> Result<(Self, PathBuf)> {
        let dir = PathBuf::from("/var/log/gild-storm");
        let fallback = std::env::temp_dir().join("gild-storm");
        let dir = match fs::create_dir_all(&dir) {
            Ok(()) => dir,
            Err(_) => {
                fs::create_dir_all(&fallback)?;
                fallback
            }
        };
        let path = dir.join(format!("{run_id}.log"));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok((Self { file }, path))
    }

    fn line(&mut self, msg: impl AsRef<str>) {
        let ts = humantime::format_rfc3339_seconds(SystemTime::now());
        let line = format!("{ts} {}\n", msg.as_ref());
        print!("{line}");
        let _ = self.file.write_all(line.as_bytes());
        let _ = self.file.flush();
    }
}

#[derive(Clone)]
pub struct AbortState {
    hosts: Arc<Mutex<BTreeSet<String>>>,
}

impl AbortState {
    pub fn new() -> Self {
        Self {
            hosts: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    fn add(&self, host: &str) {
        if let Ok(mut hosts) = self.hosts.lock() {
            hosts.insert(host.to_string());
        }
    }

    fn remove(&self, host: &str) {
        if let Ok(mut hosts) = self.hosts.lock() {
            hosts.remove(host);
        }
    }

    pub fn abort(&self) {
        let hosts = self
            .hosts
            .lock()
            .map(|hosts| hosts.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        ssh::abort_agents(&hosts);
    }
}

pub fn run(scenario: Scenario, abort_state: AbortState) -> Result<RunResult> {
    let run_id = format!(
        "{}-{}",
        scenario.name,
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    );
    let (mut log, log_path) = Logger::new(&run_id)?;
    log.line(format!(
        "START scenario={} duration={:?}",
        scenario.name, scenario.duration
    ));

    let mut faults = scenario.faults.clone();
    faults.sort_by_key(|fault| fault.at);
    let mut assertions = scenario.assertions.clone();
    assertions.sort_by_key(|assertion| assertion.at);
    let mut faults: VecDeque<Fault> = faults.into();
    let mut assertions: VecDeque<Assertion> = assertions.into();
    let mut children: Vec<(String, String, Child)> = Vec::new();
    let start = Instant::now();
    let mut passed = true;

    while start.elapsed() <= scenario.duration || !children.is_empty() {
        while faults
            .front()
            .is_some_and(|fault| start.elapsed() >= fault.at)
        {
            let fault = faults.pop_front().unwrap();
            log.line(format!("FAULT host={} cmd={}", fault.host, fault.cmd));
            match ssh::spawn_agent(&fault.host, &fault.cmd) {
                Ok(child) => {
                    abort_state.add(&fault.host);
                    children.push((fault.host, fault.cmd, child));
                }
                Err(err) => {
                    passed = false;
                    log.line(format!("FAIL spawn fault: {err:#}"));
                }
            }
        }

        let mut idx = 0;
        while idx < children.len() {
            let done = match children[idx].2.try_wait() {
                Ok(Some(status)) => {
                    let (host, cmd, mut child) = children.remove(idx);
                    abort_state.remove(&host);
                    let mut stderr = String::new();
                    if let Some(mut stream) = child.stderr.take() {
                        let _ = std::io::Read::read_to_string(&mut stream, &mut stderr);
                    }
                    if status.success() {
                        log.line(format!("PASS fault host={host} cmd={cmd}"));
                    } else {
                        passed = false;
                        log.line(format!(
                            "FAIL fault host={host} cmd={cmd} status={status} stderr={}",
                            stderr.trim()
                        ));
                    }
                    true
                }
                Ok(None) => false,
                Err(err) => {
                    let (host, cmd, _) = children.remove(idx);
                    abort_state.remove(&host);
                    passed = false;
                    log.line(format!("FAIL poll fault host={host} cmd={cmd}: {err}"));
                    true
                }
            };
            if !done {
                idx += 1;
            }
        }

        while assertions
            .front()
            .is_some_and(|assertion| start.elapsed() >= assertion.at)
        {
            let assertion = assertions.pop_front().unwrap();
            match run_assertion(&assertion) {
                Ok(()) => log.line(format!(
                    "PASS assertion kind={:?} check={}",
                    assertion.kind, assertion.check
                )),
                Err(err) => {
                    passed = false;
                    log.line(format!(
                        "FAIL assertion kind={:?} check={}: {err:#}",
                        assertion.kind, assertion.check
                    ));
                }
            }
        }

        if start.elapsed() > scenario.duration && children.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    if !faults.is_empty() || !assertions.is_empty() {
        passed = false;
        log.line("FAIL scenario ended before all timeline entries ran");
    }

    log.line(if passed { "RESULT PASS" } else { "RESULT FAIL" });
    Ok(RunResult {
        passed,
        run_id,
        log_path,
    })
}

fn run_assertion(assertion: &Assertion) -> Result<()> {
    match assertion.kind {
        AssertionKind::Systemd => {
            let host = assertion
                .host
                .as_deref()
                .ok_or_else(|| anyhow!("missing host"))?;
            let service = assertion
                .check
                .strip_suffix(" is-active")
                .ok_or_else(|| anyhow!("expected '<service> is-active'"))?
                .trim();
            if service.is_empty() {
                bail!("empty service");
            }
            let output = ssh::run_systemd_is_active(host, service)?;
            if !output.status.success() {
                bail!(
                    "systemctl is-active failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            let stdout = String::from_utf8(output.stdout).context("systemctl output not UTF-8")?;
            if stdout.trim() != "active" {
                bail!("{service} state is {}", stdout.trim());
            }
            Ok(())
        }
        AssertionKind::Http => {
            let expected = assertion.status.unwrap_or(200);
            let response = reqwest::blocking::get(assertion.check.trim())?;
            let actual = response.status().as_u16();
            if actual != expected {
                bail!("expected HTTP {expected}, got {actual}");
            }
            Ok(())
        }
    }
}
