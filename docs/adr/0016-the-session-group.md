# Every verb that touches a session lives under one group

The command line gains a `session` group, and every verb that reads or writes a kept session moves
under it: `session list`, `session transcribe <id|last> [--print]`, `session delete <id|last>`, and
the verbs the sessions gain later. The root keeps `start`, `stop`, `cancel`, `toggle`, `status` and
the help. The two verbs that used to live at the root, `history` and `transcribe`, are removed rather
than kept as aliases: a removed verb is an unknown argument, so the usage is printed and the exit is
non-zero, and the user learns the new name instead of finding two names for one thing. The verb names
of [ADR-0011](0011-command-line.md) are superseded by these.

`session` with no sub-verb takes no argument at all: it lists the kept sessions today, and it is the
word the browser takes over on a terminal. Bare, the word means "the sessions"; anything after it is
a sub-verb or an unknown argument, never an id.

## Considered options

Keeping `history` and `transcribe` as aliases would spare the shortcuts already bound to them, but the
grouping exists to end the drift of one verb at the root and one under `session`, and an alias is a
second name to keep in step with the first. The removal has to be loud for the same reason: the usage
is what tells a user who types `history` what the verb became.

Letting bare `session` take an id — `session <id|last>` for the listing of one session — was rejected
because the bare word is the entry point, not a verb with an argument: on a terminal it opens the
browser, and an argument there would mean two different things depending on what stdout is.

## Consequences

Every surface that shows a session reads one view module, so the row of the listing and the human
duration cannot drift apart, and the durations a person reads are human everywhere (`4:32`,
`1:04:32`, `-` for a session that died mid-recording). A session whose metadata cannot be read is no
longer skipped by the listing: it is a row of its own, because it is exactly the session
`session delete` can remove.
