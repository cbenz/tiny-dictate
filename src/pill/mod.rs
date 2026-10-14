//! The pill: the layer-shell surface the `_present` mode draws on.
//!
//! This is the only module that touches GTK. `frame` holds the pure computation; this one is the
//! adapter around it: it opens the display, builds the window, reads the state file, the waveform
//! channel and the clock every tick, and leaves by one of the three doors.

pub mod frame;

use std::cell::RefCell;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use gtk4::glib::{self, ControlFlow};
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use frame::{Frame, METER_HEIGHT_PX, METER_WIDTH_PX, Pill, Reading, SPINNER, Slice, State};

/// One tick per frame: the cadence the waveform channel is published at, and what bounds how long
/// a pill can outlive its session.
const TICK_MS: u64 = 100;
/// The bar width, and the distance between the middle of two bars, in pixels.
const BAR_WIDTH_PX: f64 = 2.0;
const BAR_STEP_PX: f64 = 4.5;
/// The side of one action slot, in pixels. The two slots are what balances the middle and what
/// gives the pill its height, so the layout and the stylesheet read the same number.
const ACTION_SLOT_PX: i32 = 26;
/// Clears the bar that usually sits at the bottom of the screen.
const MARGIN_PX: i32 = 64;
/// Enough of the waveform channel to hold whole lines whatever the cadence is.
const LEVEL_TAIL_BYTES: u64 = 4096;

/// The pill's colour: the waveform and the recording spinner. Its default is the one the
/// configuration names, so the two cannot drift apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
}

impl Default for Color {
    /// Zinc-200, the softer white a theme's foreground usually is.
    fn default() -> Self {
        Self::parse_hex(crate::config::DEFAULT_COLOR)
            .expect("the default colour is a `#rrggbb` literal")
    }
}

impl Color {
    /// `#rrggbb` as the configuration writes it. Anything else is `None`, and the caller keeps
    /// the default: a colour the presenter cannot read must not cost the pill.
    pub fn parse_hex(value: &str) -> Option<Self> {
        let digits = value.strip_prefix('#')?;
        if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }

        let channel = |index: usize| {
            u8::from_str_radix(&digits[index..index + 2], 16)
                .ok()
                .map(|value| f32::from(value) / 255.0)
        };

        Some(Self {
            red: channel(0)?,
            green: channel(2)?,
            blue: channel(4)?,
        })
    }

    /// The colour in the form GTK CSS reads.
    fn hex(self) -> String {
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!(
            "#{:02x}{:02x}{:02x}",
            channel(self.red),
            channel(self.green),
            channel(self.blue)
        )
    }
}

/// What `_present` was handed: the state file, the pid the pill outlives, the waveform channel,
/// the command the buttons run and the colour of the waveform.
pub struct Options {
    pub state_file: PathBuf,
    pub owner: u32,
    pub waveform: Option<PathBuf>,
    pub command: Option<String>,
    pub color: Color,
}

/// The `_present` usage, printed on stderr when the arguments are wrong.
pub const USAGE: &str = "usage: tiny-dictate _present <state file> <owner pid> [waveform file] \
[command] [color]";

impl Options {
    /// Read the arguments the session passes, tolerating an absent waveform or command.
    ///
    /// An empty argument is an absent one: the contract allows a presenter with no waveform
    /// channel and no command. The colour is absent too when no configuration file says otherwise.
    pub fn parse(arguments: &[String]) -> Result<Self, String> {
        if !(3..=5).contains(&arguments.len()) {
            return Err(USAGE.to_owned());
        }

        let owner = arguments[1]
            .trim()
            .parse::<u32>()
            .map_err(|_| USAGE.to_owned())?;
        let argument = |index: usize| {
            arguments
                .get(index)
                .map(String::as_str)
                .filter(|value| !value.is_empty())
        };

        Ok(Self {
            state_file: PathBuf::from(&arguments[0]),
            owner,
            waveform: argument(2).map(PathBuf::from),
            command: argument(3).map(str::to_owned),
            color: argument(4).and_then(Color::parse_hex).unwrap_or_default(),
        })
    }
}

/// `_present`: draw the pill until the state file disappears, says `stop`, or its owner is gone.
pub fn run(options: Options) -> ExitCode {
    // Only this mode initialises GTK, and only after the arguments were read: the interactive
    // commands exec the same binary and must not pay for a display, and a wrong invocation must
    // fail without one either.
    if let Err(error) = gtk::init() {
        eprintln!("tiny-dictate: cannot open a display: {error}");
        return ExitCode::FAILURE;
    }

    // This reads the GDK display, so it is asked after `gtk::init()`: before that it answers false
    // on a compositor that does support layer-shell. On one that does not, the dictation still
    // works — there is simply no pill.
    if !gtk4_layer_shell::is_supported() {
        eprintln!("tiny-dictate: layer-shell is not supported by this compositor");
        return ExitCode::FAILURE;
    }

    let window = gtk::Window::builder()
        .decorated(false)
        .resizable(false)
        .build();
    // Layer shell is configured before the window is realized; after that the calls are no-ops.
    window.init_layer_shell();
    window.set_namespace(Some("tiny-dictate"));
    window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Bottom, true);
    // Never reserve space, never take the keyboard: the paste that ends a dictation goes to the
    // focused window, and a focusable pill would receive the text itself.
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_margin(Edge::Bottom, MARGIN_PX);

    let pill = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    pill.add_css_class("pill");

    let cancel = action_button("✕");
    let spinner = gtk::Label::builder()
        .label(SPINNER[0].to_string())
        .width_chars(1)
        .build();
    spinner.add_css_class("spinner");
    spinner.set_visible(false);
    let meter = gtk::DrawingArea::new();
    // The meter is the pill's whole middle: it is stretched to the pill's height and draws its
    // bars around the middle of what it was allocated, so the waveform is centred.
    meter.set_size_request(METER_WIDTH_PX as i32, METER_HEIGHT_PX as i32);
    meter.set_vexpand(true);
    meter.set_visible(false);
    let label = gtk::Label::new(None);
    label.set_visible(false);
    let confirm = action_button("✓");

    let bars = Rc::new(RefCell::new(Vec::<f32>::new()));
    let drawn_bars = Rc::clone(&bars);
    let color = options.color;
    meter.set_draw_func(move |_area, cr, _width, height| {
        cr.set_source_rgb(
            f64::from(color.red),
            f64::from(color.green),
            f64::from(color.blue),
        );
        cr.set_line_width(BAR_WIDTH_PX);
        cr.set_line_cap(gtk::cairo::LineCap::Round);

        let middle = f64::from(height) / 2.0;
        for (index, excursion) in drawn_bars.borrow().iter().enumerate() {
            let x = index as f64 * BAR_STEP_PX + BAR_WIDTH_PX / 2.0;
            cr.move_to(x, middle - f64::from(*excursion));
            cr.line_to(x, middle + f64::from(*excursion));
        }
        let _ = cr.stroke();
    });

    pill.append(&action_slot(&cancel));
    pill.append(&spinner);
    pill.append(&meter);
    pill.append(&label);
    pill.append(&action_slot(&confirm));
    window.set_child(Some(&pill));

    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css(options.color));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let command = options.command.clone();
    cancel.connect_clicked({
        let command = command.clone();
        move |_| run_verb(command.as_deref(), "cancel")
    });
    confirm.connect_clicked(move |_| run_verb(command.as_deref(), "stop"));

    // What one frame paints, bundled so `apply` takes the frame and one set of widgets rather
    // than a dozen arguments.
    let widgets = Widgets {
        spinner,
        meter,
        label,
        cancel,
        confirm,
        pill,
        bars,
    };

    window.present();

    let main_loop = glib::MainLoop::new(None, false);
    let pill_state = Rc::new(RefCell::new(Pill::new(options.command.is_some())));

    glib::timeout_add_local(Duration::from_millis(TICK_MS), {
        let window = window.clone();
        let main_loop = main_loop.clone();
        move || {
            // Three ways out: the state file disappears, it says `stop`, or the process this pill
            // was given is gone. Removing the tick source only stops the ticks, so leaving is two
            // calls: destroy the window and quit the loop.
            let leave = |window: &gtk::Window, main_loop: &glib::MainLoop| {
                window.destroy();
                main_loop.quit();
            };

            let Some(word) = read_state(&options.state_file) else {
                leave(&window, &main_loop);
                return ControlFlow::Break;
            };
            if word == "stop" {
                leave(&window, &main_loop);
                return ControlFlow::Break;
            }
            if !owner_alive(options.owner) {
                leave(&window, &main_loop);
                return ControlFlow::Break;
            }

            // An empty or unknown word is a write in progress, not a reason to leave; the last
            // known state is what the waveform still applies to.
            let parsed = State::parse(&word);
            let known = parsed.or(pill_state.borrow().known());
            let waveform: Option<Vec<Slice>> = if known == Some(State::Recording) {
                options.waveform.as_deref().and_then(read_waveform)
            } else {
                None
            };

            let reading = Reading {
                state: parsed,
                waveform: waveform.as_deref(),
            };
            let previous = pill_state.borrow().known();
            let (frame, next) = pill_state.borrow().advance(&reading);

            apply(&frame, &next, previous, &widgets);
            *pill_state.borrow_mut() = next;

            ControlFlow::Continue
        }
    });

    main_loop.run();
    ExitCode::SUCCESS
}

/// The widgets one frame paints, bundled so `apply` takes the frame and one set of widgets
/// rather than a dozen arguments.
struct Widgets {
    spinner: gtk::Label,
    meter: gtk::DrawingArea,
    label: gtk::Label,
    cancel: gtk::Button,
    confirm: gtk::Button,
    pill: gtk::Box,
    bars: Rc<RefCell<Vec<f32>>>,
}

/// Paint one frame on the widgets, and carry the state class the pill has just entered.
fn apply(frame: &Frame, next: &Pill, previous: Option<State>, widgets: &Widgets) {
    let Widgets {
        spinner,
        meter,
        label,
        cancel,
        confirm,
        pill,
        bars,
    } = widgets;

    if let Some(glyph) = frame.spinner {
        spinner.set_text(&glyph.to_string());
    }
    spinner.set_visible(frame.spinner.is_some());

    label.set_text(frame.label.unwrap_or(""));
    label.set_visible(frame.label.is_some());

    cancel.set_visible(frame.cancel);
    confirm.set_visible(frame.confirm);

    let meter_drawn = !frame.bars.is_empty();
    meter.set_visible(meter_drawn);
    if meter_drawn {
        *bars.borrow_mut() = frame.bars.clone();
        meter.queue_draw();
    }

    // Classes are never removed: the pill carries every state it has ever been in.
    if next.known() != previous
        && let Some(state) = next.known()
    {
        pill.add_css_class(state.class());
    }
}

/// A button at one end of the pill: a glyph, hidden until a frame offers it.
fn action_button(glyph: &str) -> gtk::Button {
    let button = gtk::Button::with_label(glyph);
    button.add_css_class("action");
    button.set_valign(gtk::Align::Center);
    button.set_visible(false);
    button
}

/// The slot one action lives in: a square that stays allocated whatever the frame offers.
///
/// The two action slots are the tallest children and the only thing balancing the middle, so a
/// state that offers one button and not the other would pull the middle towards the offered side
/// and drop the pill to the label's height when neither is offered. The button inside is shown or
/// hidden; the slot it sits in never is.
fn action_slot(button: &gtk::Button) -> gtk::Box {
    let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    slot.set_size_request(ACTION_SLOT_PX, ACTION_SLOT_PX);
    slot.append(button);
    slot
}

/// Run the tool's `verb`, as a keyboard shortcut would ask for it.
///
/// The command is detached from the pill: leaving the screen must not take it down, and a command
/// that outlives the presenter is reparented rather than left a zombie. Its stderr stays the
/// pill's, which is the session's: the journal does not care whether a command came from a button
/// or from the keyboard (`docs/adr/0008-the-log.md`).
fn run_verb(command: Option<&str>, verb: &str) {
    let Some(command) = command else {
        return;
    };

    let mut child = Command::new(command);
    child.arg(verb).stdin(Stdio::null()).stdout(Stdio::null());
    // `start_new_session` in the reference presenter: the exact Rust equivalent is `setsid`.
    // Safety: `setsid` in the child before `exec` is async-signal-safe.
    unsafe {
        child.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    match child.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => eprintln!("tiny-dictate: cannot run {command} {verb}: {error}"),
    }
}

/// The word the state file holds, or `None` once it is gone or unreadable.
fn read_state(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

/// The newest whole waveform line, or `None` when there is nothing to draw.
///
/// An absent or unreadable channel is not an error: the spinner stands in.
fn read_waveform(path: &Path) -> Option<Vec<Slice>> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(LEVEL_TAIL_BYTES)))
        .ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    frame::parse_waveform(&tail)
}

/// Whether the process the pill was given still lives; a signal to a zombie still succeeds.
fn owner_alive(pid: u32) -> bool {
    // Safety: signal 0 probes the process without touching it.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// The stylesheet, the waveform colour substituted in.
fn css(color: Color) -> String {
    format!(
        r#"
window {{ background-color: transparent; }}
.pill {{
    background-color: rgba(24, 24, 27, 0.88);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 999px;
    padding: 5px 7px;
}}
.pill label {{ color: #fafafa; font-size: 14px; }}
.spinner {{ font-family: monospace; font-size: 16px; }}
.pill.recording .spinner {{ color: {waveform}; }}
.pill.transcribing .spinner {{ color: #fbbf24; }}
.pill.cancelled label {{ color: #d4d4d8; }}
.pill .action {{
    min-width: {slot}px;
    min-height: {slot}px;
    padding: 0;
    border: none;
    border-radius: 999px;
    background: none;
    box-shadow: none;
    color: #a1a1aa;
    font-size: 13px;
}}
.pill .action:hover {{ color: #fafafa; background-color: rgba(255, 255, 255, 0.14); }}
"#,
        waveform = color.hex(),
        slot = ACTION_SLOT_PX
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Result<Options, String> {
        let arguments: Vec<String> = values.iter().map(|value| (*value).to_owned()).collect();
        Options::parse(&arguments)
    }

    #[test]
    fn the_session_arguments_are_read() {
        let options = parse(&["state", "1234", "wave", "tool", "#ff0000"]).expect("five arguments");

        assert_eq!(options.state_file, PathBuf::from("state"));
        assert_eq!(options.owner, 1234);
        assert_eq!(options.waveform, Some(PathBuf::from("wave")));
        assert_eq!(options.command.as_deref(), Some("tool"));
    }

    #[test]
    fn an_empty_argument_is_an_absent_one() {
        let options = parse(&["state", "1234", "", "", ""]).expect("five arguments");

        assert_eq!(options.waveform, None);
        assert_eq!(options.command, None);
        assert_eq!(options.color, Color::default());
    }

    #[test]
    fn an_empty_waveform_or_command_parses_to_none() {
        // An empty waveform with a command beside it, and an empty command with a waveform: each
        // alone is an absent one, not a parse failure.
        let no_waveform = parse(&["state", "1234", "", "tool"]).expect("four arguments");
        assert_eq!(no_waveform.waveform, None);
        assert_eq!(no_waveform.command.as_deref(), Some("tool"));

        let no_command = parse(&["state", "1234", "wave", ""]).expect("four arguments");
        assert_eq!(no_command.waveform, Some(PathBuf::from("wave")));
        assert_eq!(no_command.command, None);
    }

    #[test]
    fn the_three_argument_call_parses() {
        let options = parse(&["state", "1234", "wave"]).expect("three arguments");

        assert_eq!(options.command, None);
        assert_eq!(options.color, Color::default());
    }

    #[test]
    fn a_missing_colour_keeps_the_default() {
        let options = parse(&["state", "1234", "wave", "tool"]).expect("four arguments");

        assert_eq!(options.color, Color::default());
    }

    #[test]
    fn the_colour_is_read_from_the_fifth_argument() {
        let options = parse(&["state", "1234", "wave", "tool", "#ff0000"]).expect("five");

        assert_eq!(
            options.color,
            Color {
                red: 1.0,
                green: 0.0,
                blue: 0.0,
            }
        );

        // An unreadable or absent colour keeps the default rather than failing the pill.
        let default = parse(&["state", "1234", "wave", "tool", "red"]).expect("five");
        assert_eq!(default.color, Color::default());
    }

    #[test]
    fn a_wrong_call_prints_the_usage() {
        assert!(parse(&["state"]).is_err());
        assert!(parse(&["state", "not-a-pid", "wave", "tool"]).is_err());
        assert!(parse(&["a", "1", "b", "c", "d", "e"]).is_err());
    }
}
