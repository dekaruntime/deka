//! Corpus gate (deka#1214): run listed corpus cases against `deka run` and
//! fail if any stops matching its expectation. `--all` runs every case and
//! prints matched slugs — that is how tests/corpus-passing.txt is generated
//! and how a language PR finds the programs it fixed.
use corpus_gate::{evaluate, load_cases, read_passing_list, run_case};
use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ! {
    eprintln!("usage: corpus-gate <corpus-dir> <passing-list> <deka-binary> [--all]");
    std::process::exit(2);
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all = args.iter().any(|arg| arg == "--all");
    let positional: Vec<&String> = args.iter().filter(|arg| !arg.starts_with('-')).collect();
    let [corpus, list, deka] = positional.as_slice() else {
        usage();
    };
    let (corpus, list, deka) = (
        PathBuf::from(corpus),
        PathBuf::from(list),
        PathBuf::from(deka),
    );
    // run_case sets the child's cwd to the staged case directory, so the
    // binary path must be absolute before any case runs.
    let deka = match deka.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("corpus-gate: deka binary {}: {error}", deka.display());
            return ExitCode::from(1);
        }
    };
    let cases = load_cases(&corpus);
    if cases.is_empty() {
        eprintln!("corpus-gate: no cases found under {}", corpus.display());
        return ExitCode::from(1);
    }
    let scratch = std::env::temp_dir().join(format!("corpus-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("scratch dir");

    let listed: Vec<String> = if all {
        cases.iter().map(|case| case.slug.clone()).collect()
    } else {
        let list = match read_passing_list(&list) {
            Ok(list) => list,
            Err(error) => {
                eprintln!("corpus-gate: {error}");
                return ExitCode::from(1);
            }
        };
        let known: std::collections::BTreeSet<String> =
            cases.iter().map(|case| case.slug.clone()).collect();
        let mut missing: Vec<String> = list.difference(&known).cloned().collect();
        if !missing.is_empty() {
            missing.sort();
            for slug in &missing {
                eprintln!("corpus-gate: listed case not in the corpus: {slug}");
            }
            return ExitCode::from(1);
        }
        list.into_iter().collect()
    };

    let mut matched = 0usize;
    let mut failed = 0usize;
    for case in cases.iter().filter(|case| listed.contains(&case.slug)) {
        match run_case(&deka, case, &scratch) {
            Ok(result) => {
                let reasons = evaluate(case, &result);
                if reasons.is_empty() {
                    matched += 1;
                    if all {
                        println!("{}", case.slug);
                    }
                } else {
                    failed += 1;
                    eprintln!("FAIL {}", case.slug);
                    for reason in reasons {
                        eprintln!("  {reason}");
                    }
                }
            }
            Err(error) => {
                failed += 1;
                eprintln!("FAIL {}: {error}", case.slug);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    if all {
        eprintln!(
            "corpus-gate: {matched} of {} cases match their expectation",
            listed.len()
        );
    } else {
        eprintln!(
            "corpus-gate: {matched} of {} listed cases match their expectation",
            listed.len()
        );
    }
    if failed > 0 && !all {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
