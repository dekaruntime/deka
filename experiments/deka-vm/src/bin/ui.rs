use deka_vm_experiment::*;
fn main() {
    if let Err(e) = run() {
        eprintln!("dvm-ui: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .ok_or("usage: dvm-ui counter.dsx|counter.dvm.json [--exercise N | --snapshot N]")?;
    if args.len() != 1
        && !(args.len() == 3 && matches!(args[1].as_str(), "--exercise" | "--snapshot"))
    {
        return Err("unexpected arguments".into());
    }
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
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
