//! The browser: the kept sessions read on a terminal.
//!
//! Two levels, following the pill's precedent: `frame` is the pure computation that turns the list
//! state and the selection into what one frame draws, and this module is the adapter that owns the
//! terminal. The browser takes no command lock and reads files only, so it opens while a dictation
//! runs and works on a machine without the encoder or the clipboard tools.

use std::io::IsTerminal;
use std::process::ExitCode;

/// `session browse`: the kept sessions on a terminal.
///
/// A stdout that is not a terminal is refused with a message: drawing escape sequences into a pipe
/// would corrupt whatever reads it, and a terminal the tool cannot see is a terminal it must not
/// take over.
pub fn run() -> ExitCode {
    if !std::io::stdout().is_terminal() {
        eprintln!("tiny-dictate: session browse needs a terminal");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
