//! How a session is shown to a reader: the row of the listing and the durations it carries.
//!
//! One module owns the shape of a session on screen, so the surfaces cannot drift: every reader of
//! a session comes here. It computes text from the durable side and never writes anything.

use std::fmt;
use std::fs;
use std::path::Path;

use crate::store;

/// The widest summary a listing row carries: enough words to recognize a session, not a page.
const SUMMARY_WORDS: usize = 60;

/// What a row shows where the session itself has nothing: a duration that was never recorded, a
/// transcript that holds no word and no error.
const NONE: &str = "-";

/// What a row shows where the view knows nothing: a session whose metadata cannot be read.
const UNKNOWN: &str = "?";

/// One row of the listing: `<id>  <duration>  <outcome>  <summary>`.
pub struct Row {
    /// The session's id: the name of the directory it lives in.
    pub id: String,
    /// The duration a person reads: `4:32`, `1:04:32`, or `-` when the session died
    /// mid-recording.
    pub duration: String,
    /// The outcome, in the vocabulary `metadata.json` holds, or `?` when it cannot be read.
    pub outcome: String,
    /// The error when there is one, else the first words of the transcript, else `-`.
    pub summary: String,
}

impl Row {
    /// The row of the session a directory holds.
    ///
    /// A session whose metadata cannot be read is a row of `?` rather than no row at all: it is
    /// exactly the session `session delete` can still remove, so it is shown.
    pub fn read(sessions: &Path, id: &str) -> Self {
        let Some(metadata) = read_metadata(sessions, id) else {
            return Self {
                id: id.to_owned(),
                duration: UNKNOWN.to_owned(),
                outcome: UNKNOWN.to_owned(),
                summary: UNKNOWN.to_owned(),
            };
        };

        Self {
            id: id.to_owned(),
            duration: duration(metadata.duration),
            outcome: metadata.outcome.as_str().to_owned(),
            summary: metadata
                .error
                .or_else(|| transcript_words(&sessions.join(id).join(store::TRANSCRIPT_FILE)))
                .unwrap_or_else(|| NONE.to_owned()),
        }
    }
}

impl fmt::Display for Row {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}  {}  {}  {}",
            self.id, self.duration, self.outcome, self.summary
        )
    }
}

/// The kept sessions as rows, newest first.
///
/// The listing and the browser read the same rows through here, so the two surfaces cannot show
/// different fields or a different order.
pub fn rows(sessions: &Path) -> Vec<Row> {
    store::session_ids(sessions)
        .iter()
        .map(|id| Row::read(sessions, id))
        .collect()
}

/// The duration a person reads: `4:32` under an hour, `1:04:32` beyond, `-` when the session has
/// none because it died mid-recording.
pub fn duration(seconds: Option<f64>) -> String {
    let Some(seconds) = seconds.filter(|seconds| seconds.is_finite() && *seconds >= 0.0) else {
        return NONE.to_owned();
    };

    let whole = seconds as u64;
    let (hours, minutes, seconds) = (whole / 3600, whole % 3600 / 60, whole % 60);

    if hours == 0 {
        format!("{minutes}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

/// The first words of a transcript, or of an error, on a row.
pub fn first_words(text: &str, limit: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        return collapsed;
    }

    // Take `limit` characters; when the cut would land inside a word, back up to the end of the
    // last complete one, so a row never ends in half a word.
    let end = collapsed
        .char_indices()
        .nth(limit)
        .map_or(collapsed.len(), |(index, _)| index);
    let cut = if collapsed[end..].starts_with(' ') {
        end
    } else {
        collapsed[..end].rfind(' ').unwrap_or(end)
    };

    format!("{}…", &collapsed[..cut])
}

/// The metadata of one session, or `None` when the directory holds none this version can read.
fn read_metadata(sessions: &Path, id: &str) -> Option<store::Metadata> {
    store::Session::load(sessions, id).map(|session| session.metadata)
}

/// The first words of a transcript file, when it holds any.
fn transcript_words(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let words = first_words(&text, SUMMARY_WORDS);
    if words.is_empty() { None } else { Some(words) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A metadata file as the tool writes one: `ended` and `duration` are `null` while a session
    /// died mid-recording, and `error` holds the failure, or `null`.
    fn metadata(id: &str, duration: &str, outcome: &str, error: &str) -> String {
        format!(
            "{{\n  \"id\": \"{id}\",\n  \"started\": 1759855522.12,\n  \"ended\": 1759855601.45,\n  \"duration\": {duration},\n  \"audio_bytes\": 365720,\n  \"outcome\": \"{outcome}\",\n  \"pasted\": true,\n  \"transcriber\": \"/usr/local/bin/tiny-dictate-transcribe\",\n  \"error\": {error}\n}}\n"
        )
    }

    /// A sessions directory holding one session, built by hand: the reader under test must agree
    /// with the files, not with the writer.
    fn sessions(
        test: &str,
        id: &str,
        metadata_json: Option<&str>,
        transcript: Option<&str>,
    ) -> PathBuf {
        let root = std::env::temp_dir().join(format!("td-view-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let directory = root.join(id);
        fs::create_dir_all(&directory).expect("a session directory");
        if let Some(json) = metadata_json {
            fs::write(directory.join(store::METADATA_FILE), json).expect("a metadata file");
        }
        if let Some(text) = transcript {
            fs::write(directory.join(store::TRANSCRIPT_FILE), text).expect("a transcript");
        }

        root
    }

    #[test]
    fn durations_read_as_minutes_under_an_hour_and_carry_the_hours_beyond() {
        assert_eq!(duration(None), "-");
        assert_eq!(duration(Some(0.0)), "0:00");
        assert_eq!(duration(Some(59.9)), "0:59");
        assert_eq!(duration(Some(60.0)), "1:00");
        assert_eq!(duration(Some(272.0)), "4:32");
        assert_eq!(duration(Some(3600.0)), "1:00:00");
        assert_eq!(duration(Some(3872.5)), "1:04:32");
    }

    #[test]
    fn first_words_flattens_and_cuts_on_a_word_boundary() {
        assert_eq!(first_words("  hello\n\tworld ", 60), "hello world");
        assert_eq!(first_words("one two three", 7), "one two…");
        assert_eq!(first_words("one two three", 13), "one two three");
    }

    #[test]
    fn a_row_carries_the_id_the_human_duration_the_outcome_and_the_first_words() {
        let id = "2026-10-07T19-05-22";
        let root = sessions(
            "row",
            id,
            Some(&metadata(id, "79.33", "transcribed", "null")),
            Some("hello world, this is what was said"),
        );

        let row = Row::read(&root, id);

        assert_eq!(
            row.to_string(),
            "2026-10-07T19-05-22  1:19  transcribed  hello world, this is what was said"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_error_is_the_summary_when_the_session_has_one() {
        let id = "2026-10-07T19-05-22";
        let root = sessions(
            "error",
            id,
            Some(&metadata(
                id,
                "79.33",
                "failed",
                "\"groq: Error code: 401 - invalid api key\"",
            )),
            Some("stale text"),
        );

        let row = Row::read(&root, id);

        assert_eq!(row.outcome, "failed");
        assert_eq!(row.summary, "groq: Error code: 401 - invalid api key");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_session_that_died_mid_recording_has_no_duration_and_no_summary() {
        let id = "2026-10-07T19-05-22";
        let root = sessions(
            "mid-recording",
            id,
            Some(&metadata(id, "null", "recording", "null")),
            None,
        );

        let row = Row::read(&root, id);

        assert_eq!(row.to_string(), "2026-10-07T19-05-22  -  recording  -");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_session_whose_metadata_cannot_be_read_is_a_row_of_unknowns() {
        let id = "2026-10-07T19-05-22";
        let missing = sessions("unreadable-missing", id, None, Some("orphan text"));
        let corrupt = sessions("unreadable-corrupt", id, Some("not json"), None);

        assert_eq!(
            Row::read(&missing, id).to_string(),
            format!("{id}  ?  ?  ?")
        );
        assert_eq!(
            Row::read(&corrupt, id).to_string(),
            format!("{id}  ?  ?  ?")
        );

        let _ = fs::remove_dir_all(&missing);
        let _ = fs::remove_dir_all(&corrupt);
    }
}
