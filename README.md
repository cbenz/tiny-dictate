# tiny-dictate

Minimal desktop voice dictation tool: no daemon, no GUI, just keyboard shortcuts and notifications.

Opinionated workflow: press a keyboard shortcut to start recording, press it again to stop and
transcribe (or cancel with another keyboard shortcut).

Built on the following tools:

- audio recording with `arecord`, encoded to MP3 on-the-fly with `lame`
- transcription by a command you provide (see [Transcribers](#transcribers))
- result pasted into the active window via clipboard + `ydotool Shift+Insert`
- status notifications with `dunstify`

## Installation

### Dependencies

- `arecord` (alsa-utils)
- `curl`
- `lame`
- [dunst](https://dunst-project.org/)
- [wl-clipboard](https://github.com/bugaevc/wl-clipboard)
- [ydotool](https://github.com/ReimuNotMoe/ydotool)

### Install the tool and a transcriber

```bash
install -m 755 tiny-dictate ~/.local/bin/
install -m 755 transcribers/groq ~/.local/bin/tiny-dictate-transcribe
```

`tiny-dictate-transcribe` is the default command name, so a transcriber installed under that name
is used without any further configuration.

### Configure keyboard shortcuts

Example with i3/sway:

```text
bindsym $mod+backslash exec tiny-dictate toggle
bindsym $mod+Shift+backslash exec tiny-dictate cancel
```

## Transcribers

`tiny-dictate` has no transcription backend of its own: it calls a command you provide. That
command:

- receives the recorded audio file as `$1`
- writes the transcribed text to stdout
- exits non-zero on failure, in which case its stderr is shown to the user

It is resolved with `command -v`: `$TINY_DICTATE_TRANSCRIBE` if set, else `tiny-dictate-transcribe`
on `PATH`. An unresolvable command is reported before any recording starts.

The transcribers shipped with the tool live in `transcribers/`. `transcribers/groq` is the
reference implementation (Groq Whisper):

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

### Keys from a password manager

A transcriber is a plain script, so it can fetch its credentials wherever you keep them, as long as
the lookup does not prompt — the tool runs from a keyboard shortcut, with no terminal. Wrap the
reference transcriber:

```bash
#!/usr/bin/env bash
# ~/.local/bin/tiny-dictate-transcribe
GROQ_API_KEY="$(my-key-helper 'Groq API key')" exec ~/path/to/tiny-dictate/transcribers/groq "$@"
```

`my-key-helper` is whatever prints the key: `keepassxc-cli show -s -a Password <database> <entry>`,
a `secret-tool` lookup, `pass`, `rbw`, your own script. Note that a shell function from your
interactive shell (say a zsh function in your `.zshrc`) is not available to a script: either make
it a script too, or call it explicitly as `zsh -ic 'my-key-helper "Groq API key"'`.

## Usage

```text
Usage: tiny-dictate <command>

Commands:
  start   Start recording
  stop    Stop recording and transcribe
  cancel  Cancel recording
  toggle  Start if idle, stop if recording
  status  Show status (idle or working)
```

## Tests

```bash
tests/run.sh
```

The test suite stubs the recorder, the encoder, the transcription command, the notifier and the
keyboard injector: it needs no microphone, no notification daemon and no network.
