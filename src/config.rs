//! The configuration file: `$XDG_CONFIG_HOME/tiny-dictate/config.toml`.
//!
//! Optional, and every key has a default, so a fresh clone works without one. The file holds no
//! secret: the transcriber's key comes from the environment only
//! ([ADR-0010](../docs/adr/0010-configuration.md)). A file that is missing, unreadable, malformed,
//! or that carries a key this version does not know falls back to the defaults, silently: a
//! configuration file must never be the reason a dictation is refused.

use std::fs;
use std::path::PathBuf;

use crate::xdg;

/// The pill's default colour: zinc-200, the softer white a theme's foreground usually is.
pub const DEFAULT_COLOR: &str = "#e4e4e7";

/// What a configuration file can say. `Config::default()` is what a missing file means.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The number of past sessions retained.
    pub keep: usize,
    /// The pill's colour: the waveform and the recording spinner.
    pub color: String,
    /// How long the transcript stays offered on the clipboard, in seconds; `0` means no expiry.
    /// Consumed by the clipboard lease ([ADR-0007](../docs/adr/0007-the-clipboard-lease.md)).
    pub clipboard_seconds: u64,
    /// The percentage the microphone is set to when the audio server publishes no readable base
    /// volume ([ADR-0014](../docs/adr/0014-the-microphone-at-its-base-volume.md)). Never a target:
    /// the base the server publishes is what a dictation writes when there is one.
    pub input_level_pct: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keep: 10,
            color: DEFAULT_COLOR.to_owned(),
            clipboard_seconds: 60,
            input_level_pct: 10,
        }
    }
}

/// Read the configuration file, falling back to the defaults for everything a file does not say.
pub fn load() -> Config {
    path()
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// The configuration file: `$XDG_CONFIG_HOME/tiny-dictate/config.toml`, `$HOME/.config` by
/// default. `None` when neither variable names a directory.
pub fn path() -> Option<PathBuf> {
    xdg::config().map(|base| base.join("tiny-dictate/config.toml"))
}

/// Read the flat keys of a TOML file. A line that is not `key = value`, a key this version does
/// not know, a table header and a value that does not parse are all ignored the same way: a typo
/// must not cost the user a dictation, and the defaults are always a working answer.
fn parse(text: &str) -> Config {
    let mut config = Config::default();

    for line in text.lines() {
        let line = strip_comment(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();

        match key.trim() {
            "keep" => config.keep = value.parse().unwrap_or(config.keep),
            "clipboard_seconds" => {
                config.clipboard_seconds = value.parse().unwrap_or(config.clipboard_seconds);
            }
            "input_level_pct" => {
                config.input_level_pct = value.parse().unwrap_or(config.input_level_pct);
            }
            "color" => {
                let color = unquote(value);
                if is_color(color) {
                    config.color = color.to_owned();
                }
            }
            _ => {}
        }
    }

    config
}

/// A line without its trailing comment; a `#` inside a quoted value is part of the value, which
/// is what lets `color` be written as `"#e4e4e7"`.
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;

    for (index, character) in line.char_indices() {
        match character {
            '"' | '\'' if quote == Some(character) => quote = None,
            '"' | '\'' if quote.is_none() => quote = Some(character),
            '#' if quote.is_none() => return &line[..index],
            _ => {}
        }
    }

    line
}

/// The contents of a quoted value, or the value as it stands.
fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..value.len() - 1];
        }
    }

    value
}

/// `#rrggbb`, the only form the pill reads: anything else keeps the default rather than handing
/// the presenter a colour it cannot parse.
fn is_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}
