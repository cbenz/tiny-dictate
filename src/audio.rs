//! The audio pipeline: the recorder, the encoder and the waveform channel.
//!
//! The recorder captures the microphone into a fifo, the encoder writes MP3 from the first
//! frame and outlives the recorder so it can flush the last frames, and the waveform
//! channel carries a copy of the stream to the pill. The copy lives in the `_pump` process,
//! spawned by `start` and outliving it: a copier thread inside `start` would die with it.

use std::ffi::CString;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::FromRawFd;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use crate::session::{Paths, self_executable};

/// One slice is 64 S16_LE samples, 4 ms of mono audio at 16 kHz.
pub const SLICE_SAMPLES: usize = 64;
/// The width of a slice in bytes: two bytes per S16_LE sample.
pub const SLICE_BYTES: usize = SLICE_SAMPLES * 2;
/// A waveform line holds 24 slices, 96 ms of audio, renewed ten times a second.
pub const SLICES_PER_LINE: usize = 24;

/// The encoder's MP3 bitrate: `lame … -b 64`, i.e. 64 kbit/s. Eight bits to the byte, so the
/// encoder writes 8000 bytes per second and a transcriber's byte limit is that many seconds of
/// audio.
pub const BITRATE_KBPS: u64 = 64;
/// The bytes one second of the encoded recording costs at that bitrate.
pub const BYTES_PER_SECOND: u64 = BITRATE_KBPS * 1000 / 8;

/// Cut a raw S16_LE mono stream into waveform lines.
///
/// Each slice is reduced to its lowest and highest sample, and 24 slices make one line of 48
/// space-separated integers. Only complete lines are produced: a trailing partial slice, or a
/// partial line, is never written, so the pill never reads half a line.
pub struct WaveformSlicer {
    pending: Vec<u8>,
    slices: Vec<(i16, i16)>,
}

impl WaveformSlicer {
    /// An empty slicer.
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            slices: Vec::new(),
        }
    }

    /// Feed the next bytes of the stream; return the complete lines they finished.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);

        let mut lines = Vec::new();
        while self.pending.len() >= SLICE_BYTES {
            let slice: Vec<u8> = self.pending.drain(..SLICE_BYTES).collect();
            self.slices.push(slice_extremes(&slice));

            if self.slices.len() == SLICES_PER_LINE {
                lines.push(format_line(&self.slices));
                self.slices.clear();
            }
        }

        lines
    }
}

impl Default for WaveformSlicer {
    fn default() -> Self {
        Self::new()
    }
}

/// The lowest and highest sample of one slice of S16_LE samples.
fn slice_extremes(slice: &[u8]) -> (i16, i16) {
    let mut low = i16::MAX;
    let mut high = i16::MIN;

    for pair in slice.as_chunks::<2>().0 {
        let sample = i16::from_le_bytes(*pair);
        low = low.min(sample);
        high = high.max(sample);
    }

    (low, high)
}

/// One line of the waveform channel: the slices' extrema, space-separated.
fn format_line(slices: &[(i16, i16)]) -> String {
    let mut line = String::with_capacity(slices.len() * 12);

    for (index, (low, high)) in slices.iter().enumerate() {
        if index > 0 {
            line.push(' ');
        }
        line.push_str(&format!("{low} {high}"));
    }

    line.push('\n');
    line
}

/// Create the fifo the recorder's copy writes into and the encoder reads.
pub fn create_fifo(path: &Path) -> io::Result<()> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path holds a NUL byte"))?;

    // Safety: `mkfifo` with a valid C string and a plain mode.
    let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
    if result == 0 {
        return Ok(());
    }

    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::AlreadyExists {
        return Ok(());
    }

    Err(error)
}

/// Start the encoder and the recorder, with the waveform pump between them.
///
/// The encoder reads the fifo and writes the durable `audio.mp3` from the first frame; the
/// pump copies the recorder's stdout into that fifo and into the waveform channel. Both pids
/// are recorded here, synchronously, so the session can wait on them without a race.
pub fn start_pipeline(paths: &Paths, audio: &Path) -> io::Result<()> {
    let reader = open_fifo_reader(&paths.fifo)?;
    let encoder = Command::new("lame")
        .args([
            "-r",
            "-s",
            "16",
            "--bitwidth",
            "16",
            "--signed",
            "--little-endian",
            "-m",
            "m",
            "-b",
        ])
        .arg(BITRATE_KBPS.to_string())
        .args(["-S", "-"])
        .arg(audio)
        .stdin(Stdio::from(reader))
        .spawn()?;
    fs::write(&paths.encoder_pid, encoder.id().to_string())?;

    fs::File::create(&paths.waveform)?;

    // The pump outlives `start`, so it is a process of its own; it is handed the fifo path in
    // its argv, which is what the suite's leftover check looks for.
    let mut pump = Command::new(self_executable())
        .arg("_pump")
        .arg(&paths.fifo)
        .arg(&paths.waveform)
        .stdin(Stdio::piped())
        .spawn()?;
    let pump_stdin = pump
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("the pump has no stdin"))?;

    let recorder = Command::new("arecord")
        .args(["-f", "S16_LE", "-r", "16000", "-c", "1", "-t", "raw"])
        .stdout(Stdio::from(pump_stdin))
        .stderr(Stdio::from(fs::File::create(&paths.recorder_error)?))
        .spawn()?;
    fs::write(&paths.recorder_pid, recorder.id().to_string())?;

    Ok(())
}

/// `_pump`: copy the recorder's stream into the fifo and append the waveform lines.
pub fn pump(arguments: &[String]) -> ExitCode {
    let (Some(fifo), Some(waveform)) = (arguments.first(), arguments.get(1)) else {
        eprintln!("tiny-dictate: _pump needs a fifo path and a waveform path");
        return ExitCode::FAILURE;
    };

    if let Err(error) = run_pump(Path::new(fifo), Path::new(waveform)) {
        eprintln!("tiny-dictate: the waveform pump stopped: {error}");
    }

    ExitCode::SUCCESS
}

fn run_pump(fifo: &Path, waveform: &Path) -> io::Result<()> {
    // Opening for writing blocks until the encoder has the read end, exactly as the shell's
    // `tee` does; the recorder is already writing into the pipe before us either way.
    let mut channel = fs::OpenOptions::new().write(true).open(fifo)?;
    let mut channel_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(waveform)?;

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut slicer = WaveformSlicer::new();
    let mut buffer = [0u8; 16 * 1024];

    loop {
        let read = match input.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };

        // A write error into a dead fifo is swallowed: the shell's `tee -p`. A dead meter can
        // never end the recording.
        let _ = channel.write_all(&buffer[..read]);

        for line in slicer.push(&buffer[..read]) {
            let _ = channel_file.write_all(line.as_bytes());
        }
    }

    let _ = channel_file.flush();
    Ok(())
}

/// Open the fifo's read end without blocking on a writer, then clear `O_NONBLOCK` so the
/// encoder waits on the stream the way it does in the shell.
fn open_fifo_reader(path: &Path) -> io::Result<fs::File> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path holds a NUL byte"))?;

    // Safety: `open` with a valid C string and two plain flags.
    let descriptor = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }

    // Safety: a descriptor we just opened.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags >= 0 {
        // Safety: same descriptor, clearing one flag.
        unsafe {
            libc::fcntl(descriptor, libc::F_SETFL, flags & !libc::O_NONBLOCK);
        }
    }

    // Safety: the descriptor is owned by the returned file and closed with it.
    Ok(unsafe { fs::File::from_raw_fd(descriptor) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes of one complete line: 24 slices of the sample value `sample`.
    fn constant_line(sample: i16) -> Vec<u8> {
        let mut bytes = Vec::new();
        for _ in 0..SLICES_PER_LINE * SLICE_SAMPLES {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn a_full_line_is_48_extrema_for_24_slices() {
        let mut slicer = WaveformSlicer::new();

        let lines = slicer.push(&constant_line(1000));

        assert_eq!(lines.len(), 1);
        let fields: Vec<&str> = lines[0].split_whitespace().collect();
        assert_eq!(fields.len(), 48);
        assert!(fields.iter().all(|field| *field == "1000"));
        assert!(lines[0].ends_with('\n'));
    }

    #[test]
    fn a_slice_is_reduced_to_its_lowest_and_highest_sample() {
        let mut slicer = WaveformSlicer::new();
        let mut stream = Vec::new();
        // First slice: -5, -3, 7, 0, then 60 zeroes.
        for sample in [-5i16, -3, 7, 0] {
            stream.extend_from_slice(&sample.to_le_bytes());
        }
        for _ in 0..SLICE_SAMPLES - 4 {
            stream.extend_from_slice(&0i16.to_le_bytes());
        }
        // The 23 remaining slices of the line are all 1.
        for _ in 0..(SLICES_PER_LINE - 1) * SLICE_SAMPLES {
            stream.extend_from_slice(&1i16.to_le_bytes());
        }

        let lines = slicer.push(&stream);

        assert_eq!(lines.len(), 1);
        let fields: Vec<&str> = lines[0].split_whitespace().collect();
        assert_eq!(fields[0], "-5");
        assert_eq!(fields[1], "7");
        assert_eq!(fields[2], "1");
        assert_eq!(fields[3], "1");
    }

    #[test]
    fn a_partial_line_is_held_back_until_it_is_complete() {
        let mut slicer = WaveformSlicer::new();
        let full = constant_line(2);

        let lines = slicer.push(&full[..SLICE_BYTES * (SLICES_PER_LINE - 1)]);
        assert!(lines.is_empty());

        let lines = slicer.push(&full[SLICE_BYTES * (SLICES_PER_LINE - 1)..]);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn a_trailing_partial_slice_is_never_written() {
        let mut slicer = WaveformSlicer::new();
        let mut stream = constant_line(3);
        stream.extend_from_slice(&[0x01, 0x00, 0x02]); // one and a half samples

        let lines = slicer.push(&stream);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].split_whitespace().count(), 48);
    }

    #[test]
    fn bytes_split_across_pushes_still_make_a_line() {
        let mut slicer = WaveformSlicer::new();
        let stream = constant_line(-32768);

        let mut lines = Vec::new();
        for chunk in stream.chunks(7) {
            lines.extend(slicer.push(chunk));
        }

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].split_whitespace().next(), Some("-32768"));
    }
}
