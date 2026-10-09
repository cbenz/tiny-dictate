//! The durable side of a dictation: the session directory under the state directory.
//!
//! One directory per dictation, named by the local start timestamp. The encoder writes
//! `audio.mp3` there from the first frame, the core writes `transcript.txt` (never the
//! transcriber), the transcriber leaves the provider's own answer in `transcript.raw.json`
//! (stored verbatim, never modified), and `metadata.json` says what happened. The metadata
//! is rewritten atomically at each stage, so a session left on `recording` is one that died
//! mid-recording. The ephemeral side of the dictation — the claim directory, the fifo, the
//! waveform channel, the state file — stays in the runtime directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::files;

/// What became of a session, as `GLOSSARY.md` names it: a closed vocabulary, written verbatim
/// into `metadata.json` and shown by `session list`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Recording,
    Transcribing,
    Transcribed,
    Failed,
    Cancelled,
}

/// The word a cancelled dictation carries, in its metadata and in the state file alike: the one
/// word the outcome vocabulary and the pill's share, so it is written once.
pub const CANCELLED: &str = "cancelled";

impl Outcome {
    /// The word the metadata holds.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Transcribing => "transcribing",
            Self::Transcribed => "transcribed",
            Self::Failed => "failed",
            Self::Cancelled => CANCELLED,
        }
    }

    /// The outcome a word names, or `None` for a word this version does not know.
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "recording" => Self::Recording,
            "transcribing" => Self::Transcribing,
            "transcribed" => Self::Transcribed,
            "failed" => Self::Failed,
            _ if word == CANCELLED => Self::Cancelled,
            _ => return None,
        })
    }
}

/// The encoded recording, written by the encoder from the first frame.
pub const AUDIO_FILE: &str = "audio.mp3";
/// The transcript, written by the core and never by the transcriber.
pub const TRANSCRIPT_FILE: &str = "transcript.txt";
/// The metadata, rewritten atomically at each stage.
pub const METADATA_FILE: &str = "metadata.json";

/// Which sessions to remove: the oldest beyond the newest `keep`, oldest first.
///
/// `ids` is the sessions directory as `session_ids` lists it, newest first; `current` is the
/// dictation that has just been pasted: it is counted among the sessions — it is one of them —
/// but it is never removed, even when the limit has been reached. The result is oldest first,
/// the order the removals happen in.
pub fn sessions_to_prune(ids: &[String], keep: usize, current: &str) -> Vec<String> {
    // Everything past the newest `keep` is old enough to go; reversed, that is oldest first.
    let mut candidates: Vec<String> = ids.iter().skip(keep).rev().cloned().collect();
    candidates.retain(|id| id != current);

    candidates
}

/// Remove the oldest sessions beyond `keep` from the sessions directory.
///
/// Returns the ids that were removed, oldest first; directory entries that are not session
/// directories are left alone. A missing or unreadable directory removes nothing. The listing is
/// `session_ids`, the same one `session list` and `resolve_id` read, so the two agree on what the
/// newest session is.
pub fn prune_sessions(sessions: &Path, keep: usize, current: &str) -> Vec<String> {
    let removed = sessions_to_prune(&session_ids(sessions), keep, current);

    for id in &removed {
        let _ = fs::remove_dir_all(sessions.join(id));
    }

    removed
}

/// What became of one session, as it is written to `metadata.json`.
///
/// `started`, `ended` and `duration` are Unix epoch seconds; the fractional part is kept, so
/// a short dictation still has a duration. `ended`, `duration`, `transcriber` and `error`
/// are `null` until the stage that sets them.
pub struct Metadata {
    pub id: String,
    pub started: f64,
    pub ended: Option<f64>,
    pub duration: Option<f64>,
    pub audio_bytes: u64,
    pub outcome: Outcome,
    pub pasted: bool,
    pub transcriber: Option<String>,
    pub error: Option<String>,
}

impl Metadata {
    /// Read back the metadata the tool wrote. A missing or unparsable field is `None`: a reader
    /// shows what it can rather than refusing a directory a human edited.
    pub fn from_json(json: &str) -> Option<Self> {
        Some(Self {
            id: json_field(json, "id").and_then(json_decode_string)?,
            started: json_field(json, "started").and_then(json_number)?,
            ended: json_field(json, "ended").and_then(json_number),
            duration: json_field(json, "duration").and_then(json_number),
            audio_bytes: json_field(json, "audio_bytes")
                .and_then(json_number)
                .map(|bytes| bytes.max(0.0) as u64)
                .unwrap_or(0),
            outcome: json_field(json, "outcome")
                .and_then(json_decode_string)
                .and_then(|word| Outcome::parse(&word))?,
            pasted: json_field(json, "pasted") == Some("true"),
            transcriber: json_field(json, "transcriber").and_then(json_decode_string),
            error: json_field(json, "error").and_then(json_decode_string),
        })
    }

    /// The metadata as JSON, one field per line: a reader never has to guess the shape. This is
    /// the one writer of this object: `metadata.json` and the metadata inside a `--output json`
    /// document both come from here, so the disk and stdout cannot disagree.
    pub fn to_json(&self) -> String {
        format!("{}\n", self.at(""))
    }

    /// The same object nested in a larger document drawn at `indent`: every line but the first is
    /// drawn at the element's own indentation, so a listing can hold it without a second writer of
    /// the metadata's shape.
    pub fn to_json_at(&self, indent: &str) -> String {
        self.at(indent)
    }

    fn at(&self, indent: &str) -> String {
        let field = format!("{indent}  ");
        let id = json_string(&self.id);
        let started = self.started;
        let ended = optional_number(self.ended);
        let duration = optional_number(self.duration);
        let audio_bytes = self.audio_bytes;
        let outcome = json_string(self.outcome.as_str());
        let pasted = self.pasted;
        let transcriber = optional_string(self.transcriber.as_deref());
        let error = optional_string(self.error.as_deref());

        format!(
            "{{\n{field}\"id\": {id},\n{field}\"started\": {started},\n{field}\"ended\": {ended},\n{field}\"duration\": {duration},\n{field}\"audio_bytes\": {audio_bytes},\n{field}\"outcome\": {outcome},\n{field}\"pasted\": {pasted},\n{field}\"transcriber\": {transcriber},\n{field}\"error\": {error}\n{indent}}}"
        )
    }
}

/// One dictation's session on disk: its directory, its metadata, and the files in it.
pub struct Session {
    /// The id: the local start timestamp, and the directory's name.
    pub id: String,
    /// The durable directory the whole dictation lives in.
    pub directory: PathBuf,
    /// The wall clock the recording started on, for `duration`.
    started_at: SystemTime,
    /// The metadata as it currently stands; `save` writes it.
    pub metadata: Metadata,
}

impl Session {
    /// Create the session directory of a dictation that is about to start. The directory is
    /// created with `mkdir`, so two dictations in the same second cannot share one.
    pub fn create(
        sessions: &Path,
        transcriber: Option<&Path>,
        started_at: SystemTime,
    ) -> io::Result<Self> {
        fs::create_dir_all(sessions)?;

        let stamp = local_timestamp(started_at);
        let mut id = stamp.clone();
        let mut suffix = 1;
        let directory = loop {
            let candidate = sessions.join(&id);
            match fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    suffix += 1;
                    id = format!("{stamp}-{suffix}");
                }
                Err(error) => return Err(error),
            }
        };

        Ok(Self::new(id, directory, transcriber, started_at))
    }

    /// Reopen the session a forked session process owns, by id: the metadata `start` wrote is
    /// the trace, so the child carries its `started` over rather than stamping one of its own.
    /// A directory whose metadata cannot be read is rebuilt from the fields this process knows.
    pub fn open(
        sessions: &Path,
        id: &str,
        transcriber: Option<&Path>,
        started_at: SystemTime,
    ) -> Self {
        Self::load(sessions, id)
            .unwrap_or_else(|| Self::new(id.to_owned(), sessions.join(id), transcriber, started_at))
    }

    /// Read a kept session back from disk, metadata included: the durable trace is the file,
    /// not what a fresh process could reconstruct.
    pub fn load(sessions: &Path, id: &str) -> Option<Self> {
        let directory = sessions.join(id);
        let json = fs::read_to_string(directory.join(METADATA_FILE)).ok()?;
        let metadata = Metadata::from_json(&json)?;
        let started_at = UNIX_EPOCH + Duration::from_secs_f64(metadata.started.max(0.0));

        Some(Self {
            id: metadata.id.clone(),
            directory,
            started_at,
            metadata,
        })
    }

    fn new(
        id: String,
        directory: PathBuf,
        transcriber: Option<&Path>,
        started_at: SystemTime,
    ) -> Self {
        let metadata = Metadata {
            id: id.clone(),
            started: epoch_seconds(started_at),
            ended: None,
            duration: None,
            audio_bytes: 0,
            outcome: Outcome::Recording,
            pasted: false,
            transcriber: transcriber.map(|path| path.display().to_string()),
            error: None,
        };

        Self {
            id,
            directory,
            started_at,
            metadata,
        }
    }

    /// The encoded recording the transcriber is handed.
    pub fn audio(&self) -> PathBuf {
        self.directory.join(AUDIO_FILE)
    }

    /// The transcript the core writes, never the transcriber.
    pub fn transcript(&self) -> PathBuf {
        self.directory.join(TRANSCRIPT_FILE)
    }

    /// Finish the recording stage: the recorder has stopped and the encoder has flushed.
    pub fn finish_recording(&mut self, ended_at: SystemTime) {
        self.metadata.ended = Some(epoch_seconds(ended_at));
        self.metadata.duration = Some(
            ended_at
                .duration_since(self.started_at)
                .unwrap_or_default()
                .as_secs_f64(),
        );
        self.metadata.audio_bytes = files::file_size(&self.audio());
    }

    /// Write the metadata atomically: a reader never sees a half-written file.
    pub fn save(&self) -> io::Result<()> {
        let path = self.directory.join(METADATA_FILE);

        files::write_atomic(&path, &self.metadata.to_json())
    }
}

/// The kept session ids, newest first: the directory names, which are start timestamps, sorted
/// descending. A symlink or a plain file under the sessions root is not a session.
pub fn session_ids(sessions: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(sessions) else {
        return Vec::new();
    };

    let mut ids: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    ids.sort_unstable_by(|left, right| right.cmp(left));
    ids
}

/// The id a `<id|last>` argument names, or `None` when no kept session answers to it. `last` is
/// the newest session; anything else must be one of the ids on disk, so a path can never name a
/// session that is not one.
pub fn resolve_id(sessions: &Path, target: &str) -> Option<String> {
    let ids = session_ids(sessions);
    if target == "last" {
        return ids.into_iter().next();
    }

    ids.into_iter().find(|id| id == target)
}

/// The raw value of a top-level key in the metadata JSON. The tool writes one field per line and
/// is the only writer, but the key is matched wherever it appears and whatever follows it, so a
/// reformatted file still reads.
fn json_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let mut from = 0;

    while let Some(offset) = json[from..].find(&needle) {
        let index = from + offset;
        let previous = json[..index].chars().next_back();
        // A key is a JSON object member: it follows `{`, `,` or whitespace. Anything else is a
        // key name that happens to live inside a string value, which is not what is meant.
        if matches!(
            previous,
            None | Some('{') | Some(',') | Some(' ') | Some('\n') | Some('\r') | Some('\t')
        ) && let Some(rest) = json[index + needle.len()..].trim_start().strip_prefix(':')
        {
            return Some(json_token(rest.trim_start()));
        }
        from = index + needle.len();
    }

    None
}

/// The value token at the start of `text`: a quoted string with its quotes, or the run of
/// characters up to the next structural one.
fn json_token(text: &str) -> &str {
    let mut characters = text.char_indices();

    if let Some((_, '"')) = characters.next() {
        let mut escaped = false;
        for (index, character) in characters {
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => return &text[..index + 1],
                _ => {}
            }
        }
        return text;
    }

    let end = text.find([',', '}', '\n']).unwrap_or(text.len());
    text[..end].trim_end()
}

/// Decode a JSON string token, or `None` when it is not one.
fn json_decode_string(token: &str) -> Option<String> {
    let inner = token.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut characters = inner.chars();

    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }

        match characters.next()? {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            '/' => out.push('/'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let hex: String = characters.by_ref().take(4).collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            other => out.push(other),
        }
    }

    Some(out)
}

/// A JSON number, or `None` when the token is `null` or anything else.
fn json_number(token: &str) -> Option<f64> {
    token.trim().parse::<f64>().ok()
}

/// The local start timestamp an id is made of: `2026-10-07T19-05-22`, sortable newest last.
fn local_timestamp(time: SystemTime) -> String {
    let seconds = epoch_seconds(time) as libc::time_t;
    let mut broken_down: libc::tm = unsafe { std::mem::zeroed() };

    // Safety: `broken_down` is a valid, writable `struct tm`.
    if unsafe { libc::localtime_r(&seconds, &mut broken_down) }.is_null() {
        return "session".to_owned();
    }

    format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
        broken_down.tm_year + 1900,
        broken_down.tm_mon + 1,
        broken_down.tm_mday,
        broken_down.tm_hour,
        broken_down.tm_min,
        broken_down.tm_sec
    )
}

/// Seconds since the Unix epoch, fractional part kept; `0.0` for a clock before it.
fn epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

fn optional_number(value: Option<f64>) -> String {
    value.map_or_else(|| "null".to_owned(), |number| number.to_string())
}

fn optional_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_string)
}

/// A JSON string literal: an id, a path, the error and the transcriber path can hold anything.
pub(crate) fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');

    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if control.is_control() => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }

    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn metadata_survives_a_write_and_a_read_back() {
        let metadata = Metadata {
            id: "2026-10-07T19-05-22".to_owned(),
            started: 1759855522.12,
            ended: Some(1759855601.45),
            duration: Some(79.33),
            audio_bytes: 365_720,
            outcome: Outcome::Transcribed,
            pasted: true,
            transcriber: Some("/home/me/.local/bin/tiny-dictate-transcribe".to_owned()),
            error: None,
        };

        let parsed = Metadata::from_json(&metadata.to_json()).expect("the metadata reads back");

        assert_eq!(parsed.id, metadata.id);
        assert_eq!(parsed.started, metadata.started);
        assert_eq!(parsed.ended, metadata.ended);
        assert_eq!(parsed.duration, metadata.duration);
        assert_eq!(parsed.audio_bytes, metadata.audio_bytes);
        assert_eq!(parsed.outcome, metadata.outcome);
        assert!(parsed.pasted);
        assert_eq!(parsed.transcriber, metadata.transcriber);
        assert_eq!(parsed.error, None);
    }

    #[test]
    fn an_error_with_quotes_and_newlines_reads_back_verbatim() {
        let metadata = Metadata {
            id: "2026-10-07T19-05-22".to_owned(),
            started: 0.0,
            ended: None,
            duration: None,
            audio_bytes: 0,
            outcome: Outcome::Failed,
            pasted: false,
            transcriber: None,
            error: Some("groq: \"invalid\" key\nretry".to_owned()),
        };

        let parsed = Metadata::from_json(&metadata.to_json()).expect("the metadata reads back");

        assert_eq!(
            parsed.error.as_deref(),
            Some("groq: \"invalid\" key\nretry")
        );
        assert_eq!(parsed.ended, None);
        assert_eq!(parsed.transcriber, None);
    }

    #[test]
    fn a_session_id_resolves_to_the_newest_or_an_exact_name() {
        let root = std::env::temp_dir().join(format!("td-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("2026-10-07T19-05-22")).expect("a session directory");
        fs::create_dir_all(root.join("2026-10-07T19-06-00")).expect("a session directory");

        assert_eq!(session_ids(&root)[0], "2026-10-07T19-06-00");
        assert_eq!(
            resolve_id(&root, "last").as_deref(),
            Some("2026-10-07T19-06-00")
        );
        assert_eq!(
            resolve_id(&root, "2026-10-07T19-05-22").as_deref(),
            Some("2026-10-07T19-05-22")
        );
        assert_eq!(resolve_id(&root, "nope"), None);
        assert_eq!(resolve_id(&root, "../escape"), None);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_prune_removes_the_oldest_beyond_the_number_kept() {
        // `session_ids` order: newest first, so `d` is the dictation just pasted and `a` the
        // oldest.
        let sessions = ids(&["d", "c", "b", "a"]);

        assert_eq!(sessions_to_prune(&sessions, 2, "d"), ids(&["a", "b"]));
        assert_eq!(sessions_to_prune(&sessions, 3, "d"), ids(&["a"]));
        assert_eq!(sessions_to_prune(&sessions, 4, "d"), Vec::<String>::new());
        assert_eq!(sessions_to_prune(&[], 2, "d"), Vec::<String>::new());
    }

    #[test]
    fn the_session_being_recorded_is_never_pruned() {
        let sessions = ids(&["d", "c", "b", "a"]);

        // Even when it is among the oldest, the session just recorded is kept: it is the
        // one whose text was pasted a moment ago.
        assert_eq!(sessions_to_prune(&sessions, 2, "a"), ids(&["b"]));
        assert_eq!(sessions_to_prune(&sessions, 0, "d"), ids(&["a", "b", "c"]));
    }

    #[test]
    fn the_prune_and_last_share_one_notion_of_newest() {
        let root = std::env::temp_dir().join(format!("td-store-newest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for id in ["2026-10-07T19-05-22", "2026-10-07T19-06-00"] {
            fs::create_dir_all(root.join(id)).expect("a session directory");
        }

        let listed = session_ids(&root);
        let newest = resolve_id(&root, "last").expect("the newest session");

        // `session list` reads `session_ids` in order, `last` is its first entry, and the prune
        // never removes that entry: one listing, one notion of newest.
        assert_eq!(listed.first().map(String::as_str), Some(newest.as_str()));
        assert!(!sessions_to_prune(&listed, 1, &newest).contains(&newest));

        let _ = fs::remove_dir_all(&root);
    }
}
