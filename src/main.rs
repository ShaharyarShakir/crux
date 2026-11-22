use std::env;
use std::io::{self, IsTerminal};
use std::process;

fn main() {
    let stderr_is_terminal = io::stderr().is_terminal();
    // A closed stdout pipe (e.g. `crux resolve | head -1`) ends output
    // quietly instead of failing the command.
    let stdout = crux::ui::PipeTolerantWriter::new(io::stdout().lock());
    // stderr is deliberately *not* locked for the whole run: with `-vv`,
    // subprocess output is streamed to stderr from reader threads, which
    // would block forever on a lock held by this thread. Each write locks
    // stderr briefly instead (line-sized writes keep output intact).
    let stderr = io::stderr();

    match crux::cli::run_with_terminal(env::args(), stdout, stderr, stderr_is_terminal) {
        Ok(()) => {}
        Err(crux::cli::CliError::AppExit(code)) => process::exit(code),
        Err(_) => process::exit(1),
    }
}
