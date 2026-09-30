use deka_vm_experiment::*;
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("usage: dvmc source.ds output.dvm.json".into());
    }
    // This CLI shares the real adapter catalog through the host feature.
    #[cfg(feature = "host")]
    let (hosts, _) = demo::hosts()?;
    #[cfg(not(feature = "host"))]
    let hosts = Hosts::default();
    let source = std::fs::read_to_string(&args[1]).map_err(|e| e.to_string())?;
    let program = compiler::compile(&source, &hosts)?;
    std::fs::write(
        &args[2],
        serde_json::to_vec(&program).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
