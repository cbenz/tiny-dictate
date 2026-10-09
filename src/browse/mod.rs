//! The browser: the kept sessions read on a terminal.
//!
//! Two levels, following the pill's precedent: `frame` is the pure computation that turns the list
//! state and the selection into what one frame draws, and this module is the adapter that owns the
//! terminal. The browser takes no command lock and reads files only, so it opens while a dictation
//! runs and works on a machine without the encoder or the clipboard tools.

pub mod frame;

use std::io::{self, IsTerminal, Write};
use std::panic;
use std::path::Path;
use std::process::ExitCode;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::{Attribute, SetAttribute};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};

use crate::session::Runtime;
use crate::view;
use frame::List;

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

    let mut terminal = match Terminal::enter() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("tiny-dictate: cannot open a terminal: {error}");
            return ExitCode::FAILURE;
        }
    };

    // The result is read before the terminal is given back, and the error is printed after: a
    // message and the shell prompt must not land on the alternate screen.
    let result = browse(&mut terminal, &mut list, &sessions);
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
fn browse(terminal: &mut Terminal, list: &mut List, sessions: &Path) -> io::Result<()> {
    loop {
        terminal.draw(list)?;

        match event::read()? {
            // A release carries the key that was pressed before it: acting on it would move the
            // selection twice per press wherever the terminal reports releases.
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Char('r') => list.reload(view::rows(sessions)),
                KeyCode::Up | KeyCode::Char('k') => list.select_previous(),
                KeyCode::Down | KeyCode::Char('j') => list.select_next(),
                KeyCode::Home => list.select_first(),
                KeyCode::End => list.select_last(),
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

    /// Draw one frame: the whole screen, from the top left.
    fn draw(&self, list: &List) -> io::Result<()> {
        let (width, height) = terminal::size()?;
        let frame = frame::frame(list, usize::from(width), usize::from(height));

        let mut out = io::stdout();
        execute!(out, Clear(ClearType::All), MoveTo(0, 0))?;
        write_line(&mut out, &frame.title, false)?;
        for line in &frame.lines {
            write_line(&mut out, &line.text, line.selected)?;
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

/// One drawn line, the selected one in reverse video so the selection reads at a glance.
fn write_line(out: &mut impl Write, text: &str, selected: bool) -> io::Result<()> {
    if selected {
        execute!(out, SetAttribute(Attribute::Reverse))?;
    }
    write!(out, "{text}\r\n")?;
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
