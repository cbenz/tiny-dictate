# Prune the oldest sessions after a pasted dictation

After a dictation whose text was pasted, the sessions beyond `keep` are removed, oldest first. A
dictation that fails or is cancelled never prunes, and a crashed run prunes nothing. The session
being recorded is never removed.

Tying the prune to the successful end of a dictation is what keeps the directory bounded without a
timer, a daemon or a background job, and makes a failure the moment when nothing is deleted. It also
keeps a failed session for `tiny-dictate transcribe` even when `keep` has been reached.
