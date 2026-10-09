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

This is the production path: copy the built binary.

```bash
install -m 755 target/release/tiny-dictate ~/.local/bin/
install -m 755 transcribers/groq ~/.local/bin/tiny-dictate-transcribe
```

The pill is built into the binary. The only thing installed beside it is a transcriber, and
`tiny-dictate-transcribe` is its default command name, so the reference one needs no further
configuration once installed under that name.

### Dev install (reversible)

To make the F19/F24 shortcuts and the bar drive this checkout instead of the main one, without
touching the compositor's configuration:

```bash
./install.sh          # build the release binary and link it onto PATH
./install.sh restore  # put the main checkout's install back
```

`install.sh` links `~/.local/bin/tiny-dictate` at this checkout's `target/release/tiny-dictate`, so
a later `cargo build --release` is picked up without re-running it, and removes the
`tiny-dictate-present` symlink the bash implementation installed — the binary draws the pill itself.
It also links `~/.local/bin/tiny-dictate-transcribe` at this checkout's `transcribers/groq`, keeping
whatever held that name as `tiny-dictate-transcribe.bak` (once, so a personal wrapper is never
lost): the reference transcriber answers the audio-limit probe the progress bar is drawn from, and
reads `GROQ_API_KEY` from the environment. It refuses to replace `~/.local/bin/tiny-dictate` when
that is not a symlink, since `restore` can only undo a symlink and a copy left by the production
install is not promised back. `restore` rebuilds the two symlinks from the main worktree and puts
the transcriber backup back when there is one.

Missing build dependencies stop the install before the build, each named with its Arch package;
missing runtime commands are only listed.

### Configure keyboard shortcuts

Example with i3/sway:

```text
bindsym $mod+backslash exec tiny-dictate toggle
bindsym $mod+Shift+backslash exec tiny-dictate cancel
```

Bindings through a key remapper such as keyd work the same way. The
[automation page](docs/features/automation.md) describes the reference setup.

### Configure the tool

`$XDG_CONFIG_HOME/tiny-dictate/config.toml` (`~/.config/tiny-dictate/config.toml` by default) is
optional, and every key has a default:

```toml
keep = 10                # the past sessions retained, oldest beyond it pruned after a dictation
color = "#e4e4e7"        # the pill's waveform
clipboard_seconds = 60   # how long the transcript stays offered on the clipboard, 0 means no expiry
```

A missing, unreadable or malformed file, or an unknown key, falls back to the defaults and never
blocks a dictation. The file holds no secret: a transcriber's key comes from the environment only.
The full page is [`docs/features/configuration.md`](docs/features/configuration.md).

### Waybar module (optional)

`contrib/waybar/tiny-dictate-module.jsonc` puts the dictation state in the bar: `tiny-dictate status`
prints `idle` or `working`, and the module maps either to an icon, a tooltip and a CSS class. A click
runs `tiny-dictate toggle`.

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
  start                                   Start recording
  stop                                    Stop recording and transcribe
  cancel                                  Cancel recording
  toggle                                  Start if idle, stop if recording
  status                                  Show status (idle or working)
  session                                 List the kept sessions, or browse them on a terminal
  session list                            List the kept sessions, newest first
  session browse                          Browse the kept sessions on a terminal
  session transcribe <id|last> [--print]  Transcribe a session again, to the clipboard or to stdout
  session delete <id|last>                Remove exactly one session
```

Every dictation is kept as a [session](docs/features/sessions.md): the recording, the transcript,
the provider's raw response and the metadata. `session list` prints them newest first, `session
browse` reads them on a terminal (bare `session` does one or the other, depending on whether its
stdout is a terminal), `session transcribe` runs the transcriber again on one (copying the text to
the clipboard, or printing it with `--print`), and `session delete` is the only command that
destroys one. All of them run on a machine without the encoder or the clipboard tools.
