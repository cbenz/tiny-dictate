# tiny-dictate

Minimal desktop voice dictation tool: no daemon, just keyboard shortcuts and a pill on screen.

Opinionated workflow: press a keyboard shortcut to start recording, press it again to stop and
transcribe (or cancel with another keyboard shortcut).

Built on the following tools:

- audio recording with `arecord`, encoded to MP3 on-the-fly with `lame`
- transcription by a command you provide (see [Transcribers](#transcribers))
- the pill, the live status and the input waveform, drawn by the tool itself
- result pasted into the active window via clipboard + `ydotool Shift+Insert`
- failures notified with `notify-send`, the cancellation shown as a verdict on the pill

How it works, and the decisions behind it, live in [`docs/SPECS.md`](docs/SPECS.md),
[`GLOSSARY.md`](GLOSSARY.md) and [`docs/adr/`](docs/adr/).

## Installation

The tool is a Rust binary. It links GTK4 and [gtk4-layer-shell](https://github.com/wmww/gtk4-layer-shell)
for its pill, so their development packages, a Rust toolchain and a C compiler must be installed to
build it; there is nothing to install beside it at runtime.

### Build

```bash
cargo build --release
```

The GUI toolkit is linked into every invocation, `tiny-dictate status` included. That is deliberate,
not a packaging mistake.

### Runtime dependencies

- `arecord` (alsa-utils)
- `lame`
- [ydotool](https://github.com/ReimuNotMoe/ydotool)
- `notify-send` (libnotify)
- [wl-clipboard](https://github.com/bugaevc/wl-clipboard)
- a compositor with `wlr-layer-shell` support (sway, Hyprland, niri, KDE), for the pill only: without
  one the dictation still works, nothing is drawn on screen
- `curl`, for the reference transcriber

A notification daemon (dunst, mako, your desktop's) displays the failures.

### Install the binary and a transcriber

```bash
install -m 755 target/release/tiny-dictate ~/.local/bin/
install -m 755 transcribers/groq ~/.local/bin/tiny-dictate-transcribe
```

The pill is built into the binary. The only thing installed beside it is a transcriber, and
`tiny-dictate-transcribe` is its default command name, so the reference one needs no further
configuration once installed under that name.

### Configure keyboard shortcuts

Example with i3/sway:

```text
bindsym $mod+backslash exec tiny-dictate toggle
bindsym $mod+Shift+backslash exec tiny-dictate cancel
```

Bindings through a key remapper such as keyd work the same way. The
[automation page](docs/features/automation.md) describes the reference setup.

### Waybar module (optional)

`contrib/waybar/tiny-dictate-module.jsonc` puts the dictation state in the bar: the module reads the
session state file, maps the word it holds to an icon, a tooltip and a CSS class, and treats a
missing file as idle. A click runs `tiny-dictate toggle`.

```bash
cp contrib/waybar/tiny-dictate-module.jsonc ~/.config/waybar/
```

Then include the fragment and add the module to a bar:

```jsonc
{
  "include": ["tiny-dictate-module.jsonc"],
  "modules-right": ["custom/tiny-dictate"]
}
```

## Transcribers

`tiny-dictate` has no transcription backend of its own: it calls a command you provide. That command
receives the recorded audio file as `$1`, writes the transcribed text to stdout, and exits non-zero
on failure, in which case its stderr is shown to the user.

It is resolved with `command -v`: `$TINY_DICTATE_TRANSCRIBE` if set, else `tiny-dictate-transcribe`
on `PATH`. An unresolvable command is reported before any recording starts. The full contract is in
[`docs/features/transcriber.md`](docs/features/transcriber.md).

`transcribers/groq` is the reference implementation (Groq Whisper):

```bash
export GROQ_API_KEY=...            # create one at https://console.groq.com/keys
export GROQ_WHISPER_MODEL=...      # optional, defaults to whisper-large-v3;
                                   # whisper-large-v3-turbo is faster and cheaper
```

Plug in anything else — another service, a local `whisper.cpp`, your own script — by pointing
`TINY_DICTATE_TRANSCRIBE` at it:

```bash
export TINY_DICTATE_TRANSCRIBE=~/.local/bin/my-transcriber
```

Tuning what a transcriber makes of your audio (hallucinations on silence, a vocabulary) is on the
[transcriber page](docs/features/transcriber.md).

### Keys from a password manager

A transcriber is a plain script, so it can fetch its credentials wherever you keep them, as long as
the lookup does not prompt — the tool runs from a keyboard shortcut, with no terminal. Wrap the
reference transcriber:

```bash
#!/usr/bin/env bash
# ~/.local/bin/tiny-dictate-transcribe
GROQ_API_KEY="$(my-key-helper 'Groq API key')" exec ~/path/to/tiny-dictate/transcribers/groq "$@"
```

`my-key-helper` is whatever prints the key without prompting: a `secret-tool lookup Title 'Groq API
key'` (KeePassXC's Secret Service integration, as long as the database is unlocked), a
`keepassxc-cli show -s -a Password <database> <entry>`, `pass`, `rbw`, your own script. A shell
function from your interactive shell is not available to a script: make it a script, or call it
explicitly as `zsh -ic 'my-key-helper "Groq API key"'`.

## Usage

```text
Usage: tiny-dictate <command>

Commands:
  start                           Start recording
  stop                            Stop recording and transcribe
  cancel                          Cancel recording
  toggle                          Start if idle, stop if recording
  status                          Show status (idle or working)
  history                         List the kept sessions, newest first
  transcribe <id|last> [--print]  Transcribe a session again, to the clipboard or to stdout
  session delete <id|last>        Remove exactly one session
```

Every dictation is kept as a [session](docs/features/sessions.md): the recording, the transcript,
the provider's raw response and the metadata. `history` lists them newest first, `transcribe` runs
the transcriber again on one (copying the text to the clipboard, or printing it with `--print`), and
`session delete` is the only command that destroys one. Both `history` and `transcribe` run on a
machine without the encoder or the clipboard tools.
