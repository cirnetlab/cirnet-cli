fn main() -> std::process::ExitCode {
    match cirnet_cli::cli::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cnl: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
