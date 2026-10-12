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

The vocabulary lives in [`GLOSSARY.md`](GLOSSARY.md) and the decisions taken during design in
[`docs/adr/`](docs/adr/).

## How a dictation works

1. `start`, or `toggle` from idle, resolves the transcriber, asks it for its audio limit, claims the
   microphone, sets the default source to its base volume and unmutes it, and forks a session
   process.
2. The recorder captures the microphone, the encoder writes MP3 from the first frame, and the
   session publishes the input waveform for the pill to draw.
3. `stop`, or `toggle` while recording, stops the recorder; the encoder flushes its last frames.
4. The session runs the transcriber command on the audio file and shows `transcribing` while it
   runs. A cancel that lands during the transcription drops the text.
5. The session writes `transcript.txt` into the session directory, copies the text to both clipboard
   selections, waits for the offer to serve it, injects it into the focused window with `ydotool
   Shift+Insert`, and removes its runtime directory: the session stays on disk.

The tool is one Rust binary — the command line, the audio pipeline and the pill — and nothing runs
as a daemon: `start` forks a session process that owns the whole lifecycle, and nothing of the tool
outlives a dictation beyond the clipboard offer that carries the text.

A dictation is batch: the whole utterance is recorded, then transcribed once
([ADR-0019](docs/adr/0019-a-dictation-is-batch.md)).

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
  one the dictation still works, nothing is drawn on screen. GNOME/Mutter has no layer-shell, so the
  pill exits with an error there and the dictation runs on
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

Every desktop environment that binds a key to a command can drive the tool; example with i3/sway:

```text
bindsym $mod+backslash exec tiny-dictate toggle
bindsym $mod+Shift+backslash exec tiny-dictate cancel
```

`toggle` covers the common case — one key for start and stop. `cancel` needs its own key: a dictation
ended by accident must remain cancellable without being transcribed.

### keyd (optional)

A key remapper is useful when the shortcut is a chord the compositor cannot see, or a double tap. On
the reference setup the double-tap detection lives in `/etc/keyd/default.conf`, outside this
repository: a single tap of Super does nothing, a second tap within `oneshot_timeout` emits `F19`,
and a tap with Shift held emits `F24`. The compositor then binds those two plain function keys:

```text
bindsym F19 exec systemd-cat -t tiny-dictate tiny-dictate toggle
bindsym F24 exec systemd-cat -t tiny-dictate tiny-dictate cancel
```

`systemd-cat -t tiny-dictate` tags the tool's stderr in the journal, which is where the log reads as a
timeline ([ADR-0008](docs/adr/0008-the-log.md)); an untagged line is what `tiny-dictate` writes when
it is run directly.

Two details are worth keeping in mind when choosing keys:

- `F24`, not `F20`: the evdev keymap maps the twentieth function key to `XF86AudioMicMute`, so a
  binding on `F20` never matches, and a binding on `XF86AudioMicMute` mutes the microphone instead.
- `F24`, not `Shift+F19`: the compositor only sees the Shift it has been told about, so a Shift
  released a few milliseconds before the tap turns a cancel into a toggle. The key is chosen by the
  keyd layer, which is where the Shift is known.

### Configure the tool

`$XDG_CONFIG_HOME/tiny-dictate/config.toml` (`~/.config/tiny-dictate/config.toml` by default) is
optional, and every key has a default:

```toml
keep = 10                # the past sessions retained, oldest beyond it pruned after a dictation
color = "#e4e4e7"        # the pill's waveform
clipboard_seconds = 60   # how long the transcript stays offered on the clipboard, 0 means no expiry
```

A missing, unreadable or malformed file, or an unknown key, falls back to the defaults and never
blocks a dictation. The file holds no secret: a transcriber's key comes from the environment only
([ADR-0010](docs/adr/0010-configuration.md)).

After a dictation whose text was pasted, the sessions beyond `keep` are removed, oldest first. A
dictation that failed or was cancelled prunes nothing, a crashed run prunes nothing, and the session
being recorded is never removed ([ADR-0005](docs/adr/0005-prune.md)).

### Waybar module (optional)

`contrib/waybar/tiny-dictate-module.jsonc` puts the dictation state in the bar: `tiny-dictate status`
prints `idle` or `working`, and the module maps either to an icon, a tooltip and a CSS class. A click
runs `tiny-dictate toggle`. The module runs the command rather than reading the state file, which is
internal to the tool ([ADR-0013](docs/adr/0013-the-bar-goes-through-the-cli.md)).

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
on `PATH`. An unresolvable command is reported before any recording starts. The command may declare
the largest audio file it accepts by answering `--print-audio-limit` with a byte count; the probe is
asked once per dictation before the microphone is opened, and that count becomes a maximum duration
at the encoder's 64 kbit/s, at which the recording stops as if the shortcut had been pressed
([ADR-0006](docs/adr/0006-the-limit-and-the-bar.md)). A transcriber that declares nothing leaves the
dictation unbounded.

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

Tuning what a transcriber makes of your audio is done by rewriting its output — the transcript is
plain text, and a wrapper does that in three lines.

### When it hallucinates on silence

Whisper answers `"."` or `"♪♪"` to audio that holds no speech, and no single signal settles whether a
segment is real: the same audio gives `no_speech_prob` of 0.0001 on one run and 0.993 on the next. The
reference setup drops a segment when any of three heuristics fires —

- `no_speech_prob >= 0.6`, Whisper's own default,
- `avg_logprob <= -1.0`, the one that catches non-speech here,
- `compression_ratio >= 2.4`, repetition loops,

— then drops a transcript that holds no word at all. A recording under half a second, or one whose
peak volume is under -45 dB, is not sent: a muted microphone reads as `-inf`. `verbose_json` is the
response format that reports `no_speech_prob`, and it costs the request, which Groq bills with a
ten-second minimum. The robust answer is a real voice activity detector on the audio before the
request.

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

Every dictation is kept as a session: the recording, the transcript,
the provider's raw response and the metadata, one directory per dictation under
`$XDG_STATE_HOME/tiny-dictate/sessions`, named by the local time it started
([ADR-0003](docs/adr/0003-sessions-on-disk.md)). `session list` prints them newest first, `session
browse` reads them on a terminal (bare `session` does one or the other, depending on whether its
stdout is a terminal), `session show <id|last>` reads one in full (its fields, its path and its
transcript), `session copy` puts one's transcript on the clipboard, transcribing it first when it
has none, `session transcribe` runs the transcriber again on one (copying the text to the
clipboard, or printing it with `--print`), and `session delete` is the only command that destroys
one. All of them run on a machine without the encoder or the clipboard tools. `session list` and
`session show` take `--output text|json`: `text` is the default and `json` prints the same session
as one document for a script.
