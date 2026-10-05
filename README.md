# tiny-dictate

Minimal desktop voice dictation tool: no daemon, just keyboard shortcuts and a pill on screen.

Opinionated workflow: press a keyboard shortcut to start recording, press it again to stop and
transcribe (or cancel with another keyboard shortcut).

Built on the following tools:

- audio recording with `arecord`, encoded to MP3 on-the-fly with `lame`
- transcription by a command you provide (see [Transcribers](#transcribers))
- live status drawn as a layer-shell pill by the presenter (see [Presenter](#presenter))
- result pasted into the active window via clipboard + `ydotool Shift+Insert`
- failures notified with `notify-send`, the cancellation shown as a verdict on the pill

## Installation

### Dependencies

- `arecord` (alsa-utils)
- `curl`
- `lame`
- `notify-send` (libnotify)
- [wl-clipboard](https://github.com/bugaevc/wl-clipboard)
- [ydotool](https://github.com/ReimuNotMoe/ydotool)
- a compositor with `wlr-layer-shell` support (sway, Hyprland, niri, KDE) for the presenter
- for the presenter: `python-gobject`, `gtk4`, [gtk4-layer-shell](https://github.com/wmww/gtk4-layer-shell)

A notification daemon (dunst, mako, your desktop's) displays the failures and the cancellation.

### Install the tool, a transcriber and a presenter

```bash
install -m 755 tiny-dictate ~/.local/bin/
install -m 755 transcribers/groq ~/.local/bin/tiny-dictate-transcribe
install -m 755 presenters/layer-shell ~/.local/bin/tiny-dictate-present
```

`tiny-dictate-transcribe` and `tiny-dictate-present` are the default command names, so both are used
without any further configuration once installed under those names.

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

### Hallucinations on silence

Whisper was trained on subtitles, so on non-speech audio it completes with whatever it finds
likely: "Thank you.", "Sous-titres réalisés par…", or any plausible sentence in the language it
detected. Measured on this setup, the signals do not behave as one would hope:

| recording | mean volume | peak volume | no_speech_prob | avg_logprob | transcription |
|-----------|-------------|-------------|----------------|-------------|---------------|
| digital silence | -91.0 dB | -91.0 dB | — | — | nothing |
| 440 Hz tone | -21.5 dB | -18.5 dB | 0.0001 | -1.03 | `d` |
| the same tone, another run | | | 0.993 | — | `.` |
| a room with people talking | | | 0.001 | -0.27 | coherent sentences |

The same audio gives `no speech` on one run and `certainly speech` on the next, so **no single
signal settles this**. Three things, in this order, cover most of it.

A duration floor, for a keystroke that captured nothing:

```bash
duration="$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$1")"
if awk -v d="$duration" 'BEGIN { exit !(d < 0.5) }'; then
    exit 0
fi
```

Digital silence, which is what a muted microphone produces. A room's noise is louder (measured:
-18.6 dB peak for a room containing a conversation), so treat the threshold as something to
calibrate on your own room, by recording three seconds of it and reading the `max_volume` line:

```bash
peak="$(ffmpeg -hide_banner -nostats -i "$1" -af volumedetect -f null - 2>&1 |
    sed -n 's/.*max_volume: \([^ ]*\) dB.*/\1/p' | tail -1)"
[ "$peak" = "-inf" ] && peak=-99        # ffmpeg reports -inf for digital silence
if awk -v p="$peak" 'BEGIN { exit !(p < -45) }'; then
    exit 0
fi
```

And then Whisper's own three suspicion heuristics, which apply to the segments of a
`response_format=verbose_json` answer:

```bash
GROQ_RESPONSE_FORMAT=verbose_json transcribers/groq "$1" | my-segment-filter
# drop a segment when any of these fires, and say which:
#   no_speech_prob       >= 0.6      Whisper's default
#   avg_logprob          <= -1.0     the one that catches non-speech here
#   compression_ratio    >= 2.4      repetition loops
# then drop a transcript that holds no word at all: Whisper answers "." or "♪♪" to non-speech
```

Dropping only the flagged segments keeps the speech around a long pause, and printing the reason
on stderr is what makes the thresholds tunable from the journal. This is still a heuristic: a
mumble can be dropped, and loud non-speech can get through. The robust answer is a real voice
activity detector on the audio before the request.

`sox file -n stat` reports the same volumes as a 0..1 amplitude if you prefer it to ffmpeg.

Groq also reports `no_speech_prob` per segment with `response_format=verbose_json`, if you prefer
to let the API decide (it costs the request, and Groq bills a 10 second minimum).

### A vocabulary

Groq's transcription request takes a `prompt` that "guides the model's style or specifies how to
spell unfamiliar words". It rides along in the same request, so it costs nothing extra, and it
belongs to the transcriber, not to the core:

```bash
export GROQ_LANGUAGE=fr
export GROQ_PROMPT="DBnomics, herdr, keyd, Vicinae"      # or GROQ_PROMPT_FILE
```

It is a bias, not a rule: it can fail, and Whisper sometimes echoes the prompt in the transcript
instead of transcribing. That is the only reason the list has to stay short — Groq caps the prompt
at 224 tokens, so a dozen terms, not a glossary. In this setup the list lives in
`~/.config/tiny-dictate/words`, one term per line, managed by `tiny-dictate-words`, and the
transcriber joins it with commas before each request.

Rewriting the transcript afterwards is the deterministic alternative: a table of `as the
transcriber hears it` → `what you want`, applied to the text. It guarantees the spelling, at the
price of anticipating every mistake, and it cannot repair a word the model heard as something
unrelated. Worth keeping for the few terms the prompt keeps missing.

## Presenter

The pill that shows what `tiny-dictate` is doing is a command you provide, on the same terms as the
transcriber. It receives:

- `$1`: the path of a **state file**, holding one word: `recording`, `transcribing`, `cancelled`,
  or `stop`
- `$2`: the pid of the session that owns the pill

It leaves when the state file disappears, when it reads `stop`, or when that pid is gone. Those
three exits are what keep a pill from surviving its dictation, so a presenter has to follow all of
them — a screen that shows "Recording" over a dead session is worse than no pill at all.

It must never take keyboard focus: the paste that ends a dictation goes to the focused window, so a
focusable pill would receive the text itself.

The presenter shipped with the tool, `presenters/layer-shell`, draws a pill anchored to the bottom
of the screen with GTK4 and gtk4-layer-shell. It needs a compositor with `wlr-layer-shell` support
(sway, Hyprland, niri, KDE) and exits with an error on GNOME/Mutter, where the dictation still works
but nothing is shown.

Failures are not the pill's business: they are desktop notifications sent with `notify-send`, once
each, never replaced. There is nothing to keep in sync, so no notification id and no `dunstify`. The
cancellation is, because it is a verdict about the dictation itself: `cancel` writes the `cancelled`
state, the pill shows it for a second, and the session goes down without transcribing anything.

Plug in anything else — a notification-based presenter, a bar module, your own script — by pointing
`TINY_DICTATE_PRESENT` at it. The three lines above are the whole interface: nothing in the core
knows what a pill is.

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

The test suite stubs the recorder, the encoder, the transcription command, the presenter, the
notifier and the keyboard injector: it needs no microphone, no notification daemon, no layer-shell
surface and no network.
