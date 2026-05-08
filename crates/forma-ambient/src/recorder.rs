//! WAV recording: captures the stereo output stream to a file on disk.
//!
//! Usage:
//!   1. Call `Recorder::start(path, sample_rate)` to begin recording.
//!      This spawns a background writer thread and returns a `Recorder` handle.
//!   2. Feed samples via `Recorder::push(l, r)` from the audio callback.
//!      Uses a non-blocking try_send — dropped samples are skipped rather than
//!      blocking the real-time thread.
//!   3. Call `Recorder::stop()` (or drop the handle) to finalise the WAV file.

use std::sync::mpsc;
use std::thread;

pub struct Recorder {
    tx: mpsc::SyncSender<[f32; 2]>,
    thread: Option<thread::JoinHandle<Result<(), hound::Error>>>,
    pub path: String,
}

impl Recorder {
    /// Begin recording to `path` at the given sample rate.
    pub fn start(path: String, sample_rate: u32) -> anyhow::Result<Self> {
        // 4096-sample ring — about 90 ms at 44.1 kHz. Enough headroom so
        // the writer thread never starves, while bounding latency.
        let (tx, rx) = mpsc::sync_channel::<[f32; 2]>(4096);
        let path_clone = path.clone();

        let handle = thread::spawn(move || -> Result<(), hound::Error> {
            let spec = hound::WavSpec {
                channels: 2,
                sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            };
            let mut writer = hound::WavWriter::create(&path_clone, spec)?;
            while let Ok(frame) = rx.recv() {
                writer.write_sample(frame[0])?;
                writer.write_sample(frame[1])?;
            }
            // Channel closed (Recorder dropped / stop() called) — finalise.
            writer.finalize()
        });

        Ok(Self {
            tx,
            thread: Some(handle),
            path,
        })
    }

    /// Push a stereo sample. Non-blocking: silently drops if the ring is full.
    #[inline]
    pub fn push(&self, l: f32, r: f32) {
        let _ = self.tx.try_send([l, r]);
    }

    /// Close the channel and wait for the writer thread to flush and finalise.
    /// Returns any error from the writer thread.
    pub fn stop(mut self) -> anyhow::Result<()> {
        // Dropping tx closes the channel; the writer thread will drain and exit.
        drop(self.tx);
        // SAFETY: we only call stop() once; thread is Some until here.
        if let Some(h) = self.thread.take() {
            h.join()
                .map_err(|_| anyhow::anyhow!("recorder thread panicked"))?
                .map_err(|e| anyhow::anyhow!("WAV write error: {e}"))?;
        }
        Ok(())
    }
}
