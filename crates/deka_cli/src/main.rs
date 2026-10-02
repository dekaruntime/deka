fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(deka_cli::cli::main_entry(std::env::args().skip(1).collect()) as u8)
}
