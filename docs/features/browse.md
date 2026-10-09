# The browser

`tiny-dictate session browse` reads the kept sessions on a terminal: the list on the left, the
selected session's detail in a pane on the right, and the keys that move through both, copy what a
session holds and delete one behind a confirmation. `tiny-dictate session` on its own is the same
surface when stdout is a terminal, and the plain listing when it is a pipe, so one command serves a
shortcut and a script.

## The list

The list comes first, newest first, one row per session: the id, the human duration, the outcome and
the first words of the transcript or the error, exactly the row [`session list`](sessions.md) prints,
read through the same view. A session whose metadata cannot be read is a row of `?` and is
selectable like any other. The selected row carries a `>` marker and is drawn in reverse video, the
footer names the keys that mean something right now, and the line above it carries the status of the
last action.

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

## The copy actions

`c`, `i` and `p` put the selected session's transcript, its id and its path on the clipboard, and
the browser stays open. The write goes through `text::copy_to_clipboard`, the one path a live
dictation, `session copy` and `session transcribe` take: both selections, the KDE password hint, and
the configured lease ([the clipboard](clipboard.md)). The browser re-implements none of that, and it
injects nothing: the `Shift+Insert` chord stays the live dictation's business, so a browser run
never touches `ydotool`.

The copy follows the selection, never the newest row. `c` copies the `transcript.txt` already on
disk, the same file `session copy` offers; a session that holds no word of transcript is reported on
the status line rather than copied empty, and the browser never transcribes a session itself. Moving
the selection or reloading the list clears the status line; each copy action sets it again, naming
what was copied. With nothing selected, `c`, `i` and `p` say so on that line and touch neither the
clipboard nor the keyboard. The status line takes its own row above the footer, so a message never
covers the rows or the pane.

| key | what it does |
|-----|--------------|
| `↑` / `↓`, `k` / `j` | move the selection, reloading the pane on the session it lands on |
| `Home` / `End` | jump to the newest or the oldest session, and take the transcript to that end |
| `PgUp` / `PgDn` | scroll the transcript by a page |
| `c` | copy the selected session's transcript |
| `i` | copy its id |
| `p` | copy its path |
| `d` | ask to delete the selected session, behind a confirmation |
| `r` | reload the list, keeping the selection on the same session while it is there |
| `q`, `ctrl-c` | leave |

## Deleting a session

`d` destroys nothing on its own: it asks first. The question names the selected session's id and
offers the two keys that answer it, `delete <id>? y/n`, and the footer narrows to those two keys
while it stands. Answering is a pure decision of the frame's `Confirm` state: only `y` and `n`
answer, and every other key answers nothing — the question stands, so a stray key can never destroy
a recording. `q` included: a pending question must be answered before the browser leaves.

`n` answers nothing at all: the question goes and the browser is where it was, the session
untouched. `y` removes the session directory and reloads the list, so the row disappears without
leaving the browser, and the status line names the session that went.

The removal is `rescue::remove_session`, the one place a session is destroyed: `session delete`
calls it too, so the dictation in flight is refused by one guard, in one sentence, on both surfaces.
The browser shows the refusal on the status line and removes nothing; it takes no command lock and
signals nothing, and the dictation runs on. A session whose metadata cannot be read is deletable
like any other, because the guard reads the session directory, not the metadata.

An empty history is an empty screen and not an error: the browser says nothing is kept and leaves on
`q` with exit code 0.

The browser takes no command lock and reads files only: it opens while a dictation runs, and
everything but a copy action works on a machine without the encoder or the clipboard tools. Looking
at the sessions never depends on dictating; only a copy asks for `wl-copy`, exactly as `session
copy` does, and its absence costs the copy, never the browsing. The browser never refreshes on its
own — `r` is the only reload — so nothing on screen changes under the user's hands.

A stdout that is not a terminal is refused with a message on stderr and a non-zero exit: escape
sequences written into a pipe would corrupt whatever reads it.

Two levels, following the pill's precedent
([ADR-0017](../adr/0017-the-browser-takes-crossterm.md)): `browse::frame` is the pure computation —
the rows, the selection, the selected session's detail, the scroll offset, the status line and the
terminal's size in, what one frame draws out — and `browse` is the adapter that owns the terminal:
raw mode, the alternate screen, the key decoding and the screen itself. The terminal is given back
through a guard and a panic hook. The frame module is unit-tested without a terminal; the adapter is
driven by the pseudo-terminal cases in [tests](tests.md).

## Decisions

- [ADR-0016: the session group](../adr/0016-the-session-group.md) puts `browse` under `session` and
  makes bare `session` browse on a terminal.
- [ADR-0017: the browser takes crossterm](../adr/0017-the-browser-takes-crossterm.md) records the
  terminal dependency, the hand-rolled drawing and the guard.
