# tiny-dictate

Minimal voice dictation for Linux: a command starts a recording, the same command stops it and
pastes what was said, and a pill on screen shows the dictation while it runs. No daemon, no service
to install.

## Layout

- `src/tiny-dictate`: the tool itself, the only file that has to be on `PATH`
- `src/presenters/`: the presenters shipped with the tool
- `transcribers/`: the transcribers shipped with the tool
- `tests/run.sh`: the integration suite, every external command stubbed
- `contrib/`: integrations with other tools, none of which the tool needs at runtime

## Toolchain

- `tests/run.sh`: the integration suite. It stubs every external command, and needs no microphone,
  no notification daemon, no layer-shell surface and no network.
- `SCRIPT=<path> tests/run.sh`: run the suite against another executable.
- `ONLY=<test> tests/run.sh` runs one test; `DIAG=1 tests/run.sh` dumps the state after each test.
- The tool and its reference plugins are plain executables: bash for `src/tiny-dictate` and the
  reference transcriber, Python with GTK4 for the reference presenter. There is nothing to build.

## Invariants

Rules the tool does not break, whatever implementation runs them:

- One dictation at a time: the session directory is the claim on the microphone, and creating it
  with `mkdir` is atomic.
- The recorder's pid is recorded before the session process is forked, and the session waits for that
  pid: no window in which it could believe the recorder finished when it has not started.
- `stop` and `cancel` signal the recorder, never the session. A signal never removes the pidfile it
  was read from.
- The encoder outlives the recorder, so it can flush the last frames.
- One presenter per dictation, and the session is the only writer of the state file: the presenter is
  the only writer of the screen.
- The session waits for the presenter to be gone before it notifies anything.
- The pill never takes keyboard focus: the paste goes to the focused window.
- A cancelled dictation is neither transcribed nor reported as a failure.
- The waveform channel is a copy of the stream: a meter that dies cannot end a recording.
- No secret is ever written into the repository; secrets come from the environment.

## Documentation

- `docs/SPECS.md` and `docs/features/`: how the tool works today, one file per feature.
- `GLOSSARY.md`: the vocabulary. Use its terms.
- `docs/adr/`: the decisions taken during design, numbered. Read the ones that touch your area.
- `docs/agents/`: how the engineering skills consume the three above.

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues, driven by the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default Matt Pocock vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
