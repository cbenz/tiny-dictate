# Every dictation leaves a session on disk

One directory per dictation under the state directory (`$XDG_STATE_HOME/tiny-dictate/sessions/<id>`,
the id being the local start timestamp): `audio.mp3` written from the first frame, `transcript.txt`
written by the core and never by the transcriber, `transcript.raw.json` written by the transcriber
and stored verbatim, and `metadata.json` holding the id, the start, the end, the duration, the audio
bytes, the outcome, the paste, the transcriber and the error. The metadata is rewritten atomically at
each stage, so a session left on `recording` is a session that died mid-recording.

A long dictation was lost because the audio lived in the runtime directory and was removed on the way
out. Writing to durable storage from the first frame makes a crash, a `kill` or a power loss cost at
most what has not been written, and makes a failed dictation re-transcribable. The ephemeral side —
the claim directory, the fifo, the waveform channel, the state file — stays in the runtime directory,
where it belongs.

`metadata.json`'s `paste` is a boolean (`pasted`), not an instant: ticket #10 asks "whether the text
was pasted", so the field records the yes-or-no the rescue surface reads rather than when it
happened.
