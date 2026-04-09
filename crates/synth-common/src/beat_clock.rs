//! Beat clock — sample-accurate musical time source.
//!
//! # Design
//! - Driven by `tick(frames, sr)` called once per audio buffer (same pattern as `ArpState`).
//! - Position is tracked as **sample offset** from the start of the current subdivision,
//!   avoiding floating-point drift over long sessions.
//! - BPM is an atomic `Shared` — readable/writable from any thread with no locking.
//! - `BeatEvents` reports which musical boundaries (subdivision, beat, bar) were crossed
//!   in the current buffer. Callers check the flags; no heap allocation.
//!
//! # Definitions
//! - 1 bar  = `beats_per_bar` beats  (default 4)
//! - 1 beat = `subdivisions` subdivisions  (default 4 → 16th notes)
//! - Position: (bar, beat, subdivision) — all zero-indexed

use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use fundsp::prelude32::{shared, Shared};

// ---------------------------------------------------------------------------
// BeatPosition
// ---------------------------------------------------------------------------

/// A precise musical position expressed as (bar, beat, subdivision).
/// All fields are zero-indexed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BeatPosition {
    pub bar: u64,
    pub beat: u32,
    pub subdivision: u32,
}

// ---------------------------------------------------------------------------
// BeatEvents
// ---------------------------------------------------------------------------

/// Which musical boundaries were crossed during the last `tick`.
/// More than one flag can be set when a buffer spans multiple boundaries.
#[derive(Clone, Copy, Debug, Default)]
pub struct BeatEvents {
    /// A new subdivision boundary was crossed (finest pulse, e.g. 16th note).
    pub subdivision: bool,
    /// A new beat boundary was crossed (e.g. quarter note).
    pub beat: bool,
    /// A new bar boundary was crossed.
    pub bar: bool,
    /// The position at which the first crossed boundary occurred (if any).
    pub position: BeatPosition,
}

impl BeatEvents {
    pub fn any(&self) -> bool {
        self.subdivision || self.beat || self.bar
    }
}

// ---------------------------------------------------------------------------
// BeatClockShared — config visible to other threads
// ---------------------------------------------------------------------------

/// Thread-safe configuration shared between the audio callback and any host thread.
#[derive(Clone)]
pub struct BeatClockShared {
    /// Beats per minute. Clamp to [1, 999] before writing.
    pub bpm: Shared,
    /// Transport playing state. When false, tick() returns empty events.
    pub playing: Arc<AtomicBool>,
}

impl BeatClockShared {
    pub fn new(bpm: f32) -> Self {
        Self { bpm: shared(bpm), playing: Arc::new(AtomicBool::new(false)) }
    }

    pub fn bpm(&self) -> f32 {
        self.bpm.value()
    }

    pub fn set_bpm(&self, bpm: f32) {
        self.bpm.set_value(bpm.clamp(1.0, 999.0));
    }

    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Ordering::Relaxed);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }
}

impl Default for BeatClockShared {
    fn default() -> Self {
        Self::new(120.0)
    }
}

// ---------------------------------------------------------------------------
// BeatClock — mutable state, audio-thread-only
// ---------------------------------------------------------------------------

/// Mutable beat clock state. Lives on the audio thread; not Send.
pub struct BeatClock {
    /// Samples elapsed within the current subdivision.
    samples_in_subdiv: u64,
    /// Current musical position.
    position: BeatPosition,
    /// Number of beats per bar (time signature numerator, e.g. 4).
    pub beats_per_bar: u32,
    /// Number of subdivisions per beat (e.g. 4 → 16th notes).
    pub subdivisions: u32,
    /// Seed for generators (deterministic mode). Set before playback starts.
    pub seed: u64,
    /// Whether the clock is running.
    pub running: bool,
}

impl BeatClock {
    pub fn new(beats_per_bar: u32, subdivisions: u32, seed: u64) -> Self {
        Self {
            samples_in_subdiv: 0,
            position: BeatPosition::default(),
            beats_per_bar,
            subdivisions,
            seed,
            running: true,
        }
    }

    /// Current position (snapshot).
    pub fn position(&self) -> BeatPosition {
        self.position
    }

    /// Reset to bar 0, beat 0, subdivision 0.
    pub fn reset(&mut self) {
        self.samples_in_subdiv = 0;
        self.position = BeatPosition::default();
    }

    /// Advance the clock by `frames` samples at sample rate `sr` and BPM from `shared`.
    ///
    /// Returns a `BeatEvents` describing the *first* musical boundary crossed in this buffer.
    /// If multiple subdivisions fit in one buffer (very low BPM or very large buffer), only
    /// the first crossing is reported — callers processing rhythmic events should call
    /// `tick` once per buffer and act on the coarsest flag needed.
    pub fn tick(&mut self, frames: usize, sr: f64, shared: &BeatClockShared) -> BeatEvents {
        self.running = shared.is_playing();
        if !self.running || frames == 0 {
            return BeatEvents::default();
        }

        let bpm = shared.bpm().max(1.0) as f64;
        // Samples per subdivision at current BPM and SR.
        // subdivisions_per_minute = bpm * subdivisions_per_beat
        // samples_per_subdiv = sr * 60 / subdivisions_per_minute
        let samples_per_subdiv =
            (sr * 60.0) / (bpm * self.subdivisions as f64);
        let samples_per_subdiv_u = samples_per_subdiv as u64;

        let mut events = BeatEvents::default();
        let mut remaining = frames as u64;

        while remaining > 0 {
            let room = samples_per_subdiv_u.saturating_sub(self.samples_in_subdiv);
            if remaining >= room {
                // We cross a subdivision boundary.
                remaining -= room;
                self.samples_in_subdiv = 0;
                self.advance_position();

                if !events.subdivision {
                    // Record first crossing.
                    events.subdivision = true;
                    events.position = self.position;
                    if self.position.subdivision == 0 {
                        events.beat = true;
                    }
                    if self.position.subdivision == 0 && self.position.beat == 0 {
                        events.bar = true;
                    }
                }
            } else {
                self.samples_in_subdiv += remaining;
                remaining = 0;
            }
        }

        events
    }

    // Advance position by one subdivision, wrapping beat and bar.
    fn advance_position(&mut self) {
        self.position.subdivision += 1;
        if self.position.subdivision >= self.subdivisions {
            self.position.subdivision = 0;
            self.position.beat += 1;
            if self.position.beat >= self.beats_per_bar {
                self.position.beat = 0;
                self.position.bar += 1;
            }
        }
    }
}

impl Default for BeatClock {
    fn default() -> Self {
        Self::new(4, 4, 0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> (BeatClock, BeatClockShared) {
        (BeatClock::default(), BeatClockShared::new(120.0))
    }

    /// At 120 BPM, 44100 Hz, 4/4, 4 subdivisions per beat:
    /// samples_per_subdiv = 44100 * 60 / (120 * 4) = 5512.5 → 5512
    #[test]
    fn first_subdiv_fires_after_enough_samples() {
        let (mut clk, sh) = make();
        let sr = 44100.0_f64;
        // 5511 samples — not quite one subdiv yet (5512 is the integer threshold)
        let ev = clk.tick(5511, sr, &sh);
        assert!(!ev.subdivision, "should not fire yet");
        // 1 more → crosses boundary
        let ev = clk.tick(1, sr, &sh);
        assert!(ev.subdivision, "should fire now");
    }

    #[test]
    fn beat_fires_every_four_subdivisions() {
        let (mut clk, sh) = make();
        let sr = 44100.0_f64;
        // One subdiv = 5512 samples. Drive 4 of them.
        let mut beats = 0u32;
        for _ in 0..4 {
            let ev = clk.tick(5513, sr, &sh);
            if ev.beat { beats += 1; }
        }
        assert_eq!(beats, 1, "exactly one beat crossing in 4 subdivisions");
    }

    #[test]
    fn bar_fires_every_sixteen_subdivisions() {
        let (mut clk, sh) = make();
        let sr = 44100.0_f64;
        let mut bars = 0u32;
        for _ in 0..16 {
            let ev = clk.tick(5513, sr, &sh);
            if ev.bar { bars += 1; }
        }
        assert_eq!(bars, 1);
    }

    #[test]
    fn reset_returns_to_zero() {
        let (mut clk, sh) = make();
        clk.tick(5513 * 5, 44100.0, &sh);
        clk.reset();
        assert_eq!(clk.position(), BeatPosition::default());
    }

    #[test]
    fn bpm_change_takes_effect_next_tick() {
        let (mut clk, sh) = make();
        sh.set_bpm(60.0);
        // At 60 BPM: samples_per_subdiv = 44100 * 60 / (60 * 4) = 11025
        let ev = clk.tick(11026, 44100.0, &sh);
        assert!(ev.subdivision);
    }
}
