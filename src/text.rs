//! The text of a dictation: the transcriber command, the transcript and the paste.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

/// The largest error shown to the user, in characters.
const ERROR_LIMIT: usize = 180;
/// How long a key is held in the synthesised chord, when the environment names no delay.
const DEFAULT_HOLD: &str = "0.04";

/// Resolve the transcriber command: `$TINY_DICTATE_TRANSCRIBE` when set, else
/// `tiny-dictate-transcribe` on `PATH`.
pub fn resolve_transcriber() -> Option<PathBuf> {
    let name = std::env::var("TINY_DICTATE_TRANSCRIBE")
        .unwrap_or_else(|_| "tiny-dictate-transcribe".to_owned());

    resolve_command(&name)
}

/// Resolve one command name the way `command -v` does: a path is used as it is, anything else
/// is searched on `PATH`.
pub fn resolve_command(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let path = PathBuf::from(name);
        return is_executable(&path).then_some(path);
    }

    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Run the transcriber: the audio file as `$1`, the session directory as `$2`, the text on
/// stdout, the errors on stderr, a non-zero exit a failure.
pub fn transcribe(
    command: &Path,
    audio: &Path,
    session: &Path,
    error_file: &Path,
) -> Result<String, ()> {
    let mut child = Command::new(command);
    child.arg(audio).arg(session).stdout(Stdio::piped());

    if let Ok(file) = fs::File::create(error_file) {
        child.stderr(Stdio::from(file));
    }

    let output = child.output().map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }

    // Command substitution strips the trailing newlines; the transcript is the text itself.
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.trim_end_matches(['\n', '\r']).to_owned())
}

/// Flatten an error into the one line the user is shown: whitespace runs collapsed, trimmed,
/// empty becomes "Unknown error", truncated.
pub fn format_error(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return "Unknown error".to_owned();
    }

    collapsed.chars().take(ERROR_LIMIT).collect()
}

/// Send one desktop notification. Failures are silent: a notification is a courtesy, never a
/// reason for the dictation to fail.
pub fn notify(message: &str, expire_ms: u32, urgency: &str) {
    let _ = Command::new("notify-send")
        .arg("--app-name=tiny-dictate")
        .arg(format!("--urgency={urgency}"))
        .arg(format!("--expire-time={expire_ms}"))
        .arg("tiny-dictate")
        .arg(message)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Offer the text on both selections and inject it with the `Shift+Insert` chord.
///
/// Wayland has no API to insert text into another client, so the text is copied and the chord
/// pressed. The chord carries no Enter: the text is inserted, never submitted.
pub fn paste(value: &str) -> bool {
    clipboard_write(value, false);
    clipboard_write(value, true);

    if !ydotool(&["key", "42:1"]) {
        return false;
    }
    sleep(hold_delay());

    let inserted = ydotool(&["key", "110:1", "110:0"]);
    sleep(hold_delay());

    // The modifier is always released, so a failure in the middle cannot leave Shift stuck.
    let _ = ydotool(&["key", "42:0"]);

    inserted
}

fn clipboard_write(value: &str, primary: bool) {
    let mut child = Command::new("wl-copy");
    if primary {
        child.arg("--primary");
    }
    child
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Ok(mut process) = child.spawn() {
        if let Some(mut stdin) = process.stdin.take() {
            let _ = stdin.write_all(value.as_bytes());
        }
        let _ = process.wait();
    }
}

fn ydotool(arguments: &[&str]) -> bool {
    Command::new("ydotool")
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn hold_delay() -> Duration {
    let seconds = std::env::var("LLM_SHIFT_INSERT_HOLD_DELAY_SEC")
        .unwrap_or_else(|_| DEFAULT_HOLD.to_owned())
        .parse::<f64>()
        .unwrap_or(0.04);

    Duration::from_secs_f64(seconds.clamp(0.0, 10.0))
}
