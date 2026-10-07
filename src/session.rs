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
use std::time::Duration;

use crate::audio;
use crate::claim;
use crate::presentation::{self, Presenter};
use crate::text;

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

    /// The runtime directory itself.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The directory the dictation in flight owns; creating it is the claim on the
    /// microphone.
    pub fn session_directory(&self) -> PathBuf {
        self.directory.join("session")
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
    /// The encoded recording.
    pub audio: PathBuf,
    /// The pipe between the recorder's copy and the encoder.
    pub fifo: PathBuf,
    /// The waveform channel.
    pub waveform: PathBuf,
    /// The recorder's stderr.
    pub recorder_error: PathBuf,
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
            audio: session.join("audio.mp3"),
            fifo: session.join("audio.fifo"),
            waveform: session.join("waveform"),
            recorder_error: session.join("arecord.err"),
            transcribe_error: session.join("transcribe.err"),
            session,
        }
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

/// `start`: claim the microphone, start the pipeline, fork the session process, exit.
pub fn start() -> ExitCode {
    let runtime = Runtime::from_env();
    if let Err(error) = fs::create_dir_all(runtime.directory()) {
        eprintln!("tiny-dictate: cannot create the runtime directory: {error}");
        return ExitCode::FAILURE;
    }
    let paths = runtime.paths();

    // The transcriber is the one mandatory plugin: refuse before anything is claimed.
    if text::resolve_transcriber().is_none() {
        eprintln!(
            "No transcription command: set TINY_DICTATE_TRANSCRIBE or install tiny-dictate-transcribe"
        );
        text::notify(
            "❌ No transcription command. Set TINY_DICTATE_TRANSCRIBE or install tiny-dictate-transcribe",
            8000,
            "critical",
        );
        return ExitCode::FAILURE;
    }

    if !claim::claim(&paths) {
        // A session is running: its pill already shows what is going on, and it is the only
        // writer of the screen.
        eprintln!("tiny-dictate is already busy");
        return ExitCode::FAILURE;
    }

    if muted() {
        text::notify(
            "🎙️ Microphone is muted. Unmute it before dictating.",
            4500,
            "critical",
        );
        let _ = fs::remove_dir_all(&paths.session);
        return ExitCode::FAILURE;
    }

    if audio::create_fifo(&paths.fifo).is_err() {
        text::notify("❌ Cannot create the audio pipe", 6000, "normal");
        let _ = fs::remove_dir_all(&paths.session);
        return ExitCode::FAILURE;
    }

    if let Err(error) = audio::start_pipeline(&paths) {
        eprintln!("tiny-dictate: cannot start the audio pipeline: {error}");
        let _ = fs::remove_dir_all(&paths.session);
        return ExitCode::FAILURE;
    }

    // The recorder's and the encoder's pids are recorded before this fork: that is what makes
    // the session's wait race-free.
    match Command::new(self_executable()).arg("_session").spawn() {
        Ok(session) => {
            let _ = fs::write(&paths.session_pid, session.id().to_string());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("tiny-dictate: cannot fork the session: {error}");
            let _ = fs::remove_dir_all(&paths.session);
            ExitCode::FAILURE
        }
    }
}

/// `stop`: signal the recorder, never the encoder, never the session.
pub fn stop() -> ExitCode {
    let runtime = Runtime::from_env();
    let paths = runtime.paths();

    if pidfile_lives(&paths.recorder_pid) {
        signal_pidfile(&paths.recorder_pid, libc::SIGINT);
    } else if pidfile_lives(&paths.session_pid) && !presentation::cancelled(&paths.state) {
        // The pill already shows "Transcribing"; there is nothing to signal. A cancelled
        // session is not transcribing anything the user still wants.
        eprintln!("Transcription already in progress");
    }

    ExitCode::SUCCESS
}

/// `cancel`: write the verdict, signal the recorder, never the session.
pub fn cancel() -> ExitCode {
    let runtime = Runtime::from_env();
    let paths = runtime.paths();

    let recording = pidfile_lives(&paths.recorder_pid);
    let session = pidfile_lives(&paths.session_pid);

    if !recording && !session {
        println!("No recording in progress");
        return ExitCode::SUCCESS;
    }

    // A verdict is a state like any other: writing it is what shows it, so it is written
    // before anything is signalled. The session reads it and shows it rather than reporting
    // the failure of a recording that was cut short.
    presentation::write_state(&paths.state, presentation::CANCELLED);

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

/// `toggle`: stop when recording, report a transcription in flight, start otherwise.
pub fn toggle() -> ExitCode {
    let runtime = Runtime::from_env();
    let paths = runtime.paths();

    if pidfile_lives(&paths.recorder_pid) {
        stop()
    } else if pidfile_lives(&paths.session_pid) && !presentation::cancelled(&paths.state) {
        eprintln!("Transcription already in progress");
        ExitCode::SUCCESS
    } else {
        // Nothing running, or only the cancelled verdict's dwell: start a new dictation.
        start()
    }
}

/// `_session`: the process that owns the lifecycle, forked by `start`.
pub fn run_session() -> ExitCode {
    let runtime = Runtime::from_env();
    let paths = runtime.paths();

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

    let code = session_body(&paths);
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

fn session_body(paths: &Paths) -> ExitCode {
    let mut presenter = Presenter::start(paths, "recording");

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
        sleep(Duration::from_millis(100));
    }

    // The encoder exits once the recorder is gone and it has flushed its last frames. Bounded:
    // it can block in open() until the recorder opens the pipe, so never wait forever. When
    // nothing was captured there is nothing to flush, so do not wait for it at all.
    let audio_present = file_size(&paths.audio) > 0;
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
        return show_verdict(paths, &mut presenter);
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
        let reason = match fs::read_to_string(&paths.recorder_error) {
            Ok(error) if !error.trim().is_empty() => {
                format!("{reason} ({})", text::format_error(&error))
            }
            _ => reason,
        };

        // The pill goes before the notification: one thing on screen at a time.
        presenter.stop();
        text::notify(&format!("❌ Recording failed: {reason}"), 6000, "normal");
        return ExitCode::SUCCESS;
    }

    presentation::write_state(&paths.state, "transcribing");

    let Some(transcriber) = text::resolve_transcriber() else {
        presenter.stop();
        text::notify("❌ Transcription failed: Unknown error", 6000, "normal");
        return ExitCode::SUCCESS;
    };

    match text::transcribe(
        &transcriber,
        &paths.audio,
        &paths.session,
        &paths.transcribe_error,
    ) {
        Ok(transcript) => {
            // A cancel that landed while the transcription was running: the text is dropped
            // rather than pasted behind the user's back.
            if presentation::cancelled(&paths.state) {
                return show_verdict(paths, &mut presenter);
            }

            // The live state is over once there is text: nothing is left for the pill to say.
            presenter.stop();
            if !transcript.is_empty() && !text::paste(&transcript) {
                text::notify("❌ Could not paste the transcription", 6000, "critical");
            }
        }
        Err(()) => {
            presenter.stop();
            let error = fs::read_to_string(&paths.transcribe_error)
                .map(|text| text::format_error(&text))
                .unwrap_or_else(|_| "Unknown error".to_owned());
            text::notify(&format!("❌ Transcription failed: {error}"), 6000, "normal");
        }
    }

    ExitCode::SUCCESS
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

/// Show the verdict for as long as a notification of it used to stay on screen: the pill is
/// already drawing the `cancelled` word `cancel` wrote, and this is the dwell before the
/// session takes it down. A new dictation cuts it short with `SIGUSR1`; `cancel` never does.
fn show_verdict(paths: &Paths, presenter: &mut Presenter) -> ExitCode {
    signal_pidfile(&paths.encoder_pid, libc::SIGTERM);

    let steps = VERDICT_SECONDS * 10;
    let mut waited = 0;
    while waited < steps && !VERDICT_INTERRUPTED.load(Ordering::Relaxed) {
        sleep(DWELL_STEP);
        waited += 1;
    }

    presenter.stop();
    ExitCode::SUCCESS
}

/// Whether the default source is muted. Best effort, like the shell: an unreadable probe means
/// "not muted", so the dictation is never refused on a guess.
fn muted() -> bool {
    if let Ok(output) = Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SOURCE@"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if !text.trim().is_empty() {
            return text.contains("[MUTED]");
        }
    }

    if let Ok(output) = Command::new("pactl")
        .args(["get-source-mute", "@DEFAULT_SOURCE@"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if text.contains("Mute: yes") {
            return true;
        }
        if text.contains("Mute: no") {
            return false;
        }
    }

    for control in ["Capture", "Mic"] {
        if let Ok(output) = Command::new("amixer")
            .args(["-D", "default", "sget", control])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            if !text.trim().is_empty() {
                if text.contains("[on]") {
                    return false;
                }
                if text.contains("[off]") {
                    return true;
                }
            }
        }
    }

    false
}
