# One log line per event on stderr

Every event is one line on stderr, prefixed with the tool's name and, when the event concerns a
session, with its id; events that concern none — the prune, `session delete` — carry no id. The
events are the start, the stop, the cancel, the toggle with the effect it had, the recording started
and ended, the transcription started and ended, the text written, the paste succeeded or not, a late
cancel dropping the text, every failure, the prune and `session delete`.

The source of a command — keyboard, pill button or command line — is not distinguished. The encoder's
and recorder's own output is redirected out of the journal, because their chatter is not the tool's
timeline. There is no global log file: the session's metadata is the durable trace, and the journal
is the timeline.
