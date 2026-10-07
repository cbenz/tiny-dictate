# tiny-dictate

A Linux desktop tool that records one dictation at a time, has it transcribed by a command the user
provides, and pastes the text into the focused window.

## The dictation

**Dictation**:
The whole act of speaking into the tool, from the moment the microphone is claimed to the moment the
text is pasted, kept or dropped. A dictation outlives the command that started it.
_Avoid_: recording (the microphone capture alone)

**Claim**:
The exclusive hold on the microphone. A dictation takes the claim when it starts and frees it when
its session ends; a second dictation cannot run while the claim is held.
_Avoid_: lock

**Session**:
What one dictation leaves behind: the recording, the transcript, the transcriber's raw response, and
the metadata saying what happened. Cancelling keeps the session; only `session delete` destroys it.
_Avoid_: dictation (when the durable trace is meant)

**Outcome**:
What became of a session — `recording`, `transcribing`, `transcribed`, `failed` or `cancelled`. A
session left on `recording` is one that died mid-recording.
_Avoid_: status (that is the tool's word for `idle` or `working`), state

**State**:
One of the words a session publishes for the pill: `recording`, `transcribing`, `cancelled`, or
`stop`.

**Verdict**:
The word a cancellation leaves on screen for the user to read. It belongs to the cancellation, not to
the session it interrupts.
_Avoid_: notification

**Dwell**:
The short time a verdict stays on screen after the recording has stopped, before the pill leaves.
_Avoid_: timeout

## The plugins

**Presenter**:
The command that draws the live state of a dictation; a plugin surface the user can replace.
_Avoid_: pill

**Pill**:
What the reference presenter draws: the small status surface on screen while a dictation runs — the
state, and the input waveform while recording.
_Avoid_: notification, presenter

**Waveform channel**:
The copy of the recorded stream the pill draws the input waveform from. It sits beside the voice
path, so a channel that dies costs the waveform only.

**Transcriber**:
The command that turns the recorded audio into text; a plugin surface the user provides.
_Avoid_: backend, provider

**Audio limit**:
The largest recording a transcriber accepts, declared by the transcriber. It bounds a dictation's
duration and drives the progress bar; a transcriber that declares none leaves both unbounded.
_Avoid_: cap, timeout

**Progress bar**:
The bar under the waveform showing how much of the audio limit has elapsed.

## The result

**Paste**:
How the transcript reaches the focused window: both clipboard selections are written, then a
`Shift+Insert` keypress is injected.
_Avoid_: insertion, typing

**Lease**:
How long the transcript stays offered on the clipboard before it is withdrawn.
_Avoid_: timeout, expiry

**Notification**:
How a failure is shown to the user: once, with an expiry of its own. A verdict is not a notification.
_Avoid_: alert, toast

## Keeping the recording

**History**:
The list of kept sessions, newest first.
_Avoid_: journal (the journal is where the log lines land)

**Keep**:
The number of past sessions retained.
_Avoid_: history size

**Prune**:
Removing the sessions beyond `keep`, oldest first.
_Avoid_: cleanup

## The log

**Log**:
One line per event the tool writes to stderr: the timeline of what happened, carrying the session id
when the event concerns one.
_Avoid_: global log file

**Journal**:
Where the tool's log lines land on the reference machine — the systemd journal. The session metadata
is the durable trace; the journal is the timeline.
