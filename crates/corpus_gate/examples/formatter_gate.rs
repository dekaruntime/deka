//! A real CLI consumer of the formatter gate, including an independent golden.
use corpus_gate::{Case, Stage, Status, evaluate, run_case};
use std::{fs, path::PathBuf};

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let deka = PathBuf::from(args.next().ok_or("expected the built deka executable")?)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if args.next().is_some() {
        return Err("usage: formatter_gate <deka-executable>".into());
    }
    let case = Case {
        slug: "formatter-actual-cli-golden".into(),
        status: Status::Pass,
        stage: Stage::Run,
        source: "import { echo } from \"io\"\n// keep the explanation\nconst answer=7\necho(string(answer))\n".into(),
        entry_path: "main.ds".into(),
        files: vec![],
        expected_stdout: Some("7\n".into()),
        expected_diagnostic_contains: None,
        deka_json: None,
        packages: vec![],
    };
    let scratch = std::env::temp_dir().join(format!("formatter-gate-{}", std::process::id()));
    fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
    let result = (|| {
        let observed = run_case(&deka, &case, &scratch)?;
        let reasons = evaluate(&case, &observed);
        if !reasons.is_empty() {
            return Err(reasons.join("\n"));
        }
        let actual = fs::read_to_string(scratch.join(&case.slug).join("test.ds"))
            .map_err(|error| error.to_string())?;
        let expected = "import { echo } from \"io\"\n// keep the explanation\nconst answer = 7\necho(string(answer))\n";
        if actual != expected {
            return Err(format!(
                "formatter gate did not produce the independent golden:\n{actual}"
            ));
        }
        println!("actual formatter gate preserves output and produces canonical source");
        Ok(())
    })();
    let cleanup = fs::remove_dir_all(&scratch).map_err(|error| error.to_string());
    result.and(cleanup)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
