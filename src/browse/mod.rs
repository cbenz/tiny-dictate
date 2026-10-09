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
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::{Attribute, SetAttribute};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};

use crate::config;
use crate::rescue;
use crate::session::Runtime;
use crate::store;
use crate::text;
use crate::view::{self, Detail};
use frame::{Answer, Confirm, List, Pane, Run};

/// What the status line says when an action is asked for and the list holds nothing to act on.
const NO_SELECTION: &str = "no session selected";

/// How long the browser waits for a key between two looks at a transcription in flight: short
/// enough that a run which returns on its own is noticed without a keypress, long enough not to
/// spin.
const POLL: Duration = Duration::from_millis(50);

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

    let runtime = Runtime::from_env();
    // The clipboard lease is read once, as a live dictation reads it once: every copy the browser
    // makes carries the same duration as `session copy` and a dictation's paste.
    let config = config::load();
    let mut browser = Browser::new(runtime, config.clipboard_seconds);

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

/// The browser's state between frames: the list, the detail pane that follows its selection, the
/// status line the last action left, the pending confirmation, and the sessions directory the two
/// are read from.
struct Browser {
    /// The runtime directory, read for the delete guard: the id the dictation in flight publishes
    /// and the pidfiles it holds. The browser takes no command lock and signals nothing.
    runtime: Runtime,
    list: List,
    detail: Option<Detail>,
    pane: Pane,
    /// The confirmation `d` asked for, while it stands: the status line draws its question and
    /// only `y` or `n` answers it.
    confirm: Option<Confirm>,
    /// What the last action did, above the footer; empty until one runs.
    status: String,
    /// The lease every copy carries, read once from the configuration.
    clipboard_seconds: u64,
    /// The transcription in flight, when one runs: the keys are then inert but for the ones that
    /// cancel and leave.
    running: Option<Transcription>,
}

/// What the browser does with a successful transcription: `c` copies the text, `t` only rewrites
/// the session.
enum Intent {
    /// `t`: transcribe again, whatever transcript the session holds.
    Again,
    /// `c`: transcribe because the session has no transcript, then copy the text.
    Copy,
}

/// A transcription in flight, started from the browser: the runner, the session it writes into,
/// and what the user asked for.
struct Transcription {
    session: store::Session,
    running: rescue::Running,
    intent: Intent,
}

impl Browser {
    /// A browser on the sessions directory, the selection on the newest session.
    fn new(runtime: Runtime, clipboard_seconds: u64) -> Self {
        let sessions = runtime.sessions_directory();
        let list = List::new(view::rows(&sessions));
        let detail = detail_of(&list, &sessions);

        Self {
            runtime,
            list,
            detail,
            pane: Pane::new(),
            confirm: None,
            status: String::new(),
            clipboard_seconds,
            running: None,
        }
    }

    /// Whether a transcription is in flight: the keys, and the footer, follow it.
    fn transcribing(&self) -> bool {
        self.running.is_some()
    }

    /// The browser's run state, as the frame draws it.
    fn run(&self) -> Run {
        if self.transcribing() {
            Run::Transcribing
        } else {
            Run::Idle
        }
    }

    /// The root the kept sessions are read from, and the one a delete removes from.
    fn sessions(&self) -> PathBuf {
        self.runtime.sessions_directory()
    }

    /// Follow the selection: the pane is reloaded on the session it now sits on, from its top.
    fn follow(&mut self) {
        self.detail = detail_of(&self.list, &self.sessions());
        self.pane.reset();
    }

    /// Reload the list, keeping the selection on the same session while it is there; the pane
    /// follows it and keeps its place.
    fn reload(&mut self) {
        self.list.reload(view::rows(&self.sessions()));
        self.detail = detail_of(&self.list, &self.sessions());
        self.status.clear();
    }

    /// `d`: ask whether to destroy the selected session. Nothing is removed until the question is
    /// answered, and the answer a key gives is decided by the pure `Confirm`.
    fn ask_delete(&mut self) {
        let Some(row) = self.list.selected() else {
            self.status = NO_SELECTION.to_owned();
            return;
        };

        self.status.clear();
        self.confirm = Some(Confirm::ask(&row.id));
    }

    /// Answer the pending question with `y`: destroy the session it names, through the one guard
    /// `session delete` takes, then reload the list so the row is gone without leaving the
    /// browser. A refusal leaves the session on disk and says why on the status line.
    fn delete(&mut self) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        let id = confirm.id().to_owned();

        match rescue::remove_session(&self.runtime, &id) {
            Ok(()) => {
                self.list.reload(view::rows(&self.sessions()));
                self.follow();
                self.status = format!("{id} deleted");
            }
            Err(refusal) => self.status = format!("{id}: {refusal}"),
        }
    }

    /// Answer the pending question with `n`: the browser is where it was, the session untouched.
    fn keep(&mut self) {
        self.confirm = None;
        self.status.clear();
    }

    fn select_previous(&mut self) {
        self.list.select_previous();
        self.follow();
        self.status.clear();
    }

    fn select_next(&mut self) {
        self.list.select_next();
        self.follow();
        self.status.clear();
    }

    fn select_first(&mut self) {
        self.list.select_first();
        self.follow();
        self.status.clear();
    }

    /// Jump to the oldest session, and read its detail from its end.
    fn select_last(&mut self, limit: usize) {
        self.list.select_last();
        self.follow();
        self.pane.last(limit);
        self.status.clear();
    }

    /// The page a scroll key moves by and the last offset the pane takes, for the terminal as it
    /// is now; zero when the pane gives way or nothing is selected. The page follows the frame's
    /// body: the title, the footer and, when there is one, the status line.
    fn metrics(&self) -> (usize, usize) {
        let Ok((width, height)) = terminal::size() else {
            return (0, 0);
        };
        let (width, height) = (usize::from(width), usize::from(height));
        let page = height
            .saturating_sub(2 + usize::from(self.confirm.is_some() || !self.status.is_empty()));

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

    /// `c`: the selected session's transcript on the clipboard, through the one writer `session
    /// copy` and a live dictation use. A transcript already on disk is offered as it is; a session
    /// that has none is transcribed first, exactly as `session copy` does, and the text is offered
    /// when the run returns. A transcript that exists but holds no word is reported rather than
    /// paid for twice.
    fn copy_transcript(&mut self) {
        let Some(row) = self.list.selected() else {
            self.status = NO_SELECTION.to_owned();
            return;
        };
        let id = row.id.clone();

        match fs::read_to_string(self.sessions().join(&id).join(store::TRANSCRIPT_FILE)) {
            Ok(transcript) if view::holds_a_word(&transcript) => {
                self.status = offer(&transcript, "transcript", self.clipboard_seconds);
            }
            Ok(_) => {
                self.status = format!("nothing to copy: {id} has no transcript");
            }
            // No transcript on disk: run the transcriber first, as `session copy` does, and offer
            // what it writes when it returns.
            Err(_) => self.start_transcription(Intent::Copy),
        }
    }

    /// `t`: run the transcriber again on the selected session, even when it already has a
    /// transcript -- a changed transcriber is the reason to ask.
    fn transcribe_again(&mut self) {
        self.start_transcription(Intent::Again);
    }

    /// Start the transcriber on the selected session, through the runner `session transcribe`
    /// and `session copy` use; nothing is written until it returns, so a run that is cancelled
    /// leaves the session exactly as it was.
    fn start_transcription(&mut self, intent: Intent) {
        let Some(row) = self.list.selected() else {
            self.status = NO_SELECTION.to_owned();
            return;
        };
        let Some(mut session) = store::Session::load(&self.sessions(), &row.id) else {
            // A session whose metadata cannot be read has no audio to hand the transcriber and no
            // metadata to rewrite: `session transcribe` refuses it too.
            self.status = format!("cannot read the session's metadata: {}", row.id);
            return;
        };

        match rescue::start_transcription(&mut session) {
            Ok(running) => {
                self.status = format!("transcribing {}", row.id);
                self.running = Some(Transcription {
                    session,
                    running,
                    intent,
                });
            }
            Err(rescue::NotStarted::NoTranscriber) => {
                self.status = "no transcription command".to_owned();
            }
            Err(rescue::NotStarted::Failed(error)) => {
                self.status = format!("transcription failed: {error}");
            }
        }
    }

    /// Look at the transcription in flight: when it has returned, finish it.
    fn poll_transcription(&mut self) {
        let Some(transcription) = self.running.as_mut() else {
            return;
        };

        if let Some(success) = transcription.running.exited() {
            self.finish_transcription(success);
        }
    }

    /// A transcription has returned: write what it produced and say what happened. Whatever the
    /// run wrote, the pane reads it back, so a rewritten transcript or metadata never waits for a
    /// reload key.
    fn finish_transcription(&mut self, success: bool) {
        let Some(Transcription {
            mut session,
            running,
            intent,
        }) = self.running.take()
        else {
            return;
        };
        let id = session.id.clone();
        let result = running.finish(&mut session, success);
        self.follow();

        self.status = match result {
            rescue::Transcription::Text(text) => match intent {
                Intent::Again => format!("transcribed {id}"),
                Intent::Copy if !view::holds_a_word(&text) => {
                    format!("nothing to copy: {id} has no transcript")
                }
                Intent::Copy => offer(&text, "transcript", self.clipboard_seconds),
            },
            rescue::Transcription::Failed(error) => format!("transcription failed: {error}"),
            // The transcriber resolved when the run started, so a finished run that names no
            // command cannot happen; reported rather than silently ignored.
            rescue::Transcription::NoTranscriber => "no transcription command".to_owned(),
        };
    }

    /// `ctrl-c` during a transcription: signal the child, reap it and keep the browser open on
    /// the verdict. Nothing it wrote is kept: the session stays exactly as it was.
    fn cancel_transcription(&mut self) {
        let Some(Transcription {
            session, running, ..
        }) = self.running.take()
        else {
            return;
        };
        running.cancel();
        self.status = format!("transcription cancelled: {}", session.id);
    }

    /// `i`: the selected session's id on the clipboard.
    fn copy_id(&self) -> String {
        let Some(row) = self.list.selected() else {
            return NO_SELECTION.to_owned();
        };

        offer(&row.id, "id", self.clipboard_seconds)
    }

    /// `p`: the selected session's path on the clipboard -- the directory `session show` prints as
    /// a field, so the two surfaces name the same place.
    fn copy_path(&self) -> String {
        let Some(row) = self.list.selected() else {
            return NO_SELECTION.to_owned();
        };

        offer(
            &self.sessions().join(&row.id).display().to_string(),
            "path",
            self.clipboard_seconds,
        )
    }
}

/// The detail of the selected session, or nothing when the list is empty.
fn detail_of(list: &List, sessions: &Path) -> Option<Detail> {
    list.selected().map(|row| Detail::read(sessions, &row.id))
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

/// Read keys until the user leaves, drawing one frame before each.
///
/// A transcription in flight does not block the loop: the child is polled between the keys, so
/// the browser notices it return without a keypress, and only the keys that cancel mean anything
/// until it does. A second transcription can never start: no key sequence reaches the start
/// while one runs.
fn browse(terminal: &mut Terminal, browser: &mut Browser) -> io::Result<()> {
    loop {
        terminal.draw(browser)?;
        let (page, limit) = browser.metrics();

        if browser.transcribing() {
            if event::poll(POLL)? {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            browser.cancel_transcription();
                        }
                        KeyCode::Char('q') => {
                            browser.cancel_transcription();
                            return Ok(());
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }

            browser.poll_transcription();
            continue;
        }

        match event::read()? {
            // A release carries the key that was pressed before it: acting on it would move the
            // selection twice per press wherever the terminal reports releases.
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                // A pending confirmation owns the keyboard: only `y` and `n` answer it, and every
                // other key, `q` included, answers nothing and leaves the question standing, so a
                // stray key can never destroy a recording.
                if let Some(confirm) = &browser.confirm {
                    let answer = match key.code {
                        KeyCode::Char(character) => confirm.answer(character),
                        _ => Answer::Nothing,
                    };

                    match answer {
                        Answer::Yes => browser.delete(),
                        Answer::No => browser.keep(),
                        Answer::Nothing => {}
                    }
                    continue;
                }
                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('r') => browser.reload(),
                    KeyCode::Char('d') => browser.ask_delete(),
                    KeyCode::Up | KeyCode::Char('k') => browser.select_previous(),
                    KeyCode::Down | KeyCode::Char('j') => browser.select_next(),
                    KeyCode::Home => browser.select_first(),
                    KeyCode::End => browser.select_last(limit),
                    KeyCode::PageUp => browser.pane.page_up(page),
                    KeyCode::PageDown => browser.pane.page_down(page, limit),
                    KeyCode::Char('c') => browser.copy_transcript(),
                    KeyCode::Char('i') => browser.status = browser.copy_id(),
                    KeyCode::Char('p') => browser.status = browser.copy_path(),
                    KeyCode::Char('t') => browser.transcribe_again(),
                    _ => {}
                }
            }
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

    /// Draw one frame: the whole screen, from the top left, the list beside the detail pane, the
    /// status line above the footer.
    fn draw(&self, browser: &Browser) -> io::Result<()> {
        let (width, height) = terminal::size()?;
        let frame = frame::frame(
            &browser.list,
            browser.detail.as_ref(),
            browser.run(),
            &browser.status,
            browser.confirm.as_ref(),
            browser.pane.scroll(),
            usize::from(width),
            usize::from(height),
        );

        let mut out = io::stdout();
        execute!(out, Clear(ClearType::All), MoveTo(0, 0))?;
        write_line(&mut out, 0, &frame.title, false)?;
        for (index, line) in frame.lines.iter().enumerate() {
            let row = u16::try_from(index + 1).unwrap_or(u16::MAX);
            write_body_line(
                &mut out,
                row,
                line,
                frame.pane.get(index),
                frame.list_columns,
            )?;
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

/// One drawn line, at a row, in reverse video when it carries the selection.
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

/// One body row: the list cell, padded to its pane's width and in reverse video when it carries
/// the selection, then -- when the detail pane is drawn -- the separator and the pane's line for
/// that row, so the two panes stay aligned.
fn write_body_line(
    out: &mut impl Write,
    row: u16,
    left: &frame::Line,
    right: Option<&String>,
    list_columns: usize,
) -> io::Result<()> {
    execute!(out, MoveTo(0, row))?;
    if left.selected {
        execute!(out, SetAttribute(Attribute::Reverse))?;
    }
    write!(out, "{}", frame::pad(&left.text, list_columns))?;
    if left.selected {
        execute!(out, SetAttribute(Attribute::Reset))?;
    }
    if let Some(right) = right {
        write!(out, "{}{}", frame::SEPARATOR, right)?;
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
