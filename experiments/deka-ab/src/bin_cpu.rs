fn main() {
    use deka_ab::common::{self, App};
    let args = common::Args::parse();
    common::watchdog(&args);
    match args.app {
        App::Coverage => deka_ab::sparse::cpu::coverage_offscreen(),
        _ => deka_ab::sparse::run_windowed::<deka_ab::sparse::cpu::CpuPresenter>(args),
    }
}
