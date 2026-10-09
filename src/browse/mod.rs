//! The browser: the kept sessions read on a terminal.
//!
//! Two levels, following the pill's precedent: `frame` is the pure computation that turns the list
//! state and the selection into what one frame draws, and this module is the adapter that owns the
//! terminal. The browser takes no command lock and reads files only, so it opens while a dictation
//! runs and works on a machine without the encoder or the clipboard tools.

pub mod frame;

use std::io::{self, IsTerminal, Write};
use std::panic;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::{Attribute, SetAttribute};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};

use crate::session::Runtime;
use crate::view::{self, Detail};
use frame::{List, Pane};

/// `session browse`: the kept sessions on a terminal.
///
/// A stdout that is not a terminal is refused with a message: drawing escape sequences into a pipe
/// would corrupt whatever reads it, and a terminal the tool cannot see is a terminal it must not
/// take over.
pub fn run() -> ExitCode {
    if !io::stdout().is_terminal() {
        eprintln!("tiny-dictate: session browse needs a terminal");
        return ExitCode::FAILURE;
    }

    let mut browser = Browser::new(Runtime::from_env().sessions_directory());

    let mut terminal = match Terminal::enter() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("tiny-dictate: cannot open a terminal: {error}");
            return ExitCode::FAILURE;
        }
    };

    // The result is read before the terminal is given back, and the error is printed after: a
    // message and the shell prompt must not land on the alternate screen.
    let result = browse(&mut terminal, &mut browser);
    drop(terminal);

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tiny-dictate: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The browser's state between frames: the list, the detail pane that follows its selection, and
/// the sessions directory the two are read from.
struct Browser {
    sessions: PathBuf,
    list: List,
    detail: Option<Detail>,
    pane: Pane,
}

impl Browser {
    /// A browser on the sessions directory, the selection on the newest session.
    fn new(sessions: PathBuf) -> Self {
        let list = List::new(view::rows(&sessions));
        let detail = detail_of(&list, &sessions);

        Self {
            sessions,
            list,
            detail,
            pane: Pane::new(),
        }
    }

    /// Follow the selection: the pane is reloaded on the session it now sits on, from its top.
    fn follow(&mut self) {
        self.detail = detail_of(&self.list, &self.sessions);
        self.pane.reset();
    }

    /// Reload the list, keeping the selection on the same session while it is there; the pane
    /// follows it and keeps its place.
    fn reload(&mut self) {
        self.list.reload(view::rows(&self.sessions));
        self.detail = detail_of(&self.list, &self.sessions);
    }

    fn select_previous(&mut self) {
        self.list.select_previous();
        self.follow();
    }

    fn select_next(&mut self) {
        self.list.select_next();
        self.follow();
    }

    fn select_first(&mut self) {
        self.list.select_first();
        self.follow();
    }

    /// Jump to the oldest session, and read its detail from its end.
    fn select_last(&mut self, limit: usize) {
        self.list.select_last();
        self.follow();
        self.pane.last(limit);
    }

    /// The page a scroll key moves by and the last offset the pane takes, for the terminal as it
    /// is now; zero when the pane gives way or nothing is selected.
    fn metrics(&self) -> (usize, usize) {
        let Ok((width, height)) = terminal::size() else {
            return (0, 0);
        };
        let (width, height) = (usize::from(width), usize::from(height));
        let page = height.saturating_sub(2);

        match (&self.detail, frame::panes(width, height)) {
            (Some(detail), Some((_, pane_width))) => (
                page,
                frame::pane_lines(detail, pane_width)
                    .len()
                    .saturating_sub(page),
            ),
            _ => (page, 0),
        }
    }
}

/// The detail of the selected session, or nothing when the list is empty.
fn detail_of(list: &List, sessions: &Path) -> Option<Detail> {
    list.selected().map(|row| Detail::read(sessions, &row.id))
}

/// Read keys until the user leaves, drawing one frame before each.
fn browse(terminal: &mut Terminal, browser: &mut Browser) -> io::Result<()> {
    loop {
        terminal.draw(&browser.list, browser.detail.as_ref(), &browser.pane)?;
        let (page, limit) = browser.metrics();

        match event::read()? {
            // A release carries the key that was pressed before it: acting on it would move the
            // selection twice per press wherever the terminal reports releases.
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Char('r') => browser.reload(),
                KeyCode::Up | KeyCode::Char('k') => browser.select_previous(),
                KeyCode::Down | KeyCode::Char('j') => browser.select_next(),
                KeyCode::Home => browser.select_first(),
                KeyCode::End => browser.select_last(limit),
                KeyCode::PageUp => browser.pane.page_up(page),
                KeyCode::PageDown => browser.pane.page_down(page, limit),
                _ => {}
            },
            _ => {}
        }
    }
}

/// The terminal the browser owns: raw mode and the alternate screen, given back on every path out.
struct Terminal;

impl Terminal {
    /// Take the terminal over: raw mode, the alternate screen, no cursor.
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, Hide) {
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }
        install_panic_hook();

        Ok(Self)
    }

    /// Draw one frame: the whole screen, from the top left, the list beside the detail pane.
    fn draw(&self, list: &List, detail: Option<&Detail>, pane: &Pane) -> io::Result<()> {
        let (width, height) = terminal::size()?;
        let frame = frame::frame(
            list,
            detail,
            pane.scroll(),
            usize::from(width),
            usize::from(height),
        );

        let mut out = io::stdout();
        execute!(out, Clear(ClearType::All), MoveTo(0, 0))?;
        write!(out, "{}\r\n", frame.title)?;
        for (index, line) in frame.lines.iter().enumerate() {
            // The selection reads at a glance: the row is drawn in reverse video.
            if line.selected {
                execute!(out, SetAttribute(Attribute::Reverse))?;
            }
            write!(out, "{}", frame::pad(&line.text, frame.list_width))?;
            if line.selected {
                execute!(out, SetAttribute(Attribute::Reset))?;
            }
            if let Some(right) = frame.pane.get(index) {
                write!(out, "{}{}", frame::SEPARATOR, right)?;
            }
            write!(out, "\r\n")?;
        }
        execute!(out, MoveTo(0, height.saturating_sub(1)))?;
        write!(out, "{}", frame.footer)?;

        out.flush()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}

/// Give the terminal back: raw mode off, the alternate screen left, the cursor shown.
///
/// Idempotent on purpose: a panic runs the hook first and the guard when it unwinds.
fn restore() {
    let _ = terminal::disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

/// A panic must not leave the user's shell in raw mode. The hook restores the terminal before the
/// previous hook prints where the panic happened.
fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}
