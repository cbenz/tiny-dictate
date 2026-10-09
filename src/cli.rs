//! The command line: one verb per invocation, and the usage text.

/// The usage, as `docs/SPECS.md` documents it.
pub const USAGE: &str = "\
Usage: tiny-dictate <command>

Commands:
  start                                   Start recording
  stop                                    Stop recording and transcribe
  cancel                                  Cancel recording
  toggle                                  Start if idle, stop if recording
  status                                  Show status (idle or working)
  session                                 List the kept sessions, newest first
  session list                            List the kept sessions, newest first
  session transcribe <id|last> [--print]  Transcribe a session again, to the clipboard or to stdout
  session delete <id|last>                Remove exactly one session";

/// What one invocation asks the tool to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Start,
    Stop,
    Cancel,
    Toggle,
    Status,
    Help,
    /// `session`, or `session list`: the kept sessions, newest first.
    SessionList,
    SessionTranscribe {
        target: String,
        print: bool,
    },
    SessionDelete {
        target: String,
    },
}

/// Why a command line cannot be run.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// A word that is not a verb, or that a verb does not take.
    Unknown(String),
    /// More than one verb, or more than one argument, in one invocation.
    Conflicting,
    /// A verb missing what it needs.
    Missing(String),
}

impl Error {
    /// The line to show the user, before the usage.
    pub fn message(&self) -> String {
        match self {
            Self::Unknown(word) => format!("Unknown argument: {word}"),
            Self::Conflicting => "Only one command can be provided".to_owned(),
            Self::Missing(message) => message.clone(),
        }
    }
}

/// Read one invocation. At most one verb is accepted; the verbs that take a session id read it
/// here, and the layer that runs them resolves it.
pub fn parse(arguments: &[String]) -> Result<Command, Error> {
    let Some((first, rest)) = arguments.split_first() else {
        // Invoked with no verb, the tool shows its usage and succeeds.
        return Ok(Command::Help);
    };

    match first.as_str() {
        "start" => alone(Command::Start, rest),
        "stop" => alone(Command::Stop, rest),
        "cancel" => alone(Command::Cancel, rest),
        "toggle" => alone(Command::Toggle, rest),
        "status" => alone(Command::Status, rest),
        "-h" | "--help" | "help" => alone(Command::Help, rest),
        "session" => session(rest),
        word => Err(Error::Unknown(word.to_owned())),
    }
}

/// A verb that takes no argument: anything after it is either a second verb or an unknown word.
fn alone(command: Command, rest: &[String]) -> Result<Command, Error> {
    match rest.first() {
        None => Ok(command),
        Some(word) if is_verb(word) => Err(Error::Conflicting),
        Some(word) => Err(Error::Unknown(word.clone())),
    }
}

/// The `session` group: every verb that touches a kept session, and nothing else.
///
/// Bare, it takes no argument of its own and lists the sessions; a sub-verb it does not know is
/// an unknown argument, like one at the root.
fn session(rest: &[String]) -> Result<Command, Error> {
    match rest.first().map(String::as_str) {
        None => Ok(Command::SessionList),
        Some("list") => alone(Command::SessionList, &rest[1..]),
        Some("transcribe") => transcribe(&rest[1..]),
        Some("delete") => delete(&rest[1..]),
        Some(word) => Err(Error::Unknown(word.to_owned())),
    }
}

/// `session transcribe <id|last> [--print]`.
fn transcribe(rest: &[String]) -> Result<Command, Error> {
    let mut target: Option<String> = None;
    let mut print = false;

    for argument in rest {
        match argument.as_str() {
            "--print" => print = true,
            word if word.starts_with('-') && target.is_none() => {
                return Err(Error::Unknown(word.to_owned()));
            }
            word => {
                if target.is_some() {
                    return Err(Error::Conflicting);
                }
                target = Some(word.to_owned());
            }
        }
    }

    match target {
        Some(target) => Ok(Command::SessionTranscribe { target, print }),
        None => Err(Error::Missing(
            "session transcribe needs a session id or `last`".to_owned(),
        )),
    }
}

/// `session delete <id|last>`: the verb that destroys one session, and only one. The single word
/// is the session to remove; more than one is a conflict, and none is the argument it needs.
fn delete(rest: &[String]) -> Result<Command, Error> {
    match rest {
        [target] => Ok(Command::SessionDelete {
            target: target.clone(),
        }),
        [] => Err(Error::Missing(
            "session delete needs a session id or `last`".to_owned(),
        )),
        _ => Err(Error::Conflicting),
    }
}

/// Whether a word is a verb of this command line: what makes a second one conflicting rather
/// than unknown. The verbs removed from the root are deliberately absent: they are unknown
/// arguments now, so the usage teaches the new name.
fn is_verb(word: &str) -> bool {
    matches!(
        word,
        "start" | "stop" | "cancel" | "toggle" | "status" | "session" | "help" | "-h" | "--help"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> Result<Command, Error> {
        let arguments: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
        parse(&arguments)
    }

    #[test]
    fn the_session_group_reads_its_verbs() {
        assert_eq!(parse_words(&["session"]), Ok(Command::SessionList));
        assert_eq!(parse_words(&["session", "list"]), Ok(Command::SessionList));
        assert_eq!(
            parse_words(&["session", "transcribe", "last"]),
            Ok(Command::SessionTranscribe {
                target: "last".to_owned(),
                print: false
            })
        );
        assert_eq!(
            parse_words(&["session", "transcribe", "last", "--print"]),
            Ok(Command::SessionTranscribe {
                target: "last".to_owned(),
                print: true
            })
        );
        assert_eq!(
            parse_words(&["session", "delete", "2026-10-07T19-05-22"]),
            Ok(Command::SessionDelete {
                target: "2026-10-07T19-05-22".to_owned()
            })
        );
    }

    #[test]
    fn the_removed_root_verbs_are_unknown_arguments() {
        // The removal is announced by the usage, never silent: neither word is a verb any more.
        assert_eq!(
            parse_words(&["history"]),
            Err(Error::Unknown("history".to_owned()))
        );
        assert_eq!(
            parse_words(&["transcribe", "last"]),
            Err(Error::Unknown("transcribe".to_owned()))
        );
    }

    #[test]
    fn a_missing_or_misplaced_word_is_reported() {
        assert!(matches!(
            parse_words(&["session", "transcribe"]),
            Err(Error::Missing(_))
        ));
        assert!(matches!(
            parse_words(&["session", "delete"]),
            Err(Error::Missing(_))
        ));
        assert_eq!(
            parse_words(&["session", "now"]),
            Err(Error::Unknown("now".to_owned()))
        );
        assert!(matches!(
            parse_words(&["session", "prune"]),
            Err(Error::Unknown(_))
        ));
        assert!(matches!(
            parse_words(&["session", "list", "now"]),
            Err(Error::Unknown(_))
        ));
        assert!(matches!(
            parse_words(&["start", "stop"]),
            Err(Error::Conflicting)
        ));
        assert!(matches!(
            parse_words(&["session", "session"]),
            Err(Error::Unknown(_))
        ));
        assert!(matches!(
            parse_words(&["session", "list", "session"]),
            Err(Error::Conflicting)
        ));
    }
}
