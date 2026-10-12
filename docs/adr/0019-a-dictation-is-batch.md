# A dictation is batch

The whole utterance is recorded, then transcribed once. The tool hands the transcriber a file and
reads its stdout back ([ADR-0002](0002-transcriber-stays-an-external-command.md)): one process per
dictation, one answer, and the text is pasted when it arrives.

Streaming transcription is a different contract, not an extension of this one: a long-lived process,
incremental text, partials that get revised, and key injection while the user is still speaking. It
would put four things on a clock the tool does not own — the transcriber plugin, the pill, the paste
and the clipboard lease — and none of them is built for it.

## Considered options

**Streaming from the start**, with the text injected as it is revised. Every transcriber worth
pointing this tool at takes a file: Groq Whisper, `whisper.cpp`, the local models, the reference
script. The plugin contract would have to change from "a command that is given an audio file" to
something carrying a socket and partial results, and the paste would become a sequence of edits into
a window the user is typing in rather than one insertion.

**The end of a dictation only, with a live preview on the pill** was left out of scope for the same
reason: a preview that can be revised is a second rendering of text the tool does not yet own, and
the pill shows state, not text.

## Consequences

The latency is the transcription of the whole utterance, and the pill carries no partial text: while
transcribing, it says `transcribing` and nothing else. A long dictation is bounded by the
transcriber's declared audio limit instead, at which the recording stops by itself
([ADR-0006](0006-the-limit-and-the-bar.md)).
