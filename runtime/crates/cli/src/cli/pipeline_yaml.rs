use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Pipeline {
    pub name: Option<String>,
    pub jobs: HashMap<String, Job>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub runs_on: String,
    pub target: Option<String>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub run: Option<String>,
    pub uses: Option<String>,
    pub with: HashMap<String, String>,
}

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Yaml(String),
    Validation(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Io(e) => write!(f, "io error: {}", e),
            ParseError::Yaml(e) => write!(f, "yaml parse error: {}", e),
            ParseError::Validation(e) => write!(f, "pipeline validation error: {}", e),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<std::io::Error> for ParseError {
    fn from(e: std::io::Error) -> Self {
        ParseError::Io(e)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPipeline {
    name: Option<String>,
    on: Option<RawOnTrigger>,
    jobs: Option<HashMap<String, RawJob>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOnTrigger {
    push: Option<serde_yaml::Value>,
    tag: Option<RawTagTrigger>,
    dispatch: Option<RawDispatchTrigger>,
}

#[derive(Debug, Deserialize)]
struct RawTagTrigger {
    pattern: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawDispatchTrigger {
    inputs: Option<Vec<RawDispatchInput>>,
}

#[derive(Debug, Deserialize)]
struct RawDispatchInput {
    name: Option<String>,
    #[serde(rename = "type")]
    input_type: Option<String>,
    options: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawJob {
    runs_on: Option<String>,
    target: Option<serde_yaml::Value>,
    steps: Option<Vec<RawStep>>,
}

#[derive(Debug, Deserialize)]
struct RawStep {
    run: Option<String>,
    uses: Option<String>,
    with: Option<HashMap<String, String>>,
}

pub fn parse_pipeline_yaml(path: &str) -> Result<Pipeline, ParseError> {
    let content = std::fs::read_to_string(path)?;
    parse_pipeline_yaml_from_str(&content)
}

pub fn parse_pipeline_yaml_from_str(content: &str) -> Result<Pipeline, ParseError> {
    let raw: RawPipeline =
        serde_yaml::from_str(content).map_err(|e| ParseError::Yaml(e.to_string()))?;

    let jobs = raw.jobs.unwrap_or_default();
    let mut parsed_jobs: HashMap<String, Job> = HashMap::new();

    if jobs.is_empty() {
        return Err(ParseError::Validation(
            "pipeline must define at least one job".to_string(),
        ));
    }

    for (name, raw_job) in jobs {
        let runs_on = raw_job.runs_on.unwrap_or_else(|| "gild".to_string());
        let target = raw_job.target.map(|v| match v {
            serde_yaml::Value::String(s) => s,
            other => format!("{:?}", other),
        });

        let steps = raw_job.steps.unwrap_or_default();
        let parsed_steps: Vec<Step> = steps
            .into_iter()
            .map(|s| Step {
                run: s.run,
                uses: s.uses,
                with: s.with.unwrap_or_default(),
            })
            .collect();

        parsed_jobs.insert(
            name,
            Job {
                runs_on,
                target,
                steps: parsed_steps,
            },
        );
    }

    Ok(Pipeline {
        name: raw.name,
        jobs: parsed_jobs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_pipeline() {
        let yaml = r#"
name: test-pipeline
jobs:
  build:
    runs-on: gild
    steps:
      - run: echo hello
"#;
        let pipeline = parse_pipeline_yaml_from_str(yaml).unwrap();
        assert_eq!(pipeline.name.as_deref(), Some("test-pipeline"));
        let build = pipeline.jobs.get("build").unwrap();
        assert_eq!(build.runs_on, "gild");
        assert_eq!(build.steps.len(), 1);
        assert_eq!(build.steps[0].run.as_deref(), Some("echo hello"));
    }

    #[test]
    fn parses_multiple_steps_mixed_actions() {
        let yaml = r#"
jobs:
  build:
    runs-on: gild
    steps:
      - uses: tana-actions/checkout@v1
      - run: cargo build --release
      - uses: tana-actions/upload-artifact@v1
        with:
          name: my-artifact
          path: ./dist
"#;
        let pipeline = parse_pipeline_yaml_from_str(yaml).unwrap();
        let build = pipeline.jobs.get("build").unwrap();
        assert_eq!(build.steps.len(), 3);
        assert_eq!(
            build.steps[0].uses.as_deref(),
            Some("tana-actions/checkout@v1")
        );
        assert_eq!(build.steps[1].run.as_deref(), Some("cargo build --release"));
        assert_eq!(
            build.steps[2].uses.as_deref(),
            Some("tana-actions/upload-artifact@v1")
        );
        assert_eq!(
            build.steps[2].with.get("name").map(|s| s.as_str()),
            Some("my-artifact")
        );
    }

    #[test]
    fn defaults_runs_on_to_gild() {
        let yaml = r#"
jobs:
  build:
    steps:
      - run: make
"#;
        let pipeline = parse_pipeline_yaml_from_str(yaml).unwrap();
        let build = pipeline.jobs.get("build").unwrap();
        assert_eq!(build.runs_on, "gild");
    }

    #[test]
    fn rejects_empty_jobs() {
        let yaml = r#"name: empty"#;
        let err = parse_pipeline_yaml_from_str(yaml).unwrap_err();
        assert!(matches!(err, ParseError::Validation(_)));
        assert!(err.to_string().contains("at least one job"));
    }

    #[test]
    fn parses_target_field() {
        let yaml = r#"
jobs:
  build:
    runs-on: gild
    target: x86_64-unknown-linux-gnu
    steps:
      - run: echo ok
"#;
        let pipeline = parse_pipeline_yaml_from_str(yaml).unwrap();
        let build = pipeline.jobs.get("build").unwrap();
        assert_eq!(build.target.as_deref(), Some("x86_64-unknown-linux-gnu"));
    }

    #[test]
    fn parses_on_triggers() {
        let yaml = r#"
on:
  push:
    branches: [main]
  tag:
    pattern: "v*"
jobs:
  build:
    steps:
      - run: echo ok
"#;
        let pipeline = parse_pipeline_yaml_from_str(yaml).unwrap();
        assert_eq!(pipeline.jobs.len(), 1);
    }

    #[test]
    fn handles_io_error_for_missing_file() {
        let err = parse_pipeline_yaml("/tmp/nonexistent_pipeline_xxxx.yaml").unwrap_err();
        assert!(matches!(err, ParseError::Io(_)));
    }
}
