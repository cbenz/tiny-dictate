//! How a session is shown to a reader: the row of the listing, the fields of the detail and the
//! durations they carry.
//!
//! One module owns the shape of a session on screen, so the surfaces cannot drift: every reader of
//! a session comes here. It computes text from the durable side and never writes anything.

use std::fmt;
use std::fs;
use std::path::Path;

use crate::store;

/// The widest summary a listing row carries: enough words to recognize a session, not a page.
const SUMMARY_WORDS: usize = 60;

/// The column the detail's labels are padded to, so their values line up: the longest label is
/// `audio bytes`.
const LABEL_WIDTH: usize = 11;

/// What a row shows where the session itself has nothing: a duration that was never recorded, a
/// transcript that holds no word and no error.
const NONE: &str = "-";

/// What a row shows where the view knows nothing: a session whose metadata cannot be read.
const UNKNOWN: &str = "?";

/// What a session whose metadata cannot be read says, wherever it is shown.
const METADATA_UNREADABLE: &str = "the metadata cannot be read";

/// What a session holding no word of transcript says, wherever it is shown.
const NO_TRANSCRIPT: &str = "no transcript";

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

/// One field of a session's detail: what it is called, and what it reads.
///
/// The label is a `&'static str` because the detail's fields are a closed set, and the order they
/// are built in is the order every surface renders them.
pub struct Field {
    pub label: &'static str,
    pub value: String,
}

/// One kept session read in full: the fields of the detail, in the one order every surface renders
/// them, and its transcript.
///
/// The fields always start with the id and the path -- a directory has both, whatever its metadata
/// says -- and a session whose metadata cannot be read carries no field beyond them: the surfaces
/// say so rather than drawing an empty block.
pub struct Detail {
    /// The fields, in the one order: `id`, `path`, then the metadata's.
    pub fields: Vec<Field>,
    /// Whether the metadata could be read. `false` means `fields` stops after the path.
    pub metadata_reads: bool,
    /// The transcript, trimmed of the blank space around it; `None` when the session holds no word.
    pub transcript: Option<String>,
}

impl Detail {
    /// The detail of the session a directory holds: the id, the path, the metadata's fields and
    /// the transcript. It reads files and never writes one.
    pub fn read(sessions: &Path, id: &str) -> Self {
        let directory = sessions.join(id);
        let transcript = read_transcript(&directory.join(store::TRANSCRIPT_FILE));
        let mut fields = vec![
            Field {
                label: "id",
                value: id.to_owned(),
            },
            Field {
                label: "path",
                value: directory.display().to_string(),
            },
        ];

        let Some(session) = store::Session::load(sessions, id) else {
            return Self {
                fields,
                metadata_reads: false,
                transcript,
            };
        };

        let metadata = session.metadata;
        fields.extend([
            field("outcome", metadata.outcome.as_str()),
            field("started", metadata.started.to_string()),
            field(
                "ended",
                metadata
                    .ended
                    .map_or_else(|| NONE.to_owned(), |s| s.to_string()),
            ),
            field("duration", duration(metadata.duration)),
            field("audio bytes", metadata.audio_bytes.to_string()),
            field("pasted", if metadata.pasted { "yes" } else { "no" }),
            field(
                "transcriber",
                metadata.transcriber.unwrap_or_else(|| NONE.to_owned()),
            ),
            field("error", metadata.error.unwrap_or_else(|| NONE.to_owned())),
        ]);

        Self {
            fields,
            metadata_reads: true,
            transcript,
        }
    }
}

impl Detail {
    /// The detail as the lines every surface renders: the fields in their one order, the verdict
    /// when the metadata cannot be read, then the transcript -- or what the session has instead of
    /// one. The command line prints them as they are; the browser's pane wraps and scrolls them.
    pub fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .fields
            .iter()
            .map(|field| {
                format!(
                    "{:<width$}  {}",
                    field.label,
                    field.value,
                    width = LABEL_WIDTH
                )
            })
            .collect();

        if !self.metadata_reads {
            lines.push(METADATA_UNREADABLE.to_owned());
        }

        lines.push(String::new());

        match &self.transcript {
            Some(text) => lines.extend(text.lines().map(str::to_owned)),
            None => lines.push(NO_TRANSCRIPT.to_owned()),
        }

        lines
    }
}

impl fmt::Display for Detail {
    /// The lines every surface renders, one per line, and no trailing blank one. Both this and the
    /// browser's pane come from `lines`, so the two surfaces cannot drift.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, line) in self.lines().iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            write!(formatter, "{line}")?;
        }

        Ok(())
    }
}

/// One field of the detail, label and value already rendered.
fn field(label: &'static str, value: impl Into<String>) -> Field {
    Field {
        label,
        value: value.into(),
    }
}

/// The transcript a session holds, trimmed of the blank space around it, or `None` when the file
/// is missing or holds no word: a session with nothing to read says so rather than printing an
/// empty block.
fn read_transcript(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let text = text.trim();

    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
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

/// The kept sessions as one JSON array, newest first: the same sessions, in the same order, as data
/// a script reads rather than a layout a person reads.
///
/// Each element is the session's id, its path, and its metadata -- the object `metadata.json` holds,
/// from the one writer that writes that file -- or `null` when the metadata cannot be read. A
/// session whose metadata cannot be read appears rather than being skipped: the array is never
/// shorter than the sessions directory.
pub fn json_list(sessions: &Path) -> String {
    let elements: Vec<String> = store::session_ids(sessions)
        .iter()
        .map(|id| json_element(sessions, id, "  ", false))
        .collect();

    if elements.is_empty() {
        return "[]\n".to_owned();
    }

    format!("[\n  {}\n]\n", elements.join(",\n  "))
}

/// One kept session as one JSON object: the same fields as a listing element, plus the transcript
/// the session holds, or `null` when it holds none. `id`, `path` and `metadata` are the durable
/// values, never the human fields the text format renders.
pub fn json_show(sessions: &Path, id: &str) -> String {
    format!("{}\n", json_element(sessions, id, "", true))
}

/// One session as a JSON object drawn at `indent`: `id`, `path`, `metadata`, and, when asked,
/// `transcript`.
///
/// The metadata object is `Metadata::to_json_at`'s, the one writer of that shape, and the transcript
/// comes from the same reading the text view uses, so the two formats cannot disagree.
fn json_element(sessions: &Path, id: &str, indent: &str, with_transcript: bool) -> String {
    let field = format!("{indent}  ");
    let directory = sessions.join(id);
    let metadata = store::Session::load(sessions, id)
        .map(|session| session.metadata.to_json_at(&field))
        .unwrap_or_else(|| "null".to_owned());

    let mut element = format!(
        "{{\n{field}\"id\": {},\n{field}\"path\": {},\n{field}\"metadata\": {metadata}",
        store::json_string(id),
        store::json_string(&directory.display().to_string()),
    );

    if with_transcript {
        let value = read_transcript(&directory.join(store::TRANSCRIPT_FILE))
            .map_or_else(|| "null".to_owned(), |text| store::json_string(&text));
        element.push_str(&format!(",\n{field}\"transcript\": {value}"));
    }

    element.push_str(&format!("\n{indent}}}"));

    element
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
        write_session(&root, id, metadata_json, transcript);

        root
    }

    /// Add one session directory to a sessions root, built by hand.
    fn write_session(root: &Path, id: &str, metadata_json: Option<&str>, transcript: Option<&str>) {
        let directory = root.join(id);
        fs::create_dir_all(&directory).expect("a session directory");
        if let Some(json) = metadata_json {
            fs::write(directory.join(store::METADATA_FILE), json).expect("a metadata file");
        }
        if let Some(text) = transcript {
            fs::write(directory.join(store::TRANSCRIPT_FILE), text).expect("a transcript");
        }
    }

    #[test]
    fn the_detail_lines_are_the_fields_the_blank_line_and_the_transcript() {
        let detail = Detail {
            fields: vec![
                Field {
                    label: "id",
                    value: "2026-10-07T19-05-22".to_owned(),
                },
                Field {
                    label: "path",
                    value: "/sessions/2026-10-07T19-05-22".to_owned(),
                },
            ],
            metadata_reads: true,
            transcript: Some("hello\nworld".to_owned()),
        };

        assert_eq!(
            detail.lines(),
            [
                "id           2026-10-07T19-05-22",
                "path         /sessions/2026-10-07T19-05-22",
                "",
                "hello",
                "world",
            ]
        );
        assert_eq!(detail.to_string(), detail.lines().join("\n"));
    }

    #[test]
    fn the_detail_lines_carry_the_verdict_and_the_missing_transcript_where_the_surfaces_show_them()
    {
        let detail = Detail {
            fields: vec![Field {
                label: "id",
                value: "2026-10-07T19-05-22".to_owned(),
            }],
            metadata_reads: false,
            transcript: None,
        };

        assert_eq!(
            detail.lines(),
            [
                "id           2026-10-07T19-05-22",
                "the metadata cannot be read",
                "",
                "no transcript",
            ]
        );
        assert_eq!(detail.to_string(), detail.lines().join("\n"));
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

    #[test]
    fn the_detail_lists_the_id_the_path_and_the_metadata_fields_in_one_order() {
        let id = "2026-10-07T19-05-22";
        let root = sessions(
            "detail",
            id,
            Some(&metadata(id, "79.33", "transcribed", "null")),
            Some("hello world\n"),
        );

        let detail = Detail::read(&root, id);
        let labels: Vec<&str> = detail.fields.iter().map(|field| field.label).collect();
        let values: Vec<&str> = detail
            .fields
            .iter()
            .map(|field| field.value.as_str())
            .collect();

        assert!(detail.metadata_reads);
        assert_eq!(
            labels,
            [
                "id",
                "path",
                "outcome",
                "started",
                "ended",
                "duration",
                "audio bytes",
                "pasted",
                "transcriber",
                "error"
            ]
        );
        assert_eq!(values[0], id);
        assert_eq!(values[1], root.join(id).display().to_string());
        assert_eq!(values[2], "transcribed");
        assert_eq!(values[3], "1759855522.12");
        assert_eq!(values[4], "1759855601.45");
        assert_eq!(values[5], "1:19");
        assert_eq!(values[6], "365720");
        assert_eq!(values[7], "yes");
        assert_eq!(values[8], "/usr/local/bin/tiny-dictate-transcribe");
        assert_eq!(values[9], "-");
        assert_eq!(detail.transcript.as_deref(), Some("hello world"));
        assert!(detail.to_string().ends_with("\nhello world"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_session_with_no_transcript_says_so_rather_than_printing_an_empty_block() {
        let id = "2026-10-07T19-05-22";
        let blank = sessions(
            "detail-blank",
            id,
            Some(&metadata(id, "null", "cancelled", "null")),
            Some("\n \n"),
        );
        let missing = sessions(
            "detail-missing",
            id,
            Some(&metadata(id, "null", "cancelled", "null")),
            None,
        );

        for root in [&blank, &missing] {
            let detail = Detail::read(root, id);

            assert_eq!(detail.transcript, None);
            assert!(detail.to_string().ends_with("\nno transcript"));
        }

        let _ = fs::remove_dir_all(&blank);
        let _ = fs::remove_dir_all(&missing);
    }

    #[test]
    fn a_session_whose_metadata_cannot_be_read_keeps_its_id_and_its_path_and_says_so() {
        let id = "2026-10-07T19-05-22";
        let corrupt = sessions("detail-corrupt", id, Some("not json"), Some("orphan text"));

        let detail = Detail::read(&corrupt, id);
        let labels: Vec<&str> = detail.fields.iter().map(|field| field.label).collect();

        assert!(!detail.metadata_reads);
        assert_eq!(labels, ["id", "path"]);
        assert_eq!(detail.fields[0].value, id);
        assert_eq!(
            detail.fields[1].value,
            corrupt.join(id).display().to_string()
        );
        assert!(detail.to_string().contains("the metadata cannot be read"));

        let _ = fs::remove_dir_all(&corrupt);
    }

    #[test]
    fn the_json_listing_is_the_sessions_directory_as_data_newest_first() {
        let id = "2026-10-07T19-05-22";
        let unreadable = "2026-10-07T18-02-10";
        let root = std::env::temp_dir().join(format!("td-view-json-list-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        write_session(
            &root,
            id,
            Some(&metadata(id, "79.33", "transcribed", "null")),
            Some("hello world"),
        );
        write_session(&root, unreadable, Some("not json"), None);

        let document = json_list(&root);

        assert!(document.starts_with("[\n") && document.ends_with("]\n"));
        // Newest first: the readable, later timestamped session comes before the earlier one.
        assert!(document.find(id).unwrap() < document.find(unreadable).unwrap());
        assert!(document.contains(&format!("\"path\": \"{}\"", root.join(id).display())));
        // The metadata object is the durable one, and the times stay raw seconds.
        assert!(document.contains("\"duration\": 79.33"));
        assert!(document.contains("\"started\": 1759855522.12"));
        assert!(!document.contains("1:19"));
        // The session whose metadata cannot be read is an element with `null`, not a missing one.
        assert!(document.contains(&format!("\"id\": \"{unreadable}\"")));
        assert!(document.contains("\"metadata\": null"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_json_detail_is_the_element_plus_the_transcript_or_null() {
        let id = "2026-10-07T19-05-22";
        let cancelled = "2026-10-07T18-02-10";
        let root = std::env::temp_dir().join(format!("td-view-json-show-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        write_session(
            &root,
            id,
            Some(&metadata(id, "79.33", "transcribed", "null")),
            Some("hello world\n"),
        );
        write_session(
            &root,
            cancelled,
            Some(&metadata(cancelled, "null", "cancelled", "null")),
            None,
        );

        let transcribed = json_show(&root, id);
        let no_transcript = json_show(&root, cancelled);

        assert!(transcribed.contains(&format!("\"path\": \"{}\"", root.join(id).display())));
        assert!(transcribed.contains("\"transcript\": \"hello world\""));
        // A session with no word of transcript is `null`, never an empty string.
        assert!(no_transcript.contains("\"transcript\": null"));
        assert!(no_transcript.contains("\"outcome\": \"cancelled\""));

        let _ = fs::remove_dir_all(&root);
    }
}
