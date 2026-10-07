# The claim

The microphone is reserved by creating the claim directory atomically; a stale claim is reclaimable
only when neither the session process nor the recorder lives. The invariants of the current runtime
model survive the port unchanged: the recorder's pid is recorded before the session is forked, `stop`
and `cancel` signal the recorder and never the session, a signal never removes the pidfile, the
recorder is killed before the encoder, one presenter per dictation with the session as its only
writer, and the session waits for the presenter to be gone before notifying anything.

Those invariants are what the integration suite asserts, and the suite is the port's acceptance
criterion. Rewriting the core in another language is not a reason to revisit the model that makes the
claim and the process lifetimes race-free.
