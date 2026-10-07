//! The audio pipeline: the recorder, the encoder and the waveform channel.
//!
//! The recorder captures the microphone into a fifo, the encoder writes MP3 from the first
//! frame and outlives the recorder so it can flush the last frames, and the waveform
//! channel carries a copy of the stream to the pill.
