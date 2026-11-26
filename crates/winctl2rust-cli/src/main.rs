#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::Parser;
use winctl2rust_cli::{Cli, run_cli};
use winctl2rust_core::init_logging;

fn init_signal_handler() {
    let _ = ctrlc::set_handler(|| {
        eprintln!("[winctl2rust] interrupt received; exiting cleanly.");
        std::process::exit(130);
    });
}

fn main() -> ExitCode {
    init_signal_handler();
    let cli = Cli::parse();
    init_logging(&cli.log_level);

    match run_cli(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
