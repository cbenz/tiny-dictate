# The clipboard and the paste

## Both selections

The transcript is written to the **CLIPBOARD and the PRIMARY** selection, then injected as a
keypress:

- CLIPBOARD (`wl-copy`) for modern applications (VS Code, browsers)
- PRIMARY (`wl-copy --primary`) for classic Unix applications (terminals, xterm, vim)

Writing both is what makes the paste land wherever the user is typing, and it leaves the text
available on both selections if the injected keypress misses.

`wl-copy` and `wl-paste` must both be on `PATH`. Their absence is reported before anything runs, like
any other missing dependency.

## The injected keypress

The transcript is inserted with `ydotool`, which replays a `Shift+Insert` chord at the input layer:

- `ydotool key 42:1` (42 = Left Shift, pressed)
- `ydotool key 110:1 110:0` (110 = Insert, pressed and released)
- `ydotool key 42:0` (Shift released)

A short delay (`LLM_SHIFT_INSERT_HOLD_DELAY_SEC`, 0.04 s by default) separates the presses, so the
chord is not collapsed by a target that samples the keyboard faster than the events arrive. The
release of Shift is always attempted, so a failure in the middle cannot leave the modifier stuck.

Wayland has no API to insert text into another client, which is why the tool copies and injects
rather than typing. The chord carries no `Enter`: the text is inserted into the active field, and
nothing is submitted. When the injection fails, the session notifies `❌ Could not paste the
transcription` with critical urgency; the text is still on both selections, so it can be pasted by
hand.

## Decisions

- [ADR-0007: the clipboard lease](../adr/0007-the-clipboard-lease.md) keeps the transcript on the
  clipboard for a minute instead of leaving it there, and keeps it out of clipboard histories. The
  injected `Shift+Insert` chord stays the injection mechanism.
- [ADR-0008: the log](../adr/0008-the-log.md) makes the paste, successful or not, one line in the
  journal.
