//! The tool's own log: one line per event on standard error.
//!
//! Every line is prefixed with the tool's name and, when the event concerns a session, with its
//! durable id: `tiny-dictate: 2026-10-07T19-05-22: recording started`. Events that concern no
//! session — the prune, `session delete` — carry no id. The encoder's and the recorder's own
//! output is redirected away, so the journal holds the tool's timeline and nothing else. There is
//! no global log file: the session's metadata is the durable trace, the journal is the timeline
//! (see `docs/adr/0008-the-log.md`).

/// The name every line is prefixed with.
pub const NAME: &str = "tiny-dictate";

/// Record one event.
///
/// `session` is the durable id when the event concerns a session, `None` when it concerns none.
/// Every event goes through here, so the lines have one shape and naming a session is one
/// argument rather than a format of its own.
pub fn log(session: Option<&str>, event: &str) {
    match session {
        Some(id) => eprintln!("{NAME}: {id}: {event}"),
        None => eprintln!("{NAME}: {event}"),
    }
}
