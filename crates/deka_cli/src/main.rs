fn main() -> std::process::ExitCode {
    deka_cli::cli::main_entry(std::env::args().skip(1).collect())
}
