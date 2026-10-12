//! The audio server's default source: the base volume it publishes and the mute state the tool
//! clears.
//!
//! `pactl` is the only command this module runs. It names the source the server calls default
//! (`get-default-source`) and publishes that source's base volume — the level at which the server
//! adds no gain, the neutral point of that machine's hardware — and it clears the mute state of the
//! same source. The tool sets the default source to that base and unmutes it before every
//! dictation, after the claim and before the recorder opens, so the level the recorder gets is the
//! level the server was told
//! ([ADR-0014](../../docs/adr/0014-the-microphone-at-its-base-volume.md)). The base is read at that
//! moment, so a default source that changed since the last dictation — a headset came or went — is
//! the source that is set, and the source that is recorded.
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

    match parse_base_volume(&sources, default_source().as_deref()) {
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

/// The source the server calls default, e.g. `alsa_input.pci-0000_00_1f.3`; `None` when the
/// command cannot be run or prints nothing. The caller then reads a listing that publishes a
/// single source, or writes the configured fallback.
fn default_source() -> Option<String> {
    let output = pactl(&["get-default-source"])?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();

    (!name.is_empty()).then_some(name)
}

/// One source in a `pactl list sources` listing: the name it publishes and the raw base volume,
/// either of which the server may omit.
struct Source {
    name: Option<String>,
    base: Option<String>,
}

/// The sources a `pactl list sources` listing publishes, in the order the server printed them.
///
/// A listing is a sequence of `Source #N` blocks, each carrying one source's properties indented
/// under its header.
fn sources(listing: &str) -> Vec<Source> {
    let mut sources: Vec<Source> = Vec::new();

    for line in listing.lines() {
        let line = line.trim();

        if line.starts_with("Source #") {
            sources.push(Source {
                name: None,
                base: None,
            });
        } else if let Some(current) = sources.last_mut() {
            if let Some(value) = line.strip_prefix("Name:") {
                current.name = Some(value.trim().to_owned());
            } else if let Some(value) = line.strip_prefix("Base Volume:") {
                current.base = raw_value(value);
            }
        }
    }

    sources
}

/// The raw value on a `Base Volume:` line, e.g. `6554` on
/// `Base Volume: 6554 / 10% / -60.00 dB`.
///
/// Read raw and written raw, with no round trip through the rounded percentage or the dB string.
/// The shape of the line varies with the server version, so an absent, empty or non-numeric value
/// is `None` rather than an error: the caller then writes the configured fallback.
fn raw_value(value: &str) -> Option<String> {
    value
        .split_whitespace()
        .next()
        .filter(|raw| !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit()))
        .map(str::to_owned)
}

/// The raw base volume of the source the server names as default, or of a listing that publishes a
/// single source when the server names none.
///
/// Only the default source's base is ever written: with several sources and no name to pick one,
/// the base of another source is never substituted. `None` means the caller writes the configured
/// fallback.
fn parse_base_volume(listing: &str, default_source: Option<&str>) -> Option<String> {
    let sources = sources(listing);

    match default_source {
        Some(name) => sources
            .iter()
            .find(|source| source.name.as_deref() == Some(name))
            .and_then(|source| source.base.clone()),
        // No name to pick the source by: a listing that publishes exactly one source leaves no
        // room for a mistake, so that one is the default; several leave a choice the tool must
        // not make.
        None => match sources.as_slice() {
            [only] => only.base.clone(),
            _ => None,
        },
    }
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

        assert_eq!(parse_base_volume(sources, None).as_deref(), Some("6554"));
    }

    // A listing a server version prints without the line is a normal answer, not an error.
    #[test]
    fn a_listing_without_a_base_volume_line_has_no_base() {
        let sources = "Source #0\n\tName: alsa_input.pci-0000_00_1f.3\n\tMute: no\n";

        assert_eq!(parse_base_volume(sources, None), None);
    }

    // The line the parser cannot read is a normal answer too: the caller writes the fallback.
    #[test]
    fn a_base_volume_line_with_no_readable_value_has_no_base() {
        for value in [
            "nonsense / 10% / -60.00 dB",
            "/ 10% / -60.00 dB",
            "12.5 / 10% / -60.00 dB",
        ] {
            let sources =
                format!("Source #0\n\tName: alsa_input.pci-0000_00_1f.3\n\tBase Volume: {value}\n");

            assert_eq!(parse_base_volume(&sources, None), None, "{value}");
        }
    }

    // With several sources the base of the one the server names is the base, never the first
    // listing's: another source's base is never written on the default one.
    #[test]
    fn a_listing_with_several_sources_reads_the_named_source_base() {
        let sources = "Source #0\n\tName: alsa_output.stub.monitor\n\tBase Volume: 3277 / 5% / -66.00 dB\nSource #1\n\tName: alsa_input.pci-0000_00_1f.3\n\tBase Volume: 6554 / 10% / -60.00 dB\n";

        assert_eq!(
            parse_base_volume(sources, Some("alsa_input.pci-0000_00_1f.3")).as_deref(),
            Some("6554")
        );
        assert_eq!(
            parse_base_volume(sources, Some("alsa_output.stub.monitor")).as_deref(),
            Some("3277")
        );
    }

    // The server named no source and the listing publishes exactly one: there is no room for a
    // mistake, so that source's base is the base.
    #[test]
    fn a_single_source_listing_is_the_default_when_the_server_names_none() {
        let sources = "Source #0\n\tName: alsa_input.stub\n\tBase Volume: 6554 / 10% / -60.00 dB\n";

        assert_eq!(parse_base_volume(sources, None).as_deref(), Some("6554"));
    }

    // The server named no source and several are published: picking one would risk writing
    // another source's base, so there is no base and the caller writes the configured level.
    #[test]
    fn several_sources_with_no_name_have_no_base() {
        let sources = "Source #0\n\tName: alsa_output.stub.monitor\n\tBase Volume: 3277 / 5% / -66.00 dB\nSource #1\n\tName: alsa_input.stub\n\tBase Volume: 6554 / 10% / -60.00 dB\n";

        assert_eq!(parse_base_volume(sources, None), None);
    }

    // A name the listing does not publish is a name the tool does not substitute: even a single
    // source is left alone rather than written under a name the server did not confirm.
    #[test]
    fn a_name_the_listing_does_not_publish_has_no_base() {
        let sources = "Source #0\n\tName: alsa_input.stub\n\tBase Volume: 6554 / 10% / -60.00 dB\n";

        assert_eq!(parse_base_volume(sources, Some("alsa_input.other")), None);
    }
}
