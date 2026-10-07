//! Whether the default source is muted.
//!
//! Best effort, like the shell: three probes, each optional, and an unreadable answer means "not
//! muted", so a dictation is never refused on a guess.

use std::process::Command;

/// Whether the default source is muted.
pub fn muted() -> bool {
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
