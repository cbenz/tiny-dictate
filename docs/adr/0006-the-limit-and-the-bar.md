# The limit and the bar

A dictation gets a duration limit derived from the byte limit the transcriber declares and the
encoder's bitrate, so a dictation can never exceed what can be transcribed. At the limit the
recording stops as if the shortcut had been pressed: the text is transcribed and pasted, never
dropped. The limit is passed to the pill, which measures elapsed time on its own monotonic clock
from the moment it reads `recording`; the bar is 3 px high under the meter, on the meter's width,
plain colour, white below 80%, amber between 80 and 95, red above.

A transcriber that declares no limit gets no limit and no bar, rather than a ceiling the user did not
ask for. The pill measuring elapsed time itself keeps the core from publishing a timer tick, and the
three bands are the pill's business, like the rest of what it draws.
