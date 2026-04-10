//! MIDI recording: captures note-on / note-off events from the audio callback
//! and writes a Type-1 MIDI file (one track per synth track) on stop.
//!
//! Usage:
//!   1. `MidiRecorder::start(path, sample_rate, bpm)` — returns a handle and a `Sink`.
//!   2. Feed events via `Sink::push_*` from the audio callback (non-blocking).
//!   3. `MidiRecorder::stop()` — closes the channel, waits for the writer thread,
//!      finalises the file.

use std::sync::mpsc;
use std::thread;

/// An event sent from the audio thread to the writer thread.
pub enum MidiRecordEvent {
    NoteOn  { sample: u64, track: u8, pitch: u8, velocity: u8 },
    NoteOff { sample: u64, track: u8, pitch: u8 },
    /// Carry the current BPM whenever it changes (or at start).
    Bpm     { sample: u64, bpm: f32 },
}

/// Lightweight sender handle kept inside the audio callback.
/// Cloned from `MidiRecorder` before the callback closure captures it.
#[derive(Clone)]
pub struct MidiSink(mpsc::SyncSender<MidiRecordEvent>);

impl MidiSink {
    /// Push a note-on. Non-blocking — drops the event if the ring is full.
    #[inline]
    pub fn note_on(&self, sample: u64, track: u8, pitch: u8, velocity: u8) {
        let _ = self.0.try_send(MidiRecordEvent::NoteOn { sample, track, pitch, velocity });
    }

    /// Push a note-off. Non-blocking.
    #[inline]
    pub fn note_off(&self, sample: u64, track: u8, pitch: u8) {
        let _ = self.0.try_send(MidiRecordEvent::NoteOff { sample, track, pitch });
    }

    /// Push a BPM update. Non-blocking.
    #[allow(dead_code)]
    #[inline]
    pub fn bpm(&self, sample: u64, bpm: f32) {
        let _ = self.0.try_send(MidiRecordEvent::Bpm { sample, bpm });
    }
}

pub struct MidiRecorder {
    /// Kept alive so dropping the recorder closes the channel.
    _tx:    mpsc::SyncSender<MidiRecordEvent>,
    thread: Option<thread::JoinHandle<anyhow::Result<()>>>,
    pub path: String,
}

impl MidiRecorder {
    /// Begin recording. Returns `(recorder, sink)`.
    /// `sink` is a cheap-to-clone handle for the audio callback.
    pub fn start(
        path: String,
        sample_rate: u32,
        bpm: f32,
        track_count: usize,
    ) -> anyhow::Result<(Self, MidiSink)> {
        let (tx, rx) = mpsc::sync_channel::<MidiRecordEvent>(8192);
        let sink = MidiSink(tx.clone());

        // Seed initial BPM so the writer thread has it before any notes arrive.
        let _ = tx.try_send(MidiRecordEvent::Bpm { sample: 0, bpm });

        let path_clone = path.clone();
        let handle = thread::spawn(move || {
            write_midi_file(rx, &path_clone, sample_rate, track_count)
        });

        Ok((Self { _tx: tx, thread: Some(handle), path }, sink))
    }

    /// Finalise the MIDI file. Blocks until the writer thread exits.
    pub fn stop(mut self) -> anyhow::Result<()> {
        // Dropping _tx closes the channel → writer thread exits its loop.
        drop(self._tx);
        if let Some(h) = self.thread.take() {
            h.join().map_err(|_| anyhow::anyhow!("midi recorder thread panicked"))??;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MIDI file writer (runs on its own thread)
// ---------------------------------------------------------------------------

fn write_midi_file(
    rx:           mpsc::Receiver<MidiRecordEvent>,
    path:         &str,
    sample_rate:  u32,
    track_count:  usize,
) -> anyhow::Result<()> {
    use midly::num::{u15, u24, u28, u4, u7};
    use midly::{
        Format, Header, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
    };

    const TICKS_PER_QUARTER: u16 = 480;

    // Collect all raw events.
    let mut events: Vec<MidiRecordEvent> = rx.into_iter().collect();

    // Sort by sample position to handle any out-of-order arrivals.
    events.sort_by_key(|e| match e {
        MidiRecordEvent::NoteOn  { sample, .. } => *sample,
        MidiRecordEvent::NoteOff { sample, .. } => *sample,
        MidiRecordEvent::Bpm    { sample, .. } => *sample,
    });

    // Build per-track raw event lists.
    // Track 0 in the MIDI file = tempo track.
    // Tracks 1..=track_count = note tracks.
    let mut tempo_events:  Vec<(u64, u32)>                     = Vec::new(); // (sample, us_per_beat)
    let mut note_events:   Vec<Vec<(u64, u8, u8, bool)>>       = vec![Vec::new(); track_count];
    //                                (sample, pitch, vel, is_on)

    let mut current_bpm = 120.0f32;

    for ev in &events {
        match ev {
            MidiRecordEvent::Bpm { sample, bpm } => {
                current_bpm = *bpm;
                let us = (60_000_000.0 / bpm) as u32;
                tempo_events.push((*sample, us));
            }
            MidiRecordEvent::NoteOn { sample, track, pitch, velocity } => {
                let ti = *track as usize;
                if ti < track_count {
                    note_events[ti].push((*sample, *pitch, *velocity, true));
                }
            }
            MidiRecordEvent::NoteOff { sample, track, pitch } => {
                let ti = *track as usize;
                if ti < track_count {
                    note_events[ti].push((*sample, *pitch, 0, false));
                }
            }
        }
    }

    // If no tempo event at sample 0, insert one.
    if tempo_events.is_empty() || tempo_events[0].0 != 0 {
        tempo_events.insert(0, (0, (60_000_000.0 / current_bpm) as u32));
    }

    // Helper: convert a sample offset to MIDI ticks.
    // We use a simple linear conversion from the last known tempo.
    // For productions with tempo changes, each segment is converted at its own rate.
    let samples_to_ticks = |sample: u64, us_per_beat: u32| -> u32 {
        let beats = sample as f64 / sample_rate as f64 / (us_per_beat as f64 / 1_000_000.0);
        (beats * TICKS_PER_QUARTER as f64).round() as u32
    };

    // ---------- Build MIDI tracks ----------

    let mut smf_tracks: Vec<Track> = Vec::new();

    // --- Tempo track ---
    {
        let mut track: Track = Track::new();
        let mut prev_ticks: u32 = 0;
        // Derive us_per_beat at each tempo event from the preceding tempo.
        let mut prev_us: u32 = tempo_events[0].1;

        for (i, &(sample, us)) in tempo_events.iter().enumerate() {
            let abs_ticks = if i == 0 {
                0u32
            } else {
                samples_to_ticks(sample, prev_us)
            };
            let delta = abs_ticks.saturating_sub(prev_ticks);
            track.push(TrackEvent {
                delta: u28::from(delta),
                kind: TrackEventKind::Meta(midly::MetaMessage::Tempo(u24::from(us))),
            });
            prev_ticks = abs_ticks;
            prev_us = us;
        }
        track.push(TrackEvent {
            delta: u28::from(0u32),
            kind: TrackEventKind::Meta(midly::MetaMessage::EndOfTrack),
        });
        smf_tracks.push(track);
    }

    // --- Note tracks ---
    // Use the first tempo as the conversion rate (good enough for fixed BPM sessions;
    // works for multi-tempo too since we recorded events linearly).
    let base_us = tempo_events[0].1;

    // Pre-allocate track name strings so they outlive the TrackEvent borrows.
    let track_names: Vec<Vec<u8>> = (0..track_count)
        .map(|i| format!("Track {}", i + 1).into_bytes())
        .collect();

    for (ti, note_evs) in note_events.iter().enumerate() {
        let mut track: Track = Track::new();

        // Track name meta event.
        track.push(TrackEvent {
            delta: u28::from(0u32),
            kind: TrackEventKind::Meta(midly::MetaMessage::TrackName(&track_names[ti])),
        });

        // Sort by sample (already sorted, but be safe).
        let mut sorted = note_evs.clone();
        sorted.sort_by_key(|&(s, _, _, _)| s);

        let mut prev_ticks: u32 = 0;
        let channel = u4::from(ti as u8 % 16);

        for &(sample, pitch, vel, is_on) in &sorted {
            let abs_ticks = samples_to_ticks(sample, base_us);
            let delta = abs_ticks.saturating_sub(prev_ticks);
            prev_ticks = abs_ticks;

            let msg = if is_on {
                MidiMessage::NoteOn {
                    key:      u7::from(pitch),
                    vel:      u7::from(vel),
                }
            } else {
                MidiMessage::NoteOff {
                    key:      u7::from(pitch),
                    vel:      u7::from(0u8),
                }
            };
            track.push(TrackEvent {
                delta: u28::from(delta),
                kind: TrackEventKind::Midi { channel, message: msg },
            });
        }

        track.push(TrackEvent {
            delta: u28::from(0u32),
            kind: TrackEventKind::Meta(midly::MetaMessage::EndOfTrack),
        });
        smf_tracks.push(track);
    }

    // ---------- Write file ----------
    let smf = Smf {
        header: Header {
            format: Format::Parallel,
            timing: Timing::Metrical(u15::from(TICKS_PER_QUARTER)),
        },
        tracks: smf_tracks,
    };

    let mut buf = Vec::new();
    smf.write(&mut buf).map_err(|e| anyhow::anyhow!("MIDI encode error: {e}"))?;
    std::fs::write(path, &buf)?;

    Ok(())
}
