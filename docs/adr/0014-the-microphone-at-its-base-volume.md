# The microphone is set to its base volume before a dictation

A dictation records from the audio server's default source, whose level is whatever the user last
left it at: at 100 % the ADC saturates, the transcript degrades and the waveform reads as a flat
block. The tool therefore sets that source to its base volume at each `start`, after the claim and
before the recorder opens, and unmutes it. The level stays there afterwards — the base is the sane
point, and restoring the previous one would only re-arm the saturation.

## Considered options

Warning instead of fixing asks the user to do work the tool can do. A percentage in the configuration
means a different gain per machine: the reference machine's base is 10 %, so the 20 % a user would
write there is +18 dB. `wpctl` was rejected because PipeWire publishes no base volume; only
pipewire-pulse does.

## Consequences

The change persists: WirePlumber writes it to `default-routes` and reapplies it at boot, so the tool
leaves a durable mark on the machine. The base is read as the raw value on `pactl`'s `Base Volume:`
line, and `input_level_pct` (10 by default) stands in only when that line is missing.
