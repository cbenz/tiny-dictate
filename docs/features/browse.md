# The browser

`tiny-dictate session browse` reads the kept sessions on a terminal: the list on the left, the
selected session's detail in a pane on the right, and the keys that move through both.
`tiny-dictate session` on its own is the same surface when stdout is a terminal, and the plain
listing when it is a pipe, so one command serves a shortcut and a script.

## The list

The list comes first, newest first, one row per session: the id, the human duration, the outcome and
the first words of the transcript or the error, exactly the row [`session list`](sessions.md) prints,
read through the same view. A session whose metadata cannot be read is a row of `?` and is
selectable like any other. The selected row carries a `>` marker and is drawn in reverse video.

## The pane

Beside it, the pane reads the selected session in full through the same shared view `session show`
prints, so the two surfaces cannot drift: the fields -- `id`, `path`, `outcome`, `started`, `ended`,
the human `duration`, `audio bytes`, `pasted`, `transcriber` and `error` -- then the transcript. The
transcript is wrapped at the pane's width, counting display width rather than characters, so a
transcript holding wide characters or emoji stays aligned, and it scrolls a page at a time. A
session that holds no transcript says `no transcript`; a session whose metadata cannot be read shows
its id and its path and says the metadata is unreadable; an empty history draws an empty pane rather
than a stale one. Moving the selection reloads the pane on the session it lands on, from the top.

A terminal too narrow or too short for both panes is not a panic: below the width and height two
panes need to stay readable, the pane gives way and the list keeps the whole width. The footer names
the keys that mean something right now, so it does not offer the page keys where there is no pane to
scroll.

| key | what it does |
|-----|--------------|
| `↑` / `↓`, `k` / `j` | move the selection, reloading the pane on the session it lands on |
| `Home` / `End` | jump to the newest or the oldest session, and take the transcript to that end |
| `PgUp` / `PgDn` | scroll the transcript by a page |
| `r` | reload the list, keeping the selection on the same session while it is there |
| `q`, `ctrl-c` | leave |

An empty history is an empty screen and not an error: the browser says nothing is kept and leaves on
`q` with exit code 0.

The browser takes no command lock and reads files only: it opens while a dictation runs, and it works
on a machine without the encoder or the clipboard tools. It never refreshes on its own — `r` is the
only reload — so nothing on screen changes under the user's hands.

A stdout that is not a terminal is refused with a message on stderr and a non-zero exit: escape
sequences written into a pipe would corrupt whatever reads it.

Two levels, following the pill's precedent
([ADR-0017](../adr/0017-the-browser-takes-crossterm.md)): `browse::frame` is the pure computation —
the rows, the selection, the selected session's detail, the scroll offset and the terminal's size
in, what one frame draws out — and `browse` is the adapter that owns the terminal: raw mode, the
alternate screen, the key decoding and the screen itself. The terminal is given back through a guard
and a panic hook. The frame module is unit-tested without a terminal; the adapter is driven by the
pseudo-terminal cases in [tests](tests.md).

## Decisions

- [ADR-0016: the session group](../adr/0016-the-session-group.md) puts `browse` under `session` and
  makes bare `session` browse on a terminal.
- [ADR-0017: the browser takes crossterm](../adr/0017-the-browser-takes-crossterm.md) records the
  terminal dependency, the hand-rolled drawing and the guard.
