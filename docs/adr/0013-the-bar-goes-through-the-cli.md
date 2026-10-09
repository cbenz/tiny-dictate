# The bar goes through the CLI

The waybar module runs `tiny-dictate status` and maps the two words it prints, `idle` and `working`.
It does not read the state file. The state file is internal to the tool: the session writes it and
the pill draws it, and it is not an interface for a consumer outside the binary. A reader shipped in
`contrib/` would make it one by fact, and every added reader would widen what the tool may no longer
change without breaking someone else's bar.

The port named this the other way. #9's acceptance criterion asked for the module to read the state
file instead of spawning the tool every second; that criterion is reversed here, and
`contrib/waybar/tiny-dictate-module.jsonc` is restored to the version from before the commit that
satisfied it. Nothing is lost on screen: the bar showed two words before the bypass and shows two
after.

The word the bar cannot get is `cancelled`. The glossary separates `State`, the words a session
publishes for the pill — `recording`, `transcribing`, `cancelled`, `stop` — from `status`, the tool's
word, `idle` or `working`. A verdict is a `State` the pill draws, not a `status` the bar can read.

## Consequences

Every poll spawns the binary, which links GTK4: measured on the development machine, 41 ms per
invocation, against 30 ms for the bash script it replaces. That is the cost of the interface, and it
is accepted. The loader cost is not new and not this decision's: ADR-0001 already links the GUI
toolkit into every invocation, `status` included, and measured it.
