# The paste waits for the offer to serve the text

`wl-copy --foreground` sets the selection only once it has read its stdin and completed a display
roundtrip, and it never reports that to the process that wrote the text: closing the pipe proves the
text was read, not that the offer is up. Between the two, the selection still belongs to the previous
offer, which `--foreground` keeps alive for the whole lease — 60 s by default, and the lease is
exactly what ADR-0007 bought. Injecting the chord in that window pastes the previous dictation's
text.

The paste therefore reads PRIMARY back with `wl-paste --no-newline --primary`, compares it with the
text it wrote, and injects `Shift+Insert` only once they agree. The wait is bounded at 500 ms and
polls; it never waits out the lease. An offer that never serves the text has its chord skipped rather
than pressed on a stale selection: the text is on the clipboard, the session reports the `paste
failed` line ADR-0008 asks for, and the user gets the notification a failed paste already carries.

## Considered options

Waiting inside `clipboard_write`, beside the offer, would make `session transcribe` pay the wait too,
where no chord follows and the read-back buys nothing.

Killing the previous offer before writing the new one — holding the `wl-copy` pid in the runtime
directory, outside the `session/` subdirectory the session guard wipes — removes the race without a
read-back and without `wl-paste`, but adds a runtime file and a lifecycle to maintain, and it only
covers the offers the tool itself made. A selection held by any other client has the same problem and
no pid to kill; the ordering has to hold for those too.

Bare `wl-copy` was the ordering the bash implementation relied on: without `--foreground` it forks
from `did_set_selection_callback` and returns only once the compositor confirmed. ADR-0007 needs
`--foreground` for the lease, so the guarantee is rebuilt in the tool.

## Consequences

`wl-paste` becomes a dependency of the paste path, one the port had dropped: the bash version
required both commands, the port only required `wl-copy`. A machine without `wl-paste` cannot be
asked whether the offer is up, and pastes as it did before this wait existed — losing the text to a
missing read-back would cost more than the ordering is worth.

The window is also why `tests/run.sh` polls the clipboard rather than reading it right after the
command that wrote it returned: the race the suite worked around is the one the tool now waits out,
and the suite has a case that opens it deliberately.
