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
#[cfg(feature = "runtime")]
fn runtime(args: &[String]) -> Result<(), String> {
    if !(args.len() == 8
        || (args.len() == 9 && args[8] == "--reduced-motion")
        || (args.len() == 10 && args[8] == "--exercise"))
        || args[2] != "--project"
        || args[4] != "--compiler"
        || args[6] != "--component"
    {
        return Err("usage: deka-native runtime <source.dsx> --project <root> --compiler <dsc> --component <export> [--exercise <clicks> | --reduced-motion]".into());
    }
    let session = deka_native::runtime::Session::open(
        Path::new(&args[3]),
        Path::new(&args[1]),
        Path::new(&args[5]),
        &args[7],
    )?;
    let app = deka_native::runtime::RuntimeApp::new(session);
    if args.len() == 10 {
        let clicks = args[9]
            .parse()
            .map_err(|_| "--exercise requires a nonnegative click count")?;
        println!("{}", deka_native_ui::exercise(app, clicks));
        return Ok(());
    }
    #[cfg(feature = "gpu")]
    deka_native_ui::run(app);
    #[cfg(not(feature = "gpu"))]
    {
        let _ = app;
        return Err("native runtime window requires the gpu feature".into());
    }
    #[cfg(feature = "gpu")]
    Ok(())
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    #[cfg(feature = "runtime")]
    if args.first().is_some_and(|arg| arg == "runtime") {
        return runtime(&args);
    }
    if args.as_slice() == ["--help"] || args.as_slice() == ["-h"] {
        println!(
            "deka-native runtime <source.dsx> --project <root> --compiler <dsc> --component <export> [--exercise <clicks> | --reduced-motion] (requires runtime + gpu features)\ndeka-native dev <source.dsx> --compiler <dsc-native> [--exercise <clicks> | --reduced-motion]\ndeka-native build <source.dsx> --compiler <dsc-native> --out <fresh-directory> [--runtime-shaders]"
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
            "usage: deka-native dev <source.dsx> --compiler <dsc-native> [--exercise <clicks> | --reduced-motion]"
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
    if args.len() != 4 && !(args.len() == 5 && args[4] == "--reduced-motion") {
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
