# The capture fact is the recorder's

Whether a dictation captured anything was read from the size of the encoded file: an empty `audio.mp3`
meant a microphone that heard nothing. The reference encoder breaks that. At end-of-file it writes the
bytes owed for the time the recording lasted — 8000 for every second at 64 kbit/s — even when no
sample ever reached it, so a recording that captured nothing ends with a non-empty file; and a
recording that captured something ends with an empty one for as long as the encoder holds its frames.

The second case is not hypothetical. The session reads that size as soon as the recorder is gone, and
an encoder nothing has read from yet has written nothing: on a dictation stopped within a few hundred
milliseconds the session concluded `no audio was captured` while the encoder was alive, with the audio
still on its way. The suite's recorder race lost the audio in one iteration out of twenty-five on a
loaded machine, and in every iteration on a container whose pid 1 reaps nothing, where the verdict
came ten seconds late.

The fact now comes from the recorder's side of the pipeline. The pump writes a `captured` marker at the
first byte it copies — the twin of the `clipped` marker it already writes — and the session judges an
empty recording from that file's existence. It goes down before the pump writes anything else, the copy
that feeds the encoder and the waveform lines included, so neither a dead meter nor an encoder that died
first can hold it back. This is the one marker the recording pays for, which
[the invariants](../../AGENTS.md) say of nothing else, so the pump says the line when it cannot write
it.

The session waits for the encoder before its verdict only when the marker exists: with bytes in flight
the encoder has frames to flush, and with nothing captured there is nothing to flush, so waiting would
only delay a verdict about a file that will hold padding.

## Considered options

Judging from the size of the encoded file and waiting for the encoder unconditionally: measured, it
turns the recorder-failure case red — the padding makes an empty recording look transcribed, and the
tool pastes the transcription of silence.

Reading the waveform channel's file, which fills only once 96 ms of audio went by: cheaper, but the
meter is a report, and a report that dies must never cost a recording.

Asking the recorder for a byte count: `arecord` reports none, and the pump is the one process that sees
every byte anyway.

## Consequences

A marker that cannot be written now loses the dictation rather than a warning. The audio stays in the
session directory, and `session transcribe` or the browser picks it up again, so it is recoverable —
and the pump's line is what makes it diagnosable.

`audio_bytes` keeps meaning the size of the encoded file, so a failed session can carry a few thousand
bytes of padding next to `no audio was captured`. The metadata describes the file, the outcome describes
the dictation.
