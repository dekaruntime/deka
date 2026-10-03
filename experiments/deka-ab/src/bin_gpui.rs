fn main() {
    let args = deka_ab::common::Args::parse();
    deka_ab::common::watchdog(&args);
    deka_ab::gpui_backend::run(args);
}
