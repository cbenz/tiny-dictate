//! The command line: one verb per invocation, and the usage text.

/// The usage, as `docs/SPECS.md` documents it.
pub const USAGE: &str = "\
Usage: tiny-dictate <command>

Commands:
  start   Start recording
  stop    Stop recording and transcribe
  cancel  Cancel recording
  toggle  Start if idle, stop if recording
  status  Show status (idle or working)";

/// What one invocation asks the tool to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Start,
    Stop,
    Cancel,
    Toggle,
    Status,
    Help,
}

/// Why a command line cannot be run.
#[derive(Debug)]
pub enum Error {
    /// A word that is not a verb.
    Unknown(String),
    /// More than one verb in one invocation.
    Conflicting,
}

impl Error {
    /// The line to show the user, before the usage.
    pub fn message(&self) -> String {
        match self {
            Self::Unknown(word) => format!("Unknown argument: {word}"),
            Self::Conflicting => "Only one command can be provided".to_owned(),
        }
    }
}

/// Read one invocation. At most one verb is accepted; the arguments of the verbs that take
/// any (`transcribe <id|last>`, `session delete <id|last>`) are read by their own layer.
pub fn parse(arguments: &[String]) -> Result<Command, Error> {
    let mut parsed: Option<Command> = None;

    for argument in arguments {
        let command = match argument.as_str() {
            "start" => Command::Start,
            "stop" => Command::Stop,
            "cancel" => Command::Cancel,
            "toggle" => Command::Toggle,
            "status" => Command::Status,
            "-h" | "--help" | "help" => Command::Help,
            word => return Err(Error::Unknown(word.to_owned())),
        };

        if parsed.is_some() {
            return Err(Error::Conflicting);
        }
        parsed = Some(command);
    }

    // Invoked with no verb, the tool shows its usage and succeeds, as `src/tiny-dictate` does.
    Ok(parsed.unwrap_or(Command::Help))
}
