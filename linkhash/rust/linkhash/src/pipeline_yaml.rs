use serde::Deserialize;
use thiserror::Error;

/// Top-level pipeline definition parsed from `linkhash.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pipeline {
    pub name: String,
    pub on: On,
    pub jobs: std::collections::BTreeMap<String, Job>,
}

/// Trigger configuration. At least one trigger must be present.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct On {
    #[serde(default)]
    pub push: Option<PushTrigger>,
    #[serde(default)]
    pub tag: Option<TagTrigger>,
    #[serde(default)]
    pub dispatch: Option<DispatchTrigger>,
}

impl On {
    pub fn has_any_trigger(&self) -> bool {
        self.push.is_some() || self.tag.is_some() || self.dispatch.is_some()
    }
}

/// Push trigger — fires on branch pushes.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushTrigger {
    #[serde(default)]
    pub branches: Vec<String>,
}

/// Tag trigger — fires when a tag matching the pattern is pushed.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagTrigger {
    pub pattern: String,
}

/// Manual dispatch trigger with optional inputs.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchTrigger {
    #[serde(default)]
    pub inputs: Vec<DispatchInput>,
}

/// A dispatch input parameter definition.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchInput {
    pub name: String,
    #[serde(rename = "type")]
    pub input_type: String,
    #[serde(default)]
    pub options: Vec<String>,
}

/// A single job definition within a pipeline.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    #[serde(rename = "runs-on")]
    pub runs_on: Runner,
    pub target: String,
    pub steps: Vec<Step>,
}

/// Allowed runner types. Currently only `gild` is supported.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runner {
    Gild,
}

/// A single step within a job.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default)]
    pub uses: Option<String>,
    #[serde(default)]
    pub run: Option<String>,
    #[serde(default)]
    pub with: Option<std::collections::BTreeMap<String, String>>,
}

/// Errors that can occur during pipeline parsing.
#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yml::Error),
    #[error("pipeline must define at least one trigger (push, tag, or dispatch)")]
    NoTrigger,
    #[error("pipeline must define at least one job")]
    NoJobs,
    #[error("job '{0}' must have at least one step")]
    NoSteps(String),
}

/// Parse a `linkhash.yaml` string into a validated `Pipeline`.
///
/// This function:
/// 1. Deserializes the YAML into the type-safe AST
/// 2. Validates required fields and constraints
/// 3. Returns a structured `Pipeline` or a descriptive error
pub fn parse_pipeline(yaml: &str) -> Result<Pipeline, PipelineError> {
    let pipeline: Pipeline = serde_yml::from_str(yaml)?;
    validate_pipeline(&pipeline)?;
    Ok(pipeline)
}

fn validate_pipeline(p: &Pipeline) -> Result<(), PipelineError> {
    if !p.on.has_any_trigger() {
        return Err(PipelineError::NoTrigger);
    }
    if p.jobs.is_empty() {
        return Err(PipelineError::NoJobs);
    }
    for (name, job) in &p.jobs {
        if job.steps.is_empty() {
            return Err(PipelineError::NoSteps(name.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HAPPY_PATH: &str = r#"
name: gild-guest
on:
  push:
    branches: [main]
  tag:
    pattern: "v*"
  dispatch:
    inputs:
      - name: target
        type: choice
        options: [x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu]

jobs:
  build:
    runs-on: gild
    target: ${{ github.event.inputs.target || default }}
    steps:
      - uses: tana-actions/checkout@v1
      - run: cargo build --release --target ${{ job.target }}
      - uses: tana-actions/upload-artifact@v1
        with:
          name: gild-guest-${{ job.target }}
          path: target/release/gild-guest
"#;

    #[test]
    fn test_parse_happy_path() {
        let pipeline = parse_pipeline(HAPPY_PATH).expect("happy path should parse");

        assert_eq!(pipeline.name, "gild-guest");

        // Triggers
        assert!(pipeline.on.push.is_some());
        assert_eq!(pipeline.on.push.as_ref().unwrap().branches, vec!["main"]);

        assert!(pipeline.on.tag.is_some());
        assert_eq!(pipeline.on.tag.as_ref().unwrap().pattern, "v*");

        assert!(pipeline.on.dispatch.is_some());
        let dispatch = pipeline.on.dispatch.as_ref().unwrap();
        assert_eq!(dispatch.inputs.len(), 1);
        assert_eq!(dispatch.inputs[0].name, "target");
        assert_eq!(dispatch.inputs[0].input_type, "choice");
        assert_eq!(
            dispatch.inputs[0].options,
            vec!["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
        );

        // Jobs
        assert!(pipeline.jobs.contains_key("build"));
        let job = &pipeline.jobs["build"];
        assert!(matches!(job.runs_on, Runner::Gild));
        assert_eq!(job.target, "${{ github.event.inputs.target || default }}");
        assert_eq!(job.steps.len(), 3);

        // Steps
        assert_eq!(job.steps[0].uses.as_deref(), Some("tana-actions/checkout@v1"));
        assert_eq!(
            job.steps[1].run.as_deref(),
            Some("cargo build --release --target ${{ job.target }}")
        );
        assert_eq!(
            job.steps[2].uses.as_deref(),
            Some("tana-actions/upload-artifact@v1")
        );
        let with = job.steps[2].with.as_ref().unwrap();
        assert_eq!(with.get("name").unwrap(), "gild-guest-${{ job.target }}");
        assert_eq!(
            with.get("path").unwrap(),
            "target/release/gild-guest"
        );
    }

    #[test]
    fn test_parse_missing_required_field_no_trigger() {
        let yaml = r#"
name: test-pipeline
on: {}
jobs:
  build:
    runs-on: gild
    target: x86_64-unknown-linux-gnu
    steps:
      - run: echo hello
"#;
        let result = parse_pipeline(yaml);
        assert!(result.is_err());
        match result.unwrap_err() {
            PipelineError::NoTrigger => {}
            other => panic!("expected NoTrigger, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_unknown_top_level_key() {
        let yaml = r#"
name: test-pipeline
on:
  push:
    branches: [main]
jobs:
  build:
    runs-on: gild
    target: x86_64-unknown-linux-gnu
    steps:
      - run: echo hello
unknown_field: this should be rejected
"#;
        let result = parse_pipeline(yaml);
        assert!(result.is_err(), "expected error for unknown top-level key");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("unknown"),
            "error should mention unknown field: {}",
            err
        );
    }

    #[test]
    fn test_parse_invalid_runs_on() {
        let yaml = r#"
name: test-pipeline
on:
  push:
    branches: [main]
jobs:
  build:
    runs-on: github-runner
    target: x86_64-unknown-linux-gnu
    steps:
      - run: echo hello
"#;
        let result = parse_pipeline(yaml);
        assert!(result.is_err(), "expected error for invalid runs-on value");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("gild"),
            "error should mention gild: {}",
            err
        );
    }

    #[test]
    fn test_parse_no_jobs() {
        let yaml = r#"
name: test-pipeline
on:
  push:
    branches: [main]
jobs: {}
"#;
        let result = parse_pipeline(yaml);
        assert!(result.is_err());
        match result.unwrap_err() {
            PipelineError::NoJobs => {}
            other => panic!("expected NoJobs, got: {:?}", other),
        }
    }

    #[test]
    fn test_parse_job_with_no_steps() {
        let yaml = r#"
name: test-pipeline
on:
  push:
    branches: [main]
jobs:
  build:
    runs-on: gild
    target: x86_64-unknown-linux-gnu
    steps: []
"#;
        let result = parse_pipeline(yaml);
        assert!(result.is_err());
        match result.unwrap_err() {
            PipelineError::NoSteps(name) => assert_eq!(name, "build"),
            other => panic!("expected NoSteps, got: {:?}", other),
        }
    }
}
