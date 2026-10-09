# The browser takes crossterm and unicode-width, not ratatui

`session browse` draws the kept sessions on a terminal: it needs raw mode, the alternate screen and
key events, and it needs display widths to cut a line without cutting a wide character in two.
Nothing in the tool owned a terminal before: the pill is a layer-shell surface, and every command
line writes plain lines.

The browser therefore takes two dependencies. `crossterm` owns the terminal — raw mode, the
alternate screen, the key decoding — and the drawing is hand-rolled on top of it, following the
pill's split between a pure frame module and an adapter that owns the surface. `unicode-width` gives
the display width of a character, which is what decides where a line is cut.

## Considered options

`ratatui` was not taken. The screen uses four of its widgets — a list, a paragraph, a footer and the
alternate screen — and none of its cell diffing: it would add roughly twenty crates for a frame the
browser redraws whole. What the suite can observe would not change either: a cell diff is invisible
on the pseudo-terminal the browser is driven on.

Hand-rolled `termios` and `select` were not taken, though the repository hand-rolls its JSON reader,
its command-line parser and its TOML reader. Those are pure functions, where being wrong is a test
failure; key decoding and raw-mode restoration are the two places where being wrong breaks the user's
terminal, and crossterm is maintained against terminals this machine does not have.

## Consequences

The binary gains `crossterm` (with `mio`, `signal-hook` and `rustix` on Unix) and `unicode-width`.
They are plumbing of the same kind as `libc` and GTK: taken for something the tool cannot own
itself, not for a convenience. Only `session browse` and bare `session` on a terminal touch any of
it, so a dictation pays nothing.

The terminal is given back through a guard and a panic hook, so `q`, `ctrl-c`, an error and a crash
all leave raw mode and the alternate screen behind. The suite drives the browser on a
pseudo-terminal, compares the terminal's state before and after the run, and covers the refusal of a
stdout that is not a terminal — and the terminal it does have after that failure.
