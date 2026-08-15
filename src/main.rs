use clap::Parser;
use std::error::Error;
use std::process;

use waft::cli::Cli;

fn main() {
    let cli = Cli::parse();
    // `dispatch` consumes the CLI, so decide how much of a failure to print
    // before handing it over.
    let explain = cli.verbose > 0 && !cli.quiet;
    if let Err(error) = cli.dispatch() {
        eprintln!("error: {error}");
        if explain {
            // Human-facing messages stay short; the chain underneath them is
            // what a bug report needs.
            let mut cause = error.source();
            while let Some(source) = cause {
                eprintln!("  caused by: {source}");
                cause = source.source();
            }
        }
        process::exit(1);
    }
}
