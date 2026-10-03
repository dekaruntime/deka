fn main() {
    let args = deka_ab::common::Args::parse();
    deka_ab::common::watchdog(&args);
    deka_ab::sparse::run_windowed::<deka_ab::sparse::cpu::CpuWgpuPresenter>(args);
}
