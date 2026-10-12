# The meter is relative, and the level is not the user's to tune

The pill draws the input waveform, and while a dictation records it is the whole message on screen:
its job is to answer "does it hear me?" before the transcript comes back. It draws each bar against a
reference that follows the voice, floored by a noise gate, and never against full scale
([features/pill.md](../features/pill.md)).

The level is the tool's business, not the user's. The source is set to its base volume before each
dictation ([ADR-0014](0014-the-microphone-at-its-base-volume.md)) and the tool never writes above
it. A recording that clipped anyway — the level was raised again while it ran — is reported once the
text is in, by a journal line and one notification, rather than prevented by asking the user to
watch a meter ([features/microphone.md](../features/microphone.md)).

## Considered options

**An absolute meter**, dBFS from full scale, the way a mixing desk reads. It reads well for the
microphone it was tuned on and lies about every other one: on the reference machine the internal
microphone peaks 30 dB under full scale at the base volume, so the meter is a flat line — which is
exactly what the pill used to show, and what leads a user to raise the level until the recording
clips. The same meter on a USB microphone with make-up gain reads saturated.

**A dB scale against the followed reference.** With the reference on the voice, room tone sits about
20 dB under it, and a 36 dB scale draws that at 44 % of the meter: the meter would sit half full in a
silent room and pin for anything above a whisper.

**A measured noise floor**, taken from the first ticks of each recording instead of a constant gate,
was rejected for its sequencing: the measurement would have to happen while the user is already
speaking.

**Writing a calibrated level to the source**, the way the base is written. It leaves a durable mark
on the machine that every other application pays for, and it makes the tool the author of a clipping
it cannot bound — a louder sentence clips what no calibration foresaw. It is not needed either: the
level the base gives transcribes ([features/microphone.md](../features/microphone.md)).

**`hyprwhspr`** reaches the same conclusion from the other end, and was read as prior art: its meters
carry an adaptive `AutoGain` (with a target peak, a bounded gain, and a noise gate under which
nothing is amplified) and it never runs `pactl set-source-volume`. It leaves the hot end open — a
user at 80 % records clipped, with nothing in the tool to say so — where this tool pins the level at
the base and reports when the level went over it anyway.

## Consequences

The meter cannot be used to judge a level, which is the point: it shows that the microphone is live,
not how loud the room is. The gate is a constant of the drawing and not a configuration key, so no
setting can turn the meter back into a level.
