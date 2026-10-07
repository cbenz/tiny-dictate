# The pill and the waveform channel

## The presenter

The core draws nothing itself: the live status is drawn by a **mode of the same binary**,
`_present`, spawned by the session as its child process. It links GTK4 and gtk4-layer-shell
directly, so there is no presenter to install and no `LD_PRELOAD` re-exec: the loader has the
layer-shell library before `libwayland-client` by the order the crate's dependencies are linked in.
A compositor without layer-shell support gets no pill, and the dictation runs untouched.

The session hands it:

- `$1`: the path of the **state file**, holding one word: `recording`, `transcribing`, `cancelled`,
  or `stop`
- `$2`: the pid of the session that owns the pill
- `$3`: the path of the **waveform file**, the input waveform the pill draws while recording
- `$4`: the command that re-invokes the tool, so the pill's buttons run what the keyboard shortcut
  runs (optional; a pill given no command draws no buttons)

`$5` is the recording's limit in seconds, and is empty when the transcriber declared none (see
[the audio limit](transcriber.md)). From it the pill draws a progress bar under the meter: 3 px high,
on the meter's width, plain colour until 80 % of the limit has elapsed, amber from 80 to 95, red
above. The elapsed time is the pill's own monotonic clock, started when it first reads `recording` —
the session publishes no timer tick. The bar is absent when there is no limit, and it leaves the
screen with the meter once the state is no longer `recording`
([ADR-0006](../adr/0006-the-limit-and-the-bar.md)).

It leaves on its own when the state file disappears, when it reads `stop`, or when the process it was
given is gone. Three ways out, so a killed dictation cannot leave a pill frozen on screen. A screen
that shows `Recording` over a dead session is worse than no pill at all.

It never takes keyboard focus: the paste that ends a dictation goes to the focused window, and a
focusable pill would receive the text itself.

Failures are not the pill's business. They are desktop notifications sent with `notify-send`, once
each, never replaced. There is nothing to keep in sync, so no notification id and no `dunstify`. The
cancellation is the pill's business, because it is a [verdict](cancel-and-verdict.md) about the
dictation itself.

The pill is spawned once per dictation and never replaced. Changing state is a write to the state
file it watches, and the session is the only writer of that file — so the pill is the only writer of
the screen, and no frame of it can land after it is gone. The session waits for the pill to be gone
before it notifies anything.

`TINY_DICTATE_PRESENT_COMMAND` names another command to run in its place, with the same arguments.
It exists for `tests/run.sh`, which stubs the pill like every other collaborator; it is internal, and
documented in [`../SPECS.md`](../SPECS.md) only.

## The waveform channel

The live status shows the captured waveform while recording, so a muted or misrouted microphone shows
on the pill as a flat line before the transcription comes back empty.

The waveform file holds the input waveform: one line per 96 ms of audio, 24 slices of 4 ms, each
reduced to the lowest and highest sample of the slice (48 integers per line). It is appended while
the recorder runs, exists for the whole session, and is empty before recording starts. Only whole
lines are written: the pump holds a partial slice or line back until it is complete, so the pill
never reads half a line.

It is a **copy of the stream and never the stream itself**. The microphone's stream is opened once:
`arecord` writes it into the `_pump` process of the same binary, which forwards it into the fifo the
encoder reads while, beside it, reducing each slice and appending the waveform lines. Both writes are
best-effort: an error is dropped and the pump goes on draining its input, so neither a dead encoder
nor a dead meter can close the recorder's pipe under it — the guarantee `tee -p` used to carry, now
that there is no shell pipeline to carry it. `tests/run.sh` has a case for exactly that.

The core publishes raw samples and no level — what the bars are drawn *against* is the presenter's
business.

## The reference implementation

`src/pill/` draws a pill anchored to the bottom of the screen with GTK4 and gtk4-layer-shell. It is
part of the binary; `_present` is its entry point. It draws the input waveform while recording,
normalized on the level of the voice around it: the bars fill the meter and what is left to see is
how the voice moves, which is what a pill can show at ten frames per second whatever the microphone
gain happens to be. A window too quiet to be a voice is drawn small rather than gigantic, and a
waveform file that is absent or unreadable falls back to the spinner.

It needs a compositor with `wlr-layer-shell` support (sway, Hyprland, niri, KDE) and exits with an
error on GNOME/Mutter, where the dictation still works but nothing is shown.

Details that make it work:

- The window is on the overlay layer, anchored to the bottom edge, with `exclusive_zone` set to `-1`
  (never reserve space) and the keyboard mode set to `none` (never take focus), with a bottom margin
  that clears a bar sitting there.
- It reads the state file, the waveform and the clock every 100 ms, which bounds how long a pill can
  outlive the session that owns it.
- The session sets `GSK_RENDERER=cairo` on the pill only, which puts the first frame on screen in
  about a tenth of a second instead of a third. It is never set for the interactive commands.
- `recording` carries no word: the waveform is the whole message, and a pill that announces the
  state the user just asked for announces nothing. `transcribing` carries its word. `cancelled` is a
  verdict, not a live state, so the spinner stops and it carries an emoji.
- The waveform is a soft white (`#e4e4e7`, zinc-200). Making it configurable is the configuration
  file's business, and comes with it.
- The pill carries one button at each end: a cross on the left cancels the dictation, a check on the
  right stops it and transcribes it. Each is drawn only where it means something — the cross while
  the dictation is live, the check only while recording, and neither by a pill given no command. A
  button runs the command it was handed rather than writing the verdict into the state file: the
  session owns that file, and a verdict on its own would leave the recorder running.

## References

- gtk4-layer-shell: <https://github.com/wmww/gtk4-layer-shell>

## Decisions

- [ADR-0001: one Rust binary](../adr/0001-single-rust-binary.md) moves the pill into the core binary
  as its `_present` mode, for the startup time.
- [ADR-0006: the limit and the bar](../adr/0006-the-limit-and-the-bar.md) adds a progress bar under
  the meter, driven by the transcriber's audio limit.
