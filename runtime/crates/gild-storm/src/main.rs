mod runner;
mod scenario;
mod ssh;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "gild-storm",
    version,
    about = "Scenario runner for gild-storm chaos tests"
)]
struct Cli {
    /// YAML scenario file to run.
    scenario: PathBuf,
}

fn main() {
    let abort_state = runner::AbortState::new();
    let handler_state = abort_state.clone();
    let _ = ctrlc::set_handler(move || {
        handler_state.abort();
        std::process::exit(130);
    });
    std::panic::set_hook({
        let panic_state = abort_state.clone();
        Box::new(move |info| {
            eprintln!("panic: {info}");
            panic_state.abort();
        })
    });

    match run(abort_state) {
        Ok(result) => {
            println!("run_id={} log={}", result.run_id, result.log_path.display());
            std::process::exit(if result.passed { 0 } else { 1 });
        }
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::exit(1);
        }
    }
}

fn run(abort_state: runner::AbortState) -> Result<runner::RunResult> {
    let cli = Cli::parse();
    let scenario = scenario::Scenario::load(&cli.scenario)?;
    runner::run(scenario, abort_state)
}
