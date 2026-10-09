//! The audio server's default source: the base volume it publishes and the mute state the tool
//! clears.
//!
//! `pactl` is the only command this module runs. It publishes the base volume — the level at which
//! the server adds no gain, the neutral point of that machine's hardware — and it clears the mute
//! state of the same source. The tool sets the default source to that base and unmutes it before
//! every dictation, after the claim and before the recorder opens, so the level the recorder gets
//! is the level the server was told
//! ([ADR-0014](../../docs/adr/0014-the-microphone-at-its-base-volume.md)).
//!
//! A server that publishes no readable base is answered with the configured percentage instead —
//! the level the user chose for it — and a server that cannot be reached is answered with one log
//! line and nothing written at all.

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
/// volume the server publishes and the parser can read is written back raw, a server that answers
/// without one is answered with `input_level_pct`, and a server that cannot be reached writes
/// nothing at all — a dictation is never refused on it.
pub fn set_base_volume(session_id: &str, input_level_pct: u32) {
    let Some(sources) = list_sources() else {
        log::line(
            Some(session_id),
            "microphone left at its level: no audio server answered",
        );
        return;
    };

    match parse_base_volume(&sources) {
        Some(raw) => {
            set_source_volume(&raw);
            log::line(
                Some(session_id),
                &format!("microphone set to base volume {raw}"),
            );
        }
        None => {
            // A server that publishes no readable base is not silence: the configured level stands
            // in, so the recording has a sane gain on every machine. The key is never a target, only
            // the stand-in.
            let level = format!("{input_level_pct}%");
            set_source_volume(&level);
            log::line(
                Some(session_id),
                &format!("microphone set to input level {level}: no readable base volume"),
            );
        }
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
/// is `None` rather than an error: the caller then writes the configured fallback.
fn parse_base_volume(sources: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::parse_base_volume;

    // The shape the reference machine publishes: the raw value comes out, not the rounded
    // percentage nor the dB string.
    #[test]
    fn reads_the_raw_value_of_a_base_volume_line() {
        let sources = "Source #0\n\tName: alsa_input.pci-0000_00_1f.3\n\tBase Volume: 6554 / 10% / -60.00 dB\n";

        assert_eq!(parse_base_volume(sources).as_deref(), Some("6554"));
    }

    // A listing a server version prints without the line is a normal answer, not an error.
    #[test]
    fn a_listing_without_a_base_volume_line_has_no_base() {
        let sources = "Source #0\n\tName: alsa_input.pci-0000_00_1f.3\n\tMute: no\n";

        assert_eq!(parse_base_volume(sources), None);
    }

    // The line the parser cannot read is a normal answer too: the caller writes the fallback.
    #[test]
    fn a_base_volume_line_with_no_readable_value_has_no_base() {
        for line in [
            "Base Volume: nonsense / 10% / -60.00 dB",
            "Base Volume: / 10% / -60.00 dB",
            "Base Volume: 12.5 / 10% / -60.00 dB",
        ] {
            assert_eq!(parse_base_volume(line), None, "{line}");
        }
    }
}
