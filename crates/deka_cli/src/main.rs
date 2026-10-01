fn main() {
    if let Err(error) = deka_cli::run() {
        eprintln!("deka: {error}");
        std::process::exit(1);
    }
}
