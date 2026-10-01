use deka_vm::*;
fn main() {
    if let Err(e) = run() {
        eprintln!("dvm-v8-ui: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let app = v8_control::V8App::new()?;
    match args.as_slice() {
        [] => deka_native_ui::run(app),
        [mode, count] if mode == "--snapshot" => println!(
            "{}",
            ui::snapshot(app, count.parse().map_err(|_| "expected count")?)?
        ),
        [mode, count] if mode == "--exercise" => println!(
            "{}",
            deka_native_ui::exercise(app, count.parse().map_err(|_| "expected count")?)
        ),
        _ => return Err("usage: dvm-v8-ui [--exercise N | --snapshot N]".into()),
    }
    Ok(())
}
