//! What the browser draws in one frame: the pure computation behind the terminal adapter.
//!
//! Everything here is a function of what the browser knows — the rows it read, the selection and
//! the terminal's size — and of nothing else. It opens no file, reads no clock and touches no
//! terminal, so one frame is testable without a display.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::view::{Detail, Row};

/// The two columns a row is prefixed with: the marker on the selected row, plain space otherwise.
const MARKER: &str = "> ";
const MARKER_GAP: &str = "  ";

/// The title, and what it says when nothing is kept.
const TITLE: &str = "sessions";
const EMPTY_TITLE: &str = "no kept sessions";

/// The keys that mean something right now, with the detail pane beside the list.
const FOOTER: &str = "↑/↓ move  Home/End jump  PgUp/PgDn scroll  c copy  i id  p path  t transcribe  d delete  r reload  q quit";

/// The same on a terminal too small for the pane: the page keys mean nothing there, the jump keys
/// still move the list.
const NARROW_FOOTER: &str =
    "↑/↓ move  Home/End jump  c copy  i id  p path  t transcribe  d delete  r reload  q quit";

/// The only two keys a pending confirmation reads: while the question stands, every other key is
/// ignored, so the footer names these two and nothing else.
const CONFIRM_FOOTER: &str = "y delete  n cancel";

/// The footer while a transcription runs: only `ctrl-c` means something until it returns, and the
/// footer names only that.
const RUNNING_FOOTER: &str = "ctrl-c cancel";

/// The column between the list and the detail pane.
pub const SEPARATOR: char = '│';

/// The narrowest the list pane may go while the detail pane sits beside it: room for one row.
const LIST_MIN_WIDTH: usize = 40;

/// The narrowest the detail pane may go.
const PANE_MIN_WIDTH: usize = 24;

/// The shortest terminal that shows both panes: the title, the footer and two body rows.
const MIN_PANE_HEIGHT: usize = 4;

/// What the browser is doing, as one frame draws it: the keys that mean something depend on it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Run {
    /// Nothing runs: navigation, the copy actions and `t` all mean something.
    Idle,
    /// A transcription is in flight: only `ctrl-c` means anything until it returns.
    Transcribing,
}

/// The list state: the rows on disk and the one the selection sits on.
pub struct List {
    rows: Vec<Row>,
    selection: usize,
}

impl List {
    /// A fresh list, the selection on the newest row.
    pub fn new(rows: Vec<Row>) -> Self {
        Self { rows, selection: 0 }
    }

    /// The rows, newest first.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The index of the selected row; always a valid index, or `0` on an empty list.
    pub fn selection(&self) -> usize {
        self.selection
    }

    /// The selected row, or `None` on an empty list.
    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.selection)
    }

    /// Whether nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Move down one row; the first row is where the last one stops.
    pub fn select_next(&mut self) {
        if self.selection + 1 < self.rows.len() {
            self.selection += 1;
        }
    }

    /// Move up one row; the last row is where the first one stops.
    pub fn select_previous(&mut self) {
        self.selection = self.selection.saturating_sub(1);
    }

    /// Jump to the newest row.
    pub fn select_first(&mut self) {
        self.selection = 0;
    }

    /// Jump to the oldest row.
    pub fn select_last(&mut self) {
        self.selection = self.rows.len().saturating_sub(1);
    }

    /// Replace the rows after a reload.
    ///
    /// The selection follows the session it was on when that session is still there, so a reload
    /// costs the user no place; when it is gone the selection clamps to the last row.
    pub fn reload(&mut self, rows: Vec<Row>) {
        let selected = self.selected().map(|row| row.id.clone());
        self.selection = selected
            .and_then(|id| rows.iter().position(|row| row.id == id))
            .unwrap_or_else(|| self.selection.min(rows.len().saturating_sub(1)));
        self.rows = rows;
    }
}

/// One drawn line: the text, and whether it carries the selection.
pub struct Line {
    pub text: String,
    pub selected: bool,
}

/// What one frame draws: the title, the list pane, the detail pane beside it, the status line and
/// the footer.
pub struct Frame {
    pub title: String,
    /// The list pane, one entry per body row; padded with empty lines when the two panes are drawn.
    pub lines: Vec<Line>,
    /// The detail pane, aligned with `lines` by index; empty when there is no pane to draw.
    pub pane: Vec<String>,
    /// The columns the list pane takes before the separator, or `0` when there is no pane to draw
    /// beside it.
    pub list_columns: usize,
    /// What the last action did, above the footer; empty when no action has run.
    pub status: String,
    pub footer: String,
}

/// The detail pane's scroll: how far down the selected session's detail is read.
pub struct Pane {
    scroll: usize,
}

impl Pane {
    /// A fresh pane, reading the detail from its top.
    pub fn new() -> Self {
        Self { scroll: 0 }
    }

    /// How far down the pane is scrolled.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Back to the top: what a reload of the pane's session does, and what `Home` asks for.
    pub fn reset(&mut self) {
        self.scroll = 0;
    }

    /// The end of the detail, at the last offset it can take.
    pub fn last(&mut self, limit: usize) {
        self.scroll = limit;
    }

    /// Up by `page` lines, never above the top.
    pub fn page_up(&mut self, page: usize) {
        self.scroll = self.scroll.saturating_sub(page);
    }

    /// Down by `page` lines, never past `limit`.
    pub fn page_down(&mut self, page: usize, limit: usize) {
        self.scroll = self.scroll.saturating_add(page).min(limit);
    }
}

impl Default for Pane {
    fn default() -> Self {
        Self::new()
    }
}

/// What a key answered to a pending confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// `y`: destroy the session the question names.
    Yes,
    /// `n`: keep the session.
    No,
    /// Any other key: the question stands and nothing is decided.
    Nothing,
}

/// The pending confirmation: the session a `d` asked about before anything is destroyed.
///
/// It holds the id alone, so the question it draws and the answer a key gives are pure functions
/// of it: what `d` asks is testable without a terminal and without a filesystem.
pub struct Confirm {
    id: String,
}

impl Confirm {
    /// Ask before destroying the session `id` names.
    pub fn ask(id: &str) -> Self {
        Self { id: id.to_owned() }
    }

    /// The session the question names.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The line the question draws: the session's id and the two keys that answer it.
    pub fn question(&self) -> String {
        format!("delete {}? y/n", self.id)
    }

    /// What `key` answers: `y` and `n` answer, every other key answers nothing, so the question
    /// stands until it is answered on purpose.
    pub fn answer(&self, key: char) -> Answer {
        match key {
            'y' => Answer::Yes,
            'n' => Answer::No,
            _ => Answer::Nothing,
        }
    }
}

/// The widths the list pane and the detail pane take on a terminal `width` columns by `height`
/// rows, or `None` when it is too small for both: the pane then gives way and the list keeps the
/// whole width.
pub fn panes(width: usize, height: usize) -> Option<(usize, usize)> {
    if height < MIN_PANE_HEIGHT {
        return None;
    }

    let pane = (width / 3).max(PANE_MIN_WIDTH);
    let list = width.checked_sub(pane + 1)?;

    (list >= LIST_MIN_WIDTH).then_some((list, pane))
}

/// What one frame draws, for a terminal `width` columns wide and `height` rows tall.
///
/// The title, the footer and, when there is one, the status line take a line each; the body uses
/// the rest. The footer follows `run` and `confirm`: a pending question names only the two keys
/// that answer it, and a transcription in flight names only the key that cancels it, because the
/// other keys mean nothing until it returns. The list is scrolled so its selection is always
/// visible, the detail pane is windowed by `scroll`, and the two are drawn side by side. On a
/// terminal too small for both, the pane gives way and the list keeps the whole width. A line is
/// cut to the pane's width, counting a wide character as two columns, so the cut never lands
/// inside one. A pending `confirm` replaces the status line with its question: what `d` asked is
/// what the user reads.
// The pure frame takes the whole drawing context at once -- the state, the run, the pending
// question and the geometry -- and splitting it would spread the same fields over every call site.
#[allow(clippy::too_many_arguments)]
pub fn frame(
    list: &List,
    detail: Option<&Detail>,
    run: Run,
    status: &str,
    confirm: Option<&Confirm>,
    scroll: usize,
    width: usize,
    height: usize,
) -> Frame {
    // The question a pending confirmation asks takes the status line, so a stale action's line
    // never sits under the keys that would answer it.
    let status = match confirm {
        Some(confirm) => confirm.question(),
        None => status.to_owned(),
    };
    let capacity = height.saturating_sub(2 + usize::from(!status.is_empty()));
    let split = detail.and_then(|_| panes(width, height));
    // The columns each row is drawn in: the whole width when the pane gives way, the list pane's
    // columns when it does not.
    let row_width = split.map_or(width, |(list_columns, _)| list_columns);
    let first = window_start(list.selection(), list.rows().len(), capacity);
    let mut lines: Vec<Line> = list
        .rows()
        .iter()
        .enumerate()
        .skip(first)
        .take(capacity)
        .map(|(index, row)| Line {
            text: truncate(
                &format!(
                    "{}{}",
                    if index == list.selection() {
                        MARKER
                    } else {
                        MARKER_GAP
                    },
                    row
                ),
                row_width,
            ),
            selected: index == list.selection(),
        })
        .collect();

    let mut pane = Vec::new();
    if let Some((_, pane_width)) = split {
        pane = detail_pane(detail, pane_width, capacity, scroll);
        // The two panes share the body: the shorter one is padded so the rows stay aligned.
        lines.resize_with(capacity, || Line {
            text: String::new(),
            selected: false,
        });
        pane.resize(capacity, String::new());
    }

    Frame {
        title: truncate(if list.is_empty() { EMPTY_TITLE } else { TITLE }, width),
        lines,
        pane,
        list_columns: split.map_or(0, |(list_columns, _)| list_columns),
        status: truncate(&status, width),
        footer: truncate(
            match (run, confirm) {
                (_, Some(_)) => CONFIRM_FOOTER,
                (Run::Transcribing, None) => RUNNING_FOOTER,
                (Run::Idle, None) if split.is_some() => FOOTER,
                (Run::Idle, None) => NARROW_FOOTER,
            },
            width,
        ),
    }
}

/// The lines the detail pane draws: the shared view's lines wrapped to the pane's width, windowed
/// by `scroll` and `height`. Nothing selected is nothing to draw, never a stale pane.
pub fn detail_pane(
    detail: Option<&Detail>,
    width: usize,
    height: usize,
    scroll: usize,
) -> Vec<String> {
    let Some(detail) = detail else {
        return Vec::new();
    };

    window(&pane_lines(detail, width), height, scroll)
}

/// The detail's lines wrapped to `width` display columns: what the pane scrolls through.
pub fn pane_lines(detail: &Detail, width: usize) -> Vec<String> {
    detail
        .lines()
        .iter()
        .flat_map(|line| wrap(line, width))
        .collect()
}

/// The `height` lines starting at `scroll`, never before the top nor past the last line.
fn window(lines: &[String], height: usize, scroll: usize) -> Vec<String> {
    let start = scroll.min(lines.len().saturating_sub(height));

    lines.iter().skip(start).take(height).cloned().collect()
}

/// One logical line wrapped to `width` display columns: the words keep the spaces between them
/// when the run fits, so a field's label and its value stay in their columns, and a word the pane
/// cannot hold is cut in pieces that never split a wide character. An empty line stays one empty
/// line.
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut line = String::new();
    let mut spaces = String::new();
    let mut word = String::new();

    for character in text.chars() {
        if character.is_whitespace() {
            if !word.is_empty() {
                place(&mut lines, &mut line, &mut spaces, &word, width);
                word.clear();
            }
            spaces.push(character);
        } else {
            word.push(character);
        }
    }

    if !word.is_empty() {
        place(&mut lines, &mut line, &mut spaces, &word, width);
    }

    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }

    lines
}

/// Put one word on the line, after the spaces that precede it, breaking the line first when the
/// two together would run past the pane's width.
fn place(
    lines: &mut Vec<String>,
    line: &mut String,
    spaces: &mut String,
    word: &str,
    width: usize,
) {
    let used = UnicodeWidthStr::width(line.as_str());
    let gap = UnicodeWidthStr::width(spaces.as_str());
    if !line.is_empty() && used + gap + UnicodeWidthStr::width(word) > width {
        lines.push(std::mem::take(line));
        spaces.clear();
    }

    line.push_str(spaces);
    spaces.clear();
    push_word(lines, line, word, width);
}

/// Put one word on the line, cutting it into lines of its own when the pane cannot hold it.
fn push_word(lines: &mut Vec<String>, line: &mut String, word: &str, width: usize) {
    if UnicodeWidthStr::width(word) <= width {
        line.push_str(word);
        return;
    }

    // The word is wider than the pane: every piece but the last fills a line, and the last starts
    // the next line, so a following word can still share it.
    let mut pieces = chunks(word, width);
    *line = pieces.pop().unwrap_or_default();
    lines.extend(pieces);
}

/// `word` cut into pieces at most `width` display columns wide: every piece but the last is
/// exactly `width`, and no piece splits a wide character in two.
fn chunks(word: &str, width: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    let mut used = 0;

    for character in word.chars() {
        let columns = character.width().unwrap_or(0);
        if used + columns > width && !piece.is_empty() {
            pieces.push(std::mem::take(&mut piece));
            used = 0;
        }
        piece.push(character);
        used += columns;
    }

    pieces.push(piece);
    pieces
}

/// A line padded with spaces to `width` display columns, so what follows starts in the same column
/// whatever the line holds.
pub(crate) fn pad(text: &str, width: usize) -> String {
    let mut padded = text.to_owned();
    padded.push_str(&" ".repeat(width.saturating_sub(UnicodeWidthStr::width(text))));
    padded
}

/// The first row of the visible window: the selection is shown, and the window holds whole rows.
fn window_start(selection: usize, rows: usize, capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }

    selection
        .saturating_sub(capacity - 1)
        .min(rows.saturating_sub(capacity))
}

/// The text cut to `width` display columns, a wide character counted as two.
fn truncate(text: &str, width: usize) -> String {
    let mut used = 0;
    let mut cut = String::new();

    for character in text.chars() {
        let columns = character.width().unwrap_or(0);
        if used + columns > width {
            break;
        }
        cut.push(character);
        used += columns;
    }

    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Field;
    use unicode_width::UnicodeWidthStr;

    fn row(id: &str) -> Row {
        Row {
            id: id.to_owned(),
            duration: "0:01".to_owned(),
            outcome: "transcribed".to_owned(),
            summary: "hello".to_owned(),
        }
    }

    /// The detail of a session, built by hand: the pane under test reads the view, not a file.
    fn detail(transcript: &str) -> Detail {
        Detail {
            fields: vec![
                Field {
                    label: "id",
                    value: "2026-10-09T10-00-00".to_owned(),
                },
                Field {
                    label: "path",
                    value: "/sessions/2026-10-09T10-00-00".to_owned(),
                },
                Field {
                    label: "outcome",
                    value: "transcribed".to_owned(),
                },
                Field {
                    label: "error",
                    value: "-".to_owned(),
                },
            ],
            metadata_reads: true,
            transcript: Some(transcript.to_owned()),
        }
    }

    /// A session holding only a transcript, so a wrapping test reads nothing but the text.
    fn transcript_only(transcript: &str) -> Detail {
        Detail {
            fields: Vec::new(),
            metadata_reads: true,
            transcript: Some(transcript.to_owned()),
        }
    }

    #[test]
    fn a_status_line_sits_above_the_footer_and_takes_one_row_from_the_list() {
        let list = List::new(vec![row("d"), row("c"), row("b"), row("a")]);

        let frame = frame(
            &list,
            None,
            Run::Idle,
            "id copied to the clipboard",
            None,
            0,
            80,
            5,
        );

        // The title and the footer take a line each, the status line one more: two rows fit.
        assert_eq!(frame.status, "id copied to the clipboard");
        assert_eq!(frame.lines.len(), 2);
        assert_eq!(frame.lines[0].text, "> d  0:01  transcribed  hello");
        assert!(!frame.footer.is_empty());
    }

    #[test]
    fn a_delete_asks_before_destroying_and_only_y_and_n_answer() {
        let confirm = Confirm::ask("2026-10-09T10-00-00");

        assert_eq!(confirm.id(), "2026-10-09T10-00-00");
        assert_eq!(confirm.question(), "delete 2026-10-09T10-00-00? y/n");
        assert_eq!(confirm.answer('y'), Answer::Yes);
        assert_eq!(confirm.answer('n'), Answer::No);
        assert_eq!(confirm.answer('x'), Answer::Nothing);
        assert_eq!(confirm.answer('q'), Answer::Nothing);
        assert_eq!(confirm.answer(' '), Answer::Nothing);
    }

    #[test]
    fn the_pending_question_takes_the_status_line_of_the_frame() {
        let list = List::new(vec![row("2026-10-09T10-00-00")]);
        let confirm = Confirm::ask("2026-10-09T10-00-00");

        let frame = frame(
            &list,
            None,
            Run::Idle,
            "id copied to the clipboard",
            Some(&confirm),
            0,
            80,
            5,
        );

        assert_eq!(frame.status, "delete 2026-10-09T10-00-00? y/n");
        assert_eq!(frame.footer, "y delete  n cancel");
    }

    #[test]
    fn a_frame_without_a_status_draws_no_status_line_and_keeps_the_whole_height_for_the_rows() {
        let list = List::new(vec![row("d"), row("c"), row("b"), row("a")]);

        let frame = frame(&list, None, Run::Idle, "", None, 0, 80, 5);

        assert!(frame.status.is_empty());
        assert_eq!(frame.lines.len(), 3);
    }

    #[test]
    fn a_transcription_in_flight_names_only_the_key_that_cancels() {
        let list = List::new(vec![row("c")]);

        let idle = frame(&list, None, Run::Idle, "", None, 0, 80, 24);
        let running = frame(
            &list,
            None,
            Run::Transcribing,
            "transcribing c",
            None,
            0,
            80,
            24,
        );

        assert!(idle.footer.contains("c copy"));
        assert_eq!(running.footer, "ctrl-c cancel");
        assert_eq!(running.status, "transcribing c");
    }

    #[test]
    fn a_frame_lists_one_line_per_session_newest_first_with_the_first_selected() {
        let list = List::new(vec![row("2026-10-09T17-00-00"), row("2026-10-08T17-00-00")]);

        let frame = frame(&list, None, Run::Idle, "", None, 0, 80, 24);

        assert_eq!(frame.title, "sessions");
        assert_eq!(frame.lines.len(), 2);
        assert_eq!(
            frame.lines[0].text,
            "> 2026-10-09T17-00-00  0:01  transcribed  hello"
        );
        assert!(frame.lines[0].selected);
        assert_eq!(
            frame.lines[1].text,
            "  2026-10-08T17-00-00  0:01  transcribed  hello"
        );
        assert!(!frame.lines[1].selected);
    }

    #[test]
    fn the_selection_moves_by_one_and_stops_at_each_end() {
        let mut list = List::new(vec![row("c"), row("b"), row("a")]);

        assert_eq!(list.selected().expect("a selection").id, "c");
        list.select_next();
        assert_eq!(list.selected().expect("a selection").id, "b");
        list.select_previous();
        list.select_previous();
        assert_eq!(list.selected().expect("a selection").id, "c");

        list.select_last();
        assert_eq!(list.selected().expect("a selection").id, "a");
        list.select_next();
        assert_eq!(list.selected().expect("a selection").id, "a");
        list.select_first();
        assert_eq!(list.selected().expect("a selection").id, "c");
        list.select_previous();
        assert_eq!(list.selection(), 0);
    }

    #[test]
    fn the_selection_on_an_empty_list_is_nothing_and_the_keys_are_harmless() {
        let mut list = List::new(Vec::new());

        assert!(list.is_empty());
        assert!(list.selected().is_none());
        list.select_next();
        list.select_previous();
        list.select_last();
        list.select_first();
        assert_eq!(list.selection(), 0);
    }

    #[test]
    fn an_empty_history_is_an_empty_list_and_not_an_error() {
        let list = List::new(Vec::new());

        let frame = frame(&list, None, Run::Idle, "", None, 0, 80, 24);

        assert_eq!(frame.title, "no kept sessions");
        assert!(frame.lines.is_empty());
        assert!(!frame.footer.is_empty());
    }

    #[test]
    fn a_selection_below_the_window_scrolls_the_rows_into_view() {
        let mut list = List::new(vec![row("d"), row("c"), row("b"), row("a")]);
        list.select_last();

        // The title and the footer take a line each: one row is all a three-line terminal holds.
        let frame = frame(&list, None, Run::Idle, "", None, 0, 80, 3);

        assert_eq!(frame.lines.len(), 1);
        assert_eq!(frame.lines[0].text, "> a  0:01  transcribed  hello");
        assert!(frame.lines[0].selected);
    }

    #[test]
    fn a_line_is_cut_to_the_width_counting_a_wide_character_as_two() {
        let list = List::new(vec![Row {
            id: "c".to_owned(),
            duration: "0:00".to_owned(),
            outcome: "x".to_owned(),
            summary: "日本".to_owned(),
        }]);

        // "> c  0:00  x  日本" is 18 columns; at 17 the second ideograph does not fit whole.
        let frame = frame(&list, None, Run::Idle, "", None, 0, 17, 24);

        assert_eq!(frame.lines[0].text, "> c  0:00  x  日");
        assert!(UnicodeWidthStr::width(frame.lines[0].text.as_str()) <= 17);
        assert_eq!(UnicodeWidthStr::width("日本"), 4);
    }

    #[test]
    fn a_reload_keeps_the_selection_on_the_same_session_when_it_is_still_there() {
        let mut list = List::new(vec![row("c"), row("b")]);
        list.select_next();

        list.reload(vec![row("d"), row("c"), row("b")]);

        assert_eq!(list.selected().expect("a selection").id, "b");
    }

    #[test]
    fn a_reload_whose_selected_session_is_gone_clamps_the_selection() {
        let mut list = List::new(vec![row("c"), row("b"), row("a")]);
        list.select_last();

        list.reload(vec![row("c")]);

        assert_eq!(list.selection(), 0);
        assert_eq!(list.selected().expect("a selection").id, "c");
    }

    #[test]
    fn a_reload_to_an_empty_list_leaves_no_selection_and_no_error() {
        let mut list = List::new(vec![row("c")]);
        list.select_last();

        list.reload(Vec::new());

        assert!(list.is_empty());
        assert_eq!(list.selection(), 0);
        assert!(list.selected().is_none());
    }

    #[test]
    fn the_detail_pane_lays_out_the_shared_view_lines_in_their_one_order() {
        let detail = detail("hello world");

        let lines = pane_lines(&detail, 60);

        assert_eq!(
            lines,
            [
                "id           2026-10-09T10-00-00",
                "path         /sessions/2026-10-09T10-00-00",
                "outcome      transcribed",
                "error        -",
                "",
                "hello world",
            ]
        );
    }

    #[test]
    fn the_pane_says_when_a_session_has_no_transcript_or_its_metadata_cannot_be_read() {
        let mut no_transcript = detail("");
        no_transcript.transcript = None;

        let mut unreadable = detail("orphan text");
        unreadable.metadata_reads = false;

        assert_eq!(
            pane_lines(&no_transcript, 60).last().expect("a line"),
            "no transcript"
        );
        assert!(pane_lines(&unreadable, 60).contains(&"the metadata cannot be read".to_owned()));
    }

    #[test]
    fn a_list_with_nothing_selected_draws_an_empty_pane_and_not_a_stale_one() {
        assert!(detail_pane(None, 40, 10, 0).is_empty());
    }

    #[test]
    fn the_pane_wraps_at_the_width_counting_display_width() {
        let detail = transcript_only("hello world this is a longer sentence");

        let lines = pane_lines(&detail, 20);

        assert_eq!(&lines[..1], [""]);
        assert_eq!(&lines[1..], ["hello world this is", "a longer sentence"]);
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.as_str()) <= 20)
        );
    }

    #[test]
    fn a_wrapped_line_never_splits_a_wide_character() {
        let detail = transcript_only("日本語のテキスト");

        let lines = pane_lines(&detail, 6);

        assert_eq!(&lines[1..], ["日本語", "のテキ", "スト"]);
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.as_str()) <= 6)
        );
    }

    #[test]
    fn the_pane_scrolls_a_page_at_a_time_and_clamps_at_either_end() {
        let detail =
            transcript_only("one two three four five six seven eight nine ten eleven twelve");
        let lines = pane_lines(&detail, 12);
        let height = 3;
        let limit = lines.len().saturating_sub(height);

        assert!(
            limit >= 3,
            "the transcript must be longer than the pane: {lines:?}"
        );
        assert_eq!(detail_pane(Some(&detail), 12, height, 0), lines[..height]);

        let mut pane = Pane::new();
        pane.page_down(height, limit);
        assert_eq!(pane.scroll(), height);
        assert_eq!(
            detail_pane(Some(&detail), 12, height, pane.scroll()),
            lines[height..height * 2]
        );

        pane.page_up(height);
        assert_eq!(pane.scroll(), 0);

        pane.last(limit);
        assert_eq!(pane.scroll(), limit);
        assert_eq!(
            detail_pane(Some(&detail), 12, height, pane.scroll()),
            lines[limit..]
        );

        pane.reset();
        assert_eq!(pane.scroll(), 0);

        // A scroll past the end reads the last page, never nothing.
        let end = detail_pane(Some(&detail), 12, height, 9_999);
        assert_eq!(end.len(), height);
        assert_eq!(end.last(), lines.last());
    }

    #[test]
    fn a_terminal_too_small_for_both_panes_gives_the_pane_way_and_keeps_the_list_readable() {
        let list = List::new(vec![row("c")]);
        let detail = detail("hello");

        // Too short: the title, the footer and two body rows are the least two panes need.
        let short = frame(&list, Some(&detail), Run::Idle, "", None, 0, 100, 3);
        // Too narrow: the list pane may not go below 40 columns.
        let narrow = frame(&list, Some(&detail), Run::Idle, "", None, 0, 40, 24);

        for frame in [&short, &narrow] {
            assert!(frame.pane.is_empty());
            assert_eq!(frame.list_columns, 0);
            assert_eq!(frame.lines[0].text, "> c  0:01  transcribed  hello");
            assert!(frame.lines[0].selected);
            assert!(frame.footer.contains("Home/End jump"));
        }

        assert_eq!(panes(40, 24), None);
        assert_eq!(panes(100, 3), None);
        assert_eq!(panes(100, 24), Some((66, 33)));
    }

    #[test]
    fn the_detail_pane_sits_beside_the_list_and_follows_the_selection() {
        let mut list = List::new(vec![row("2026-10-09T10-00-00"), row("2026-10-08T10-00-00")]);
        let newest = detail("newer text");
        let oldest = detail("older text");

        let first = frame(&list, Some(&newest), Run::Idle, "", None, 0, 100, 10);

        assert_eq!(first.list_columns, 66);
        assert_eq!(first.lines.len(), 8);
        assert!(first.footer.contains("Home/End jump"));
        assert!(first.footer.contains("PgUp/PgDn"));
        assert!(first.pane.iter().any(|line| line == "newer text"));
        assert!(!first.pane.iter().any(|line| line == "older text"));

        // The selection moves: the pane is reloaded on the row it now sits on.
        list.select_next();
        let second = frame(&list, Some(&oldest), Run::Idle, "", None, 0, 100, 10);

        assert!(second.pane.iter().any(|line| line == "older text"));
        assert!(!second.pane.iter().any(|line| line == "newer text"));
    }

    #[test]
    fn a_line_is_padded_by_display_width_so_what_follows_starts_in_one_column() {
        assert_eq!(pad("ab", 5), "ab   ");
        assert_eq!(pad("abc", 2), "abc");
        assert_eq!(UnicodeWidthStr::width(pad("日本", 6).as_str()), 6);
    }
}
