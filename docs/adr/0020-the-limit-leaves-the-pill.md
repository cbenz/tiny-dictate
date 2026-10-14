# The limit leaves the pill

The bar half of [ADR-0006](0006-the-limit-and-the-bar.md) is superseded; that ADR stays as written.
The pill draws no bar — the meter is alone in the space and centred — and the pill holds neither the
audio limit nor a clock. The limit is a session concern: the presenter takes the state file, the
owner pid, the waveform channel, the command and the colour, nothing more, and the deadline lives
where the recording is owned (`src/session.rs`, `src/text.rs`).

ADR-0006's promise is corrected with it. A dictation stopped by its declared audio limit is
transcribed and pasted like any other, and the file handed to the transcriber stays strictly under
the limit that transcriber declared, as long as the limit is large enough for the guard to cover the
margin below — about 32 000 bytes. Below that the guard is half the limit, and the margin can carry
the file past it: such a limit is smaller than one recording can honour. Ten seconds of audio —
80 000 bytes at the encoder's 64 kbit/s — are held back as a guard, and never more than half the
declared limit: a limit too small to carry ten seconds keeps half of itself, so the dictation still
records something. Zero bytes stays no limit at all. The guard is subtracted where a byte limit
becomes a duration
(`text::limit_from_bytes`), the single place the deadline is defined, so the guard cannot drift from
the stop.

The margin the guard covers, in the order the code spends it: the deadline is taken when the session
process takes over, after the recorder was started by the process that forked it; the limit is
checked every hundred milliseconds; the SIGINT the recorder is given takes a moment to be acted on;
and the fifo can hold up to two seconds of PCM in flight. The reference limit, 25 000 000 bytes, is
3125 seconds of audio: the recording stops at 3115 seconds and the MP3 lands a few kilobytes under
the limit.

## Consequences

A limit stops the recording early, so the tail of a long dictation is what it costs: ten seconds off
a fifty-minute one, never the head. The transcriber contract is untouched — the probe and the number
it prints belong to the plugin, and the margin belongs to the core — and a transcriber that declares
no limit is still unbounded. The other half of ADR-0006 stands: the limit is a guard, not a goal,
and nothing warns as it approaches, because the pill no longer holds the limit or the clock a
warning would need.
