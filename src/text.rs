//! The text of a dictation: the transcriber command, the transcript and the paste.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use crate::audio;

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

/// Ask the transcriber for the largest audio file it accepts, in bytes.
///
/// The probe is `--print-audio-limit`. Every failure mode means "no limit" — an unsupported flag
/// (a non-zero status), empty output, an unparsable number, a command that cannot be run — so a
/// transcriber that declares nothing gets no ceiling the user did not ask for. The probe runs
/// once per dictation, before the microphone is opened, and its stderr is swallowed: a
/// transcriber that does not know the flag must not chatter in the journal.
pub fn audio_limit(command: &Path) -> Option<u64> {
    let output = Command::new(command)
        .arg("--print-audio-limit")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    parse_limit_bytes(&output.stdout)
}

/// The byte count a probe printed, or `None` when it is empty or not a number.
fn parse_limit_bytes(stdout: &[u8]) -> Option<u64> {
    String::from_utf8_lossy(stdout).trim().parse::<u64>().ok()
}

/// The longest recording a byte limit allows, from the encoder's bitrate.
///
/// The encoder runs `lame … -b 64`, i.e. 64 kbit/s, i.e. 8000 bytes per second: seconds =
/// bytes / 8000. Zero bytes is no limit at all, not a recording of no length.
pub fn limit_from_bytes(bytes: u64) -> Option<Duration> {
    if bytes == 0 {
        return None;
    }

    Some(Duration::from_secs_f64(
        bytes as f64 / audio::BYTES_PER_SECOND as f64,
    ))
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

/// Offer the text on both selections, the way a normal dictation does, and report whether the
/// CLIPBOARD selection took it. Every clipboard write in the tool goes through here, so the lease
/// has exactly one place to replace. `clipboard_seconds` is the lease's duration, read from the
/// configuration: `0` means no expiry.
pub fn copy_to_clipboard(value: &str, clipboard_seconds: u64) -> bool {
    let clipboard = clipboard_write(value, false, clipboard_seconds);
    let _ = clipboard_write(value, true, clipboard_seconds);

    clipboard
}

/// Offer the text on both selections and inject it with the `Shift+Insert` chord.
///
/// Wayland has no API to insert text into another client, so the text is copied and the chord
/// pressed. The chord carries no Enter: the text is inserted, never submitted.
///
/// `clipboard_seconds` is the lease's duration, read from the configuration: the offer is wrapped
/// in `timeout <clipboard_seconds>`, so it is gone when the lease ends
/// ([ADR-0007](../docs/adr/0007-the-clipboard-lease.md)).
pub fn paste(value: &str, clipboard_seconds: u64) -> bool {
    copy_to_clipboard(value, clipboard_seconds);

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

/// Offer the text on one selection, under the lease.
///
/// The offer is `timeout <clipboard_seconds> wl-copy --foreground --sensitive` (no `timeout`
/// wrapper when the duration is `0`): `--foreground` keeps `wl-copy` in the process `timeout`
/// wrapped, so the wrapper, or another client taking the selection over, is what ends the offer;
/// `--sensitive` sets the KDE password hint, which keeps the transcript out of a clipboard
/// history and stops a manager from consuming the offer; `--paste-once` is deliberately absent, so
/// a stolen focus, a paste into the wrong window or a manager reading the data cannot cost the
/// user the text ([ADR-0007](../docs/adr/0007-the-clipboard-lease.md)).
///
/// The offer is not waited for: `--foreground` serves the selection for the whole lease, so
/// waiting here would wait out the lease and the paste would arrive a minute late. What is
/// reported is whether the offer took the text: a `wl-copy` that cannot reach a compositor dies on
/// the spot, which closes the pipe under the write or leaves an already-exited process behind. An
/// offer still alive once the text has been handed over is serving the selection.
fn clipboard_write(value: &str, primary: bool, clipboard_seconds: u64) -> bool {
    // A duration of `0` means no expiry: the offer is not wrapped, and serves until another
    // client takes the selection over.
    let mut command = match clipboard_seconds {
        0 => Command::new("wl-copy"),
        seconds => {
            let mut command = Command::new("timeout");
            command.arg(seconds.to_string()).arg("wl-copy");
            command
        }
    };
    if primary {
        command.arg("--primary");
    }
    command
        .arg("--foreground")
        .arg("--sensitive")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Ok(mut process) = command.spawn() {
        let written = match process.stdin.take() {
            Some(mut stdin) => stdin.write_all(value.as_bytes()).is_ok(),
            None => false,
        };

        if !written {
            // The offer never took the text (no compositor, a broken `wl-copy`): reap it.
            let _ = process.wait();
            return false;
        }

        return match process.try_wait() {
            Ok(Some(status)) => status.success(),
            _ => true,
        };
    }

    false
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_error_flattens_multiline_stderr_to_one_line() {
        let stderr = "arecord: main:7226:\taudio open error:\n\n   Device or resource busy  \n";

        assert_eq!(
            format_error(stderr),
            "arecord: main:7226: audio open error: Device or resource busy"
        );
    }

    #[test]
    fn format_error_is_unknown_when_there_is_nothing_to_show() {
        assert_eq!(format_error(""), "Unknown error");
        assert_eq!(format_error(" \n\t "), "Unknown error");
    }

    #[test]
    fn format_error_truncates_to_180_characters() {
        let stderr = format!("{}\n{}", "x".repeat(120), "y".repeat(120));

        let message = format_error(&stderr);

        // 120 x, one space for the flattened newline, then 59 y: the 180th character.
        assert_eq!(message, format!("{} {}", "x".repeat(120), "y".repeat(59)));
        assert_eq!(message.chars().count(), 180);
    }

    #[test]
    fn a_byte_limit_becomes_a_duration_at_the_encoder_bitrate() {
        // 64 kbit/s is 8000 bytes per second: 8000 bytes is one second, and Groq's 25 MB
        // free-tier ceiling is 25_000_000 / 8000 = 3125 seconds.
        assert_eq!(limit_from_bytes(8000), Some(Duration::from_secs(1)));
        assert_eq!(limit_from_bytes(4000), Some(Duration::from_secs_f64(0.5)));
        assert_eq!(
            limit_from_bytes(25_000_000),
            Some(Duration::from_secs_f64(3125.0))
        );
        assert_eq!(limit_from_bytes(0), None, "zero bytes is no limit");
    }

    #[test]
    fn an_empty_or_unparsable_probe_is_no_limit() {
        assert_eq!(parse_limit_bytes(b""), None);
        assert_eq!(parse_limit_bytes(b"  \n"), None);
        assert_eq!(parse_limit_bytes(b"not a number\n"), None);
        assert_eq!(parse_limit_bytes(b"25000000\n"), Some(25_000_000));
    }
}
