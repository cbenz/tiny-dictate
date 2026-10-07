# The transcriber command

## The contract

The core has no transcription backend of its own: transcription is delegated to a **command provided
by the user**. It is the only mandatory plugin — the tool cannot work without one.

The command:

- receives the recorded audio file as `$1`
- receives the session directory as `$2`, where it may leave the provider's own answer as
  `transcript.raw.json`; a transcriber that writes nothing there is still a transcriber
- writes the transcribed text to stdout
- exits non-zero on failure, in which case its stderr is shown to the user as a single truncated line
  (newlines flattened, at most 180 characters)

Any command can be plugged in: another service, a local `whisper.cpp`, a user script, in any
language. A transcriber is a plain executable, so it can fetch its credentials wherever the user
keeps them (password manager, secrets store), as long as nothing prompts: the tool runs from a
keyboard shortcut, with no terminal.

The plugin converts audio to text and nothing else. The core owns the status channel and the keyboard
injection, so its code path does not depend on the chosen backend.

It is resolved with `command -v` on `$TINY_DICTATE_TRANSCRIBE` when set, else on
`tiny-dictate-transcribe` on `PATH`. An unresolvable command is reported to the user before any
recording starts.

## The audio limit

A transcriber that caps the audio it accepts declares that cap, so a dictation can never run past it.
It answers `--print-audio-limit` with the largest audio file it accepts, in **bytes**, on stdout and
exits 0. Anything else — an unsupported flag, empty output, an unparsable number, a non-zero status —
means **no limit**.

The probe is asked once per dictation, before the microphone is opened, and every failure is read as
"no limit" rather than an error: a transcriber that declares nothing is one the user dictated to
before this existed, and gets no ceiling they did not ask for.

The byte count becomes a maximum duration from the encoder's bitrate: the encoder runs
`lame … -b 64`, i.e. 64 kbit/s, i.e. 8000 bytes per second, so the limit in seconds is `bytes / 8000`.
At that duration the recording stops exactly as if the shortcut had been pressed — the text is
transcribed and pasted, never dropped — and one line on stderr says the limit is why. The limit is
passed to the pill, which draws the progress bar from its own monotonic clock; with no limit there is
no bar ([ADR-0006](../adr/0006-the-limit-and-the-bar.md)).

## The reference implementation

`transcribers/groq` is the reference transcriber (Groq Whisper), shipped with the tool and installed
under the default name `tiny-dictate-transcribe`. It declares `--print-audio-limit` as 25 000 000
bytes, Groq's free-tier upload ceiling. It reads its configuration from the environment:

- `GROQ_API_KEY` — required, created at <https://console.groq.com/keys>
- `GROQ_WHISPER_MODEL` — defaults to `whisper-large-v3`; `whisper-large-v3-turbo` is faster and
  cheaper
- `GROQ_LANGUAGE` — ISO-639-1 code of the spoken language (e.g. `fr`), improves accuracy and latency
- `GROQ_PROMPT` — a short hint that biases the decoder towards the spelling of words it would
  otherwise guess wrong. `GROQ_PROMPT_FILE` reads it from a file. The API caps it at 224 tokens, and
  Whisper sometimes echoes it instead of transcribing, so keep it to a handful of words
- `GROQ_RESPONSE_FORMAT` — `text` (default) or `verbose_json`, which reports `no_speech_prob` per
  segment

```bash
export GROQ_API_KEY=...
export GROQ_WHISPER_MODEL=...      # optional, defaults to whisper-large-v3
```

## Keys from a password manager

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

## Tuning the reference transcriber

Everything below is outside the core: a transcriber is a command, so a script decides what the
recorded audio becomes before the text reaches the user's keyboard.

### Hallucinations on silence

Whisper was trained on subtitles, so on non-speech audio it completes with whatever it finds likely:
"Thank you.", "Sous-titres réalisés par…", or any plausible sentence in the language it detected.
Measured on the reference setup, the signals do not behave as one would hope:

| recording | mean volume | peak volume | no_speech_prob | avg_logprob | transcription |
|-----------|-------------|-------------|----------------|-------------|---------------|
| digital silence | -91.0 dB | -91.0 dB | — | — | nothing |
| 440 Hz tone | -21.5 dB | -18.5 dB | 0.0001 | -1.03 | `d` |
| the same tone, another run | | | 0.993 | — | `.` |
| a room with people talking | | | 0.001 | -0.27 | coherent sentences |

The same audio gives `no speech` on one run and `certainly speech` on the next, so **no single signal
settles this**. Three things, in this order, cover most of it.

A duration floor, for a keystroke that captured nothing:

```bash
duration="$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$1")"
if awk -v d="$duration" 'BEGIN { exit !(d < 0.5) }'; then
    exit 0
fi
```

Digital silence, which is what a muted microphone produces. A room's noise is louder (measured:
-18.6 dB peak for a room containing a conversation), so treat the threshold as something to calibrate
on your own room, by recording three seconds of it and reading the `max_volume` line:

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

Dropping only the flagged segments keeps the speech around a long pause, and printing the reason on
stderr is what makes the thresholds tunable from the journal. This is still a heuristic: a mumble can
be dropped, and loud non-speech can get through. The robust answer is a real voice activity detector
on the audio before the request.

`sox file -n stat` reports the same volumes as a 0..1 amplitude if you prefer it to ffmpeg.

Groq also reports `no_speech_prob` per segment with `response_format=verbose_json`, if you prefer to
let the API decide (it costs the request, and Groq bills a 10 second minimum).

### A vocabulary

Groq's transcription request takes a `prompt` that "guides the model's style or specifies how to
spell unfamiliar words". It rides along in the same request, so it costs nothing extra, and it
belongs to the transcriber, not to the core:

```bash
export GROQ_LANGUAGE=fr
export GROQ_PROMPT="DBnomics, herdr, keyd, Vicinae"      # or GROQ_PROMPT_FILE
```

It is a bias, not a rule: it can fail, and Whisper sometimes echoes the prompt in the transcript
instead of transcribing. That is the only reason the list has to stay short — Groq caps the prompt at
224 tokens, so a dozen terms, not a glossary. On the reference setup the list lives in
`~/.config/tiny-dictate/words`, one term per line, managed by `tiny-dictate-words`, and the
transcriber joins it with commas before each request.

Rewriting the transcript afterwards is the deterministic alternative: a table of `as the transcriber
hears it` → `what you want`, applied to the text. It guarantees the spelling, at the price of
anticipating every mistake, and it cannot repair a word the model heard as something unrelated.
Worth keeping for the few terms the prompt keeps missing.

## Decisions

- [ADR-0002: the transcriber stays an external command](../adr/0002-transcriber-stays-an-external-command.md)
  adds the session directory as `$2` and the `--print-audio-limit` probe.
