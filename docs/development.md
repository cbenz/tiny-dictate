# Development

Building `tiny-dictate`, running its tests, and looking at the pill without a microphone. The
user-facing documentation is the [README](../README.md).

## Build

The pill links GTK4 and [gtk4-layer-shell](https://github.com/wmww/gtk4-layer-shell), so their
development packages, a Rust toolchain and a C compiler are needed to build; nothing is needed beside
the binary at runtime.

```bash
cargo build --release
```

On Arch, that is `pacman -S gtk4 gtk4-layer-shell rust base-devel`. The GUI toolkit is linked into
every invocation, `tiny-dictate status` included — deliberate, not a packaging mistake.

The crate is `publish = false`: it is distributed as a binary, not through crates.io.

## Tests

`tests/run.sh` runs the built binary against stubbed collaborators. It needs no microphone, no
notification daemon, no layer-shell surface and no network; its one dependency is `python3`, which
drives the pseudo-terminal the browser's cases are observed on. It runs the binary at
`target/release/tiny-dictate` and never builds it:

```bash
cargo build --release
tests/run.sh
```

- `ONLY=<test> tests/run.sh` runs one test.
- `DIAG=1 tests/run.sh` dumps the state after each test.
- `SCRIPT=<path> tests/run.sh` runs the suite against another executable.

`cargo test` unit-tests what the process seam cannot see. The whole check, as CI runs it on Arch:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
scripts/check-vocabulary.sh
cargo test
tests/run.sh
```

`tests/run.sh` kills leftover stubs with a global `pkill -f 'arecord -D null'`; two suites cannot
run side by side unless each is wrapped in a private PID namespace
(`unshare -Urpf --mount-proc bash -c 'tests/run.sh'`).

## Preview the pill

`scripts/preview-pill.sh` fabricates the state file, the owner process and the waveform channel the
presenter reads, and launches the real presenter against them, so the pill's three states can be
looked at without recording anything: `recording` (with the meter moving), then `transcribing`, then
`cancelled`, three seconds each. It needs a display and layer-shell; Ctrl-C leaves no pill and no
child process behind.

```bash
cargo build --release
scripts/preview-pill.sh

# freeze on one state, to compare two of them deliberately
TINY_DICTATE_PREVIEW_STATE=transcribing scripts/preview-pill.sh
```

`TINY_DICTATE_BIN` names another binary than `target/release/tiny-dictate`. It is a development aid:
nothing asserts on it and the integration suite never runs it.

## Dev install (reversible)

To make the shortcuts and the bar drive this checkout instead of the installed copy, without touching
the compositor's configuration:

```bash
./install.sh          # build the release binary and link it onto PATH
./install.sh restore  # put the main checkout's install back
```

`install.sh` links `~/.local/bin/tiny-dictate` at this checkout's `target/release/tiny-dictate`, so a
later `cargo build --release` is picked up without re-running it, and links
`~/.local/bin/tiny-dictate-transcribe` at this checkout's `transcribers/groq`, keeping whatever held
that name as `tiny-dictate-transcribe.bak` (once, so a personal wrapper is never lost). It refuses to
replace `~/.local/bin/tiny-dictate` when that is not a symlink, since `restore` can only undo a
symlink. Missing build dependencies stop the install before the build, each named with its Arch
package; missing runtime commands are only listed.

## Releasing

Bump `version` in `Cargo.toml`, commit, then tag `v<version>` and push the tag. The Release workflow
builds in the same Arch container as CI and attaches `tiny-dictate-<version>-x86_64-linux.tar.gz`
(the binary, the reference transcriber, the README and the LICENSE) to a GitHub release. It refuses a
tag that does not match the crate version, and a manual run of the workflow packages the archive
without publishing it.

The AUR package in [`contrib/aur/`](../contrib/aur/) installs that archive; see its README for the
`updpkgsums` / `.SRCINFO` steps.

## How the tool is shaped

The tool is one binary: `main.rs` parses the command line, and each layer of it lives in a module.
`start` forks a session process that owns the whole dictation — the recorder, the encoder, the
waveform channel and the pill — and nothing of the tool outlives a dictation beyond the clipboard
offer that carries the text. A dictation is batch: the whole utterance is recorded, then transcribed
once. The vocabulary is in [`GLOSSARY.md`](../GLOSSARY.md), the decisions in [`docs/adr/`](adr/), and
the invariants in [`AGENTS.md`](../AGENTS.md).
