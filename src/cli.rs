//! The command line: one verb per invocation, and the usage text.

/// The usage, as `docs/SPECS.md` documents it.
pub const USAGE: &str = "\
Usage: tiny-dictate <command>

Commands:
  start                           Start recording
  stop                            Stop recording and transcribe
  cancel                          Cancel recording
  toggle                          Start if idle, stop if recording
  status                          Show status (idle or working)
  history                         List the kept sessions, newest first
  transcribe <id|last> [--print]  Transcribe a session again, to the clipboard or to stdout
  session delete <id|last>        Remove exactly one session";

/// What one invocation asks the tool to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Start,
    Stop,
    Cancel,
    Toggle,
    Status,
    Help,
    History,
    Transcribe { target: String, print: bool },
    SessionDelete { target: String },
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
        "history" => alone(Command::History, rest),
        "-h" | "--help" | "help" => alone(Command::Help, rest),
        "transcribe" => transcribe(rest),
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

/// `transcribe <id|last> [--print]`.
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
        Some(target) => Ok(Command::Transcribe { target, print }),
        None => Err(Error::Missing(
            "transcribe needs a session id or `last`".to_owned(),
        )),
    }
}

/// `session delete <id|last>`: the only verb under `session`, and it means destroying one.
fn session(rest: &[String]) -> Result<Command, Error> {
    match rest.first().map(String::as_str) {
        Some("delete") => {
            let mut target: Option<String> = None;
            for argument in &rest[1..] {
                if target.is_some() {
                    return Err(Error::Conflicting);
                }
                target = Some(argument.clone());
            }

            match target {
                Some(target) => Ok(Command::SessionDelete { target }),
                None => Err(Error::Missing(
                    "session delete needs a session id or `last`".to_owned(),
                )),
            }
        }
        Some(word) => Err(Error::Unknown(word.to_owned())),
        None => Err(Error::Missing("session needs `delete`".to_owned())),
    }
}

/// Whether a word is a verb of this command line: what makes a second one conflicting rather
/// than unknown.
fn is_verb(word: &str) -> bool {
    matches!(
        word,
        "start"
            | "stop"
            | "cancel"
            | "toggle"
            | "status"
            | "history"
            | "transcribe"
            | "session"
            | "help"
            | "-h"
            | "--help"
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
    fn the_new_verbs_read_their_arguments() {
        assert_eq!(parse_words(&["history"]), Ok(Command::History));
        assert_eq!(
            parse_words(&["transcribe", "last"]),
            Ok(Command::Transcribe {
                target: "last".to_owned(),
                print: false
            })
        );
        assert_eq!(
            parse_words(&["transcribe", "last", "--print"]),
            Ok(Command::Transcribe {
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
    fn a_missing_or_misplaced_word_is_reported() {
        assert!(matches!(
            parse_words(&["transcribe"]),
            Err(Error::Missing(_))
        ));
        assert!(matches!(
            parse_words(&["session", "delete"]),
            Err(Error::Missing(_))
        ));
        assert!(matches!(
            parse_words(&["session", "prune"]),
            Err(Error::Unknown(_))
        ));
        assert!(matches!(
            parse_words(&["start", "stop"]),
            Err(Error::Conflicting)
        ));
        assert!(matches!(
            parse_words(&["history", "now"]),
            Err(Error::Unknown(_))
        ));
    }
}
