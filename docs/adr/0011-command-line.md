# The command line gains history, transcribe and session delete

`start`, `stop`, `cancel`, `toggle` and `status` stay as they are, and the command line gains
`history`, `transcribe <id|last> [--print]` and `session delete <id|last>`. `transcribe` copies the
text to the clipboard by default and `--print` writes it to stdout instead. An unknown id is reported
and removes nothing.

Dependency checks move into the commands that need them, so `history` and `transcribe` run on a
machine without the encoder or the clipboard tools. A command that cannot run without a dependency
reports it when it runs, not when the binary starts.
