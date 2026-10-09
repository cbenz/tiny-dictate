//! The browser: the kept sessions read on a terminal.
//!
//! Two levels, following the pill's precedent: `frame` is the pure computation that turns the list
//! state, the status line and the selection into what one frame draws, and this module is the
//! adapter that owns the terminal. The browser takes no command lock and reads files only, so it
//! opens while a dictation runs and works on a machine without the encoder or the clipboard tools;
//! only its copy actions ask for the clipboard tool, exactly as `session copy` does.

pub mod frame;

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::panic;
use std::path::Path;
use std::process::ExitCode;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::{Attribute, SetAttribute};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};

use crate::config;
use crate::session::Runtime;
use crate::store;
use crate::text;
use crate::view;
use frame::List;

/// What the status line says when an action is asked for and the list holds nothing to act on.
const NO_SELECTION: &str = "no session selected";

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

    let sessions = Runtime::from_env().sessions_directory();
    let mut list = List::new(view::rows(&sessions));
    // The clipboard lease is read once, as a live dictation reads it once: every copy the browser
    // makes carries the same duration as `session copy` and a dictation's paste.
    let config = config::load();

    let mut terminal = match Terminal::enter() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("tiny-dictate: cannot open a terminal: {error}");
            return ExitCode::FAILURE;
        }
    };

    // The result is read before the terminal is given back, and the error is printed after: a
    // message and the shell prompt must not land on the alternate screen.
    let result = browse(
        &mut terminal,
        &mut list,
        &sessions,
        config.clipboard_seconds,
    );
    drop(terminal);

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tiny-dictate: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Read keys until the user leaves, drawing one frame before each.
fn browse(
    terminal: &mut Terminal,
    list: &mut List,
    sessions: &Path,
    clipboard_seconds: u64,
) -> io::Result<()> {
    // The line the last action left: empty until one runs, and cleared by whatever changes what it
    // described -- moving the selection or reloading the list.
    let mut status = String::new();

    loop {
        terminal.draw(list, &status)?;

        match event::read()? {
            // A release carries the key that was pressed before it: acting on it would move the
            // selection twice per press wherever the terminal reports releases.
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Char('r') => {
                    list.reload(view::rows(sessions));
                    status.clear();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    list.select_previous();
                    status.clear();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    list.select_next();
                    status.clear();
                }
                KeyCode::Home => {
                    list.select_first();
                    status.clear();
                }
                KeyCode::End => {
                    list.select_last();
                    status.clear();
                }
                KeyCode::Char('c') => status = copy_transcript(list, sessions, clipboard_seconds),
                KeyCode::Char('i') => status = copy_id(list, clipboard_seconds),
                KeyCode::Char('p') => status = copy_path(list, sessions, clipboard_seconds),
                _ => {}
            },
            _ => {}
        }
    }
}

/// `c`: the selected session's stored transcript on the clipboard, through the one writer
/// `session copy` and a live dictation use. It reads only what is on disk: a session that has no
/// transcript is reported rather than copied empty, and never transcribed here.
fn copy_transcript(list: &List, sessions: &Path, clipboard_seconds: u64) -> String {
    let Some(row) = list.selected() else {
        return NO_SELECTION.to_owned();
    };

    let transcript =
        fs::read_to_string(sessions.join(&row.id).join(store::TRANSCRIPT_FILE)).unwrap_or_default();
    if transcript.trim().is_empty() {
        return format!("nothing to copy: {} has no transcript", row.id);
    }

    offer(&transcript, "transcript", clipboard_seconds)
}

/// `i`: the selected session's id on the clipboard.
fn copy_id(list: &List, clipboard_seconds: u64) -> String {
    let Some(row) = list.selected() else {
        return NO_SELECTION.to_owned();
    };

    offer(&row.id, "id", clipboard_seconds)
}

/// `p`: the selected session's path on the clipboard -- the directory `session show` prints as a
/// field, so the two surfaces name the same place.
fn copy_path(list: &List, sessions: &Path, clipboard_seconds: u64) -> String {
    let Some(row) = list.selected() else {
        return NO_SELECTION.to_owned();
    };

    offer(
        &sessions.join(&row.id).display().to_string(),
        "path",
        clipboard_seconds,
    )
}

/// Offer one value on both selections, under the configured lease, and say so on the status line.
///
/// The write is `text::copy_to_clipboard`, the one path a live dictation, `session copy` and
/// `session transcribe` take: the browser re-implements neither the offer nor the lease. Nothing
/// is injected: the `Shift+Insert` chord stays the live dictation's business.
fn offer(value: &str, what: &str, clipboard_seconds: u64) -> String {
    if text::copy_to_clipboard(value, clipboard_seconds) {
        format!("{what} copied to the clipboard")
    } else {
        format!("could not copy the {what} to the clipboard")
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

    /// Draw one frame: the whole screen, from the top left.
    fn draw(&self, list: &List, status: &str) -> io::Result<()> {
        let (width, height) = terminal::size()?;
        let frame = frame::frame(list, status, usize::from(width), usize::from(height));

        let mut out = io::stdout();
        execute!(out, Clear(ClearType::All), MoveTo(0, 0))?;
        write_line(&mut out, 0, &frame.title, false)?;
        for (index, line) in frame.lines.iter().enumerate() {
            let row = u16::try_from(index + 1).unwrap_or(u16::MAX);
            write_line(&mut out, row, &line.text, line.selected)?;
        }
        if !frame.status.is_empty() {
            write_line(&mut out, height.saturating_sub(2), &frame.status, false)?;
        }
        write_line(&mut out, height.saturating_sub(1), &frame.footer, false)?;

        out.flush()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}

/// One drawn line, at a row, the selected one in reverse video so the selection reads at a glance.
fn write_line(out: &mut impl Write, row: u16, text: &str, selected: bool) -> io::Result<()> {
    execute!(out, MoveTo(0, row))?;
    if selected {
        execute!(out, SetAttribute(Attribute::Reverse))?;
    }
    write!(out, "{text}")?;
    if selected {
        execute!(out, SetAttribute(Attribute::Reset))?;
    }

    Ok(())
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
