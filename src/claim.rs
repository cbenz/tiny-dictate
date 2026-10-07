//! The claim on the microphone.
//!
//! One dictation at a time: the claim is the session directory in the runtime directory,
//! and creating it with `mkdir` is atomic. A stale claim is reclaimable only when neither
//! the session process nor the recorder lives.

use std::fs;

use crate::session::{Paths, pidfile_lives};

/// Take the claim, reclaiming a directory no live process owns.
///
/// Creating the directory is the whole claim: the `mkdir` fails when the directory already
/// exists, so two dictations can never run at once. A stale directory — a session killed by a
/// signal, a crash — is removed and recreated, but never while the session or the recorder
/// recorded in its pidfiles is alive.
pub fn claim(paths: &Paths) -> bool {
    if fs::create_dir(&paths.session).is_ok() {
        return true;
    }

    if pidfile_lives(&paths.session_pid) || pidfile_lives(&paths.recorder_pid) {
        return false;
    }

    if fs::remove_dir_all(&paths.session).is_err() && paths.session.exists() {
        return false;
    }

    fs::create_dir(&paths.session).is_ok()
}
