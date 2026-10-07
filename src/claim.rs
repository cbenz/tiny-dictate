//! The claim on the microphone.
//!
//! One dictation at a time: the claim is the session directory in the runtime directory,
//! and creating it with `mkdir` is atomic. A stale claim is reclaimable only when neither
//! the session process nor the recorder lives.

use std::fs;
use std::thread::sleep;
use std::time::Duration;

use crate::presentation;
use crate::session::{Paths, pid_in, pidfile_lives};

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

    // A cancelled session is only holding the claim to keep its verdict on screen. Ask it to
    // wrap up instead of making the next dictation wait out a screen the user has read; a
    // session that is transcribing is left alone, its text is not a courtesy.
    if presentation::cancelled(&paths.state) {
        end_cancelled_session(paths);
    }

    if pidfile_lives(&paths.session_pid) || pidfile_lives(&paths.recorder_pid) {
        return false;
    }

    if fs::remove_dir_all(&paths.session).is_err() && paths.session.exists() {
        return false;
    }

    fs::create_dir(&paths.session).is_ok()
}

/// End a cancelled session's dwell: the verdict has been drawn, the recorder is already
/// stopped, and the only thing left is a countdown. The session is asked to finish it now,
/// drops its pill and removes its own directory, and the claim is free for the next dictation.
///
/// `SIGUSR1` is that "the user is asking for the microphone again": it is sent to a session
/// that has already been cancelled, never by `cancel` and never to a session that is recording
/// or transcribing. The wait is bounded, like every wait here: a wedged session is reclaimed by
/// the caller below.
fn end_cancelled_session(paths: &Paths) {
    let Some(pid) = pid_in(&paths.session_pid) else {
        return;
    };

    // Safety: `kill` with a valid signal number is as safe as the shell's `kill`.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGUSR1);
    }

    for _ in 0..40 {
        if !pidfile_lives(&paths.session_pid) {
            return;
        }
        sleep(Duration::from_millis(50));
    }
}
