# The pill and the waveform channel

## The presenter contract

The core draws nothing itself: the live status is delegated to a **presenter command**, the second
plugin, and the second one that is a plain executable script. It receives:

- `$1`: the path of the **state file**, holding one word: `recording`, `transcribing`, `cancelled`,
  or `stop`
- `$2`: the pid of the session that owns the pill
- `$3`: the path of the **waveform file**, the input waveform the pill draws while recording
- `$4`: the command that re-invokes the tool, so a presenter that offers a clickable affordance can
  run what the keyboard shortcut runs (optional; a presenter given no command draws no buttons)

It leaves on its own when the state file disappears, when it reads `stop`, or when the process it was
given is gone. Three ways out, so a killed dictation cannot leave a pill frozen on screen. A screen
that shows `Recording` over a dead session is worse than no pill at all.

It must never take keyboard focus: the paste that ends a dictation goes to the focused window, and a
focusable pill would receive the text itself.

Failures are not the presenter's business. They are desktop notifications sent with `notify-send`,
once each, never replaced. There is nothing to keep in sync, so no notification id and no
`dunstify`. The cancellation is the presenter's business, because it is a
[verdict](cancel-and-verdict.md) about the dictation itself.

The presenter is spawned once per dictation and never replaced. Changing state is a write to the
state file it watches, and the session is the only writer of that file — so the presenter is the only
writer of the screen, and no frame of it can land after it is gone. The session waits for the
presenter to be gone before it notifies anything.

The presenter is resolved with `command -v` on `$TINY_DICTATE_PRESENT` when set, else on
`tiny-dictate-present` on `PATH`. An unresolvable presenter is reported to the user before any
recording starts, and the same for a transcriber that cannot be resolved.

## The waveform channel

The live status shows the captured waveform while recording, so a muted or misrouted microphone shows
on the pill as a flat line before the transcription comes back empty.

The waveform file holds the input waveform: one line per 96 ms of audio, 24 slices of 4 ms, each
reduced to the lowest and highest sample of the slice (48 integers per line). It is appended while
the recorder runs, exists for the whole session, and is empty before recording starts.

It is a **copy of the stream and never the stream itself**: a waveform channel that dies cannot
interrupt a dictation, and a presenter that ignores `$3` is still a valid presenter. The core
publishes raw samples and no level — what the bars are drawn *against* is the presenter's business.

The core slices the stream by teeing it on the way to the encoder (`tee -p`) and reducing each slice
with `od` and `awk`, so the microphone is opened once and the meter sits beside the voice path
rather than in it. The `-p` is load-bearing:
without it a write error to the pipe closes `tee`, the recorder takes `SIGPIPE`, and a dead meter
costs the dictation. `tests/run.sh` has a case for exactly that.

## The reference implementation

`src/presenters/layer-shell` draws a pill anchored to the bottom of the screen with GTK4 and
gtk4-layer-shell. It draws the input waveform in it while recording, normalized on the level of the
voice around it: the bars fill the meter and what is left to see is how the voice moves, which is
what a pill can show at ten frames per second whatever the microphone gain happens to be. A window
too quiet to be a voice is drawn small rather than gigantic, and a `$3` that is absent or unreadable
falls back to the spinner.

It needs a compositor with `wlr-layer-shell` support (sway, Hyprland, niri, KDE) and exits with an
error on GNOME/Mutter, where the dictation still works but nothing is shown.

Details that make it work, and that a replacement should account for:

- gtk4-layer-shell talks to the compositor through `libwayland-client`, and it only gets the
  protocol wired up when the loader has loaded it *before* that library. A plain import is too late:
  the script re-executes itself with `LD_PRELOAD` set. Without it, a compositor that supports
  layer-shell answers "layer-shell is not supported".
- The window is on the overlay layer, anchored to the bottom edge, with `exclusive_zone` set to `-1`
  (never reserve space) and the keyboard mode set to `none` (never take focus), with a bottom margin
  that clears a bar sitting there.
- It polls every 100 ms, which bounds how long a pill can outlive the session that owns it.
- `recording` carries no word: the waveform is the whole message, and a pill that announces the
  state the user just asked for announces nothing. `transcribing` carries its word. `cancelled` is a
  verdict, not a live state, so the spinner stops and it carries an emoji.
- The waveform is a soft white (`#e4e4e7`, zinc-200), and takes its colour from
  `TINY_DICTATE_PRESENT_COLOR` (any `#rgb` or `#rrggbb`) when the environment names one:

  ```sh
  TINY_DICTATE_PRESENT_COLOR='#285577' tiny-dictate toggle
  ```

  The pill draws over the user's screen, so its colour belongs to their theme.

- The pill carries one button at each end: a cross on the left cancels the dictation, a check on the
  right stops it and transcribes it. Each is drawn only where it means something — the cross while
  the dictation is live, the check only while recording, and neither by a presenter given no command.
  A button runs the command it was handed rather than writing the verdict into the state file: the
  session owns that file, and a verdict on its own would leave the recorder running.

Plug in anything else — a notification-based presenter, a bar module, your own script — by pointing
`TINY_DICTATE_PRESENT` at it. The four arguments above are the whole interface: nothing in the core
knows what a pill is.

## References

- gtk4-layer-shell: <https://github.com/wmww/gtk4-layer-shell>
- its linking notes, which explain the `LD_PRELOAD` requirement:
  <https://github.com/wmww/gtk4-layer-shell/blob/main/linking.md>

## Decisions

- [ADR-0001: one Rust binary](../adr/0001-single-rust-binary.md) moves the pill into the core binary
  as its `_present` mode, for the startup time.
- [ADR-0006: the limit and the bar](../adr/0006-the-limit-and-the-bar.md) adds a progress bar under
  the meter, driven by the transcriber's audio limit.
