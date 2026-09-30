mod build;
use std::{path::Path, process::ExitCode};
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("deka-native: {error}");
            ExitCode::FAILURE
        }
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--help"] || args.as_slice() == ["-h"] {
        println!(
            "deka-native dev <source.dsx> --compiler <dsc-native> [--exercise <clicks>]\ndeka-native build <source.dsx> --compiler <dsc-native> --out <fresh-directory> [--runtime-shaders]"
        );
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "build") {
        if !(args.len() == 6 || (args.len() == 7 && args[6] == "--runtime-shaders"))
            || args[2] != "--compiler"
            || args[4] != "--out"
        {
            return Err("usage: deka-native build <source.dsx> --compiler <dsc-native> --out <fresh-directory> [--runtime-shaders]".into());
        }
        let manifest = build::build(
            Path::new(&args[1]),
            Path::new(&args[3]),
            Path::new(&args[5]),
            args.len() == 7,
        )?;
        println!("Built native application from {}", manifest.display());
        return Ok(());
    }
    if args.len() < 4 || args[0] != "dev" || args[2] != "--compiler" {
        return Err(
            "usage: deka-native dev <source.dsx> --compiler <dsc-native> [--exercise <clicks>]"
                .into(),
        );
    }
    let source = Path::new(&args[1]);
    let compiler = Path::new(&args[3]);
    if args.len() == 6 && args[4] == "--exercise" {
        let count = args[5]
            .parse()
            .map_err(|_| "--exercise requires a nonnegative click count")?;
        let app = deka_native::DevApp::new(deka_native::compile_file(source, compiler)?)?;
        println!("{}", deka_native_ui::exercise(app, count));
        return Ok(());
    }
    if args.len() != 4 {
        return Err("unexpected command arguments".into());
    }
    #[cfg(feature = "gpu")]
    {
        deka_native_ui::run(deka_native::DevApp::watch(source, compiler)?);
        Ok(())
    }
    #[cfg(not(feature = "gpu"))]
    Err("this host was built without a renderer; build with --features gpu (or runtime-shaders on macOS without the Metal compiler)".into())
}
