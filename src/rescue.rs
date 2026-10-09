//! The rescue verbs: the kept sessions on disk, read again, re-transcribed or removed.
//!
//! They all live under `session`: `list` prints them, `copy` reads one's transcript to the
//! clipboard, transcribing it first when it has none, `transcribe` runs the transcriber again on
//! one, `delete` is the only verb that destroys one. They read the durable sessions and nothing
//! else: no encoder, no clipboard tool, no microphone, so a machine that cannot dictate can still
//! rescue what was kept.

use std::fs;
use std::process::ExitCode;

use crate::config;
use crate::log;
use crate::session::Runtime;
use crate::store;
use crate::text;
use crate::view;

/// `session list`: the kept sessions, newest first, one row per session.
///
/// A directory whose metadata cannot be read gets a row of its own rather than no row: it is
/// exactly the session `session delete` can remove.
pub fn session_list() -> ExitCode {
    let runtime = Runtime::from_env();
    let sessions = runtime.sessions_directory();

    for id in store::session_ids(&sessions) {
        println!("{}", view::Row::read(&sessions, &id));
    }

    ExitCode::SUCCESS
}

/// `session show <id|last>`: one kept session read in full -- the fields every surface shows it by,
/// its path, and its transcript.
///
/// It reads files and nothing else: no encoder, no clipboard tool, no microphone.
pub fn session_show(target: &str) -> ExitCode {
    let runtime = Runtime::from_env();
    let sessions = runtime.sessions_directory();

    let Some(id) = store::resolve_id(&sessions, target) else {
        return unknown_session(target);
    };

    let detail = view::Detail::read(&sessions, &id);
    println!("{detail}");

    if detail.metadata_reads {
        return ExitCode::SUCCESS;
    }

    // The listing can always show a `?` row, but a command asked to read one session and unable
    // to read it fails. The block above already says what could not be read; this is the log line,
    // named by the session (ADR-0008).
    log::line(Some(&id), "cannot read the session's metadata");
    ExitCode::FAILURE
}

/// `session copy <id|last>`: the transcript of a kept session on the clipboard.
///
/// Reading, not transcribing, is the default: a transcript already on disk is offered as it is.
/// Only when `transcript.txt` is missing -- the session was never transcribed, or the run that
/// would have written it failed -- is the transcriber run first, through the same runner
/// `session transcribe` uses, and its transcript and metadata written exactly as that verb writes
/// them, so the two cannot drift. A transcript that exists but holds no word is reported rather
/// than paid for twice: a second run would say the same thing. Nothing is ever injected: the
/// paste stays the live dictation's business.
pub fn session_copy(target: &str) -> ExitCode {
    let runtime = Runtime::from_env();
    let sessions = runtime.sessions_directory();
    let config = config::load();

    let Some(id) = store::resolve_id(&sessions, target) else {
        return unknown_session(target);
    };
    let Some(mut session) = store::Session::load(&sessions, &id) else {
        return unknown_session(target);
    };

    let transcript = match fs::read_to_string(session.transcript()) {
        Ok(transcript) => transcript,
        Err(_) => match transcribe_and_write(&mut session) {
            Transcription::Text(transcript) => transcript,
            Transcription::NoTranscriber | Transcription::Failed => return ExitCode::FAILURE,
        },
    };

    if transcript.trim().is_empty() {
        log::line(
            Some(&session.id),
            "nothing to copy: the transcript is empty",
        );
        return ExitCode::FAILURE;
    }

    offer_transcript(&session.id, &transcript, config.clipboard_seconds)
}

/// `session transcribe <id|last> [--print]`: run the transcriber again on a kept session, rewrite
/// `transcript.txt` and its metadata, then offer the text on the clipboard -- or print it.
///
/// The transcriber contract is the one a live dictation uses: `$1` the audio, `$2` the session
/// directory, the text on stdout, the error on stderr. Nothing here needs the encoder, the
/// recorder or the clipboard tools; `--print` does not even need `wl-copy`.
pub fn transcribe_session(target: &str, print: bool) -> ExitCode {
    let runtime = Runtime::from_env();
    let sessions = runtime.sessions_directory();

    // The configuration is read here too: the rescue offers the text on the same clipboard
    // lease a live dictation uses, so it takes the same duration from the same file.
    let config = config::load();

    let Some(id) = store::resolve_id(&sessions, target) else {
        return unknown_session(target);
    };
    let Some(mut session) = store::Session::load(&sessions, &id) else {
        return unknown_session(target);
    };

    let transcript = match transcribe_and_write(&mut session) {
        Transcription::Text(transcript) => transcript,
        Transcription::NoTranscriber | Transcription::Failed => return ExitCode::FAILURE,
    };

    if print {
        println!("{transcript}");
        return ExitCode::SUCCESS;
    }

    offer_transcript(&session.id, &transcript, config.clipboard_seconds)
}

/// What one run of the transcriber leaves behind.
enum Transcription {
    /// The transcriber's text, already written to the session with its metadata.
    Text(String),
    /// No transcriber command resolves; nothing ran, and the session is untouched.
    NoTranscriber,
    /// The transcriber ran and failed; the metadata records the failure.
    Failed,
}

/// Run the transcriber on a session and write what it produced exactly as `session transcribe`
/// does: the transcript first, then the metadata. `session copy` runs the same function, so the
/// two verbs cannot drift.
fn transcribe_and_write(session: &mut store::Session) -> Transcription {
    let Some(transcriber) = text::resolve_transcriber() else {
        // The session is known here, so the failure line names it (ADR-0008).
        log::line(
            Some(&session.id),
            "failure: No transcription command. Set TINY_DICTATE_TRANSCRIBE or install tiny-dictate-transcribe",
        );
        return Transcription::NoTranscriber;
    };

    // The transcriber's stderr is collapsed into the one line a failure is shown as, exactly
    // like a live dictation. It is written outside the session directory: a rescue leaves no
    // trace of its own in the session it rescues.
    let error_file = std::env::temp_dir().join(format!(
        "tiny-dictate-transcribe-{}-{}.err",
        std::process::id(),
        session.id
    ));
    let result = text::transcribe_collapsed(
        &transcriber,
        &session.audio(),
        &session.directory,
        &error_file,
    );
    let _ = fs::remove_file(&error_file);

    match result {
        Ok(transcript) => {
            // The transcript reaches the session before any clipboard write, as in a live
            // dictation: a text that only lived in the clipboard is a text that can be lost.
            let _ = fs::write(session.transcript(), &transcript);
            session.metadata.outcome = store::Outcome::Transcribed;
            session.metadata.error = None;
            session.metadata.transcriber = Some(transcriber.display().to_string());
            let _ = session.save();
            Transcription::Text(transcript)
        }
        Err(error) => {
            session.metadata.outcome = store::Outcome::Failed;
            session.metadata.error = Some(error.clone());
            session.metadata.transcriber = Some(transcriber.display().to_string());
            let _ = session.save();

            log::line(Some(&session.id), &format!("failure: {error}"));
            Transcription::Failed
        }
    }
}

/// Offer one transcript on both selections, through the one clipboard write path a live dictation
/// uses, and say so: the lease and the password hint are `text::copy_to_clipboard`'s business.
fn offer_transcript(id: &str, transcript: &str, clipboard_seconds: u64) -> ExitCode {
    if text::copy_to_clipboard(transcript, clipboard_seconds) {
        log::line(Some(id), "transcript copied to the clipboard");
        ExitCode::SUCCESS
    } else {
        log::line(Some(id), "could not copy the transcript to the clipboard");
        ExitCode::FAILURE
    }
}

/// `session delete <id|last>`: remove one session directory, and nothing else. This is the only
/// verb that destroys a recording.
pub fn session_delete(target: &str) -> ExitCode {
    let runtime = Runtime::from_env();
    let sessions = runtime.sessions_directory();

    let Some(id) = store::resolve_id(&sessions, target) else {
        return unknown_session(target);
    };

    match fs::remove_dir_all(sessions.join(&id)) {
        Ok(()) => {
            // ADR-0008: `session delete` is one of the events that carry no session id.
            log::line(None, "session deleted");
            ExitCode::SUCCESS
        }
        Err(error) => {
            log::line(None, &format!("could not delete the session: {error}"));
            ExitCode::FAILURE
        }
    }
}

/// The answer to an id no kept session answers to: reported on stderr, nothing removed.
fn unknown_session(target: &str) -> ExitCode {
    log::line(None, &format!("unknown session: {target}"));
    ExitCode::FAILURE
}
