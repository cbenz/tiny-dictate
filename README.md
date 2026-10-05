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

`my-key-helper` is whatever prints the key without prompting: a `secret-tool lookup Title 'Groq API
key'` (KeePassXC's Secret Service integration, as long as the database is unlocked), a
`keepassxc-cli show -s -a Password <database> <entry>`, `pass`, `rbw`, your own script. A shell
function from your interactive shell is not available to a script: make it a script, or call it
explicitly as `zsh -ic 'my-key-helper "Groq API key"'`.

## Tuning a transcriber

Everything below is outside the tool: a transcriber is a command, so a script decides what the
recorded audio becomes before the text reaches your keyboard.

### Skipping silence

Whisper was trained on subtitles, so on non-speech audio it completes with its most frequent
training lines: "Thank you.", "Sous-titres réalisés par…". No parameter removes that; the fix is
to not transcribe, which also avoids the API call:

```bash
#!/usr/bin/env bash
# ~/.local/bin/tiny-dictate-transcribe
if ! peak="$(ffmpeg -hide_banner -nostats -i "$1" -af volumedetect -f null - 2>&1 |
        sed -n 's/.*max_volume: \(-*[0-9.]*\) dB.*/\1/p' | tail -1)"; then
    :
fi
if [ -n "$peak" ] && awk -v p="$peak" 'BEGIN { exit !(p < -45) }'; then
    exit 0          # silence: no text on stdout, so nothing is inserted
fi
```

Digital silence sits around -91 dB; speech peaks well above -20 dB. A duration floor catches the
accidental double-tap:

```bash
duration="$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$1")"
awk -v d="$duration" 'BEGIN { exit !(d < 0.5) }' && exit 0
```

Groq also reports `no_speech_prob` per segment with `response_format=verbose_json`, if you prefer
to let the API decide (it costs the request, and Groq bills a 10 second minimum).

### A personal dictionary

Two mechanisms answer two different problems.

A **prompt** biases the decoder towards spellings it is about to guess wrong. Groq accepts one,
capped at 224 tokens — a handful of words, not a glossary, because Whisper sometimes echoes the
prompt instead of transcribing:

```bash
export GROQ_LANGUAGE=fr
export GROQ_PROMPT_FILE=~/.config/tiny-dictate/hints.txt
```

**Rewriting the transcript** is the deterministic half, and it needs no backend support. Have a
look at how `tiny-dictate-dictionary` and `tiny-dictate-fix-word` are wired in this setup if you
want the same: a TSV of `as the transcriber hears it` → `what you want`, applied word by word,
plus a launcher action that adds an entry from the text you just selected and fixes it on the
spot.

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
