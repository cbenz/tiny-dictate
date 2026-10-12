# The microphone

A dictation records from the audio server's default source, at whatever level the user last left it
at. At 100 % the ADC saturates: the recording clips, the transcript degrades and the pill's waveform
reads as a flat block. The tool therefore sets that source to its **base volume** — the level at
which the server adds no gain, the neutral point of that machine's hardware — and unmutes it before
every dictation. The level stays there afterwards, so the next dictation starts sane and no
restoration step re-arms the saturation
([ADR-0014](../adr/0014-the-microphone-at-its-base-volume.md)).

A volume written to the source persists: the server records it and reapplies it at boot, so the tool
leaves a durable mark on the machine.

## When the source is set

`start` sets the source once the claim is taken and before the recorder opens. The claim is the
exclusive hold on the microphone, so the level belongs to the dictation that owns it: a second
`start` refused by the claim writes nothing to the source, and a recording in flight is never
disturbed. The base is read at that moment, so a default source that changed since the last
dictation — a headset came or went — is the source set, and the source recorded.

## What the tool runs

`pactl` is the only command this feature runs. The source is `@DEFAULT_SOURCE@`, where the ALSA
`default` PCM the recorder opens routes.

| command | why |
|---------|-----|
| `pactl get-default-source` | name the source the base volume is read from |
| `pactl list sources` | read that source's `Base Volume:` line |
| `pactl set-source-volume @DEFAULT_SOURCE@ <raw>` | write the base back, or the configured percentage when the server publishes none |
| `pactl set-source-mute @DEFAULT_SOURCE@ 0` | unmute the source |

The base is read raw and written raw: `pactl list sources` prints it as
`Base Volume: 6554 / 10% / -60.00 dB`, and `6554` is what `set-source-volume` is given. Nothing
round-trips through the rounded percentage or the dB string, so the level follows the machine rather
than a percentage someone remembered. On the reference machine the hardware gain is that dB plus
60: 10 % is 0 dB, 20 % is +18 dB and 100 % is +60 dB, which is what saturated the ADC.

A muted source is never a refusal: the tool unmutes it and dictates. Unmuting is unconditional and
no configuration key guards it, because a tool that cannot take the microphone cannot dictate.

## When the server answers badly

`pactl`'s output is parsed, not trusted, and an unreadable answer is a normal outcome rather than an
error. The listing is read as blocks of sources, and the base is the `Base Volume:` line of the
block whose `Name:` is the one `pactl get-default-source` printed: with several sources, another
source's base is never written on the default one. A server that answers without a base volume
line, or with a line the parser cannot read, is
answered with the configured [`input_level_pct`](configuration.md) — written as a percentage — so the
recording never starts at the level the user last left it at. A server that publishes no default
source and a listing holding exactly one source has that source's base read; a listing holding
several leaves nothing to pick by, and the configured level stands in. The key is never the target:
when the server publishes a base, that raw value is what is written. The source is unmuted all the
same, so a bad answer from the server is never silence, and never a refusal.

A server that cannot be reached at all — no `pactl` on `PATH`, no server running — writes nothing:
no volume and no unmute command are attempted, one log line says so, and the dictation records as
usual. A missing audio server command never costs a dictation.

A percentage is a stand-in for a base the server would not publish, not a level the user prefers: it
gives a different gain on every machine, which is exactly why it is the fallback and not the target
([ADR-0014](../adr/0014-the-microphone-at-its-base-volume.md)).

## When the level was not the base

The tool sets the source to its base before every dictation, so a recording clips only when the level
was raised again while it ran. The pill cannot show it: its meter is drawn against the voice itself,
so a saturated recording is drawn exactly like a loud one ([the pill](pill.md)). What the level did is
therefore read off the samples.

The pump has every byte of the stream in hand and already reduces each slice to its lowest and
highest sample, so the watch costs it one comparison: a slice whose highest sample is `32767`, or
whose lowest is `-32768`, sits at full scale. Three such slices in a row — 12 ms — are the level,
where one is a knock; the pump then creates the `clipped` file in the runtime directory, once, and
nothing else is counted: the file's existence is the fact.

The session reads that fact when the recording is over. It writes one journal line, and — once the
text has reached the user, because the pill goes before the notification — it says so once:

    ⚠️ Recording clipped: the transcript may be degraded

A dictation that pasted nothing already carries a notification of its own, and two at once is noise;
the journal line is written either way. `metadata.json` keeps nothing about it: a clipped recording
is still a `transcribed` one, and the log is the durable trace
([ADR-0008](../adr/0008-the-log.md)). Nothing takes the level back while the recording runs either:
the writer is whoever raised it, and the tool does not argue with the volume control
([ADR-0018](../adr/0018-the-meter-is-relative.md)).

## The journal

One line per dictation says what became of the level, and it carries the session id
([ADR-0008](../adr/0008-the-log.md)):

| line | event |
|------|-------|
| `microphone set to base volume <raw>` | the default source was set to the base the server published |
| `microphone set to input level <pct>%: no readable base volume` | the server answered without a base the parser could read, and the configured level stood in |
| `microphone left at its level: no audio server answered` | `pactl` could not be run, or did not answer |
| `recording clipped: the level went over what a sample holds` | the recording reached full scale, so its loudest part is flattened |

## Decisions

- [ADR-0014: the microphone is set to its base volume before a dictation](../adr/0014-the-microphone-at-its-base-volume.md)
  records why the tool changes the level rather than warning about it, and why `wpctl` was rejected.
- [ADR-0018: the meter is relative, and the level is not the user's to tune](../adr/0018-the-meter-is-relative.md)
  records why the tool never writes above the base, and why a clipped recording is reported rather
  than prevented by asking the user to watch a meter.
