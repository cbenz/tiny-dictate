//! The session layer: what one dictation leaves behind, and the runtime state of the
//! dictation in flight.
//!
//! `start` claims the microphone, starts the pipeline and forks the session process, then
//! exits; the session outlives it and owns the whole lifecycle. The recorder and the encoder
//! are orphaned with it and are found through their pidfiles.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

use crate::audio;
use crate::claim;
use crate::config::{self, Config};
use crate::files;
use crate::log;
use crate::microphone;
use crate::presentation::{self, Presenter};
use crate::store;
use crate::text;
use crate::xdg;

/// How long a verdict stays on screen before the session takes the pill down: the timeout the
/// cancellation notification used to have.
const VERDICT_SECONDS: u64 = 1;

/// The dwell is cut short in ten steps, so a new dictation waits at most one step for the
/// microphone.
const DWELL_STEP: Duration = Duration::from_millis(100);

/// Set by the `SIGUSR1` handler while the verdict is on screen: a new dictation asking for the
/// microphone cuts the dwell short.
static VERDICT_INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// The `SIGUSR1` handler: the only thing it does is flag the interrupt, which is
/// async-signal-safe.
extern "C" fn interrupt_verdict(_signal: libc::c_int) {
    VERDICT_INTERRUPTED.store(true, Ordering::Relaxed);
}

/// The runtime directory, where the ephemeral side of a dictation lives: the claim
/// directory the session owns, the pidfiles the commands read liveness from, the fifo, the
/// waveform channel and the state file the pill draws.
///
/// The location is the documented one: `$XDG_RUNTIME_DIR`, falling back to
/// `$XDG_STATE_HOME`, falling back to `$HOME/.local/state`, then `tiny-dictate/`.
pub struct Runtime {
    directory: PathBuf,
}

impl Runtime {
    /// The runtime directory of this process, read from the environment.
    pub fn from_env() -> Self {
        Self {
            directory: xdg::runtime().join("tiny-dictate"),
        }
    }

    /// The runtime directory itself.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The directory the dictation in flight owns; creating it is the claim on the
    /// microphone.
    pub fn session_directory(&self) -> PathBuf {
        self.directory.join("session")
    }

    /// The root of the durable sessions: `$XDG_STATE_HOME/tiny-dictate/sessions`, falling
    /// back to `$HOME/.local/state`. Kept apart from the runtime directory so a recording
    /// survives the session process that made it.
    pub fn sessions_directory(&self) -> PathBuf {
        xdg::state().join("tiny-dictate/sessions")
    }

    /// Every file a dictation keeps in its session directory.
    pub fn paths(&self) -> Paths {
        Paths::new(self.session_directory())
    }

    /// Whether a dictation is in flight: its session process or its recorder lives.
    ///
    /// The `working` state covers both recording and transcribing, so either pidfile being
    /// alive is enough.
    pub fn dictating(&self) -> bool {
        pidfile_lives(&self.session_directory().join("session.pid"))
            || pidfile_lives(&self.session_directory().join("arecord.pid"))
    }
}

/// The files one dictation owns, laid out under the claim directory.
#[derive(Clone)]
pub struct Paths {
    /// The claim directory.
    pub session: PathBuf,
    /// The session process.
    pub session_pid: PathBuf,
    /// The recorder.
    pub recorder_pid: PathBuf,
    /// The encoder.
    pub encoder_pid: PathBuf,
    /// The word the presenter draws.
    pub state: PathBuf,
    /// The durable id of the dictation in flight, so the commands that run in a process of
    /// their own can name the session their log line concerns.
    pub session_id: PathBuf,
    /// The pipe between the recorder's copy and the encoder.
    pub fifo: PathBuf,
    /// The waveform channel.
    pub waveform: PathBuf,
    /// The recorder's stderr.
    pub recorder_error: PathBuf,
    /// The encoder's stderr.
    pub encoder_error: PathBuf,
    /// The transcriber's stderr.
    pub transcribe_error: PathBuf,
}

impl Paths {
    fn new(session: PathBuf) -> Self {
        Self {
            session_pid: session.join("session.pid"),
            recorder_pid: session.join("arecord.pid"),
            encoder_pid: session.join("lame.pid"),
            state: session.join("state"),
            session_id: session.join("session.id"),
            fifo: session.join("audio.fifo"),
            waveform: session.join("waveform"),
            recorder_error: session.join("arecord.err"),
            encoder_error: session.join("lame.err"),
            transcribe_error: session.join("transcribe.err"),
            session,
        }
    }
}

/// The exclusive command lock: `start`, `stop`, `cancel` and `toggle` take it for the duration
/// of the command, so two shortcuts pressed at once cannot interleave. `status` takes none.
///
/// The lock is the `command.lock` directory in the runtime directory, created with `mkdir`: the
/// same atomicity the claim uses, with no window between the test and the take. It is released
/// when the guard drops — the shell's `EXIT` trap, as far as it goes: a command killed by a
/// signal leaves the directory behind, and the next one reclaims it once the pid it holds is gone.
struct CommandLock {
    directory: PathBuf,
}

impl CommandLock {
    /// Take the lock, reclaiming a directory whose owner no longer lives. `None` when a live
    /// command holds it.
    fn take(runtime: &Runtime) -> Option<Self> {
        let _ = fs::create_dir_all(runtime.directory());
        let directory = runtime.directory().join("command.lock");

        if Self::create(&directory) {
            return Some(Self { directory });
        }

        // Only a readable, dead pid proves the lock stale: a directory whose owner file is not
        // there yet may be another command between its `mkdir` and its write.
        if pid_in(&directory.join("pid")).is_some_and(|pid| !process_alive(pid)) {
            let _ = fs::remove_dir_all(&directory);
            if Self::create(&directory) {
                return Some(Self { directory });
            }
        }

        None
    }

    /// `mkdir` the lock and record this process as its owner.
    fn create(directory: &Path) -> bool {
        if fs::create_dir(directory).is_err() {
            return false;
        }

        // A pidfile that cannot be written still leaves a lock this process holds: the next
        // command then waits for it rather than reclaiming a live one.
        let _ = fs::write(directory.join("pid"), std::process::id().to_string());
        true
    }
}

impl Drop for CommandLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// Refuse a command while another one holds the lock: the notification says so, and the non-zero
/// exit is what a shortcut needs to see that nothing happened.
fn busy_with_another_command() -> ExitCode {
    log::line(None, "failure: already handling another command");
    text::notify(
        "⏳ tiny-dictate is already handling another command",
        1200,
        "normal",
    );
    ExitCode::FAILURE
}

/// The durable id of the dictation in flight, as `start` published it in the runtime directory.
///
/// `stop`, `cancel` and `toggle` are processes of their own: this file is how they can name the
/// session their line concerns. Nothing there is a session that concerns none.
fn current_session_id(paths: &Paths) -> Option<String> {
    let id = fs::read_to_string(&paths.session_id).ok()?;
    let id = id.trim();

    if id.is_empty() {
        None
    } else {
        Some(id.to_owned())
    }
}

/// The path of this very binary, as the presenter is handed it to re-invoke the tool.
///
/// `readlink -f "$0"` in the shell: the resolved path, so the command works wherever the
/// presenter was started from, a symlink on `PATH` included.
pub fn self_executable() -> PathBuf {
    std::env::current_exe()
        .and_then(fs::canonicalize)
        .unwrap_or_else(|_| PathBuf::from("tiny-dictate"))
}

/// The pid a pidfile names, when it holds one.
pub fn pid_in(pidfile: &Path) -> Option<u32> {
    fs::read_to_string(pidfile).ok()?.trim().parse::<u32>().ok()
}

/// Whether a process lives, read through `/proc` rather than `kill -0`.
pub fn process_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).is_dir()
}

/// Whether the process recorded in a pidfile is alive. The pidfile is read, never removed: a
/// query does not perturb a live session, and the session reads the same file for liveness.
pub fn pidfile_lives(pidfile: &Path) -> bool {
    pid_in(pidfile).map(process_alive).unwrap_or(false)
}

/// Signal the process a pidfile names, when it lives. The pidfile is deliberately not removed:
/// the session reads it to tell whether the recorder is still alive.
pub fn signal_pidfile(pidfile: &Path, signal: libc::c_int) {
    if !pidfile_lives(pidfile) {
        return;
    }
    let Some(pid) = pid_in(pidfile) else {
        return;
    };

    // Safety: `kill` with a valid signal number is as safe as the shell's `kill`.
    unsafe {
        libc::kill(pid as libc::pid_t, signal);
    }
}

/// `start`: take the command lock, claim the microphone, start the pipeline, fork the session
/// process, exit.
pub fn start() -> ExitCode {
    let Some(_lock) = CommandLock::take(&Runtime::from_env()) else {
        return busy_with_another_command();
    };

    start_command("start")
}

/// The start path of a command. The `command` is the name the user ran — `start`, or `toggle`
/// when it decided to start — so the log names what was asked for rather than what it
/// delegated to.
fn start_command(command: &str) -> ExitCode {
    let runtime = Runtime::from_env();
    if let Err(error) = fs::create_dir_all(runtime.directory()) {
        log::line(
            None,
            &format!("failure: cannot create the runtime directory: {error}"),
        );
        return ExitCode::FAILURE;
    }
    let paths = runtime.paths();

    // The transcriber is the one mandatory plugin: refuse before anything is claimed.
    let Some(transcriber) = text::resolve_transcriber() else {
        log::line(
            None,
            "failure: No transcription command. Set TINY_DICTATE_TRANSCRIBE or install tiny-dictate-transcribe",
        );
        text::notify(
            "❌ No transcription command. Set TINY_DICTATE_TRANSCRIBE or install tiny-dictate-transcribe",
            8000,
            "critical",
        );
        return ExitCode::FAILURE;
    };

    // The transcriber declares its audio limit once per dictation, before the microphone is
    // opened: the byte count becomes the duration the encoder's bitrate allows. Every failure
    // mode is "no limit", never an error — a transcriber that declares nothing is one the user
    // dictated to before this limit existed.
    let limit = text::audio_limit(&transcriber);

    if !claim::claim(&paths) {
        // A session is running: its pill already shows what is going on, and it is the only
        // writer of the screen. The line names the session it collided with.
        log::line(
            current_session_id(&paths).as_deref(),
            "failure: already busy",
        );
        return ExitCode::FAILURE;
    }

    if microphone::muted() {
        log::line(None, "failure: the microphone is muted");
        text::notify(
            "🎙️ Microphone is muted. Unmute it before dictating.",
            4500,
            "critical",
        );
        let _ = fs::remove_dir_all(&paths.session);
        return ExitCode::FAILURE;
    }

    if audio::create_fifo(&paths.fifo).is_err() {
        log::line(None, "failure: cannot create the audio pipe");
        text::notify("❌ Cannot create the audio pipe", 6000, "normal");
        let _ = fs::remove_dir_all(&paths.session);
        return ExitCode::FAILURE;
    }

    // The session directory is created before the pipeline: the encoder writes the recording
    // into it from the first frame, so a crash costs at most what was never flushed.
    let session = match store::Session::create(
        &runtime.sessions_directory(),
        Some(&transcriber),
        SystemTime::now(),
    ) {
        Ok(session) => session,
        Err(error) => {
            log::line(
                None,
                &format!("failure: cannot create the session directory: {error}"),
            );
            let _ = fs::remove_dir_all(&paths.session);
            return ExitCode::FAILURE;
        }
    };
    // The stamp is written here, once: the session process carries this metadata over rather
    // than dating the dictation from its own clock.
    let _ = session.save();
    // The id is published before the pipeline: the commands that run beside this dictation read
    // it to name the session their lines concern.
    let _ = fs::write(&paths.session_id, &session.id);

    if let Err(error) = audio::start_pipeline(&paths, &session.audio(), &session.id) {
        log::line(
            Some(&session.id),
            &format!("failure: cannot start the audio pipeline: {error}"),
        );
        discard(&paths, &session);
        return ExitCode::FAILURE;
    }

    // The recording is live: the recorder and the encoder are running, and the timeline can say
    // so before the session process takes over.
    log::line(Some(&session.id), &format!("{command}: recording started"));
    log::line(Some(&session.id), "recording started");

    // The recorder's and the encoder's pids are recorded before this fork: that is what makes
    // the session's wait race-free. The id is handed over so the session writes the metadata
    // of the directory this command created; the transcriber's byte limit rides along, so the
    // session never has to ask for it again once the microphone is open.
    let mut fork = Command::new(self_executable());
    fork.arg("_session").arg(&session.id);
    if let Some(bytes) = limit {
        fork.arg(bytes.to_string());
    }

    match fork.spawn() {
        Ok(child) => {
            let _ = fs::write(&paths.session_pid, child.id().to_string());
            ExitCode::SUCCESS
        }
        Err(error) => {
            log::line(
                Some(&session.id),
                &format!("failure: cannot fork the session: {error}"),
            );
            discard(&paths, &session);
            ExitCode::FAILURE
        }
    }
}

/// Give up a dictation that never started: nothing was recorded, so neither the claim nor
/// the session directory is worth keeping.
fn discard(paths: &Paths, session: &store::Session) {
    let _ = fs::remove_dir_all(&paths.session);
    let _ = fs::remove_dir_all(&session.directory);
}

/// `stop`: take the command lock, then signal the recorder, never the encoder, never the session.
pub fn stop() -> ExitCode {
    let Some(_lock) = CommandLock::take(&Runtime::from_env()) else {
        return busy_with_another_command();
    };

    stop_command("stop")
}

/// The stop path of a command; `command` names what the user ran.
fn stop_command(command: &str) -> ExitCode {
    let runtime = Runtime::from_env();
    let paths = runtime.paths();
    let id = current_session_id(&paths);

    if pidfile_lives(&paths.recorder_pid) {
        log::line(id.as_deref(), &format!("{command}: recorder signalled"));
        signal_pidfile(&paths.recorder_pid, libc::SIGINT);
    } else if transcribing(&paths) {
        // The pill already shows "Transcribing"; there is nothing to signal.
        return transcription_in_progress(&paths, command);
    } else {
        log::line(id.as_deref(), &format!("{command}: nothing to stop"));
    }

    ExitCode::SUCCESS
}

/// `cancel`: take the command lock, then write the verdict and signal the recorder, never the
/// session.
pub fn cancel() -> ExitCode {
    let Some(_lock) = CommandLock::take(&Runtime::from_env()) else {
        return busy_with_another_command();
    };

    cancel_command()
}

/// The cancel path, under the lock its caller holds.
fn cancel_command() -> ExitCode {
    let runtime = Runtime::from_env();
    let paths = runtime.paths();

    let recording = pidfile_lives(&paths.recorder_pid);
    let session = pidfile_lives(&paths.session_pid);

    if !recording && !session {
        log::line(None, "cancel: nothing to cancel");
        println!("No recording in progress");
        return ExitCode::SUCCESS;
    }

    log::line(current_session_id(&paths).as_deref(), "cancel: cancelled");

    // A verdict is a state like any other: writing it is what shows it, so it is written
    // before anything is signalled. The session reads it and shows it rather than reporting
    // the failure of a recording that was cut short.
    presentation::write_state(&paths.state, store::CANCELLED);

    // Stop the recorder, never the encoder: the encoder outlives it to flush the last frames.
    signal_pidfile(&paths.recorder_pid, libc::SIGINT);

    if session {
        // And never the session either: it is the one showing the verdict, waiting out the
        // dwell, and removing its own directory.
        return ExitCode::SUCCESS;
    }

    // No session left to do any of that: only its leftovers are here.
    signal_pidfile(&paths.encoder_pid, libc::SIGTERM);
    let _ = fs::remove_dir_all(&paths.session);
    ExitCode::SUCCESS
}

/// `toggle`: take the command lock, then stop when recording, report a transcription in flight,
/// start otherwise.
pub fn toggle() -> ExitCode {
    let Some(_lock) = CommandLock::take(&Runtime::from_env()) else {
        return busy_with_another_command();
    };

    toggle_command()
}

/// The toggle path, under the lock its caller holds.
fn toggle_command() -> ExitCode {
    let runtime = Runtime::from_env();
    let paths = runtime.paths();

    if pidfile_lives(&paths.recorder_pid) {
        stop_command("toggle")
    } else if transcribing(&paths) {
        transcription_in_progress(&paths, "toggle")
    } else {
        // Nothing running, or only the cancelled verdict's dwell: start a new dictation.
        start_command("toggle")
    }
}

/// Whether a live session is transcribing, as opposed to recording or holding a verdict. The
/// pill already shows `Transcribing`; a cancelled session is not transcribing anything the user
/// still wants.
fn transcribing(paths: &Paths) -> bool {
    pidfile_lives(&paths.session_pid) && !presentation::cancelled(&paths.state)
}

/// Tell the user the session is already transcribing, and that there is nothing to signal.
fn transcription_in_progress(paths: &Paths, command: &str) -> ExitCode {
    log::line(
        current_session_id(paths).as_deref(),
        &format!("{command}: transcription already in progress"),
    );
    eprintln!("Transcription already in progress");
    ExitCode::SUCCESS
}

/// `_session`: the process that owns the lifecycle, forked by `start` with the session id.
///
/// When the transcriber declared one, the remaining argument is its audio limit in bytes.
pub fn run_session(id: &str, arguments: &[String]) -> ExitCode {
    if id.is_empty() {
        eprintln!("tiny-dictate: _session needs a session id");
        return ExitCode::FAILURE;
    }

    let runtime = Runtime::from_env();
    let paths = runtime.paths();

    // The limit in bytes becomes a duration at the encoder's bitrate; anything unparsable or
    // absent is no limit, so a garbled argument can only relax the deadline, never shorten it.
    let limit = arguments
        .first()
        .and_then(|bytes| bytes.parse::<u64>().ok())
        .and_then(text::limit_from_bytes);

    // The transcriber is resolved now rather than at the transcription, so a session that is
    // cancelled before it is ever transcribed still records which command it would have run.
    let transcriber = text::resolve_transcriber();
    // `start` wrote the metadata and stamped the start; this process carries it over rather than
    // dating the dictation from its own clock, so `started` and `duration` are the recording's.
    let mut session = store::Session::open(
        &runtime.sessions_directory(),
        id,
        transcriber.as_deref(),
        SystemTime::now(),
    );

    // The configuration is read once, here in the session that needs all of it: the pill's
    // colour and the prune's `keep` now, the clipboard lease's duration when it lands.
    let config = config::load();
    // A new dictation ends the cancelled verdict's dwell early; the handler only flags it, and
    // installing it before anything else means the signal is never the default one, which would
    // kill the session.
    // Safety: `signal` with a valid handler; the handler only stores into an atomic.
    unsafe {
        libc::signal(
            libc::SIGUSR1,
            interrupt_verdict as *const () as libc::sighandler_t,
        );
    }

    let guard = SessionGuard {
        paths: paths.clone(),
        pid: std::process::id(),
    };

    let code = session_body(
        &paths,
        &mut session,
        limit,
        &config,
        &runtime.sessions_directory(),
    );
    drop(guard);
    code
}

/// Remove the session directory, but only if it is still ours: a newer session may have
/// reclaimed the same paths, and its files are not ours to remove.
struct SessionGuard {
    paths: Paths,
    pid: u32,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        if pid_in(&self.paths.session_pid) == Some(self.pid) {
            let _ = fs::remove_dir_all(&self.paths.session);
        }
    }
}

fn session_body(
    paths: &Paths,
    session: &mut store::Session,
    limit: Option<Duration>,
    config: &Config,
    sessions: &Path,
) -> ExitCode {
    let mut presenter = Presenter::start(paths, "recording", limit, &config.color);

    // The deadline is the limit counted from the moment the session takes over the recording:
    // the recorder was started just before the fork. When it passes, the recording is stopped
    // exactly as the shortcut's `stop` stops it — the text is transcribed and pasted, never
    // dropped — and one stderr line says why.
    let deadline = limit.map(|limit| Instant::now() + limit);
    let mut limit_reached = false;

    let mut recording_failed = false;
    let mut encoder_failed = false;

    // Wait for the recorder the parent started and recorded before forking us. If the encoder
    // dies first, the recorder would write into a pipe nobody reads and block forever: give
    // up instead of spinning forever.
    while pidfile_lives(&paths.recorder_pid) {
        if !pidfile_lives(&paths.encoder_pid) {
            recording_failed = true;
            break;
        }
        if !limit_reached && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            limit_reached = true;
            // The recorder is signalled, the encoder outlives it to flush the last frames: the
            // very path `stop` takes.
            signal_pidfile(&paths.recorder_pid, libc::SIGINT);
            log::line(
                Some(&session.id),
                "recording stopped at the transcriber's audio limit",
            );
        }
        sleep(Duration::from_millis(100));
    }

    // The encoder exits once the recorder is gone and it has flushed its last frames. Bounded:
    // it can block in open() until the recorder opens the pipe, so never wait forever. When
    // nothing was captured there is nothing to flush, so do not wait for it at all.
    let audio_present = files::file_size(&session.audio()) > 0;
    if audio_present {
        for _ in 0..100 {
            if !pidfile_lives(&paths.encoder_pid) {
                break;
            }
            sleep(Duration::from_millis(50));
        }

        if pidfile_lives(&paths.encoder_pid) {
            recording_failed = true;
            encoder_failed = true;
        }
    } else {
        recording_failed = true;
    }

    // The verdict owns the last word: a cancelled dictation is neither a failure nor something
    // to transcribe, whichever moment the cancel landed in.
    if presentation::cancelled(&paths.state) {
        return show_verdict(paths, session, &mut presenter);
    }

    if recording_failed {
        signal_pidfile(&paths.recorder_pid, libc::SIGINT);
        signal_pidfile(&paths.encoder_pid, libc::SIGTERM);

        let reason = if encoder_failed {
            "the audio encoder did not finish".to_owned()
        } else if audio_present {
            "the audio encoder stopped while recording".to_owned()
        } else {
            "no audio was captured".to_owned()
        };
        // The detail comes from the collaborator that failed: the recorder never captured a
        // byte, or the encoder stopped under a live recorder. Both were kept out of the
        // log and read back here, so the failure line carries what the user can act on.
        let detail = if audio_present {
            &paths.encoder_error
        } else {
            &paths.recorder_error
        };
        let reason = match fs::read_to_string(detail) {
            Ok(error) if !error.trim().is_empty() => {
                format!("{reason} ({})", text::format_error(&error))
            }
            _ => reason,
        };

        end_recording(session);
        session.metadata.outcome = store::Outcome::Failed;
        session.metadata.error = Some(reason.clone());
        let _ = session.save();
        log::line(Some(&session.id), &format!("failure: {reason}"));

        // The pill goes before the notification: one thing on screen at a time.
        presenter.stop();
        text::notify(&format!("❌ Recording failed: {reason}"), 6000, "normal");
        return ExitCode::SUCCESS;
    }

    // The recording is over and on disk: the metadata now carries its end, its duration and
    // its size, and says what the session is doing next.
    end_recording(session);
    session.metadata.outcome = store::Outcome::Transcribing;
    let _ = session.save();

    presentation::write_state(&paths.state, "transcribing");

    let Some(transcriber) = text::resolve_transcriber() else {
        session.metadata.outcome = store::Outcome::Failed;
        session.metadata.error = Some("Unknown error".to_owned());
        let _ = session.save();
        log::line(Some(&session.id), "failure: Unknown error");
        presenter.stop();
        text::notify("❌ Transcription failed: Unknown error", 6000, "normal");
        return ExitCode::SUCCESS;
    };
    session.metadata.transcriber = Some(transcriber.display().to_string());

    log::line(Some(&session.id), "transcription started");
    let started_transcribing = Instant::now();

    match text::transcribe_collapsed(
        &transcriber,
        &session.audio(),
        &session.directory,
        &paths.transcribe_error,
    ) {
        Ok(transcript) => {
            let characters = transcript.chars().count();
            log::line(
                Some(&session.id),
                &format!(
                    "transcription ended duration={:.2}s chars={characters}",
                    started_transcribing.elapsed().as_secs_f64()
                ),
            );

            // A cancel that landed while the transcription was running: the text is dropped
            // rather than pasted behind the user's back.
            if presentation::cancelled(&paths.state) {
                log::line(Some(&session.id), "late cancel, text dropped");
                return show_verdict(paths, session, &mut presenter);
            }

            // The text reaches the session before any clipboard write: a paste that lands
            // wrong stays recoverable, a text that only lived in the clipboard does not.
            let _ = fs::write(session.transcript(), &transcript);
            log::line(
                Some(&session.id),
                &format!("text written chars={characters}"),
            );
            session.metadata.outcome = store::Outcome::Transcribed;
            session.metadata.error = None;
            session.metadata.pasted = false;
            let _ = session.save();

            // The live state is over once there is text: nothing is left for the pill to say.
            presenter.stop();

            let pasted =
                !transcript.is_empty() && text::paste(&transcript, config.clipboard_seconds);
            if transcript.is_empty() {
                log::line(
                    Some(&session.id),
                    "paste failed: the transcription is empty",
                );
            } else if pasted {
                log::line(Some(&session.id), "paste succeeded");
            } else {
                log::line(Some(&session.id), "paste failed");
                text::notify("❌ Could not paste the transcription", 6000, "critical");
            }
            if pasted {
                session.metadata.pasted = true;
                let _ = session.save();

                // Bounding the directory belongs to a dictation that reached the user: a
                // failure or a cancel prunes nothing (ADR-0005). The prune concerns no session,
                // so its line carries none.
                let removed = store::prune_sessions(sessions, config.keep, &session.id);
                if !removed.is_empty() {
                    log::line(None, &format!("pruned {} sessions", removed.len()));
                }
            }
        }
        Err(error) => {
            log::line(
                Some(&session.id),
                &format!(
                    "transcription ended duration={:.2}s chars=0",
                    started_transcribing.elapsed().as_secs_f64()
                ),
            );
            session.metadata.outcome = store::Outcome::Failed;
            session.metadata.error = Some(error.clone());
            let _ = session.save();
            log::line(Some(&session.id), &format!("failure: {error}"));

            presenter.stop();
            text::notify(&format!("❌ Transcription failed: {error}"), 6000, "normal");
        }
    }

    ExitCode::SUCCESS
}

/// Close the recording stage and log it: the duration it lasted and the bytes it produced.
///
/// One line per stage, and the recording stage ends in exactly one place per path — the failure
/// path, the normal path, and the cancel that lands before either — so the line appears once
/// and only when the recording actually ended.
fn end_recording(session: &mut store::Session) {
    session.finish_recording(SystemTime::now());
    log::line(
        Some(&session.id),
        &format!(
            "recording ended duration={:.2}s bytes={}",
            session.metadata.duration.unwrap_or(0.0),
            session.metadata.audio_bytes
        ),
    );
}

/// Show the verdict for as long as a notification of it used to stay on screen: the pill is
/// already drawing the `cancelled` word `cancel` wrote, and this is the dwell before the
/// session takes it down. A new dictation cuts it short with `SIGUSR1`; `cancel` never does.
///
/// Cancelling keeps the recording: the session is marked `cancelled`, and nothing is ever
/// transcribed or pasted from it.
fn show_verdict(
    paths: &Paths,
    session: &mut store::Session,
    presenter: &mut Presenter,
) -> ExitCode {
    signal_pidfile(&paths.encoder_pid, libc::SIGTERM);

    // A cancel that landed while recording ends the recording here; a cancel that landed
    // during the transcription leaves the end the transcription stage already recorded.
    if session.metadata.ended.is_none() {
        end_recording(session);
    }
    session.metadata.outcome = store::Outcome::Cancelled;
    let _ = session.save();

    let steps = VERDICT_SECONDS * 10;
    let mut waited = 0;
    while waited < steps && !VERDICT_INTERRUPTED.load(Ordering::Relaxed) {
        sleep(DWELL_STEP);
        waited += 1;
    }

    presenter.stop();
    ExitCode::SUCCESS
}
