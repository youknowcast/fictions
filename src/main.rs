use std::process::ExitCode;

use fictions::cli;

fn main() -> ExitCode {
    match cli::parse_from(std::env::args_os()) {
        Ok(_cli) => ExitCode::from(2),
        Err(err) => err.exit(),
    }
}
