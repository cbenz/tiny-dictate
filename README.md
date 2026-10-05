# tiny-dictate

Minimal desktop voice dictation tool: no daemon, no GUI, just keyboard shortcuts and notifications.

Opinionated workflow: press a keyboard shortcut to start recording, press it again to stop and transcribe (or cancel with another keyboard shortcut).

Based on the following tools:

- audio recording with `arecord`, encoded to MP3 on-the-fly with `lame`
- transcription with a command you provide: `tiny-dictate-transcribe` (Groq Whisper) by default
- result pasted into the active window via clipboard + `ydotool Shift+Insert`
- status notifications with `dunstify`

## Installation

### Dependencies

- `arecord` (alsa-utils)
- `curl`
- `lame`
- [dunst](https://dunst-project.org/)
- [wl-clipboard](https://github.com/bugaevc/wl-clipboard)
- [ydotool](https://github.com/ReimuNotMoe/ydotool)

### Install the scripts

Install both scripts in your PATH, for example `~/.local/bin`:

```bash
install -m 755 tiny-dictate tiny-dictate-transcribe ~/.local/bin
```

### Configure the transcription command

`tiny-dictate` has no transcription backend of its own: it calls a command you provide, which
receives the recorded audio file as `$1` and writes the transcribed text to stdout. The bundled
`tiny-dictate-transcribe` is the Groq Whisper implementation:

```bash
export GROQ_API_KEY=...   # create one at https://console.groq.com/keys
```

Plug another backend — another service, a local `whisper.cpp`, your own script — by pointing
`TINY_DICTATE_TRANSCRIBE` at it:

```bash
export TINY_DICTATE_TRANSCRIBE=~/.local/bin/my-transcriber
```

### Configure keyboard shortcuts

Example with i3/sway:

```text
bindsym $mod+backslash exec tiny-dictate toggle
bindsym $mod+Shift+backslash exec tiny-dictate cancel
```

## Usage

```text
Usage: tiny-dictate <command>

Commands:
  start   Start recording
  stop    Stop recording and transcribe
  cancel  Cancel recording
  toggle  Start if idle, stop if recording
  status  Show status (idle or working)
```

## Tests

```bash
tests/run.sh
```

The test suite stubs the recorder, the encoder, the transcription command, the notifier and the
keyboard injector: it needs no microphone, no notification daemon and no network.
