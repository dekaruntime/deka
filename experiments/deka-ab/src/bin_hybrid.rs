fn main() {
    use deka_ab::common::{self, App};
    let args = common::Args::parse();
    common::watchdog(&args);
    match args.app {
        App::Coverage => deka_ab::sparse::hybrid::offscreen(false),
        App::WebGl2 => deka_ab::sparse::hybrid::offscreen(true),
        App::Bump => deka_ab::sparse::hybrid::bench(args.frames),
        App::Verify => {
            if !deka_ab::sparse::hybrid::verify() {
                std::process::exit(1);
            }
        }
        App::Scaling => {
            let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
            let mut counts = common::list_arg("--threads");
            if counts.is_empty() {
                counts = vec![1, 2, 4, 8];
                if n / 2 > 8 { counts.push(n / 2); }
                if n > 8 { counts.push(n); }
            }
            deka_ab::sparse::hybrid::scaling(args.frames, &counts);
        }
        _ => deka_ab::sparse::run_windowed::<deka_ab::sparse::hybrid::HybridPresenter>(args),
    }
}
