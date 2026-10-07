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
one has, `notify-send`, `wl-copy`, `wl-paste`, `ydotool`, and the microphone mute probes.
Assertions are made on the state file, on a log of the events the stubs appended to, and on `pgrep`
for process lifetimes.

The recorder stub uses the real `arecord` on the null device, because a shell script cannot install
the `SIGINT` handler a capture installs: a background job of a non-interactive shell gets `SIGINT`
set to `SIG_IGN`, and POSIX forbids trapping a signal that was ignored on entry.

## What it covers

The suite asserts external behaviour only: the states written to the state file, the events the
presenter was asked to draw, the lifecycle of the processes, the exit codes.

- the normal flow: start, record, stop, paste
- the waveform channel carries the recording to the pill, and hands the pill the waveform file and
  the command to run
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
- the recorder race run `RACE_ITERATIONS` times, optionally against another implementation
- the journal: one line per stage of a dictation, each line carrying the session id it concerns and
  none when it concerns no session, the failure line agreeing with the metadata and the
  notification, and the encoder's and the recorder's chatter kept out

## The suite is the port's acceptance criterion

The suite stays green and unchanged from before the port to Rust to after it, the Rust binary taking
the place of the script. It is the safety net that makes the port boring, and it is why the tests are
not to be rewritten in Rust.

Two things the process seam cannot see are planned as focused Rust unit tests: the prune selection (a
list of sessions in, the ones to remove out) and the pill's frame computation (state, waveform, limit
and elapsed time in, what to draw out, including the three bands of the progress bar). Both are pure
functions. Nothing else is unit-tested: the rest is observable from outside, and the fewer seams
across the codebase the better.

## Decisions

- [ADR-0001: one Rust binary](../adr/0001-single-rust-binary.md) is the port this suite is the
  acceptance criterion for.
- [ADR-0005: prune](../adr/0005-prune.md) and
  [ADR-0006: the limit and the bar](../adr/0006-the-limit-and-the-bar.md) are the two pure functions
  the port unit-tests; everything else stays observable through this suite.
