use deka_vm_experiment::*;
fn main() {
    if let Err(e) = run() {
        eprintln!("dvm-ui: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    // Finder supplies no source path. Resolve the payload from this executable's
    // bundle, never the working directory or the developer's source checkout.
    if args.first().is_none_or(|arg| arg.starts_with("--")) {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let directory = executable.parent().ok_or("executable has no directory")?;
        #[cfg(target_os = "macos")]
        let payload = directory.join("../Resources/app.dvm.json");
        #[cfg(not(target_os = "macos"))]
        let payload = directory.join("app.dvm.json");
        args.insert(0, payload.to_string_lossy().into_owned());
    }
    if args.len() != 1
        && !(args.len() == 3 && matches!(args[1].as_str(), "--exercise" | "--snapshot"))
    {
        return Err("unexpected arguments".into());
    }
    let path = &args[0];
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read application payload {path}: {e}"))?;
    let program = if path.ends_with(".dsx") {
        #[cfg(feature = "compiler")]
        {
            compiler::compile_entry(&source, &Hosts::default(), "Counter")?
        }
        #[cfg(not(feature = "compiler"))]
        {
            return Err("build with compiler feature or supply precompiled bytecode".into());
        }
    } else {
        serde_json::from_str(&source).map_err(|e| e.to_string())?
    };
    let app = ui::VmApp::new(program)?;
    if args.len() == 3 {
        let count = args[2].parse().map_err(|_| "expected nonnegative count")?;
        if args[1] == "--snapshot" {
            println!("{}", ui::snapshot(app, count)?);
        } else {
            println!("{}", deka_native_ui::exercise(app, count));
        }
    } else {
        deka_native_ui::run(app);
    }
    Ok(())
}
