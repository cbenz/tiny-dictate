//! The audio server's default source: the base volume it publishes and the mute state the tool
//! clears.
//!
//! `pactl` is the only command this module runs. It publishes the base volume — the level at which
//! the server adds no gain, the neutral point of that machine's hardware — and it clears the mute
//! state of the same source. The tool sets the default source to that base and unmutes it before
//! every dictation, after the claim and before the recorder opens, so the level the recorder gets
//! is the level the server was told
//! ([ADR-0014](../../docs/adr/0014-the-microphone-at-its-base-volume.md)).

use std::process::{Command, Output};

use crate::log;

/// The source the recorder opens: the ALSA `default` PCM routes to it, so setting it is setting
/// the level recorded.
const SOURCE: &str = "@DEFAULT_SOURCE@";

/// Put the default source at its base volume and unmute it, and say so in one line naming the
/// session.
///
/// Called once per dictation, after the claim and once the durable session exists: the claim is
/// the exclusive hold on the microphone, and touching the source while another dictation records
/// would disturb it ([ADR-0009](../../docs/adr/0009-the-claim.md)). Best effort, like the shell: a
/// server that answers badly leaves the level alone and one line says why, and a server that
/// cannot be reached writes nothing at all — a dictation is never refused on it.
pub fn set_base_volume(session_id: &str) {
    let Some(sources) = list_sources() else {
        log::line(
            Some(session_id),
            "microphone left at its level: no audio server answered",
        );
        return;
    };

    if let Some(raw) = base_volume(&sources) {
        set_source_volume(&raw);
        log::line(
            Some(session_id),
            &format!("microphone set to base volume {raw}"),
        );
    } else {
        log::line(
            Some(session_id),
            "microphone left at its level: no base volume published",
        );
    }

    unmute();
}

/// Run one `pactl` command, its output captured so a server that fails cannot chatter in the
/// journal. `None` when the command cannot be run or answers non-zero.
fn pactl(arguments: &[&str]) -> Option<Output> {
    Command::new("pactl")
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
}

/// What `pactl list sources` publishes about the sources.
fn list_sources() -> Option<String> {
    let output = pactl(&["list", "sources"])?;

    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The raw value on the `Base Volume:` line, e.g. `6554` on
/// `Base Volume: 6554 / 10% / -60.00 dB`.
///
/// Read raw and written raw, with no round trip through the rounded percentage or the dB string.
/// The shape of the line varies with the server version, so an absent, empty or non-numeric value
/// is `None` rather than an error.
fn base_volume(sources: &str) -> Option<String> {
    sources
        .lines()
        .find_map(|line| line.trim().strip_prefix("Base Volume:"))
        .and_then(|value| value.split_whitespace().next())
        .filter(|raw| !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit()))
        .map(str::to_owned)
}

fn set_source_volume(raw: &str) {
    let _ = pactl(&["set-source-volume", SOURCE, raw]);
}

/// Clear the source's mute state. Unconditional: no configuration key guards it, because a tool
/// that cannot take the microphone cannot dictate.
fn unmute() {
    let _ = pactl(&["set-source-mute", SOURCE, "0"]);
}
