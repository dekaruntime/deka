use deka_vm::*;
#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() < 2 {
        return Err("usage: dvm program.dvm.json (with compiler feature: source.ds)".into());
    }
    let (hosts, output) = demo::hosts()?;
    if let Some(arg) = args.get(2) {
        return Err(format!("unknown argument: {arg}"));
    }
    let source = std::fs::read_to_string(&args[1]).map_err(|e| e.to_string())?;
    let program: Program = if args[1].ends_with(".ds") {
        #[cfg(feature = "compiler")]
        {
            compiler::compile(&source, &hosts)?
        }
        #[cfg(not(feature = "compiler"))]
        {
            return Err("this runtime has no compiler; compile with dvmc first".into());
        }
    } else {
        serde_json::from_str(&source).map_err(|e| e.to_string())?
    };
    let mut vm = Vm::new(program, hosts)?;
    let value = vm.run().await?;
    for line in output.borrow().iter() {
        println!("{line}");
    }
    println!("result: {value:?}");
    println!("heap: {}", serde_json::to_string(&vm.stats()).unwrap());
    println!("instructions: {}", vm.instructions());
    Ok(())
}
