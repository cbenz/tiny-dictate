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

The session and everything it needs to exchange with the other commands live in a session directory
under the runtime directory (`$XDG_RUNTIME_DIR` when set, `$XDG_STATE_HOME` or `~/.local/state`
otherwise):

| file | contents |
|------|----------|
| `session.pid` | the session process |
| `arecord.pid` | the recorder |
| `lame.pid` | the encoder |
| `state` | the word the presenter draws: `recording`, `transcribing`, `cancelled`, `stop` |
| `audio.mp3` | the encoded recording |
| `audio.fifo` | the pipe between the recorder and the encoder |
| `waveform` | the [waveform channel](pill.md) |
| `arecord.err` | the recorder's stderr |
| `transcribe.err` | the transcriber's stderr |

The directory is removed when the session ends. It is also the [claim on the microphone](claim.md),
which is why it is created before anything else runs.

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

`arecord` captures the microphone as raw S16_LE, 16 kHz, mono, and writes it into a fifo. `lame`
reads that fifo and encodes MP3 (64 kbps, mono) into `audio.mp3` from the first frame: it writes
progressively instead of buffering the whole recording, so a killed encoder leaves a file that
decoders still open. The normal stop path, where the recorder dies first and the encoder sees
end-of-file, finalises the file properly. The recorder and the encoder are two processes;
the [waveform channel](pill.md) is a copy of the same stream.

The pipeline exists only while a dictation runs, and its files go with the session directory.

## `start`, `stop`, `toggle`, `status`

- `start` resolves the transcriber and the presenter, refuses to run when either is missing, claims
  the session directory, refuses when the microphone is muted, and starts the pipeline.
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
- `No transcription command. Set TINY_DICTATE_TRANSCRIBE or install tiny-dictate-transcribe`, and
  the same for the presenter, before any recording starts.
- `Cannot create the audio pipe`, and `Microphone is muted. Unmute it before dictating.` with
  critical urgency.
- `Missing dependencies`, when `arecord`, `lame`, `ydotool`, `notify-send`, `curl`, `wl-copy` or
  `wl-paste` is not on `PATH`. The same message also goes to stderr with a line telling the user to
  install the missing commands.

A cancelled dictation is never a failure: its verdict replaces the notification it used to be.

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
  removing the session directory on the way out.
- [ADR-0008: the log](../adr/0008-the-log.md) gives the lifecycle one journal line per event.
