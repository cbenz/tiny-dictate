//! The session layer: what one dictation leaves behind, and the runtime state of the
//! dictation in flight.

use std::path::{Path, PathBuf};

/// The runtime directory, where the ephemeral side of a dictation lives: the claim
/// directory the session owns, the pidfiles the commands read liveness from, the fifo, the
/// waveform channel and the state file the pill draws.
///
/// The location follows the convention `src/tiny-dictate` documents:
/// `$XDG_RUNTIME_DIR`, falling back to `$XDG_STATE_HOME`, falling back to
/// `$HOME/.local/state`, then `tiny-dictate/`.
pub struct Runtime {
    directory: PathBuf,
}

impl Runtime {
    /// The runtime directory of this process, read from the environment.
    pub fn from_env() -> Self {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_STATE_HOME").map(PathBuf::from))
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .unwrap_or_else(std::env::temp_dir);

        Self {
            directory: base.join("tiny-dictate"),
        }
    }

    /// The directory the dictation in flight owns; creating it is the claim on the
    /// microphone.
    fn session_directory(&self) -> PathBuf {
        self.directory.join("session")
    }

    /// Whether a dictation is in flight: its session process or its recorder lives.
    ///
    /// The `working` state covers both recording and transcribing, so either pidfile being
    /// alive is enough.
    pub fn dictating(&self) -> bool {
        self.pidfile_lives("session.pid") || self.pidfile_lives("arecord.pid")
    }

    /// Whether the process a pidfile names is alive. The pidfile is read, never removed: a
    /// query does not perturb a live session.
    fn pidfile_lives(&self, name: &str) -> bool {
        let Ok(contents) = std::fs::read_to_string(self.session_directory().join(name)) else {
            return false;
        };
        let Ok(pid) = contents.trim().parse::<u32>() else {
            return false;
        };

        // liveness through /proc: the same answer `kill -0` gives in the bash tool, without
        // a libc dependency for the one signal the tool does not send.
        Path::new("/proc").join(pid.to_string()).is_dir()
    }
}
