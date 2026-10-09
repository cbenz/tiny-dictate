# The clipboard and the paste

## Both selections

The transcript is written to the **CLIPBOARD and the PRIMARY** selection, then injected as a
keypress:

- CLIPBOARD (`wl-copy`) for modern applications (VS Code, browsers)
- PRIMARY (`wl-copy --primary`) for classic Unix applications (terminals, xterm, vim)

Writing both is what makes the paste land wherever the user is typing, and it leaves the text
available on both selections if the injected keypress misses.

`wl-copy` is looked up by the command that offers the text, not before anything runs: the dependency
checks live in the commands that need them, so `session list` and `session transcribe --print` run on
a machine without a clipboard tool, and a missing one costs the paste, never the recording.

## The lease

Both offers carry the KDE password hint and live for the lease
([ADR-0007](../adr/0007-the-clipboard-lease.md)):

```
timeout <clipboard_seconds> wl-copy --foreground --sensitive
timeout <clipboard_seconds> wl-copy --primary --foreground --sensitive
```

`--foreground` keeps `wl-copy` in the process `timeout` wrapped, so the offer serves the selection
for the whole lease, as many pastes as the user needs, and the wrapper is what ends it. When the
lease ends the transcript is gone from the clipboard; when the user copies something else in the
meantime, `wl-copy` exits by itself and leaves that newer copy alone. `--paste-once` is deliberately
not used: a window that steals focus, a paste into the wrong window or a clipboard manager reading
the data would cost the user the text.

`--sensitive` sets `x-kde-passwordManagerHint` (presence only), which is what keeps the transcript
out of a clipboard history, and what stops a clipboard manager from consuming the offer the lease
depends on.

The duration is `clipboard_seconds`, read from the [configuration file](configuration.md), `60` by
default; `0` means no expiry, and then no `timeout` wrapper at all. The text is written to the
session before any clipboard write, and every clipboard write goes through the one function, so the
browser's `c`, `i` and `p`, `session copy` and `session transcribe` get the same lease as a
dictation. The browser injects nothing: it copies, and the `Shift+Insert` chord stays the live
dictation's business.

## Waiting for the offer

`wl-copy --foreground` sets the selection a display roundtrip after its stdin was written, and never
says so: the write returning proves the text was read, not that the offer is up. Until it is, the
selection still belongs to the previous offer, which the lease keeps alive. A chord injected there
would carry the previous dictation's text.

`paste` therefore reads PRIMARY back with `wl-paste --no-newline --primary` and presses the chord
only once the read matches the text it wrote ([ADR-0015](../adr/0015-the-paste-waits-for-the-offer.md)).
The wait is bounded at 500 ms and polls; it never waits out the lease. An offer that never serves the
text is not injected from: the chord is skipped rather than pressed on a selection that may still
hold the previous text, and the failure takes the path a failed paste already takes. Without
`wl-paste` the read-back cannot be asked, and the chord is pressed as it was before the wait existed
— losing the paste would cost more than the ordering is worth.

`session transcribe` writes through the same offer and does not wait: no chord follows its write.
`session copy` does the same, on the transcript already on disk, running the transcriber first only
when the session has none.

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

`session transcribe` writes through the same path: it offers the rescued text on both selections and
says so
on stderr, or writes it to stdout with `--print`, which touches no clipboard at all.

## Decisions

- [ADR-0007: the clipboard lease](../adr/0007-the-clipboard-lease.md) keeps the transcript on the
  clipboard for a minute instead of leaving it there, and keeps it out of clipboard histories. The
  injected `Shift+Insert` chord stays the injection mechanism.
- [ADR-0008: the log](../adr/0008-the-log.md) makes the paste, successful or not, one line in the
  journal.
- [ADR-0015: the paste waits for the offer](../adr/0015-the-paste-waits-for-the-offer.md) makes the
  chord wait for the offer to serve the text, so it can no longer inject the previous dictation's.
