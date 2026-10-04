mod app;
mod cli;
mod config;
mod constants;
mod domain;
mod infra;
#[cfg(test)]
mod test_support;

use std::process::ExitCode;

use cli::Command;
use constants::{DEFAULT_LOG_LEVEL, EXIT_USAGE, ONCE_LOG_LEVEL, PROGRAM_NAME, VERSION};

fn main() -> ExitCode {
    let options = match cli::parse(std::env::args().skip(1)) {
        // Output that was asked for goes to stdout with success, which is what
        // the GNU conventions and anything probing `--version` expect.
        Ok(Command::Help) => {
            print!("{}", cli::usage());
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("{PROGRAM_NAME} {VERSION}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Run(options)) => options,
        Err(error) => {
            eprint!("error: {error}\n\n{}", cli::usage());
            return ExitCode::from(EXIT_USAGE);
        }
    };
    // `--once` is documented as a verbose single cycle, so it defaults to debug
    // and shows the per-disk readings. RUST_LOG still overrides either default.
    infra::logging::init(if options.once {
        ONCE_LOG_LEVEL
    } else {
        DEFAULT_LOG_LEVEL
    });
    match app::run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Logged rather than returned from main, which would print it
            // unprefixed and land it in the journal at info like any other line.
            log::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}
