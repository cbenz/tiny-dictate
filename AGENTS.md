# AGENTS.md

Minimal voice dictation tool for Linux (speech-to-text, STT).

## Layout

- `src/tiny-dictate`: the tool itself, the only file that has to be on `PATH`
- `src/presenters/`: the presenters shipped with the tool
- `src/transcribers/`: the transcribers shipped with the tool
- `tests/run.sh`: the integration suite, every external command stubbed
- `contrib/`: integrations with other tools, none of which the tool needs at runtime

## Use case

- the user configures a keyboard shortcut (e.g. `mod+backspace`) in their desktop environment to run `tiny-dictate toggle`
- when the shortcut is pressed, a pill reading "Recording" appears at the bottom of the screen
- the user speaks for as long as they want
- the recording can be canceled by running `tiny-dictate cancel` (e.g. bound to another shortcut or run manually): the pill reads "🛑 Cancelled" for a second, then disappears
- when the same shortcut is pressed again: recording stops, the pill switches to "Transcribing" and stays visible for the whole transcription
- after processing, the transcribed text is inserted into the active text field (as if the user had typed it)
- the pill disappears

## Specs

### Functional specs

- the user starts dictation by running the script and stops it by running it again
- the user can configure a keyboard shortcut if they want (outside the script scope)
- the transcribed text is inserted into the active field without sending an equivalent "Enter" keypress
- the user can cancel an ongoing recording, and the pill says so: a cancelled dictation is neither
  transcribed nor reported as a failure
- the live status is drawn by the presenter and nothing else draws while it runs: a pill never
  fights a stale frame of itself, and a killed dictation cannot leave one behind
- the pill never takes keyboard focus: the paste that ends a dictation goes to the focused window,
  and a pill that could be focused would receive the text itself

### Scope

- batch dictation only: the whole utterance is recorded, then transcribed once
- streaming ("on the fly") transcription is **out of scope**: it is a different contract
  (long-lived process, incremental text, partials that get revised, key injection while
  speaking), not an extension of the batch plugin contract

### Transcription plugin

The core has no transcription backend of its own: transcription is delegated to a
**command provided by the user**. It is the only mandatory plugin — the tool cannot work
without one.

- the command receives the recorded audio file as `$1` and writes the transcribed text to stdout
- a non-zero exit status means failure: stderr is shown to the user as a single truncated line
- any command can be plugged in: another service, a local `whisper.cpp`, a user script
- the reference implementation targets Groq Whisper and ships with the tool under
  `src/transcribers/` (`src/transcribers/groq`)
- a transcriber is a plain executable script, so it can fetch its credentials wherever the user
  keeps them (password manager, secrets store), as long as nothing prompts: the tool runs from a
  keyboard shortcut, with no terminal
- the plugin converts audio to text and nothing else: the core owns the status channel and
  keyboard injection, so its code path does not depend on the chosen backend
- resolved with `command -v`: `$TINY_DICTATE_TRANSCRIBE` if set, else `tiny-dictate-transcribe` on
  `PATH`; an unresolvable command is reported to the user before any recording starts

### Presentation plugin

The core draws nothing itself: the **live status** is delegated to a presenter command. It is the
second plugin, and the second one that is a plain executable script.

- the command receives the path of a **state file** as `$1`, and the pid of the session that owns
  it as `$2`
- the state file holds one word: `recording`, `transcribing`, `cancelled`, or `stop`, which asks the
  presenter to leave
- the presenter leaves on its own when the state file disappears, when it reads `stop`, or when the
  process it was given is gone: three ways out, so a killed dictation cannot leave a pill frozen on
  screen
- it draws every state the core asks for, the `cancelled` verdict included. Failures are the only
  thing left to notify: they are desktop notifications sent once with `notify-send`, so nothing has to
  replace them, there is no notification id to track, and no daemon beyond libnotify to require
- the reference implementation targets `wlr-layer-shell` and ships with the tool under
  `src/presenters/` (`src/presenters/layer-shell`). gtk4-layer-shell must be loaded *before*
  `libwayland-client`, which a plain import is too late for: the script re-executes itself with
  `LD_PRELOAD` set
- a presenter on a compositor without layer-shell support exits non-zero: the dictation still
  works, only the pill is missing
- resolved with `command -v`: `$TINY_DICTATE_PRESENT` if set, else `tiny-dictate-present` on `PATH`;
  an unresolvable command is reported to the user before any recording starts

### Runtime model

- written in bash, no persistent state, no daemonization
- one **session process** per dictation owns the whole lifecycle: waiting for the recorder,
  transcribing, injecting, and the pill. `start` forks it, it cleans up after itself
- session state (recorder/encoder/session PIDs, audio file, presenter state file) lives in a session
  directory under the XDG runtime directory, created atomically with `mkdir`: that directory *is*
  the claim on the microphone, so two dictations can never run at once
- `start` records the recorder PID **before** forking the session process, and the session waits for
  that PID: there is no window in which the session could believe the recorder already finished
- the presenter is spawned once per dictation and never replaced: the session changes the state by
  writing the state file, so the session is the only writer of the screen and the presenter is the
  only process drawing on it
- the session waits for the presenter to be gone before it notifies anything, so a notification
  never lands on top of the pill
- `cancel` writes its verdict into the state file and signals the recorder, never the session: the
  session is the one that reads the verdict, keeps the pill up for the dwell, and tears itself down
  without transcribing anything. It reads the verdict twice, before transcribing and before pasting,
  so a cancel that lands during the transcription drops the text instead of pasting it
- `stop` signals the recorder only, never the encoder: the encoder must outlive it to flush the last
  frames, and the session waits for both to be gone before it reads the audio file
- sending a signal must never remove the pidfile it was read from: the session relies on that pidfile
  to tell whether the recorder is still alive

### Technical specs

- audio recording via `arecord` (S16_LE, 16 kHz, mono), encoded to MP3 on the fly with `lame`
- transcription via the plugin command described above: `$TINY_DICTATE_TRANSCRIBE` if set, else
  `tiny-dictate-transcribe` on `PATH`; the reference implementation is `src/transcribers/groq`
  (Groq Whisper), installed under that default name
- keyboard result injection: copy text to **CLIPBOARD and PRIMARY** (`wl-copy` and `wl-copy --primary`) followed by `ydotool key Shift-Insert`
  - CLIPBOARD for modern applications (VS Code, browsers)
  - PRIMARY for classic Unix applications (terminals, xterm, vim)
- live status via the presenter command described above: `$TINY_DICTATE_PRESENT` if set, else
  `tiny-dictate-present` on `PATH`; the reference implementation is `src/presenters/layer-shell`
  (GTK4 + gtk4-layer-shell), installed under that default name. It draws a pill anchored to the
  bottom edge, on the overlay layer, with the keyboard mode set to none
- failures via `notify-send`: one notification per event, never replaced, always with an expiry, so
  nothing can be left sticky on screen
- the session directory is a subdirectory of the XDG runtime directory and is removed when the
  session ends

## Docs

layer shell:

- <https://github.com/wmww/gtk4-layer-shell>
- <https://github.com/wmww/gtk4-layer-shell/blob/main/linking.md>

dunst (the daemon that displays the remaining notifications):

- <https://dunst-project.org/documentation/>
- <https://dunst-project.org/documentation/dunst/>
- <https://dunst-project.org/documentation/guides/>
- <https://dunst-project.org/documentation/dunstify/>
- <https://dunst-project.org/documentation/faq/>
- <https://wiki.archlinux.org/title/Dunst>
