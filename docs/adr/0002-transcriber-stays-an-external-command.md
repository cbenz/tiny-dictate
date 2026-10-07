# The transcriber stays an external command

Transcription stays delegated to a command the user provides: it is the one plugin surface the
project keeps. The contract is `$1` the audio file, `$2` the session directory, the transcript on
stdout, errors on stderr, and exit status 0 on success. `--print-audio-limit` prints the largest
audio file the transcriber accepts, in bytes; anything else — an unsupported flag, empty output, an
unparsable number, a non-zero status — means the transcriber declares no limit. Resolution is
`$TINY_DICTATE_TRANSCRIBE`, else `tiny-dictate-transcribe` on `PATH`. The limit is asked once per
dictation, before the microphone is opened.

The reference implementation stays a shell script and is not rewritten: the contract is "any
command", in any language, and rewriting it in Rust would suggest otherwise.

## Considered options

- An HTTP integration with the provider, which would replace the plugin surface with a provider list
  and lock the tool to one family of APIs.
- Rewriting the reference transcriber in Rust, which would make the script the exception and the
  contract harder to see.
