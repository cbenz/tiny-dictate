# Sessions on disk

Every dictation leaves a **session** behind: one directory per dictation under the state directory
(`$XDG_STATE_HOME/tiny-dictate/sessions`, defaulting to `$HOME/.local/state/tiny-dictate/sessions`),
named by the local start timestamp.

| file | contents |
|------|----------|
| `audio.mp3` | the encoded recording, written by the encoder from the first frame |
| `transcript.txt` | the transcript, written by the core, never by the transcriber |
| `transcript.raw.json` | the transcriber's own answer, stored verbatim, never modified |
| `metadata.json` | what happened to the session, rewritten atomically at each stage |

`transcript.raw.json` is present when the transcriber writes one: a transcriber that leaves nothing
in the session directory is still a valid transcriber, and the text it printed is what the core
writes and pastes.

## The metadata

```json
{
  "id": "2026-10-07T19-05-22",
  "started": 1759855522.12,
  "ended": 1759855601.45,
  "duration": 79.33,
  "audio_bytes": 365720,
  "outcome": "transcribed",
  "pasted": true,
  "transcriber": "/home/me/.local/bin/tiny-dictate-transcribe",
  "error": null
}
```

`started`, `ended` and `duration` are Unix epoch seconds. `ended` and `duration` are `null` while
recording, so a session left on `recording` is one that died mid-recording. `outcome` is
`recording`, `transcribing`, `transcribed`, `failed` or `cancelled` — the state vocabulary plus the
terminal words. `pasted` says whether the text reached the focused window. `transcriber` is the
resolved command, and `error` is the failure on one line, or `null`.

## The `session` group

Every command that touches a kept session lives under `tiny-dictate session`, and none of them
touches the microphone, the encoder or the pill:

- `tiny-dictate session list` lists the kept sessions, newest first, one line per session:

  ```text
  <id>  <duration>  <outcome>  <summary>
  ```

  the duration as a person reads one -- `4:32` under an hour, `1:04:32` beyond, and `-` when the
session died mid-recording; the outcome in the vocabulary above; and the summary, which is the error
when there is one, else the first words of the transcript, else `-`. A session whose metadata cannot
be read is a row of its own, its id and `?` where the other fields would be, rather than a skipped
line: it is exactly the session `session delete` can remove. The line is plain text on stdout, and
`session list` reads nothing but the sessions directory. `tiny-dictate session` on its own is the
same listing.
- `tiny-dictate session show <id|last>` reads one session in full: its fields, one per line, then
  its transcript. The fields are the shared view's, in the one order every surface renders them --
  `id`, `path`, `outcome`, `started`, `ended`, `duration`, `audio bytes`, `pasted`, `transcriber`,
  `error` -- so this command and the browser's pane cannot drift. `duration` is the human one,
  `started` and `ended` are the raw epoch seconds `metadata.json` holds, `pasted` reads `yes` or
  `no`, and a field the session has nothing for is `-`. A session that holds no word of transcript
  says `no transcript` rather than printing an empty block; a directory whose metadata cannot be
  read prints its id and its path, says the metadata cannot be read, and exits non-zero. The
  command reads files only: no encoder, no clipboard tool, no microphone.
- `tiny-dictate session copy <id|last>` puts the session's transcript on the clipboard. A
  transcript already on disk is offered as it is, through the same write path a dictation uses: both
  selections, the password hint, the same lease. When `transcript.txt` is missing -- the session was
  never transcribed, or the run that would have written it failed -- the transcriber runs first,
  through the same runner `session transcribe` uses, and writes the transcript and the metadata
  exactly as that verb does, so the user never has to know which verb comes first. A transcript that
  exists but holds no word is reported and nothing more: a second transcriber run would say the same
  thing. A cancelled session is transcribed on request. Nothing is ever injected: the `Shift+Insert`
  chord stays the live dictation's business.
- `tiny-dictate session transcribe <id|last> [--print]` runs the transcriber command again on the
  session's `audio.mp3`, with the same contract a live dictation uses: `$1` the audio, `$2` the
session directory, the text on stdout, the error on stderr. It rewrites `transcript.txt` and the
session's metadata (`outcome`, `error`, `transcriber`), and a transcriber that writes
`transcript.raw.json` overwrites the previous one -- that file is the transcriber's, the core never
rewrites its bytes. By default it offers the text on the clipboard, through the same write path a
normal dictation uses, and says so on stderr; `--print` writes it to stdout instead, and needs no
clipboard tool at all.
- `tiny-dictate session delete <id|last>` removes exactly one session directory. It is the only verb
  that destroys a recording; cancelling never does. It refuses the session in flight -- the id the
  runtime directory publishes while a dictation runs -- reporting the refusal on stderr, exiting
  non-zero and removing nothing; the guard reads the runtime directory's files only, so it takes no
  command lock and never signals the live dictation. An older session still deletes while a
  dictation runs, and the session in flight deletes normally once its dictation has ended.

A verb that touches a session is not available anywhere else: `history` and `transcribe` used to
live at the root, and they are unknown arguments now. `last` is the newest session. An id that no
kept session answers to is reported on stderr, exits non-zero, and removes or writes nothing.

Every one of those surfaces reads the session through the same view, one module that owns the row of
the listing, the fields of the detail and the human duration, so the surfaces cannot drift apart.

## What survives

The recording, the transcript, the raw response and the metadata survive the session process:
nothing that matters lives only in the runtime directory, where the claim, the fifo, the waveform
channel and the state file stay. A crash, a `kill` or a power loss costs at most what the encoder
has not flushed yet, and leaves a session that can still be transcribed later.

Cancelling keeps the recording and marks the session `cancelled`; it is never transcribed or
pasted. Destroying a session is a separate verb, and only a deliberate one removes it. A session
that already has a transcript can be transcribed again: the new transcript and the new raw response
replace the old ones.

## Keeping the directory bounded

After a dictation whose text was pasted, the sessions beyond `keep` are removed, oldest first
([ADR-0005](../adr/0005-prune.md)). The count includes the session just recorded, which is never one
of the removed ones. A dictation that failed or was cancelled prunes nothing, and a crashed run
prunes nothing: the directory is bounded at the only moment when something reached the user. `keep`
comes from the [configuration file](configuration.md), 10 by default.

## Decisions

- [ADR-0003: sessions on disk](../adr/0003-sessions-on-disk.md) keeps the recording instead of
  removing it on the way out.
- [ADR-0004: cancelling versus destroying](../adr/0004-cancelling-versus-destroying.md) makes
  `session delete` the only verb that destroys a session.
- [ADR-0005: prune the oldest sessions after a pasted dictation](../adr/0005-prune.md) bounds the
  directory without a timer or a daemon.
- [ADR-0011: the command line](../adr/0011-command-line.md) adds the verbs that read a session and
  moves the dependency checks into the commands that need them;
  [ADR-0016](../adr/0016-the-session-group.md) moves those verbs under `session`.
