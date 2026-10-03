fn main() {
    use deka_ab::common::{self, App};
    common::mark("main entered");
    // `--stock-occlusion`: the stock wgpu-hal behaviour (no frames before the
    // window server reports the window visible).
    if std::env::args().any(|a| a == "--stock-occlusion") {
        wgpu_hal::metal::PRE_VISIBLE_FRAMES.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    // `--redraw-until-visible` keeps drawing every frame until the window is
    // reported visible; allow enough drawables for that.
    if std::env::args().any(|a| a == "--redraw-until-visible") {
        wgpu_hal::metal::PRE_VISIBLE_LIMIT.store(64, std::sync::atomic::Ordering::Relaxed);
    }
    common::start_visibility_probe();
    deka_ab::sparse::hybrid::start_early_gpu();
    deka_ab::sparse::start_early_work();
    let args = common::Args::parse();
    common::watchdog(&args);
    match args.app {
        App::Coverage => deka_ab::sparse::hybrid::offscreen(false),
        App::WebGl2 => deka_ab::sparse::hybrid::offscreen(true),
        App::Bump => deka_ab::sparse::hybrid::bench(args.frames),
        App::Startup => deka_ab::sparse::hybrid::startup_offscreen(),
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
