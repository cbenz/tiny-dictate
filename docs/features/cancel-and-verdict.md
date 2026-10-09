# Cancelling and the verdict

## The act

`cancel` is a command like any other, and a shortcut is only a way to run it. It does two things, in
this order:

1. It writes the `cancelled` word into the state file. Writing it is what shows it: the presenter
   polls the file, so the verdict may arrive at any moment and is never missed.
2. It signals the recorder with `SIGINT`, so the capture stops. It never signals the session, which
   is the process showing the verdict, waiting out its dwell, and removing its own directory; and it
   never signals the encoder while a session is alive, because the encoder must outlive the recorder
   to flush its last frames.

When no session is alive, only the leftovers of one are here: `cancel` kills the encoder and removes
the session directory itself.

When neither a recording nor a session is running, `cancel` prints `No recording in progress` and
exits 0. It is not an error to cancel nothing.

The pill offers the same thing to the mouse: the reference presenter draws a cross that runs the
command it was handed as `$4`; see [the pill](pill.md).

## A cancelled dictation is not a failure

The verdict is written before anything is signalled, and the session reads it from the state file
rather than waiting for a signal. A cancelled dictation is therefore neither transcribed nor reported
as a failure, whichever moment the cancel lands in:

- The session checks the verdict after waiting for the recorder and the encoder, before transcribing.
  It shows the verdict and returns.
- It checks again after the transcriber returns and before pasting. A cancel that landed while the
  transcription was running drops the text rather than pasting it behind the user's back.

Cancelling notifies nothing, at any time.

## The dwell

The verdict is drawn, not notified: it is a state of the pill like `recording` and `transcribing`.
The session kills the encoder with `SIGTERM` if it is still alive, waits out `VERDICT_SECONDS`
(1 second, the timeout the cancellation notification used to have), and only then takes the pill
down. Taking it down writes `stop` into the state file, waits — bounded — for the presenter to leave,
and kills it if it is wedged.

A new dictation does not wait that second out. `claim::claim` (`src/claim.rs`) reads the `cancelled`
verdict, asks the session to wrap up with `SIGUSR1`, and waits, bounded, for it to be gone. The
session traps the signal, cuts the dwell short, stops the presenter and removes its directory; the
claim is then free and the new dictation records. `start` and `toggle` both take that path, so
pressing the shortcut again right after a cancel records immediately.

`SIGUSR1` is sent only to a session that has already been cancelled, never by `cancel` itself, and
never to a session that is recording or transcribing.

## The states

`cancelled` is one of the words the session publishes, with `recording`, `transcribing` and `stop`.
`stop` is not a verdict: it asks the presenter to leave.

## Decisions

- [ADR-0004: cancelling versus destroying](../adr/0004-cancelling-versus-destroying.md) makes
  cancelling drop the text and keep the recording, and moves destruction to `session delete`.
