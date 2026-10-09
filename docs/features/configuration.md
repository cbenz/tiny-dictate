# The configuration file

`$XDG_CONFIG_HOME/tiny-dictate/config.toml`, `$HOME/.config/tiny-dictate/config.toml` by default.
Optional: every key has a default, so a fresh clone works without one. The file holds no secret: the
transcriber's key comes from the environment only
([ADR-0010](../adr/0010-configuration.md)).

| key | default | meaning |
|-----|---------|---------|
| `keep` | `10` | the number of past sessions retained; the oldest beyond it are pruned |
| `color` | `#e4e4e7` | the pill's colour: the waveform, and the plain band of the progress bar |
| `clipboard_seconds` | `60` | how long the transcript stays offered on the clipboard, `0` meaning no expiry |
| `input_level_pct` | `10` | the level the microphone falls back to when the audio server publishes no readable base volume, as a percentage |

```toml
keep = 5
color = "#89b4fa"
clipboard_seconds = 120
input_level_pct = 20
```

A missing, unreadable, malformed file, or one carrying a key this version does not know, falls back
to the defaults — silently, and per key: a typo costs a default, never a dictation
([ADR-0010](../adr/0010-configuration.md)). The file is parsed by the session process, once per
dictation: the colour is handed to the pill, `keep` to the prune, `clipboard_seconds` to the paste
and `input_level_pct` to [the microphone](microphone.md).

## The prune

After a dictation whose text was pasted, the sessions beyond `keep` are removed, oldest first
([ADR-0005](../adr/0005-prune.md)). A dictation that failed or was cancelled prunes nothing, a
crashed run prunes nothing, and the session just recorded is never removed: it is counted among the
sessions, because it is one, but it is the one whose text reached the user a moment ago. One line is
written to the log when sessions are removed.

## Decisions

- [ADR-0005: prune the oldest sessions after a pasted dictation](../adr/0005-prune.md) bounds the
  sessions directory without a timer or a daemon.
- [ADR-0010: configuration file, secrets in the environment only](../adr/0010-configuration.md)
  puts the keys in the file and keeps the secret in the environment.
