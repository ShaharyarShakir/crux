use std::env;
use std::io;
use std::process;

fn main() {
    let stdout = io::stdout();
    let stderr = io::stderr();

    match crux::cli::run(env::args(), stdout.lock(), stderr.lock()) {
        Ok(()) => {}
        Err(crux::cli::CliError::AppExit(code)) => process::exit(code),
        Err(_) => process::exit(1),
    }
}
