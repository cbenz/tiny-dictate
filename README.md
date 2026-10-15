# tiny-dictate

Minimal voice dictation for Linux: press a shortcut, speak, press it again, and the text lands in the
focused window. No daemon, nothing running between dictations.

## Features

- records the microphone and encodes MP3 on the fly;
- transcribes by calling **any command you provide**, so you can plug in a cloud API or a local
  model — the one shipped here is Groq Whisper;
- shows a pill while it runs: the input waveform, a spinner while it transcribes, the verdict on a
  cancel;
- pastes the transcript into the focused window;
- notifies failures;
- keeps every dictation on disk and lets you list, show, copy, re-transcribe and delete them.

## Install

tiny-dictate is a single binary. Download the archive for your architecture from the
[releases page](https://github.com/cbenz/tiny-dictate/releases), unpack it, and put `tiny-dictate`
and `tiny-dictate-transcribe` on your `PATH`.

At runtime it needs:

- `arecord` — recording (alsa-utils)
- `lame` — MP3 encoding
- `ydotool` — pasting
- `notify-send` — failure notifications (libnotify)
- `wl-clipboard` — the clipboard
- `curl` — the reference transcriber
- the `gtk4` and `gtk4-layer-shell` shared libraries — the pill
- a compositor with `wlr-layer-shell`, for the pill only: sway, Hyprland, niri, KDE. Without one a
  dictation still works, nothing is drawn on screen. GNOME/Mutter has no layer-shell.

To build it yourself instead, see [docs/development.md](docs/development.md).

## Set it up

### Bind a shortcut

Any desktop environment that binds a key to a command can drive it; example with sway/i3:

```text
bindsym $mod+backslash exec tiny-dictate toggle
bindsym $mod+Shift+backslash exec tiny-dictate cancel
```

`toggle` starts and stops; `cancel` needs its own key, so that a dictation ended by accident can be
cancelled without being transcribed.

### Choose a transcriber

`tiny-dictate` has no transcription backend of its own: it calls a command you provide. That command
receives the audio file as `$1`, writes the transcribed text on stdout, and exits non-zero on
failure, in which case its stderr is shown to you. It is resolved with `command -v`:
`$TINY_DICTATE_TRANSCRIBE` if set, otherwise `tiny-dictate-transcribe` on `PATH`.

The transcriber shipped here, `transcribers/groq`, calls Groq Whisper:

```bash
export GROQ_API_KEY=...            # create one at https://console.groq.com/keys
export GROQ_WHISPER_MODEL=...      # optional, defaults to whisper-large-v3
```

Plug in anything else — another service, a local `whisper.cpp`, your own script — by pointing
`TINY_DICTATE_TRANSCRIBE` at it:

```bash
export TINY_DICTATE_TRANSCRIBE=~/.local/bin/my-transcriber
```

A transcriber can declare the largest audio file it accepts by answering `--print-audio-limit` with a
byte count; the recording then stops early enough that the file handed to it stays under that limit.
A transcriber that declares nothing is unbounded.

Whisper sometimes answers `"."` to audio that holds no speech; the reference transcriber drops the
segments it believes are not speech, and a transcript that holds no word at all.

A transcriber is a plain script, so it can fetch its credentials wherever you keep them — it runs
from a shortcut, with no terminal, so the lookup must not prompt. Wrap the reference one:

```bash
#!/usr/bin/env bash
# ~/.local/bin/tiny-dictate-transcribe
GROQ_API_KEY="$(secret-tool lookup Title 'Groq API key')" exec ~/path/to/transcribers/groq "$@"
```

### Configure

`$XDG_CONFIG_HOME/tiny-dictate/config.toml` (`~/.config/tiny-dictate/config.toml` by default) is
optional, and every key has a default:

```toml
keep = 10                # the past sessions retained, older ones pruned after a dictation
color = "#e4e4e7"        # the pill's waveform
clipboard_seconds = 60   # how long the transcript stays offered on the clipboard, 0 means no expiry
```

A missing, unreadable or malformed file, or an unknown key, falls back to the defaults and never
blocks a dictation. The file holds no secret.

## Usage

```text
Usage: tiny-dictate <command>

Commands:
  start                                          Start recording
  stop                                           Stop recording and transcribe
  cancel                                         Cancel recording
  toggle                                         Start if idle, stop if recording
  status                                         Show status (idle or working)
  session                                        List the kept sessions, or browse them on a terminal
  session list [--output text|json]              List the kept sessions, newest first
  session browse                                 Browse the kept sessions on a terminal
  session show <id|last> [--output text|json]    Show one session: its fields, its path and its transcript
  session copy <id|last>                         Copy a session's transcript to the clipboard
  session transcribe <id|last> [--print]         Transcribe a session again, to the clipboard or to stdout
  session delete <id|last>                       Remove exactly one session
```

Every dictation is kept as a session under `$XDG_STATE_HOME/tiny-dictate/sessions`: the recording,
the transcript, the provider's raw response and the metadata, one directory per dictation. `session`
lists them or opens a browser on a terminal; `session show` reads one in full; `session copy` puts
its transcript on the clipboard, transcribing it first when it has none; `session transcribe` runs
the transcriber again; `session delete` is the only command that destroys one.

## Optional integrations

### Waybar

`contrib/waybar/tiny-dictate-module.jsonc` shows the dictation state in the bar, and a click toggles
it:

```bash
cp contrib/waybar/tiny-dictate-module.jsonc ~/.config/waybar/
```

Then include the fragment and add `custom/tiny-dictate` to a bar.

### keyd

When the shortcut is a chord your compositor cannot see, or a double tap, a key remapper can turn it
into a plain function key. On the reference setup a double tap of Super emits `F19` and a tap with
Shift held emits `F24`, bound as `tiny-dictate toggle` and `tiny-dictate cancel`. Prefer `F24` over
`F20` (the evdev keymap maps `F20` to `XF86AudioMicMute`) and over `Shift+F19` (the compositor cannot
see a Shift the keyd layer has not emitted).

## Contributing

[Building from source, the tests and the pill preview](docs/development.md), the
[glossary](GLOSSARY.md), and the [design decisions](docs/adr/).
