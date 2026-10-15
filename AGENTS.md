# tiny-dictate

Minimal voice dictation for Linux: a command starts a recording, the same command stops it and
pastes what was said, and a pill on screen shows the dictation while it runs. No daemon, no service
to install.

## Layout

- `src/`: the Rust crate: the tool itself, the pill included. `main.rs` parses the command line,
  and each layer of the tool lives in its own module.
- `transcribers/`: the transcribers shipped with the tool
- `tests/run.sh`: the integration suite, every external command stubbed
- `contrib/`: integrations with other tools, none of which the tool needs at runtime
- `scripts/`: development aids, not part of the tool: the glossary's vocabulary check and the pill
  preview

## Toolchain

- `cargo build --release` builds the binary. The pill links GTK4 and gtk4-layer-shell, so their
  development packages are needed to build; nothing is needed beside the binary at runtime.
- `./install.sh` builds the release binary and links it onto PATH as `tiny-dictate`, links the
  reference transcriber as `tiny-dictate-transcribe` (the previous one is kept as a `.bak`), and
  removes the bash implementation's `tiny-dictate-present` link, so the shortcuts and the bar drive
  this checkout; `./install.sh restore` links the main worktree's install back. It names missing
  build dependencies and stops; missing runtime commands are only listed.
- `tests/run.sh` runs the built binary at `target/release/tiny-dictate`, and never builds it. It
  stubs every external command, and needs no microphone, no notification daemon, no layer-shell
  surface and no network; its one dependency is `python3`, which drives the pseudo-terminal the
  browser's cases are observed on.
- `SCRIPT=<path> tests/run.sh`: run the suite against another executable.
- `ONLY=<test> tests/run.sh` runs one test; `DIAG=1 tests/run.sh` dumps the state after each test.
- The reference transcriber is a plain shell script, and stays one: "any command" is the contract.

## Invariants

Rules the tool does not break, whatever implementation runs them:

- One dictation at a time: the session directory is the claim on the microphone, and creating it
  with `mkdir` is atomic; the durable session is not the claim.
- The recorder's pid is recorded before the session process is forked, and the session waits for that
  pid: no window in which it could believe the recorder finished when it has not started.
- `stop` and `cancel` signal the recorder, never the session: the session owns the lifecycle. A
  signal never removes the pidfile it was read from.
- The kill order: the recorder is signalled before the encoder, and the encoder outlives it so it can
  flush the last frames.
- One presenter per dictation: the session and `cancel`'s verdict are the only writers of the state
  file, and the presenter never writes it — the presenter is the only writer of the screen.
- The pill leaves on its own: the state file disappears, it reads `stop`, or the process it was given
  is gone. A killed dictation cannot leave a pill frozen on screen.
- The session waits for the presenter to be gone before it notifies anything: one thing on screen at
  a time.
- The pill never takes keyboard focus: the paste goes to the focused window, and a focusable pill
  would receive the text itself.
- A cancelled dictation is neither transcribed nor reported as a failure.
- The waveform channel is a copy of the stream: a meter that dies cannot end a recording.
- A report is never worth a recording: a marker the pump cannot write, or a notification the desktop
  cannot show, costs the report and nothing else.
- No secret is ever written into the repository; secrets come from the environment.

## Documentation

- `README.md`: the front door — installing the tool, configuring it and its features.
- `docs/development.md`: building it, the tests, the pill preview, and how the tool is shaped.
- `GLOSSARY.md`: the vocabulary. Use its terms.
- `docs/adr/`: the decisions taken during design, numbered. Read the ones that touch your area, and
  record a new decision there rather than in prose elsewhere.
- `docs/agents/`: where the issue tracker lives, the triage labels, and how the engineering skills
  consume the two above.

Specifications and tickets are not kept in this repository: they live in its GitHub Issues, which
`docs/agents/issue-tracker.md` describes. A page describing how a layer works belongs in that
layer's module documentation (`//!` at the top of its file).

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues, driven by the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default Matt Pocock vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
