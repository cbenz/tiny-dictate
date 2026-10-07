//! The presentation layer: the state the pill draws, and the process that draws it.
//!
//! The pill is a mode of this binary, spawned by the session as a child process. The session
//! is the only writer of the state file, so the presenter is the only writer of the screen; no
//! frame of it can land after it is gone.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode};
use std::thread::sleep;
use std::time::Duration;

use crate::session::{Paths, self_executable};

/// The word a cancellation writes into the state file: the verdict, which belongs to the
/// cancellation and not to the session it interrupts.
pub const CANCELLED: &str = "cancelled";

/// Write one word into the state file, atomically: a reader must never see half a state, and a
/// truncating write is not atomic while a rename is.
pub fn write_state(state_file: &Path, word: &str) {
    let temporary = state_file.with_extension(format!("tmp{}", std::process::id()));
    if fs::write(&temporary, format!("{word}\n")).is_ok() {
        let _ = fs::rename(&temporary, state_file);
    }
}

/// Whether the state file holds the cancelled verdict.
///
/// `cancel` writes the verdict and signals the recorder, never the session: the session reads
/// the word off the file, so the verdict can arrive at any moment and is never missed.
pub fn cancelled(state_file: &Path) -> bool {
    fs::read_to_string(state_file)
        .map(|state| state.trim() == CANCELLED)
        .unwrap_or(false)
}

/// The one presenter of this dictation, and the state file it watches.
pub struct Presenter {
    state: PathBuf,
    child: Option<Child>,
}

impl Presenter {
    /// Publish the first state and spawn the presenter.
    ///
    /// The default is this binary's own `_present` mode, handed the state file, the pid it
    /// must outlive, the waveform file, the command that re-invokes the tool and the recording's
    /// limit in seconds (empty when the transcriber declared none, in which case no bar is
    /// drawn). `TINY_DICTATE_PRESENT_COMMAND` names another command instead — the internal test
    /// seam documented in `docs/SPECS.md`. A presenter that cannot start costs the pill only: the
    /// dictation goes on without one.
    pub fn start(paths: &Paths, word: &str, limit: Option<Duration>) -> Self {
        write_state(&paths.state, word);

        Self {
            state: paths.state.clone(),
            child: spawn(paths, limit),
        }
    }

    /// Ask the presenter to leave, then wait for it, bounded; a wedged presenter is killed.
    pub fn stop(&mut self) {
        write_state(&self.state, "stop");

        if let Some(mut child) = self.child.take() {
            for _ in 0..40 {
                match child.try_wait() {
                    Ok(Some(_)) => return,
                    _ => sleep(Duration::from_millis(50)),
                }
            }

            // Bounded, like every wait here: a wedged process must not hold the session
            // hostage, and SIGKILL always works.
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn(paths: &Paths, limit: Option<Duration>) -> Option<Child> {
    let self_command = self_executable();
    let own_pid = std::process::id().to_string();

    let mut command = match std::env::var_os("TINY_DICTATE_PRESENT_COMMAND") {
        Some(program) => {
            let mut command = Command::new(program);
            command.arg(&paths.state);
            command
        }
        None => {
            let mut command = Command::new(&self_command);
            command.arg("_present").arg(&paths.state);
            command
        }
    };

    command
        .arg(own_pid)
        .arg(&paths.waveform)
        .arg(&self_command)
        // The 5th argument is the recording's limit in seconds; an empty argument means the
        // transcriber declared none, and the pill draws no bar for it.
        .arg(
            limit
                .map(|limit| limit.as_secs_f64().to_string())
                .unwrap_or_default(),
        )
        // The cairo renderer puts the window on screen in about a tenth of a second instead of a
        // third: the one reason the pill is drawn by this binary. It is set on this child only,
        // never by the pill for itself and never for the interactive modes.
        .env("GSK_RENDERER", "cairo")
        .spawn()
        .ok()
}

/// `_present`: the pill itself. The arguments are parsed here, so a wrong invocation fails
/// without a display; everything past them is the GTK adapter.
pub fn present(arguments: &[String]) -> ExitCode {
    match crate::pill::Options::parse(arguments) {
        Ok(options) => crate::pill::run(options),
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}
