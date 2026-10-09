# The clipboard lease

The transcript is offered on both selections with `wl-copy --foreground --sensitive`, wrapped in
`timeout <clipboard_seconds>` (60 by default, `0` meaning no expiry): the offer lives for the lease,
serves as many pastes as needed, and dies when the lease ends or when another client takes the
selection over. `session transcribe` uses the same lease. The old `wl-copy` plus `ydotool Shift+Insert` chord
stays the injection mechanism: Wayland has no API to insert text into another client.

`--paste-once` is deliberately not used. A window that steals focus, a paste into the wrong window or
a clipboard manager reading the data would cost the user the text.

The KDE password hint (`x-kde-passwordManagerHint`, presence only) is what keeps the transcript out
of clipboard histories; it is also what stops a clipboard manager from consuming the offer the lease
depends on. The text is written to the session before any clipboard write.
