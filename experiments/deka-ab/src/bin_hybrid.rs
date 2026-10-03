fn main() {
    use deka_ab::common::{self, App};
    let args = common::Args::parse();
    common::watchdog(&args);
    match args.app {
        App::Coverage => deka_ab::sparse::hybrid::offscreen(false),
        App::WebGl2 => deka_ab::sparse::hybrid::offscreen(true),
        _ => deka_ab::sparse::run_windowed::<deka_ab::sparse::hybrid::HybridPresenter>(args),
    }
}
