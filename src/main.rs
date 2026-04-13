use clap::Parser;
use std::process;

use waft::cli::Cli;

fn main() {
    let cli = Cli::parse();
    if let Err(e) = cli.dispatch() {
        eprintln!("error: {e}");
        process::exit(1);
    }
}
