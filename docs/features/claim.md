# The claim

## What it is

The [session directory](dictation-lifecycle.md) *is* the claim on the microphone. It is created
with `mkdir`, which is atomic: the `mkdir` fails when the directory already exists, so two
dictations can never run at once, whatever the two commands do. There is no lockfile to read, no pid
to check first, and no window between the test and the take.

`claim_session` therefore has three outcomes:

1. `mkdir` succeeds: the microphone is ours.
2. The directory exists and holds the `cancelled` verdict: a cancelled session is waiting out its
   dwell. It is asked to wrap up instead of making the next dictation wait out a screen the user has
   read; see [the verdict](cancel-and-verdict.md).
3. The directory exists otherwise. It is reclaimed — removed and recreated — **only** when neither
   the session process nor the recorder recorded in its pidfiles is alive. A stale directory (a
   session killed by a signal, a crash) can be reclaimed; a live one never.

When the claim cannot be taken, `start` prints `tiny-dictate is already busy` on stderr, exits
non-zero, and notifies nothing: the running session's pill already shows what is going on, and it is
the only writer of the screen.

## The command lock

A command is not a dictation, and the claim does not cover it. `start`, `stop`, `cancel` and `toggle`
each take a second, short-lived lock — a `command.lock` directory in the state directory — that is
released when the command exits, including on failure. Two shortcuts pressed at the same time cannot
interleave their process handling.

The lock is reclaimable: when the pid it holds is gone, the next command removes the directory and
takes it. A command that fails to take the lock notifies `⏳ tiny-dictate is already handling another
command` and exits non-zero.

`status` takes no lock, and modifies nothing.

## Liveness and pidfiles

Process liveness is read from a pidfile with `kill -0`. Two rules follow, and the port is expected to
keep them:

- Sending a signal must never remove the pidfile it was read from. The session reads that pidfile to
  tell whether the recorder is still alive; deleting it would make the session believe recording is
  over.
- The session is never signalled to end a dictation. `stop` and `cancel` signal the recorder; the
  session reads what happened and tears itself down.

## Stale claims and cleanup

`session_cleanup` removes the directory only when the pidfile still holds the session's own
`$BASHPID`. A newer session may have reclaimed the same paths after a crash; its files and its
presenter are not ours to remove.

## Decisions

- [ADR-0009: the claim](../adr/0009-the-claim.md) keeps this model across the port to Rust,
  including the recorder pid recorded before the session is forked.
