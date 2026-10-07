# The dictation lifecycle

## The use case

The user configures a keyboard shortcut in their desktop environment to run `tiny-dictate toggle`.
They press it, a pill appears at the bottom of the screen showing the input waveform, and they
speak for as long as they want. They press the same shortcut again: recording stops, the pill
switches to `Transcribing` and stays visible for the whole transcription, and the transcribed text
is inserted into the active text field as if they had typed it. The pill then disappears.

At any point the user can run `tiny-dictate cancel` from another shortcut. The pill reads
`🛑 Cancelled` for a second and disappears; nothing is transcribed and nothing is reported as a
failure.

## The session process

One session process owns the whole lifecycle of a dictation. `start` forks it and exits; the session
waits for the recorder, transcribes, injects, and cleans up after itself. It is the only process
driving the presenter, and the only writer of the [state file](pill.md).

The session and everything it needs to exchange with the other commands live in a runtime
directory under `$XDG_RUNTIME_DIR` (falling back to `$XDG_STATE_HOME` or `~/.local/state`):

| file | contents |
|------|----------|
| `session.pid` | the session process |
| `session.id` | the durable id, so `stop`, `cancel` and `toggle` can name the session they act on |
| `arecord.pid` | the recorder |
| `lame.pid` | the encoder |
| `state` | the word the presenter draws: `recording`, `transcribing`, `cancelled`, `stop` |
| `audio.fifo` | the pipe between the recorder and the encoder |
| `waveform` | the [waveform channel](pill.md) |
| `arecord.err` | the recorder's stderr |
| `lame.err` | the encoder's stderr |
| `transcribe.err` | the transcriber's stderr |

The recording and everything else that must outlive the session live in the durable
[session directory](sessions.md) under the state directory, which the encoder writes from the first
frame. Nothing that matters lives only in the runtime directory.

The runtime directory is removed when the session ends. It is also the
[claim on the microphone](claim.md), which is why it is created before anything else runs.

`start` records both the encoder's and the recorder's pid **before** forking the session process.
That is what makes the session's wait race-free: there is no window in which the session could
believe the recorder already finished.

The session waits for the recorder to exit, then for the encoder to flush its last frames:

- if the encoder dies first, the recorder would write into a pipe nobody reads and block forever;
  the session gives up instead.
- `lame` blocks in `open()` until `arecord` opens the pipe, so the wait for the encoder is always
  bounded. When nothing was captured there is nothing to flush, so the session does not wait for it
  at all.

## The pipeline

`arecord` captures the microphone as raw S16_LE, 16 kHz, mono. Its stream goes to the `_pump`
process of the same binary, which forwards it into a fifo while, beside it, appending the
[waveform lines](pill.md); `lame` reads that fifo and encodes MP3 (64 kbps, mono) into the
session's `audio.mp3` from the first frame: it writes progressively instead of buffering the whole
recording, so a killed encoder leaves a file that decoders still open. The normal stop path, where
the recorder dies first and the encoder sees end-of-file, finalises the file properly. The
recorder, the pump and the encoder are three processes; the waveform channel is a copy of the same
stream. The pump opens the fifo without blocking on the other end, so an encoder that is already
gone is a failure it reports and exits on rather than a process left waiting for a partner that
will not come.

The pipeline exists only while a dictation runs, and its runtime files go with the runtime
directory; the recording it produces goes to the [session](sessions.md).

## `start`, `stop`, `toggle`, `status`

- `start` resolves the transcriber, refuses to run without one, claims the session directory,
  refuses when the microphone is muted, and starts the pipeline. The pill is not a plugin: the
  session spawns it, and a pill that cannot start costs the pill only.
- `stop` signals the recorder only, never the encoder: the encoder must outlive it to flush the last
  frames. When no recording is active but a session is transcribing, it prints
  `Transcription already in progress` on stderr. It never notifies: the pill already shows
  `Transcribing`, and it is the only writer of the screen.
- `toggle` stops when recording, reports `Transcription already in progress` when transcribing, and
  starts otherwise. A cancelled verdict still on screen does not block it; see
  [the verdict](cancel-and-verdict.md).
- `status` prints `working` while the recorder or the session is alive, and `idle` otherwise. It
  only queries; nothing in it modifies anything, so a query cannot perturb a live session.

## Failures and notifications

The live state is drawn by the presenter. Failures are desktop notifications sent with
`notify-send`, once each, never replaced: nothing has to be kept in sync, so there is no notification
id and no `dunstify`. Every notification carries an expiry, so nothing can be left sticky on screen.

The session stops the presenter before notifying anything: one thing on screen at a time.

The failures that are notified:

- `Recording failed: <reason>`, with the reason among `no audio was captured`, `the audio encoder
  stopped while recording`, `the audio encoder did not finish`, and the recorder's own stderr
  appended when it wrote one (newlines flattened, truncated to 180 characters).
- `Transcription failed: <error>`, the transcriber's stderr flattened the same way.
- `Could not paste the transcription`, when the injected `Shift+Insert` fails.
- `No transcription command. Set TINY_DICTATE_TRANSCRIBE or install tiny-dictate-transcribe`,
  before any recording starts.
- `Cannot create the audio pipe`, and `Microphone is muted. Unmute it before dictating.` with
  critical urgency.

A missing command is not a notification of its own: the tool reports what it could not do. `start`
says the transcriber is missing before anything is claimed, and an `arecord` or `lame` that cannot be
run fails the pipeline, reported on stderr.

A cancelled dictation is never a failure: its verdict replaces the notification it used to be.

## The journal

The tool writes its own timeline to standard error, one line per event, prefixed with its name and,
when the event concerns a session, with that session's id ([ADR-0008](../adr/0008-the-log.md)).
`start` publishes the id in the runtime directory as `session.id`, which is how `stop`, `cancel` and
`toggle` — each a process of its own — can name the session their line concerns. The prune and
`session delete` concern no session and carry no id, and the source of a command — keyboard, pill
button or command line — is not distinguished.

| line | event |
|------|-------|
| `start: recording started`, `toggle: recording started` | a dictation claimed the microphone |
| `stop: recorder signalled`, `toggle: recorder signalled` | the recorder was signalled |
| `stop: transcription already in progress`, `toggle: transcription already in progress` | there was nothing to signal |
| `stop: nothing to stop` | no dictation was running |
| `cancel: cancelled` | the verdict was written |
| `cancel: nothing to cancel` | no dictation was running |
| `recording started` | the recorder and the encoder are live |
| `recording ended duration=<s>s bytes=<n>` | the recording stage closed |
| `recording stopped at the transcriber's audio limit` | the limit ended the recording |
| `transcription started` | the transcriber is running |
| `transcription ended duration=<s>s chars=<n>` | the transcriber returned, or failed |
| `text written chars=<n>` | `transcript.txt` was written |
| `paste succeeded`, `paste failed` | the chord was injected, or not; an empty transcription is never pasted and says so |
| `late cancel, text dropped` | a cancel landed while the transcription ran |
| `failure: <error>` | a failure; a session's failure carries the same text the metadata's `error` holds |

The encoder's and the recorder's own output is kept out of the journal: the recorder's stderr goes
to `arecord.err`, the encoder's to `lame.err`, and the transcriber's to `transcribe.err`, which the
failure path flattens into the one line the metadata's `error` holds and the notification shows.

## The recorder race

The transcribed audio was once lost to a race: `start` wrote the recorder's pid from a subshell that
raced with the session, so the session could believe recording was over and transcribe an empty
file. The fix is the invariant in [`../SPECS.md`](../SPECS.md): the pid is written synchronously,
before the session is forked, and the session waits for that pid. `tests/run.sh` runs the scenario
repeatedly — see [the tests](tests.md).

## Decisions

- [ADR-0001: one Rust binary](../adr/0001-single-rust-binary.md) replaces this script, the audio
  pipeline and the presenter with one process.
- [ADR-0003: sessions on disk](../adr/0003-sessions-on-disk.md) keeps the recording instead of
  removing the session directory on the way out, and writes it under the state directory — see
  [the session](sessions.md).
- [ADR-0008: the log](../adr/0008-the-log.md) gives the lifecycle one journal line per event.
