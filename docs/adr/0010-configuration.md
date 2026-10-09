# Configuration file, secrets in the environment only

Configuration moves to `$XDG_CONFIG_HOME/tiny-dictate/config.toml`, optional, with every key
defaulted: `keep = 10`, `color` (the pill's waveform, default `#e4e4e7`), `clipboard_seconds = 60`. A
fresh clone works without a configuration file; only the secret has no default.

Secrets are environment-only — `GROQ_API_KEY` for the reference transcriber — so nothing in the
repository ever holds one. A wrapper that fetches the key from a password manager stays outside this
repository.

A key this version does not know, and a value that does not parse, are ignored in silence and the
default applies: a config typo must never block a dictation, and the tool has no channel to report
anything before a session exists.
