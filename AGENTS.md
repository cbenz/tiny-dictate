# AGENTS.md

Minimal voice dictation tool for Linux (speech-to-text, STT).

## Use case

- the user configures a keyboard shortcut (e.g. `mod+backspace`) in their desktop environment to run `tiny-dictate toggle`
- when the shortcut is pressed, the notification "🎤 Recording..." appears
- the user speaks for as long as they want
- the recording can be canceled by running `tiny-dictate cancel` (e.g. bound to another shortcut or run manually)
- when the same shortcut is pressed again: recording stops, the notification "⏳ Transcribing..." appears and stays visible for the whole transcription
- after processing, the transcribed text is inserted into the active text field (as if the user had typed it)
- the notification "⏳ Transcribing..." disappears

## Specs

### Functional specs

- the user starts dictation by running the script and stops it by running it again
- the user can configure a keyboard shortcut if they want (outside the script scope)
- the transcribed text is inserted into the active field without sending an equivalent "Enter" keypress
- the user can cancel an ongoing recording
- status notifications replace each other (only one visible notification at a time)

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
  `transcribers/` (`transcribers/groq`)
- a transcriber is a plain executable script, so it can fetch its credentials wherever the user
  keeps them (password manager, secrets store), as long as nothing prompts: the tool runs from a
  keyboard shortcut, with no terminal
- the plugin converts audio to text and nothing else: the core owns notifications and
  keyboard injection, so its code path does not depend on the chosen backend
- resolved with `command -v`: `$TINY_DICTATE_TRANSCRIBE` if set, else `tiny-dictate-transcribe` on
  `PATH`; an unresolvable command is reported to the user before any recording starts

### Runtime model

- written in bash, no persistent state, no daemonization
- one **session process** per dictation owns the whole lifecycle: waiting for the recorder,
  transcribing, injecting, and the notification spinner. `start` forks it, it cleans up after itself
- session state (recorder/encoder/session PIDs, audio file, spinner stop file) lives in a session
  directory under the XDG runtime directory, created atomically with `mkdir`: that directory *is*
  the claim on the microphone, so two dictations can never run at once
- `start` records the recorder PID **before** forking the session process, and the session waits for
  that PID: there is no window in which the session could believe the recorder already finished
- the notification has exactly one writer at a time: any process about to write must first stop the
  spinner and wait for it to exit, so a killed spinner can never leave a stale notification on screen
- `stop` signals the recorder only, never the encoder: the encoder must outlive it to flush the last
  frames, and the session waits for both to be gone before it reads the audio file
- sending a signal must never remove the pidfile it was read from: the session relies on that pidfile
  to tell whether the recorder is still alive

### Technical specs

- audio recording via `arecord` (S16_LE, 16 kHz, mono), encoded to MP3 on the fly with `lame`
- transcription via the plugin command described above: `$TINY_DICTATE_TRANSCRIBE` if set, else
  `tiny-dictate-transcribe` on `PATH`; the reference implementation is `transcribers/groq`
  (Groq Whisper), installed under that default name
- keyboard result injection: copy text to **CLIPBOARD and PRIMARY** (`wl-copy` and `wl-copy --primary`) followed by `ydotool key Shift-Insert`
  - CLIPBOARD for modern applications (VS Code, browsers)
  - PRIMARY for classic Unix applications (terminals, xterm, vim)
- notifications via `dunstify`
  - use notification IDs to replace and close the current notification (tags don't work for closing notifications)
- the session directory is a subdirectory of the XDG runtime directory and is removed when the
  session ends

## Docs

dunst:

- <https://dunst-project.org/documentation/>
- <https://dunst-project.org/documentation/dunst/>
- <https://dunst-project.org/documentation/guides/>
- <https://dunst-project.org/documentation/dunstify/>
- <https://dunst-project.org/documentation/faq/>
- <https://wiki.archlinux.org/title/Dunst>
