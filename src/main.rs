//! tiny-dictate: minimal voice dictation for Linux.
//!
//! One binary carries the core, the audio pipeline and the pill (see
//! `docs/adr/0001-single-rust-binary.md`). The command line is parsed here, and each
//! layer of the tool lives in its own module.

use std::process::ExitCode;

mod audio;
mod claim;
mod cli;
mod presentation;
mod session;
mod text;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    let command = match cli::parse(&arguments) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{}", error.message());
            eprintln!("{}", cli::USAGE);
            return ExitCode::FAILURE;
        }
    };

    match command {
        cli::Command::Help => {
            println!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        cli::Command::Status => {
            println!("{}", status_line());
            ExitCode::SUCCESS
        }
        command => {
            // The other verbs are ported by the tickets that follow; refusing loudly beats a
            // silent success that would pretend a dictation happened.
            eprintln!("tiny-dictate: {} is not implemented yet", command.verb());
            ExitCode::FAILURE
        }
    }
}

/// `working` covers both recording and transcribing, `idle` means no dictation is in flight.
fn status_line() -> &'static str {
    let runtime = session::Runtime::from_env();

    if runtime.dictating() {
        "working"
    } else {
        "idle"
    }
}
