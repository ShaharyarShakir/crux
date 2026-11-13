use std::env;
use std::io;
use std::process;

fn main() {
    let stdout = io::stdout();
    let stderr = io::stderr();

    if crux::cli::run(env::args(), stdout.lock(), stderr.lock()).is_err() {
        process::exit(1);
    }
}
