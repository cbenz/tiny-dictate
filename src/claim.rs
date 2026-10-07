//! The claim on the microphone.
//!
//! One dictation at a time: the claim is the session directory in the runtime directory,
//! and creating it with `mkdir` is atomic. A stale claim is reclaimable only when neither
//! the session process nor the recorder lives.
