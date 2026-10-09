# Cancelling keeps the session, destroying is a separate verb

`cancel` writes its verdict, stops the recorder and leaves the session marked `cancelled`; nothing
about a cancelled dictation is transcribed or pasted. Destroying is a separate verb,
`tiny-dictate session delete <id|last>`, available only from the command line.

Cancelling stays a way to throw something away — the text never reaches the keyboard — but a cancel
by accident is no longer a lost dictation, because the recording is still there to
`tiny-dictate session transcribe` later ([ADR-0016](0016-the-session-group.md) moved the verb under
`session`).
