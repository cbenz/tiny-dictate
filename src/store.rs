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
use std::time::{SystemTime, UNIX_EPOCH};

/// The outcome vocabulary, shared with the state the pill draws.
pub const RECORDING: &str = "recording";
pub const TRANSCRIBING: &str = "transcribing";
pub const TRANSCRIBED: &str = "transcribed";
pub const FAILED: &str = "failed";
pub const CANCELLED: &str = "cancelled";

/// The encoded recording, written by the encoder from the first frame.
pub const AUDIO_FILE: &str = "audio.mp3";
/// The transcript, written by the core and never by the transcriber.
pub const TRANSCRIPT_FILE: &str = "transcript.txt";
/// The metadata, rewritten atomically at each stage.
pub const METADATA_FILE: &str = "metadata.json";

/// Which sessions to remove: the oldest beyond the newest `keep`, oldest first.
///
/// `ids` is the sessions directory, oldest first, and `current` is the dictation that has just
/// been pasted: it is counted among the sessions — it is one of them — but it is never removed,
/// even when the limit has been reached.
pub fn sessions_to_prune(ids: &[String], keep: usize, current: &str) -> Vec<String> {
    ids.iter()
        .take(ids.len().saturating_sub(keep))
        .filter(|id| id.as_str() != current)
        .cloned()
        .collect()
}

/// Remove the oldest sessions beyond `keep` from the sessions directory.
///
/// Returns the ids that were removed, oldest first; directory entries that are not session
/// directories are left alone. A missing or unreadable directory removes nothing.
pub fn prune_sessions(sessions: &Path, keep: usize, current: &str) -> Vec<String> {
    let removed = sessions_to_prune(&list_sessions(sessions), keep, current);

    for id in &removed {
        let _ = fs::remove_dir_all(sessions.join(id));
    }

    removed
}

/// The session directories, oldest first. The id is the local start timestamp, so sorting the
/// names sorts the dictations.
fn list_sessions(sessions: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(sessions) else {
        return Vec::new();
    };

    let mut ids: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    ids.sort();

    ids
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
    pub outcome: String,
    pub pasted: bool,
    pub transcriber: Option<String>,
    pub error: Option<String>,
}

impl Metadata {
    /// The metadata as JSON, one field per line: a reader never has to guess the shape.
    fn to_json(&self) -> String {
        let id = json_string(&self.id);
        let started = self.started;
        let ended = optional_number(self.ended);
        let duration = optional_number(self.duration);
        let audio_bytes = self.audio_bytes;
        let outcome = json_string(&self.outcome);
        let pasted = self.pasted;
        let transcriber = optional_string(self.transcriber.as_deref());
        let error = optional_string(self.error.as_deref());

        format!(
            "{{\n  \"id\": {id},\n  \"started\": {started},\n  \"ended\": {ended},\n  \"duration\": {duration},\n  \"audio_bytes\": {audio_bytes},\n  \"outcome\": {outcome},\n  \"pasted\": {pasted},\n  \"transcriber\": {transcriber},\n  \"error\": {error}\n}}\n"
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

    /// Reopen the session a forked session process owns, by id. The metadata is built again
    /// from the fields the process knows; the file on disk is the previous stage's.
    pub fn open(
        sessions: &Path,
        id: &str,
        transcriber: Option<&Path>,
        started_at: SystemTime,
    ) -> Self {
        Self::new(id.to_owned(), sessions.join(id), transcriber, started_at)
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
            outcome: RECORDING.to_owned(),
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
        self.metadata.audio_bytes = file_size(&self.audio());
    }

    /// Write the metadata atomically: a reader never sees a half-written file.
    pub fn save(&self) -> io::Result<()> {
        let path = self.directory.join(METADATA_FILE);
        let temporary = path.with_extension(format!("json.tmp.{}", std::process::id()));

        fs::write(&temporary, self.metadata.to_json())?;
        fs::rename(&temporary, &path)
    }
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

fn file_size(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn optional_number(value: Option<f64>) -> String {
    value.map_or_else(|| "null".to_owned(), |number| number.to_string())
}

fn optional_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_string)
}

/// A JSON string literal: the error and the transcriber path can hold anything.
fn json_string(value: &str) -> String {
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
    fn the_prune_removes_the_oldest_beyond_the_number_kept() {
        // Oldest first: `a` is the oldest, `d` the dictation just pasted.
        let sessions = ids(&["a", "b", "c", "d"]);

        assert_eq!(sessions_to_prune(&sessions, 2, "d"), ids(&["a", "b"]));
        assert_eq!(sessions_to_prune(&sessions, 3, "d"), ids(&["a"]));
        assert_eq!(sessions_to_prune(&sessions, 4, "d"), Vec::<String>::new());
        assert_eq!(sessions_to_prune(&[], 2, "d"), Vec::<String>::new());
    }

    #[test]
    fn the_session_being_recorded_is_never_pruned() {
        let sessions = ids(&["a", "b", "c", "d"]);

        // Even when it is among the oldest, the session just recorded is kept: it is the
        // one whose text was pasted a moment ago.
        assert_eq!(sessions_to_prune(&sessions, 2, "a"), ids(&["b"]));
        assert_eq!(sessions_to_prune(&sessions, 0, "d"), ids(&["a", "b", "c"]));
    }
}
