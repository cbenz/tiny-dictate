# Specifications

tiny-dictate records one dictation at a time, has it transcribed by a command the user provides, and
pastes the text into the focused window. It is one Rust binary: the command line, the audio pipeline
and the pill. It runs no daemon: the `start` command forks a session process that owns the whole
lifecycle, and nothing of the tool outlives a dictation beyond the clipboard offer that carries the
text.

These pages describe the tool as it works today. The decisions taken during design are recorded in
[`docs/adr/`](adr/), and each feature page names the ones that concern it.

## What a dictation does

1. `start`, or `toggle` from idle, resolves the transcriber, asks it for its audio limit, claims the
   microphone, checks that the microphone is not muted, and forks a session process.
2. The recorder captures the microphone, the encoder writes MP3 from the first frame, and the
   session publishes the input waveform for the pill to draw.
3. `stop`, or `toggle` while recording, stops the recorder; the encoder flushes its last frames.
4. The session runs the transcriber command on the audio file and shows `transcribing` while it
   runs. A cancel that lands during the transcription drops the text.
5. The session copies the text to both clipboard selections, injects it into the focused window with
   `ydotool Shift+Insert`, and removes its session directory.

A dictation is batch: the whole utterance is recorded, then transcribed once. Streaming
transcription is out of scope — it is a different contract (long-lived process, incremental text,
partials that get revised, key injection while speaking), not an extension of this one.

## The command line

```text
Usage: tiny-dictate <command>

Commands:
  start   Start recording
  stop    Stop recording and transcribe
  cancel  Cancel recording
  toggle  Start if idle, stop if recording
  status  Show status (idle or working)
```

`status` queries only: it never perturbs a live session. It prints `idle` or `working`, and the
`working` state covers both recording and transcribing.

At most one command is accepted per invocation; an unknown argument prints the usage and exits
non-zero. `start`, `stop`, `cancel` and `toggle` take an exclusive command lock for the duration of
the command, so two shortcuts pressed at once cannot interleave. `status` takes no lock.

## Plugins

The core has no transcription backend of its own: one command carries it. It is resolved with
`command -v` on `$TINY_DICTATE_TRANSCRIBE` when set and on `tiny-dictate-transcribe` otherwise, and
reported to the user before any recording starts when it cannot be resolved.

The command may declare the largest audio file it accepts: `--print-audio-limit` prints that byte
count, anything else means no limit, and the probe is asked once per dictation before the microphone
is opened. The byte count becomes a maximum duration at the encoder's 64 kbit/s, at which the
recording stops as if the shortcut had been pressed. With no declared limit there is no limit and no
bar (see [features/transcriber.md](features/transcriber.md)).

| surface | variable | default name | specification |
|---------|----------|--------------|---------------|
| transcriber | `TINY_DICTATE_TRANSCRIBE` | `tiny-dictate-transcribe` | [features/transcriber.md](features/transcriber.md) |

The transcriber is the only mandatory plugin: the tool cannot work without one. The pill is not one
— it is built into the binary, so nothing has to be installed beside it, and a pill that cannot
start costs the pill only: the dictation goes on without one (see
[features/pill.md](features/pill.md)).

The presenter is the one seam the tests keep. The session spawns the binary's own `_present` mode —
`tiny-dictate _present <state file> <pid> <waveform file> <self command> [limit seconds]` — and
`TINY_DICTATE_PRESENT_COMMAND` names another command instead. It is internal, never documented in
the README.

## Invariants

These hold whatever implementation runs them, and a port is expected to keep them:

- One dictation at a time. The session directory is the claim on the microphone, and creating it
  with `mkdir` is atomic.
- The recorder's pid is recorded before the session process is forked: the session can never
  believe the recorder finished when it has not started.
- `stop` and `cancel` signal the recorder, never the session. The session owns the lifecycle.
- A signal never removes the pidfile it was read from: the session reads liveness from it.
- The encoder outlives the recorder, so it can flush the last frames.
- One presenter per dictation, spawned by the session, and the session is the only writer of the
  state file. The presenter is therefore the only writer of the screen, and no stale frame can land
  after the pill is gone.
- The session waits for the presenter to be gone before it notifies anything: one thing on screen at
  a time.
- The pill never takes keyboard focus: the paste that ends a dictation goes to the focused window,
  and a focusable pill would receive the text itself.
- A cancelled dictation is neither transcribed nor reported as a failure.
- The waveform channel is a copy of the stream: a meter that dies cannot end a recording.

## Features

- [The dictation lifecycle](features/dictation-lifecycle.md)
- [The claim](features/claim.md)
- [Cancelling and the verdict](features/cancel-and-verdict.md)
- [The pill and the waveform channel](features/pill.md)
- [The transcriber command](features/transcriber.md)
- [The clipboard and the paste](features/clipboard.md)
- [The automation](features/automation.md)
- [The tests](features/tests.md)
