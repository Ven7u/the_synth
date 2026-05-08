//! WAV recording: captures the stereo output stream to a file on disk.

use std::sync::mpsc;
use std::thread;

pub struct Recorder {
    tx: mpsc::SyncSender<[f32; 2]>,
    thread: Option<thread::JoinHandle<Result<(), hound::Error>>>,
    pub path: String,
}

impl Recorder {
    pub fn start(path: String, sample_rate: u32) -> anyhow::Result<Self> {
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
            writer.finalize()
        });

        Ok(Self {
            tx,
            thread: Some(handle),
            path,
        })
    }

    #[inline]
    pub fn push(&self, l: f32, r: f32) {
        let _ = self.tx.try_send([l, r]);
    }

    pub fn stop(mut self) -> anyhow::Result<()> {
        drop(self.tx);
        if let Some(h) = self.thread.take() {
            h.join()
                .map_err(|_| anyhow::anyhow!("recorder thread panicked"))?
                .map_err(|e| anyhow::anyhow!("WAV write error: {e}"))?;
        }
        Ok(())
    }
}
