use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const MAX_SCENARIO_DURATION: Duration = Duration::from_secs(3600);
const MAX_FAULTS: usize = 200;
const MAX_ASSERTIONS: usize = 500;

#[derive(Debug, Clone, Deserialize)]
pub struct Scenario {
    pub name: String,
    #[serde(deserialize_with = "duration")]
    pub duration: Duration,
    #[serde(default)]
    pub faults: Vec<Fault>,
    #[serde(default)]
    pub assertions: Vec<Assertion>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Fault {
    #[serde(deserialize_with = "duration")]
    pub at: Duration,
    pub host: String,
    // cmd is a single shell-safe token sequence; quoting unsupported in Part 1;
    // revisit if Part 2 scenarios need it.
    pub cmd: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Assertion {
    #[serde(deserialize_with = "duration")]
    pub at: Duration,
    pub kind: AssertionKind,
    pub host: Option<String>,
    pub check: String,
    pub status: Option<u16>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AssertionKind {
    Systemd,
    Http,
}

impl Scenario {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let scenario: Scenario = serde_yaml::from_str(&raw).context("parse scenario YAML")?;
        scenario.validate()?;
        Ok(scenario)
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("scenario name is required");
        }
        if self.duration > MAX_SCENARIO_DURATION {
            bail!("scenario duration exceeds max 3600 seconds");
        }
        if self.faults.len() > MAX_FAULTS {
            bail!("faults array exceeds max length 200");
        }
        if self.assertions.len() > MAX_ASSERTIONS {
            bail!("assertions array exceeds max length 500");
        }
        for fault in &self.faults {
            if fault.at > self.duration {
                bail!(
                    "fault.at {:?} exceeds scenario duration {:?}",
                    fault.at,
                    self.duration
                );
            }
            if fault.host.trim().is_empty() {
                bail!("fault host is required");
            }
            if fault.cmd.trim().is_empty() {
                bail!("fault cmd is required");
            }
        }
        for assertion in &self.assertions {
            if assertion.at > self.duration {
                bail!(
                    "assertion.at {:?} exceeds scenario duration {:?}",
                    assertion.at,
                    self.duration
                );
            }
            match assertion.kind {
                AssertionKind::Systemd => {
                    if assertion
                        .host
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                    {
                        bail!("systemd assertion host is required");
                    }
                    if !assertion.check.contains(" is-active") {
                        bail!("systemd assertion check must be '<service> is-active'");
                    }
                }
                AssertionKind::Http => {
                    if assertion.check.trim().is_empty() {
                        bail!("http assertion check URL is required");
                    }
                }
            }
        }
        Ok(())
    }
}

fn duration<'de, D>(deserializer: D) -> std::result::Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    humantime::parse_duration(&value).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_smoke_scenario() {
        let raw = r#"
name: smoke-pause-then-resume
duration: 30s
faults:
  - at: 5s
    host: demon
    cmd: process pause --service harar --secs 5 --undo-by 10s
assertions:
  - at: 20s
    kind: systemd
    host: demon
    check: harar is-active
"#;
        let scenario: Scenario = serde_yaml::from_str(raw).unwrap();
        scenario.validate().unwrap();
        assert_eq!(scenario.name, "smoke-pause-then-resume");
        assert_eq!(scenario.faults.len(), 1);
        assert_eq!(scenario.assertions[0].kind, AssertionKind::Systemd);
    }

    #[test]
    fn rejects_out_of_bounds_fault() {
        let raw = r#"
name: bad
duration: 1s
faults:
  - at: 2s
    host: demon
    cmd: process start --service harar
"#;
        let scenario: Scenario = serde_yaml::from_str(raw).unwrap();
        let err = scenario.validate().unwrap_err();
        assert!(err.to_string().contains("fault.at"));
    }

    #[test]
    fn rejects_scenario_duration_above_one_hour() {
        let raw = r#"
name: bad
duration: 3601s
"#;
        let scenario: Scenario = serde_yaml::from_str(raw).unwrap();
        let err = scenario.validate().unwrap_err();
        assert!(err.to_string().contains("duration"));
    }

    #[test]
    fn rejects_too_many_faults() {
        let mut raw = "name: bad\nduration: 30s\nfaults:\n".to_string();
        for _ in 0..=MAX_FAULTS {
            raw.push_str(
                "  - at: 1s\n    host: demon\n    cmd: process start --service harar\n",
            );
        }
        let scenario: Scenario = serde_yaml::from_str(&raw).unwrap();
        let err = scenario.validate().unwrap_err();
        assert!(err.to_string().contains("faults"));
    }

    #[test]
    fn rejects_too_many_assertions() {
        let mut raw = "name: bad\nduration: 30s\nassertions:\n".to_string();
        for _ in 0..=MAX_ASSERTIONS {
            raw.push_str("  - at: 1s\n    kind: http\n    check: http://localhost\n");
        }
        let scenario: Scenario = serde_yaml::from_str(&raw).unwrap();
        let err = scenario.validate().unwrap_err();
        assert!(err.to_string().contains("assertions"));
    }
}
