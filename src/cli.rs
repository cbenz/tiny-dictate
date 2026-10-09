//! The command line: one verb per invocation, and the usage text.

/// The usage, as `docs/SPECS.md` documents it.
pub const USAGE: &str = "\
Usage: tiny-dictate <command>

Commands:
  start                                          Start recording
  stop                                           Stop recording and transcribe
  cancel                                         Cancel recording
  toggle                                         Start if idle, stop if recording
  status                                         Show status (idle or working)
  session                                        List the kept sessions, or browse them on a terminal
  session list [--output text|json]              List the kept sessions, newest first
  session browse                                 Browse the kept sessions on a terminal
  session show <id|last> [--output text|json]    Show one session: its fields, its path and its transcript
  session copy <id|last>                         Copy a session's transcript to the clipboard
  session transcribe <id|last> [--print]         Transcribe a session again, to the clipboard or to stdout
  session delete <id|last>                       Remove exactly one session";

/// What one invocation asks the tool to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Start,
    Stop,
    Cancel,
    Toggle,
    Status,
    Help,
    /// `session` alone: browses when stdout is a terminal, lists otherwise.
    Session,
    /// `session list`: the kept sessions, newest first.
    SessionList {
        output: Output,
    },
    /// `session browse`: the kept sessions read on a terminal.
    SessionBrowse,
    /// `session show <id|last>`: one kept session read in full.
    SessionShow {
        target: String,
        output: Output,
    },
    /// `session copy <id|last>`: the transcript of a kept session on the clipboard, transcribing
    /// it first when it has none.
    SessionCopy {
        target: String,
    },
    SessionTranscribe {
        target: String,
        print: bool,
    },
    SessionDelete {
        target: String,
    },
}

/// How a verb that shows a session renders it.
///
/// `text` is the human layout, and the default: a person reads a session the way every surface
/// shows one. `json` renders the same view as one document, for a script. Only the verbs that
/// show a session take it: `browse` owns a terminal, `status` is one word the bar reads, and
/// `session transcribe --print` prints a transcript, not data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Output {
    #[default]
    Text,
    Json,
}

impl Output {
    /// The format a `--output` value names; anything else is refused.
    fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(Error::BadOutput(other.to_owned())),
        }
    }
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
    /// A `--output` value that names no format.
    BadOutput(String),
}

impl Error {
    /// The line to show the user, before the usage.
    pub fn message(&self) -> String {
        match self {
            Self::Unknown(word) => format!("Unknown argument: {word}"),
            Self::Conflicting => "Only one command can be provided".to_owned(),
            Self::Missing(message) => message.clone(),
            Self::BadOutput(value) => format!("--output takes `text` or `json`, not `{value}`"),
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
        None => Ok(Command::Session),
        Some("list") => list(&rest[1..]),
        Some("browse") => alone(Command::SessionBrowse, &rest[1..]),
        Some("show") => show(&rest[1..]),
        Some("copy") => copy(&rest[1..]),
        Some("transcribe") => transcribe(&rest[1..]),
        Some("delete") => {
            let target = one_session(&rest[1..], "delete")?;
            Ok(Command::SessionDelete { target })
        }
        Some(word) => Err(Error::Unknown(word.to_owned())),
    }
}

/// `session list [--output text|json]`: the kept sessions, as a person reads them or as data.
fn list(rest: &[String]) -> Result<Command, Error> {
    let (rest, output) = output_flag(rest)?;

    match rest.first() {
        None => Ok(Command::SessionList { output }),
        Some(word) if is_verb(word) => Err(Error::Conflicting),
        Some(word) => Err(Error::Unknown(word.clone())),
    }
}

/// `session show <id|last> [--output text|json]`: one session, as a person reads it or as data.
/// The flag may come before or after the id; each is read where it is found.
fn show(rest: &[String]) -> Result<Command, Error> {
    let (rest, output) = output_flag(rest)?;
    let target = one_session(&rest, "show")?;

    Ok(Command::SessionShow { target, output })
}

/// The `--output` flag, separated (`--output json`) or joined (`--output=json`), removed from the
/// words; the words left are the verb's own. Omitting it means `text`, the format a person reads.
/// A flag given twice is a conflict, a missing value and an unknown one are the errors they are
/// anywhere else, and a verb that does not take the flag never calls this.
fn output_flag(words: &[String]) -> Result<(Vec<String>, Output), Error> {
    let mut rest = Vec::new();
    let mut output: Option<Output> = None;
    let mut index = 0;

    while index < words.len() {
        let word = &words[index];
        let value = if word == "--output" {
            index += 1;
            match words.get(index) {
                Some(value) => value,
                None => {
                    return Err(Error::Missing("--output needs `text` or `json`".to_owned()));
                }
            }
        } else if let Some(value) = word.strip_prefix("--output=") {
            value
        } else {
            rest.push(word.clone());
            index += 1;
            continue;
        };

        if output.is_some() {
            return Err(Error::Conflicting);
        }
        output = Some(Output::parse(value)?);
        index += 1;
    }

    Ok((rest, output.unwrap_or_default()))
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

/// `session copy <id|last>`: the verb that reads a transcript to the clipboard. Its single word
/// is the session to read; more than one is a conflict, and none is the argument it needs.
fn copy(rest: &[String]) -> Result<Command, Error> {
    match rest {
        [target] => Ok(Command::SessionCopy {
            target: target.clone(),
        }),
        [] => Err(Error::Missing(
            "session copy needs a session id or `last`".to_owned(),
        )),
        _ => Err(Error::Conflicting),
    }
}

/// A verb that names exactly one session: one word, and no more. The single word is the session
/// to read or to remove; more than one is a conflict, and none is the argument it needs.
fn one_session(rest: &[String], verb: &str) -> Result<String, Error> {
    match rest {
        [target] => Ok(target.clone()),
        [] => Err(Error::Missing(format!(
            "session {verb} needs a session id or `last`"
        ))),
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
        assert_eq!(parse_words(&["session"]), Ok(Command::Session));
        assert_eq!(
            parse_words(&["session", "list"]),
            Ok(Command::SessionList {
                output: Output::Text
            })
        );
        assert_eq!(
            parse_words(&["session", "browse"]),
            Ok(Command::SessionBrowse)
        );
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
            parse_words(&["session", "copy", "last"]),
            Ok(Command::SessionCopy {
                target: "last".to_owned()
            })
        );
        assert_eq!(
            parse_words(&["session", "delete", "2026-10-07T19-05-22"]),
            Ok(Command::SessionDelete {
                target: "2026-10-07T19-05-22".to_owned()
            })
        );
        assert_eq!(
            parse_words(&["session", "show", "last"]),
            Ok(Command::SessionShow {
                target: "last".to_owned(),
                output: Output::Text
            })
        );
        assert_eq!(
            parse_words(&["session", "show", "2026-10-07T19-05-22"]),
            Ok(Command::SessionShow {
                target: "2026-10-07T19-05-22".to_owned(),
                output: Output::Text
            })
        );
    }

    #[test]
    fn the_data_verbs_read_the_output_flag_in_both_forms() {
        // The separated form, the joined one, and `text` written explicitly: the format is never
        // implicit in a script that asks for it.
        assert_eq!(
            parse_words(&["session", "list", "--output", "text"]),
            Ok(Command::SessionList {
                output: Output::Text
            })
        );
        assert_eq!(
            parse_words(&["session", "list", "--output", "json"]),
            Ok(Command::SessionList {
                output: Output::Json
            })
        );
        assert_eq!(
            parse_words(&["session", "list", "--output=json"]),
            Ok(Command::SessionList {
                output: Output::Json
            })
        );
        assert_eq!(
            parse_words(&["session", "list", "--output=text"]),
            Ok(Command::SessionList {
                output: Output::Text
            })
        );

        // An id and the flag are both the verb's own: the flag reads the same on either side of it.
        for words in [
            vec!["session", "show", "last", "--output", "json"],
            vec!["session", "show", "--output", "json", "last"],
            vec!["session", "show", "--output=json", "last"],
        ] {
            assert_eq!(
                parse_words(&words),
                Ok(Command::SessionShow {
                    target: "last".to_owned(),
                    output: Output::Json
                })
            );
        }
    }

    #[test]
    fn the_output_flag_is_refused_where_it_means_nothing() {
        // A value that names no format is refused with its own line, not as an unknown word.
        assert_eq!(
            parse_words(&["session", "list", "--output", "yaml"]),
            Err(Error::BadOutput("yaml".to_owned()))
        );
        assert!(matches!(
            parse_words(&["session", "show", "last", "--output="]),
            Err(Error::BadOutput(_))
        ));
        assert!(matches!(
            parse_words(&["session", "list", "--output"]),
            Err(Error::Missing(_))
        ));
        assert!(matches!(
            parse_words(&["session", "list", "--output", "json", "--output", "text"]),
            Err(Error::Conflicting)
        ));

        // The verbs that show no session refuse the flag: a terminal, one word for the bar, and a
        // transcript on stdout are not data documents.
        for words in [
            vec!["session", "browse", "--output", "json"],
            vec!["session", "browse", "--output=json"],
            vec!["status", "--output", "json"],
            vec!["status", "--output=json"],
            vec!["session", "--output", "json"],
            vec!["session", "copy", "last", "--output", "json"],
            vec!["session", "delete", "last", "--output", "json"],
        ] {
            assert!(
                matches!(
                    parse_words(&words),
                    Err(Error::Unknown(_)) | Err(Error::Conflicting)
                ),
                "{words:?} must be refused"
            );
        }

        // `--print` prints a transcript; the flag does not join it.
        assert!(
            parse_words(&[
                "session",
                "transcribe",
                "last",
                "--print",
                "--output",
                "json"
            ])
            .is_err()
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
            parse_words(&["session", "copy"]),
            Err(Error::Missing(_))
        ));
        assert!(matches!(
            parse_words(&["session", "delete"]),
            Err(Error::Missing(_))
        ));
        assert!(matches!(
            parse_words(&["session", "show"]),
            Err(Error::Missing(_))
        ));
        assert!(matches!(
            parse_words(&["session", "show", "last", "now"]),
            Err(Error::Conflicting)
        ));
        assert!(matches!(
            parse_words(&["session", "copy", "last", "now"]),
            Err(Error::Conflicting)
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
            parse_words(&["session", "browse", "now"]),
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
