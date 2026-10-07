//! What the pill draws in one frame: the pure computation behind the GTK adapter.
//!
//! Everything here is a function of what was read — the state word, the newest waveform line,
//! the elapsed time and the limit — and of what the pill carries from tick to tick: the state it
//! last knew and the level the meter is calibrated on. Nothing here opens a file, reads a clock
//! or touches GTK, so the frame a tick draws is testable without a display.

use std::time::Duration;

/// The slices one waveform line holds, each covering 4 ms of audio.
pub const SLICES: usize = 24;
/// The full-scale magnitude of a sample, as the recorder writes them (signed 16 bit).
pub const FULL_SCALE: f32 = 32768.0;
/// The meter's height in pixels; one bar reaches at most half of it from the middle.
pub const METER_HEIGHT_PX: f32 = 18.0;
/// The meter's width in pixels: one bar per slice, spaced by `BAR_STEP_PX`.
pub const METER_WIDTH_PX: f32 = (SLICES as f32 - 1.0) * 4.5 + 2.0;
/// A silent slice still draws a bar, so the meter reads as live between words.
pub const BAR_MIN_PX: f32 = 2.0;

/// The spinner glyphs, one per tick.
pub const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// A window more than this far below the reference is the room, not the voice.
const PAUSE_MARGIN: f32 = 0.125;
/// How long a pause holds the calibration before it starts following the room down.
const HOLD_TICKS: u32 = 50;
/// The quietest level the meter ever calibrates on, so silence cannot fill it.
const REFERENCE_FLOOR: f32 = 0.03;
/// At most +6 dB per tick: a knock costs a few ticks of height, not the whole meter.
const REFERENCE_RISE: f32 = 2.0;
/// How fast the meter recalibrates to a voice that really did get quieter.
const REFERENCE_DECAY: f32 = 0.9;

/// The lowest and highest sample of one 4 ms slice of the recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slice {
    pub low: i16,
    pub high: i16,
}

impl Slice {
    /// The mirrored peak the bar is drawn from, in samples.
    ///
    /// A 4 ms slice covers about half a pitch period, so its lowest and highest samples are that
    /// half period: drawn unmirrored, one slice points up and the next down and the meter reads as
    /// noise through no fault of the voice.
    fn amplitude(self) -> u32 {
        i32::from(self.low)
            .unsigned_abs()
            .max(i32::from(self.high).unsigned_abs())
    }
}

/// One of the words a session publishes for the pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Recording,
    Transcribing,
    Cancelled,
}

impl State {
    /// The state a word names; `None` for `stop` and for an empty or half-written read.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "recording" => Some(Self::Recording),
            "transcribing" => Some(Self::Transcribing),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// The word drawn next to the spinner; `None` when the state carries no word.
    fn label(self) -> Option<&'static str> {
        match self {
            // Recording carries no word: the waveform is the whole message.
            Self::Recording => None,
            Self::Transcribing => Some("Transcribing"),
            Self::Cancelled => Some("🛑 Cancelled"),
        }
    }

    /// Whether the dictation is still live: the spinner turns and cancelling means something.
    fn live(self) -> bool {
        matches!(self, Self::Recording | Self::Transcribing)
    }

    /// The CSS class the pill carries while in this state.
    pub fn class(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Transcribing => "transcribing",
            Self::Cancelled => "cancelled",
        }
    }
}

/// The band of the progress bar: the end of the limit coming closer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Plain,
    Amber,
    Red,
}

/// The progress bar under the meter. Ticket #15 draws it; the computation lives here already.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    /// How much of the limit has elapsed, clamped to `0..=1`.
    pub fraction: f32,
    pub band: Band,
}

/// What one tick draws.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// The word next to the spinner; `None` hides the label.
    pub label: Option<&'static str>,
    /// The spinner glyph; `None` hides the spinner.
    pub spinner: Option<char>,
    /// One half-height in pixels per slice, mirrored around the meter's middle.
    pub bars: Vec<f32>,
    /// Whether the cross is offered.
    pub cancel: bool,
    /// Whether the check is offered.
    pub confirm: bool,
    /// The progress bar, when the state is recording and a limit is known.
    pub bar: Option<Bar>,
}

/// What one tick read: the word in the state file, the newest waveform line and the clock.
pub struct Reading<'a> {
    /// The state just read; `None` for an empty or unknown word, which changes nothing.
    pub state: Option<State>,
    /// The newest waveform line, when there is one to draw.
    pub waveform: Option<&'a [Slice]>,
    /// How long the recording has been running; zero outside recording.
    pub elapsed: Duration,
    /// The recording's limit, when the transcriber declared one.
    pub limit: Option<Duration>,
}

/// The level the meter is calibrated on, and how long a pause still holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Calibration {
    reference: f32,
    hold: u32,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            reference: REFERENCE_FLOOR,
            hold: 0,
        }
    }
}

impl Calibration {
    /// Recalibrate on the window just read.
    ///
    /// The meter is calibrated on the loudest window of the last second, never faster than +6 dB
    /// per tick, and decays towards a quieter voice once a pause has been held long enough that
    /// the room tone below it is not the voice merely resting.
    fn follow(&self, waveform: &[Slice]) -> Self {
        let peak = waveform
            .iter()
            .map(|slice| slice.amplitude())
            .max()
            .unwrap_or(0) as f32
            / FULL_SCALE;

        if peak < self.reference * PAUSE_MARGIN {
            // Room tone, not the voice. Hold the calibration while the hold lasts, so a pause is
            // not amplified until the meter fills; after that, follow the voice down.
            if self.hold > 0 {
                return Self {
                    reference: self.reference,
                    hold: self.hold - 1,
                };
            }
        }

        let decayed = (self.reference * REFERENCE_DECAY).max(REFERENCE_FLOOR);
        let reference = peak.max(decayed).min(self.reference * REFERENCE_RISE);
        Self {
            reference,
            hold: HOLD_TICKS,
        }
    }

    /// One bar per slice: how far it reaches from the middle of the meter, in pixels.
    fn bars(&self, waveform: &[Slice]) -> Vec<f32> {
        waveform
            .iter()
            .map(|slice| {
                let loudness = (slice.amplitude() as f32 / FULL_SCALE / self.reference).min(1.0);
                (BAR_MIN_PX / 2.0).max(loudness * METER_HEIGHT_PX / 2.0)
            })
            .collect()
    }
}

/// What the pill carries from one tick to the next.
#[derive(Clone, Debug, PartialEq)]
pub struct Pill {
    /// The last state read, or `None` before the first known one.
    known: Option<State>,
    calibration: Calibration,
    /// The index of the spinner glyph; advances every tick once a state is known.
    spinner: usize,
    /// Whether the presenter was given a command for its buttons.
    command: bool,
}

impl Pill {
    pub fn new(command: bool) -> Self {
        Self {
            known: None,
            calibration: Calibration::default(),
            spinner: 0,
            command,
        }
    }

    /// The state last known, so the caller can decide whether a waveform still applies.
    pub fn known(&self) -> Option<State> {
        self.known
    }

    /// One tick: the reading in, the frame to draw and the pill to carry into the next one.
    pub fn advance(&self, reading: &Reading) -> (Frame, Self) {
        // An empty or unknown read is a write in progress, not a reason to change anything: the
        // last known state stands.
        let known = reading.state.or(self.known);
        let mut next = self.clone();
        next.known = known;

        // The spinner turns as soon as a state is known, whatever it is: a verdict still animates
        // its neighbour, but stops the spinner by hiding it.
        if known.is_some() {
            next.spinner = (next.spinner + 1) % SPINNER.len();
        }

        // The waveform only stands in where there is audio to draw; an unreadable channel keeps
        // the spinner the pill can always draw.
        let drawn = known == Some(State::Recording) && reading.waveform.is_some();
        let bars = match reading.waveform {
            Some(waveform) if drawn => {
                next.calibration = next.calibration.follow(waveform);
                next.calibration.bars(waveform)
            }
            _ => Vec::new(),
        };

        let live = known.map(State::live).unwrap_or(false);
        let frame = Frame {
            label: known.and_then(State::label),
            spinner: if !drawn && live {
                Some(SPINNER[next.spinner])
            } else {
                None
            },
            bars,
            // One button per function, drawn only where the function means something: cancelling
            // drops the recording or the transcript, so it is offered in both live states, while
            // stopping has nothing left to do once the transcription is running.
            cancel: live && self.command,
            confirm: known == Some(State::Recording) && self.command,
            bar: bar(known, reading.elapsed, reading.limit),
        };

        (frame, next)
    }
}

/// The progress bar of a recording: how far the limit has been reached, and its band.
fn bar(known: Option<State>, elapsed: Duration, limit: Option<Duration>) -> Option<Bar> {
    if known != Some(State::Recording) {
        return None;
    }
    let limit = limit.filter(|limit| !limit.is_zero())?;
    let fraction = (elapsed.as_secs_f32() / limit.as_secs_f32()).clamp(0.0, 1.0);
    let band = if fraction > 0.95 {
        Band::Red
    } else if fraction >= 0.8 {
        Band::Amber
    } else {
        Band::Plain
    };
    Some(Bar { fraction, band })
}

/// The newest whole line of slices in the tail of the waveform channel, or `None`.
///
/// The channel is append-only and the session writes whole lines: the first line of the window may
/// be cut at the front and the last one at the back, and both are discarded by the field count
/// rather than guessed at.
pub fn parse_waveform(tail: &[u8]) -> Option<Vec<Slice>> {
    for line in tail.split(|byte| *byte == b'\n').rev() {
        let fields: Vec<&[u8]> = line
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty())
            .collect();
        if fields.len() != 2 * SLICES {
            continue;
        }

        let samples: Option<Vec<i16>> = fields
            .iter()
            .map(|field| std::str::from_utf8(field).ok()?.parse::<i16>().ok())
            .collect();
        let Some(samples) = samples else {
            continue;
        };

        return Some(
            samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| Slice {
                    low: pair[0],
                    high: pair[1],
                })
                .collect(),
        );
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(value: i16) -> Vec<Slice> {
        vec![
            Slice {
                low: value,
                high: value
            };
            SLICES
        ]
    }

    fn reading<'a>(state: Option<State>, waveform: Option<&'a [Slice]>) -> Reading<'a> {
        Reading {
            state,
            waveform,
            elapsed: Duration::ZERO,
            limit: None,
        }
    }

    #[test]
    fn a_pill_with_no_state_draws_nothing() {
        let pill = Pill::new(true);

        let (frame, next) = pill.advance(&reading(None, None));

        assert_eq!(frame.label, None);
        assert_eq!(frame.spinner, None);
        assert!(frame.bars.is_empty());
        assert!(!frame.cancel);
        assert!(!frame.confirm);
        assert_eq!(next.known(), None);
    }

    #[test]
    fn recording_without_a_waveform_falls_back_to_the_spinner() {
        let pill = Pill::new(true);

        let (frame, _) = pill.advance(&reading(Some(State::Recording), None));

        assert_eq!(frame.label, None, "recording carries no word");
        assert_eq!(
            frame.spinner,
            Some(SPINNER[1]),
            "the first tick advances the frame"
        );
        assert!(frame.bars.is_empty());
        assert!(frame.cancel);
        assert!(frame.confirm);
    }

    #[test]
    fn recording_with_a_waveform_draws_the_meter_and_no_spinner() {
        let pill = Pill::new(true);
        let waveform = flat(1000);

        let (frame, _) = pill.advance(&reading(Some(State::Recording), Some(&waveform)));

        assert_eq!(frame.spinner, None);
        assert_eq!(frame.bars.len(), SLICES);
        assert!(frame.cancel);
        assert!(frame.confirm);
    }

    #[test]
    fn transcribing_shows_its_word_and_offers_only_the_cross() {
        let pill = Pill::new(true);

        let (frame, _) = pill.advance(&reading(Some(State::Transcribing), None));

        assert_eq!(frame.label, Some("Transcribing"));
        assert_eq!(frame.spinner, Some(SPINNER[1]));
        assert!(frame.cancel);
        assert!(
            !frame.confirm,
            "stopping has nothing left to do while transcribing"
        );
    }

    #[test]
    fn a_cancelled_verdict_stops_the_spinner() {
        let pill = Pill::new(true);

        let (frame, _) = pill.advance(&reading(Some(State::Cancelled), None));

        assert_eq!(frame.label, Some("🛑 Cancelled"));
        assert_eq!(frame.spinner, None);
        assert!(!frame.cancel);
        assert!(!frame.confirm);
    }

    #[test]
    fn a_presenter_without_a_command_draws_no_buttons() {
        let pill = Pill::new(false);

        for state in [
            Some(State::Recording),
            Some(State::Transcribing),
            Some(State::Cancelled),
        ] {
            let (frame, _) = pill.advance(&reading(state, None));

            assert!(!frame.cancel);
            assert!(!frame.confirm);
        }
    }

    #[test]
    fn an_empty_or_unknown_read_changes_nothing() {
        let pill = Pill::new(true);
        let (_, pill) = pill.advance(&reading(Some(State::Recording), None));

        let (frame, next) = pill.advance(&reading(None, None));

        assert_eq!(next.known(), Some(State::Recording));
        assert_eq!(frame.label, None);
        assert!(frame.cancel, "the recording is still live");
        assert_eq!(frame.spinner, Some(SPINNER[2]), "the spinner keeps turning");
    }

    #[test]
    fn a_silent_slice_still_draws_a_bar() {
        let pill = Pill::new(true);
        let waveform = flat(0);

        let (frame, _) = pill.advance(&reading(Some(State::Recording), Some(&waveform)));

        assert!(frame.bars.iter().all(|bar| *bar == BAR_MIN_PX / 2.0));
    }

    #[test]
    fn a_loud_window_reaches_the_full_meter_and_the_rise_is_capped() {
        let pill = Pill::new(true);
        let waveform = flat(32767);

        let (frame, next) = pill.advance(&reading(Some(State::Recording), Some(&waveform)));

        assert!(frame.bars.iter().all(|bar| (*bar - 9.0).abs() < 1e-3));
        assert!(
            (next.calibration.reference - 0.06).abs() < 1e-4,
            "+6 dB per tick at most: 0.03 * 2"
        );
        assert_eq!(next.calibration.hold, 50);
    }

    #[test]
    fn a_quiet_window_is_drawn_small_against_the_reference() {
        let pill = Pill {
            calibration: Calibration {
                reference: 0.25,
                hold: 0,
            },
            ..Pill::new(true)
        };
        // The peak equals the reference, so the recalibration keeps it: only the first slice is
        // half as loud, and it is drawn at half the meter's half height.
        let mut waveform = flat(8192);
        waveform[0] = Slice { low: 0, high: 4096 };

        let (frame, next) = pill.advance(&reading(Some(State::Recording), Some(&waveform)));

        assert!((frame.bars[0] - 4.5).abs() < 1e-3);
        assert!(frame.bars[1..].iter().all(|bar| (*bar - 9.0).abs() < 1e-3));
        assert!((next.calibration.reference - 0.25).abs() < 1e-3);
    }

    #[test]
    fn a_pause_holds_the_reference_then_lets_it_decay() {
        let quiet = flat(0);

        let held = Pill {
            calibration: Calibration {
                reference: 0.4,
                hold: 50,
            },
            ..Pill::new(true)
        };
        let (_, next) = held.advance(&reading(Some(State::Recording), Some(&quiet)));
        assert!((next.calibration.reference - 0.4).abs() < 1e-6);
        assert_eq!(next.calibration.hold, 49);

        let expired = Pill {
            calibration: Calibration {
                reference: 0.4,
                hold: 0,
            },
            ..Pill::new(true)
        };
        let (_, next) = expired.advance(&reading(Some(State::Recording), Some(&quiet)));
        assert!(
            (next.calibration.reference - 0.36).abs() < 1e-6,
            "0.4 * 0.9"
        );
    }

    #[test]
    fn the_bar_band_follows_how_much_of_the_limit_has_elapsed() {
        let pill = Pill::new(true);
        let waveform = flat(0);
        let bar = |elapsed: f64, limit: Option<u64>| {
            let reading = Reading {
                state: Some(State::Recording),
                waveform: Some(&waveform),
                elapsed: Duration::from_secs_f64(elapsed),
                limit: limit.map(Duration::from_secs),
            };
            pill.advance(&reading).0.bar
        };

        assert_eq!(bar(10.0, None), None, "no limit, no bar");
        assert_eq!(bar(10.0, Some(100)).unwrap().band, Band::Plain);
        assert_eq!(bar(80.0, Some(100)).unwrap().band, Band::Amber);
        assert_eq!(bar(90.0, Some(100)).unwrap().band, Band::Amber);
        assert_eq!(bar(96.0, Some(100)).unwrap().band, Band::Red);
        assert_eq!(bar(200.0, Some(100)).unwrap().fraction, 1.0);
        assert_eq!(bar(200.0, Some(100)).unwrap().band, Band::Red);
    }

    #[test]
    fn there_is_no_bar_outside_recording() {
        let pill = Pill::new(true);
        let reading = Reading {
            state: Some(State::Transcribing),
            waveform: None,
            elapsed: Duration::from_secs(50),
            limit: Some(Duration::from_secs(100)),
        };

        assert_eq!(pill.advance(&reading).0.bar, None);
    }

    #[test]
    fn the_newest_whole_line_is_the_one_drawn() {
        let line = |value: i16| {
            (0..SLICES)
                .map(|_| format!("{value} {value}"))
                .collect::<Vec<_>>()
                .join(" ")
                + "\n"
        };
        // The last line is cut at the back by the read; the first one is whole.
        let tail = format!("{}{}", line(100), &line(200)[..20]);

        let slices = parse_waveform(tail.as_bytes()).expect("a whole line is in there");

        assert_eq!(slices.len(), SLICES);
        assert_eq!(
            slices[0],
            Slice {
                low: 100,
                high: 100
            }
        );
    }

    #[test]
    fn a_line_that_is_not_all_integers_is_skipped() {
        let mut line = String::new();
        for index in 0..SLICES {
            let value = if index == 3 { "oops" } else { "7" };
            line.push_str(&format!("{value} {value} "));
        }

        assert_eq!(parse_waveform(format!("{line}\n").as_bytes()), None);
    }
}
