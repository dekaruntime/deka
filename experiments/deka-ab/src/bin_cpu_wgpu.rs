fn main() {
    use deka_ab::common::{self, App};
    let args = common::Args::parse();
    common::watchdog(&args);
    match args.app {
        App::Bump => deka_ab::sparse::cpu::upload_bench(args.frames),
        _ => deka_ab::sparse::run_windowed::<deka_ab::sparse::cpu::CpuWgpuPresenter>(args),
    }
}
