//! What the browser draws in one frame: the pure computation behind the terminal adapter.
//!
//! Everything here is a function of what the browser knows — the rows it read, the selection and
//! the terminal's size — and of nothing else. It opens no file, reads no clock and touches no
//! terminal, so one frame is testable without a display.

use unicode_width::UnicodeWidthChar;

use crate::view::Row;

/// The two columns a row is prefixed with: the marker on the selected row, plain space otherwise.
const MARKER: &str = "> ";
const MARKER_GAP: &str = "  ";

/// The title, and what it says when nothing is kept.
const TITLE: &str = "sessions";
const EMPTY_TITLE: &str = "no kept sessions";

/// The keys that mean something right now.
const FOOTER: &str = "↑/↓ move  Home/End jump  r reload  q quit";

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

/// What one frame draws: the title, the visible rows and the footer.
pub struct Frame {
    pub title: String,
    pub lines: Vec<Line>,
    pub footer: String,
}

/// What one frame draws, for a terminal `width` columns wide and `height` rows tall.
///
/// The title and the footer take one line each; the rows use the rest, scrolled so the selection
/// is always visible. A line is cut to the width, counting a wide character as two columns so the
/// cut never lands inside one.
pub fn frame(list: &List, width: usize, height: usize) -> Frame {
    let capacity = height.saturating_sub(2);
    let first = window_start(list.selection(), list.rows().len(), capacity);
    let lines = list
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
                width,
            ),
            selected: index == list.selection(),
        })
        .collect();

    Frame {
        title: truncate(if list.is_empty() { EMPTY_TITLE } else { TITLE }, width),
        lines,
        footer: truncate(FOOTER, width),
    }
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
    use unicode_width::UnicodeWidthStr;

    fn row(id: &str) -> Row {
        Row {
            id: id.to_owned(),
            duration: "0:01".to_owned(),
            outcome: "transcribed".to_owned(),
            summary: "hello".to_owned(),
        }
    }

    #[test]
    fn a_frame_lists_one_line_per_session_newest_first_with_the_first_selected() {
        let list = List::new(vec![row("2026-10-09T17-00-00"), row("2026-10-08T17-00-00")]);

        let frame = frame(&list, 80, 24);

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

        let frame = frame(&list, 80, 24);

        assert_eq!(frame.title, "no kept sessions");
        assert!(frame.lines.is_empty());
        assert!(!frame.footer.is_empty());
    }

    #[test]
    fn a_selection_below_the_window_scrolls_the_rows_into_view() {
        let mut list = List::new(vec![row("d"), row("c"), row("b"), row("a")]);
        list.select_last();

        // The title and the footer take a line each: one row is all a three-line terminal holds.
        let frame = frame(&list, 80, 3);

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
        let frame = frame(&list, 17, 24);

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
}
