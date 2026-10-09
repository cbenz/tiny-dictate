# The tests

## Running the suite

```bash
cargo build --release
tests/run.sh
```

The suite runs the built binary at `target/release/tiny-dictate` and never builds it: a suite that
silently compiles would test whatever it produced rather than what the user asked for. When the
binary is absent it says so and exits non-zero.

It stubs the recorder, the encoder, the transcription command, the presenter, the notifier and the
keyboard injector: it needs no microphone, no notification daemon, no layer-shell surface and no
network.

| variable | effect |
|----------|--------|
| `SCRIPT` | the executable under test (defaults to `target/release/tiny-dictate` in this tree) |
| `OLD_SCRIPT` | a second executable to compare the recorder race against |
| `RACE_ITERATIONS` | how many times the race loop runs (default 25) |
| `ONLY` | run one test by function name |
| `DIAG` | dump the state after each test and keep `/tmp/td-diag` |

## The safety net

The tested executable is launched with a session bus address and a Wayland display that do not exist.
Even a stub the suite forgot to write cannot pop a notification, draw a pill or touch the real
clipboard. The stubs themselves are syntax-checked before anything runs: a broken stub must fail the
suite loudly rather than silently test nothing.

## What it stubs

Every command the tool runs is written into a temporary `bin` directory prepended to `PATH`: a
recorder that installs a real `SIGINT` handler on the ALSA null device, an encoder that writes a
plausible amount of audio slowly, a transcriber, a presenter that leaves by the three doors the real
one has, `notify-send`, `wl-copy`, `wl-paste`, `ydotool`, and the audio server's `pactl`.
Assertions are made on the state file, on a log of the events the stubs appended to, and on `pgrep`
for process lifetimes.

The recorder stub uses the real `arecord` on the null device, because a shell script cannot install
the `SIGINT` handler a capture installs: a background job of a non-interactive shell gets `SIGINT`
set to `SIG_IGN`, and POSIX forbids trapping a signal that was ignored on entry.

## The pseudo-terminal

The browser is the one surface the process seam cannot see: it draws escape sequences and reads raw
keys. `tests/run.sh` therefore drives it on a pseudo-terminal through a small `python3` helper built
on the `pty` module: it opens the browser with a known terminal size, feeds it keys, captures the
screen it drew, and compares the terminal's termios before and after the run. The screen's bytes are
never asserted as a whole — the marker on the selected row and the pane's fields are, and each key's
effect is read from where the marker sits after it.

## What it covers

The suite asserts external behaviour only: the states written to the state file, the events the
presenter was asked to draw, the lifecycle of the processes, the exit codes.

- the normal flow: start, record, stop, paste
- the waveform channel carries the recording to the pill, and hands the pill the waveform file and
  the command to run
- the default source of the audio server is set to the base volume it publishes before the recorder
  opens, the source is unmuted instead of refused, and the configured fallback level stands in when
  the server answers without a readable base volume, or is not on `PATH` at all
- a waveform meter that dies mid-recording does not end the recording: the guarantee the `tee -p`
  pipeline used to carry, kept as a regression scenario even though the in-process slicer no longer
  calls the stub the scenario breaks
- stopping immediately after starting (the reported race)
- the recorder failing at startup, with its stderr shown to the user
- the transcription command failing
- no transcription command configured, and a presenter that cannot start (the dictation runs on
  without a pill)
- the fallback to the default command names on `PATH`
- cancel while recording
- a new dictation starting before the cancelled verdict is over
- cancel while transcribing, and the text being dropped
- a second start refused while recording
- toggle while transcribing
- `session list` prints the kept sessions newest first, with the id, the human duration, the
  outcome and the first words of the transcript or the error, and shows a session whose metadata
  cannot be read as a row of `?` instead of skipping it
- `session show` prints the fields of one session -- the id, the path, the outcome, the started,
  the ended, the human duration, the audio bytes, the pasted flag, the transcriber and the error --
  then its transcript, and says so when the session holds no transcript or its metadata cannot be
  read
- `session list --output json` prints one document: an array, newest first, each element the id, the
  path and the metadata `metadata.json` holds, or `null` for a session whose metadata cannot be read
  -- the array is never shorter than the sessions directory, and the times and durations are the raw
  seconds, never the human form; `session show --output json` prints one object, the same fields plus
  the transcript or `null`
- `--output text` prints byte for byte what the command prints by default, in both the separated and
  the `=` form, and an unknown format or the flag on a verb that shows no session is an argument
  error, the usage and a non-zero exit
- `session browse` refuses a stdout that is not a terminal, and bare `session` lists a pipe where it
  browses a terminal
- the browser opens on a pseudo-terminal, lists the kept sessions newest first, moves the selection
  with the arrows and the vim keys, reloads on `r` and leaves on `q` or `ctrl-c`, giving the
  terminal back every time — after `q`, after `ctrl-c` and after a run that failed — showing a
  session whose metadata cannot be read as a selectable row of `?`, opening an empty history as an
  empty list rather than an error, and reloading the detail pane on the session the selection lands
  on
- the browser's `c`, `i` and `p` put the selected session's transcript, its id and its path on the
  clipboard through the same writer `session copy` uses — both selections, the password hint, the
  configured lease — injecting nothing, saying on the status line what was copied, and saying so
  when no session is selected or the session on disk holds an empty transcript
- the browser's `t` transcribes the selected session again, rewriting the transcript and the
  metadata as `session transcribe` does, and its `c` transcribes a session that has no transcript
  before offering the text; a transcript that exists but holds no word runs no transcriber
- a transcription in flight: a second one cannot start from any key sequence, `c` and `t` start
  nothing, `ctrl-c` cancels it, reaps the transcriber's whole tree, keeps the browser open with a
  verdict and leaves the session's transcript and metadata byte for byte what they were, while `q`
  signals the transcriber and leaves with no transcriber outliving the browser; a run that fails
  shows the collapsed one-line error, records the failure and leaves the running state
- the browser's `d` asks before destroying anything: the question names the session, `n` and a stray
  key leave the session and the list untouched while a later `y` still deletes, and `y` removes the
  session directory and its row without leaving the browser
- the browser deletes through the same guard `session delete` takes: the dictation in flight is
  refused with a message on the status line and nothing is removed
- the browser holds no command lock: a shortcut starts a dictation while it is open
- `session transcribe last` re-runs the transcriber, rewrites the transcript and the raw response,
  and copies the text to the clipboard, saying so
- `session transcribe --print` writes the text to stdout and touches no clipboard
- `session copy` offers the stored transcript through the same offer, without re-transcribing it,
  and on a session whose transcript was removed -- or that a failure or a cancel left without one
  -- runs the transcriber first, writes the transcript and the metadata, and offers the text; a
  transcript that holds no word is reported and runs no transcriber
- `session delete` removes exactly one session and leaves the others
- `session delete` refuses the session in flight -- by `last` and by its id -- without removing
  anything, taking the command lock or signalling the recorder, while an older session still
  deletes, and the same id deletes normally once the dictation has ended or was killed
- an unknown id is reported and removes or writes nothing
- `history` and `transcribe` at the root print the usage and exit non-zero, and so does an unknown
  sub-verb of `session`
- the `session` verbs run with the encoder and the clipboard tools removed from `PATH`
- the paste waits for the offer to serve the text it injects, so the chord cannot read the previous
  dictation's, and it still injects it with `wl-paste` removed from `PATH`
- the recorder race run `RACE_ITERATIONS` times, optionally against another implementation
- the journal: one line per stage of a dictation, each line carrying the session id it concerns and
  none when it concerns no session, the failure line agreeing with the metadata and the
  notification, and the encoder's, the recorder's and the presenter's chatter kept out
- the reference transcriber's contract against a stubbed `curl`: the audio file as `$1`, the raw
  answer left in the session directory as `$2/transcript.raw.json`, the transcript on stdout

## The suite is the port's acceptance criterion

The suite keeps its scenarios and stays green through the port, but its driver was rewritten: it
used to drive the shell script and now runs the Rust binary at `target/release/tiny-dictate`. It is
the safety net that makes the port boring, and it is why the tests are not to be rewritten in Rust.

The process seam is the primary one: the states written to the state file, the events the presenter
was asked to draw, the lifecycle of the processes and the exit codes are all observable there. A few
pure functions the seam cannot see are unit-tested instead: the pill's frame computation (state,
waveform, limit and elapsed time in, what to draw out, including the three bands of the progress
bar), the prune selection (a list of sessions in, the ones to remove out, sharing one listing with
`session list` and `last`), the browser's frame (the rows, the selection, the selected session's
detail, the run state, the scroll offset, the status line and the terminal's size in — the footer
names only the cancel key while a transcription runs — the list and the wrapped, windowed pane
out), and, for the session surface, reading the metadata back, resolving a
`<id|last>` argument, the human duration, the row of the listing, the fields of the detail and the
JSON composition of the listing and the detail.

## Decisions

- [ADR-0001: one Rust binary](../adr/0001-single-rust-binary.md) is the port this suite is the
  acceptance criterion for.
- [ADR-0005: prune](../adr/0005-prune.md) and
  [ADR-0006: the limit and the bar](../adr/0006-the-limit-and-the-bar.md) keep the pure functions —
  the prune selection and the pill's frame computation — out of the process seam, where the view's
  row, its duration and the metadata read-back also live.
- [ADR-0017: the browser takes crossterm](../adr/0017-the-browser-takes-crossterm.md) is why the
  browser's adapter is the one surface the suite watches on a pseudo-terminal.
