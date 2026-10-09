//! tiny-dictate: minimal voice dictation for Linux.
//!
//! One binary carries the core, the audio pipeline and the pill (see
//! `docs/adr/0001-single-rust-binary.md`). The command line is parsed here, and each
//! layer of the tool lives in its own module.

use std::process::ExitCode;

mod audio;
mod claim;
mod cli;
mod config;
mod files;
mod log;
mod microphone;
mod pill;
mod presentation;
mod rescue;
mod session;
mod store;
mod text;
mod xdg;

fn main() -> ExitCode {
    // Rust ignores `SIGPIPE` by default, so `tiny-dictate history | head` fails its write and
    // `println!` panics on a closed pipe. A tool piped into `head` leaves the way every other one
    // does, so the default disposition is restored before anything runs.
    //
    // Safety: a plain `signal(2)` call with `SIG_DFL`, before any thread exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let arguments: Vec<String> = std::env::args().skip(1).collect();

    // Hidden verbs are dispatched before anything else: they are internal, and a mode like
    // `_present` runs on a machine where the recorder and the clipboard tools need not be
    // installed.
    match arguments.first().map(String::as_str) {
        Some("_pump") => return audio::pump(&arguments[1..]),
        Some("_session") => {
            return session::run_session(
                arguments.get(1).map(String::as_str).unwrap_or(""),
                &arguments[2..],
            );
        }
        Some("_present") => return presentation::present(&arguments[1..]),
        _ => {}
    }

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
        cli::Command::Start => session::start(),
        cli::Command::Stop => session::stop(),
        cli::Command::Cancel => session::cancel(),
        cli::Command::Toggle => session::toggle(),
        cli::Command::History => rescue::history(),
        cli::Command::Transcribe { target, print } => rescue::transcribe_session(&target, print),
        cli::Command::SessionDelete { target } => rescue::session_delete(&target),
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
