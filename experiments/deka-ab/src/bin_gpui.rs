fn main() {
    deka_ab::common::mark("main entered");
    deka_ab::common::start_visibility_probe();
    let args = deka_ab::common::Args::parse();
    deka_ab::common::watchdog(&args);
    deka_ab::gpui_backend::run(args);
}
