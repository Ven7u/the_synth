//! Drum track: multi-lane step sequencer with per-lane synthesis.
//!
//! # Thread model
//! - `DrumTrackState` — shared atomics; owned by both UI and audio closure.
//! - `DrumDspState` — mutable audio-thread-only state; never touched by UI.
//!
//! # DSP
//! Each lane is a simple sine oscillator + two-stage amplitude envelope
//! (attack → decay, no sustain — suitable for percussive transients).
//! No heap allocation in the audio path.
//!
//! # Step storage
//! Patterns are stored as flat atomic arrays:
//! - `step_active[pattern * DRUM_LANES + lane]` — u16 bitmask, bit i = step i on.
//! - `step_prob[(pattern * DRUM_LANES + lane) * DRUM_STEPS + step]` — u8 probability 0–100.

use fundsp::prelude32::shared;
pub use fundsp::prelude32::Shared;
use std::sync::{
    atomic::{AtomicBool, AtomicU16, AtomicU8, AtomicUsize, Ordering},
    Arc,
};

use crate::generative::{EuclideanGen, EuclideanShared};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const DRUM_LANES: usize = 6;
pub const DRUM_STEPS: usize = 16;
pub const DRUM_PATTERNS: usize = 8;

pub const DRUM_LANE_NAMES: [&str; DRUM_LANES] = ["Kick", "Snare", "HiHat", "Tom", "Clap", "Perc"];

const LANE_DEFAULT_HZ: [f32; DRUM_LANES] = [65.0, 200.0, 8000.0, 130.0, 400.0, 700.0];
const LANE_DEFAULT_ATTACK: [f32; DRUM_LANES] = [0.003, 0.003, 0.001, 0.003, 0.002, 0.002];
const LANE_DEFAULT_DECAY: [f32; DRUM_LANES] = [0.30, 0.14, 0.05, 0.20, 0.09, 0.13];

// ---------------------------------------------------------------------------
// LCG — duplicated from generators.rs (private there); zero-alloc RT-safe RNG
// ---------------------------------------------------------------------------

struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) ^ self.0) as u32
    }
    fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
    fn next_u8_in(&mut self, n: u8) -> u8 {
        if n == 0 {
            return 0;
        }
        (self.next_u32() % n as u32) as u8
    }
}

// ---------------------------------------------------------------------------
// DrumTrackState — shared parameter store (UI ↔ audio)
// ---------------------------------------------------------------------------

/// Thread-safe state for the drum track.
///
/// UI thread writes parameters; audio callback reads them via atomics.
pub struct DrumTrackState {
    pub enabled: Arc<AtomicBool>,
    /// Active pattern slot index (0..DRUM_PATTERNS-1).
    pub active_pattern: Arc<AtomicU8>,
    /// Master volume for the drum bus (0.0–1.0).
    pub master_vol: Shared,
    /// Current playhead step; written by audio thread, read by UI for display.
    pub playhead_step: Arc<AtomicUsize>,

    // Pattern data — flat layout for cache efficiency.
    //
    // step_active[p * DRUM_LANES + l]: u16 bitmask, bit i = step i is on.
    pub step_active: Vec<Arc<AtomicU16>>,
    // step_prob[(p * DRUM_LANES + l) * DRUM_STEPS + s]: probability 0–100.
    pub step_prob: Vec<Arc<AtomicU8>>,

    // Per-lane parameters
    pub lane_vol: [Shared; DRUM_LANES],
    pub lane_muted: [Arc<AtomicBool>; DRUM_LANES],
    /// Fixed oscillator pitch for this lane (Hz).
    pub lane_pitch_hz: [Shared; DRUM_LANES],
    /// Attack time (seconds) for the amplitude envelope.
    pub lane_attack: [Shared; DRUM_LANES],
    /// Decay time (seconds) for the amplitude envelope.
    pub lane_decay: [Shared; DRUM_LANES],
    /// 0 = Fixed step, 1 = Euclidean, 2 = Probabilistic step.
    pub lane_gen_mode: [Arc<AtomicU8>; DRUM_LANES],
    /// Euclidean generator config (used when lane_gen_mode == 1).
    pub lane_euclidean: [EuclideanShared; DRUM_LANES],
}

impl DrumTrackState {
    pub fn new() -> Self {
        let total_lane_patterns = DRUM_PATTERNS * DRUM_LANES;
        let total_steps = total_lane_patterns * DRUM_STEPS;

        // Default kick pattern: beats 1, 3 (steps 0, 8 in 16-step bar).
        // Other lanes silent by default.
        let step_active: Vec<Arc<AtomicU16>> = (0..total_lane_patterns)
            .map(|idx| {
                let lane = idx % DRUM_LANES;
                let initial: u16 = match lane {
                    0 => 0b0000_0001_0000_0001, // Kick: steps 0 and 8
                    1 => 0b0001_0000_0001_0000, // Snare: steps 4 and 12
                    2 => 0b1010_1010_1010_1010, // HiHat: every other step (8th notes)
                    _ => 0,
                };
                Arc::new(AtomicU16::new(initial))
            })
            .collect();

        let step_prob: Vec<Arc<AtomicU8>> = (0..total_steps)
            .map(|_| Arc::new(AtomicU8::new(100)))
            .collect();

        let euclidean_defaults = [
            EuclideanShared::new(4, 16, 36), // Kick: 4 hits in 16 steps, C1
            EuclideanShared::new(2, 16, 38), // Snare: 2 hits in 16 steps, D1
            EuclideanShared::new(8, 16, 42), // HiHat: 8 hits in 16 steps, F#1
            EuclideanShared::new(3, 16, 41), // Tom: 3 hits, F1
            EuclideanShared::new(2, 16, 39), // Clap: 2 hits, D#1
            EuclideanShared::new(4, 16, 43), // Perc: 4 hits, G1
        ];

        Self {
            enabled: Arc::new(AtomicBool::new(true)),
            active_pattern: Arc::new(AtomicU8::new(0)),
            master_vol: shared(0.8),
            playhead_step: Arc::new(AtomicUsize::new(0)),
            step_active,
            step_prob,
            lane_vol: std::array::from_fn(|_| shared(1.0)),
            lane_muted: std::array::from_fn(|_| Arc::new(AtomicBool::new(false))),
            lane_pitch_hz: std::array::from_fn(|i| shared(LANE_DEFAULT_HZ[i])),
            lane_attack: std::array::from_fn(|i| shared(LANE_DEFAULT_ATTACK[i])),
            lane_decay: std::array::from_fn(|i| shared(LANE_DEFAULT_DECAY[i])),
            lane_gen_mode: std::array::from_fn(|_| Arc::new(AtomicU8::new(0))),
            lane_euclidean: euclidean_defaults,
        }
    }

    // -- Helper accessors ----------------------------------------------------

    pub fn get_step_active(&self, pattern: usize, lane: usize, step: usize) -> bool {
        let mask = self.step_active[pattern * DRUM_LANES + lane].load(Ordering::Relaxed);
        (mask >> step) & 1 != 0
    }

    pub fn set_step_active(&self, pattern: usize, lane: usize, step: usize, on: bool) {
        let idx = pattern * DRUM_LANES + lane;
        let mask = self.step_active[idx].load(Ordering::Relaxed);
        let new_mask = if on {
            mask | (1u16 << step)
        } else {
            mask & !(1u16 << step)
        };
        self.step_active[idx].store(new_mask, Ordering::Relaxed);
    }

    pub fn get_step_prob(&self, pattern: usize, lane: usize, step: usize) -> u8 {
        self.step_prob[(pattern * DRUM_LANES + lane) * DRUM_STEPS + step].load(Ordering::Relaxed)
    }

    pub fn set_step_prob(&self, pattern: usize, lane: usize, step: usize, prob: u8) {
        self.step_prob[(pattern * DRUM_LANES + lane) * DRUM_STEPS + step]
            .store(prob.min(100), Ordering::Relaxed);
    }

    /// Copy all step data for the given pattern into a local snapshot.
    pub fn snapshot_pattern(
        &self,
        pattern: usize,
    ) -> ([u16; DRUM_LANES], [[u8; DRUM_STEPS]; DRUM_LANES]) {
        let mut active = [0u16; DRUM_LANES];
        let mut prob = [[100u8; DRUM_STEPS]; DRUM_LANES];
        for l in 0..DRUM_LANES {
            active[l] = self.step_active[pattern * DRUM_LANES + l].load(Ordering::Relaxed);
            for s in 0..DRUM_STEPS {
                prob[l][s] = self.step_prob[(pattern * DRUM_LANES + l) * DRUM_STEPS + s]
                    .load(Ordering::Relaxed);
            }
        }
        (active, prob)
    }
}

impl Default for DrumTrackState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// DrumDspState — audio-thread mutable state
// ---------------------------------------------------------------------------

/// Per-lane oscillator + envelope state. Lives on the audio thread only.
struct DrumVoice {
    phase: f32,
    env_level: f32,
    /// 0 = idle, 1 = attack, 2 = decay.
    env_stage: u8,
    /// Noise contribution (1.0 for hat, 0.35 for snare, 0.0 for tonal).
    noise_mix: f32,
    /// Pitch envelope: current extra-frequency multiplier above the base pitch.
    /// Starts at `pitch_env_start` on gate and decays exponentially toward 1.0.
    pitch_env: f32,
    /// How high above 1.0 the pitch starts (e.g. 2.5 → starts at 2.5× base Hz).
    pitch_env_start: f32,
    /// Exponential decay coefficient per sample for the pitch envelope.
    /// Pre-computed as exp(-1 / (sweep_time_s * sr)) at construction.
    /// Stored as the ratio; recomputed on gate_on using actual sr.
    pitch_sweep_ms: f32,
}

impl DrumVoice {
    fn new(lane: usize) -> Self {
        let (noise_mix, pitch_env_start, pitch_sweep_ms) = match lane {
            0 => (0.0, 2.5, 60.0),  // Kick:  sine, pitch sweeps from 2.5× down over 60 ms
            1 => (0.35, 1.8, 30.0), // Snare: noise+sine, moderate sweep on the body
            2 => (0.90, 1.0, 0.0),  // HiHat: pure noise, no sweep
            3 => (0.0, 2.0, 40.0),  // Tom:   sine, sweep from 2× over 40 ms
            4 => (0.30, 1.0, 0.0),  // Clap:  noise burst, no sweep
            _ => (0.0, 1.0, 0.0),   // Perc:  tonal, no sweep
        };
        Self {
            phase: 0.0,
            env_level: 0.0,
            env_stage: 0,
            noise_mix,
            pitch_env: 1.0,
            pitch_env_start,
            pitch_sweep_ms,
        }
    }

    fn gate_on(&mut self) {
        self.phase = 0.0;
        self.env_stage = 1;
        self.pitch_env = self.pitch_env_start;
    }

    /// Process one sample; returns amplitude sample in [-1, 1].
    fn tick(&mut self, pitch_hz: f32, attack: f32, decay: f32, rng: &mut Lcg, sr: f32) -> f32 {
        let attack = attack.max(0.001);
        let decay = decay.max(0.001);

        // Amplitude envelope (linear attack → linear decay)
        self.env_level = match self.env_stage {
            1 => {
                let v = self.env_level + 1.0 / (attack * sr);
                if v >= 1.0 {
                    self.env_stage = 2;
                    1.0
                } else {
                    v
                }
            }
            2 => {
                let v = self.env_level - 1.0 / (decay * sr);
                if v <= 0.0 {
                    self.env_stage = 0;
                    0.0
                } else {
                    v
                }
            }
            _ => 0.0,
        };

        if self.env_stage == 0 {
            return 0.0;
        }

        // Pitch envelope — exponential decay toward 1.0 (= base pitch)
        if self.pitch_env > 1.001 && self.pitch_sweep_ms > 0.0 {
            let coeff = (-1.0 / (self.pitch_sweep_ms * 0.001 * sr)).exp();
            self.pitch_env = 1.0 + (self.pitch_env - 1.0) * coeff;
        } else {
            self.pitch_env = 1.0;
        }

        let actual_hz = pitch_hz * self.pitch_env;
        self.phase = (self.phase + actual_hz / sr).fract();
        let sine = (self.phase * std::f32::consts::TAU).sin();

        let noise = if self.noise_mix > 0.0 {
            rng.next_f32() * 2.0 - 1.0
        } else {
            0.0
        };

        let signal = sine * (1.0 - self.noise_mix) + noise * self.noise_mix;
        signal * self.env_level
    }
}

/// Mutable state for the drum step sequencer. Lives on the audio thread only.
pub struct DrumDspState {
    voices: [DrumVoice; DRUM_LANES],
    /// Current step within the 16-step bar.
    step: usize,
    rng: Lcg,
    /// Pattern snapshot used in the current bar (refreshed on bar boundary).
    pattern_active: [u16; DRUM_LANES],
    pattern_prob: [[u8; DRUM_STEPS]; DRUM_LANES],
    /// Pattern index that was active when the snapshot was taken.
    snapped_pattern: u8,
    euclidean_gens: [EuclideanGen; DRUM_LANES],
}

impl DrumDspState {
    pub fn new() -> Self {
        Self {
            voices: std::array::from_fn(DrumVoice::new),
            step: 0,
            rng: Lcg::new(0xDEAD_BEEF_CAFE_1234),
            pattern_active: [0u16; DRUM_LANES],
            pattern_prob: [[100u8; DRUM_STEPS]; DRUM_LANES],
            snapped_pattern: 255, // force snapshot on first bar
            euclidean_gens: std::array::from_fn(|_i| EuclideanGen::new()),
        }
    }

    /// Call on every bar boundary to refresh the pattern snapshot and reset the step counter.
    pub fn on_bar(&mut self, state: &DrumTrackState) {
        let pat = state.active_pattern.load(Ordering::Relaxed);
        if pat != self.snapped_pattern {
            let (active, prob) = state.snapshot_pattern(pat as usize);
            self.pattern_active = active;
            self.pattern_prob = prob;
            self.snapped_pattern = pat;
            self.step = 0;
        }
    }

    /// Call on every subdivision (step) boundary.
    ///
    /// Returns nothing — gates on the voices directly.
    pub fn on_step(&mut self, state: &DrumTrackState) {
        if !state.enabled.load(Ordering::Relaxed) {
            return;
        }

        // Refresh snapshot if pattern changed mid-bar.
        let pat = state.active_pattern.load(Ordering::Relaxed);
        if pat != self.snapped_pattern {
            let (active, prob) = state.snapshot_pattern(pat as usize);
            self.pattern_active = active;
            self.pattern_prob = prob;
            self.snapped_pattern = pat;
        }

        state.playhead_step.store(self.step, Ordering::Relaxed);

        for lane in 0..DRUM_LANES {
            if state.lane_muted[lane].load(Ordering::Relaxed) {
                continue;
            }

            let gen_mode = state.lane_gen_mode[lane].load(Ordering::Relaxed);
            let fire = match gen_mode {
                1 => {
                    // Euclidean
                    let ev = self.euclidean_gens[lane].on_subdivision(&state.lane_euclidean[lane]);
                    ev.note_on.is_some()
                }
                2 => {
                    // Probabilistic: step must be active AND pass prob roll
                    let active = (self.pattern_active[lane] >> self.step) & 1 != 0;
                    if active {
                        let prob = self.pattern_prob[lane][self.step] as f32 / 100.0;
                        self.rng.next_f32() < prob
                    } else {
                        false
                    }
                }
                _ => {
                    // Fixed step (mode 0 or any unknown)
                    (self.pattern_active[lane] >> self.step) & 1 != 0
                }
            };

            if fire {
                self.voices[lane].gate_on();
            }
        }

        self.step = (self.step + 1) % DRUM_STEPS;
    }

    /// Process one audio sample. Returns a mono sum of all lanes.
    ///
    /// Call `on_step` on subdivision boundaries; call `tick_sample` every sample.
    #[inline]
    pub fn tick_sample(&mut self, state: &DrumTrackState, sr: f32) -> f32 {
        if !state.enabled.load(Ordering::Relaxed) {
            return 0.0;
        }
        let mut out = 0.0f32;
        for lane in 0..DRUM_LANES {
            if state.lane_muted[lane].load(Ordering::Relaxed) {
                continue;
            }
            let pitch = state.lane_pitch_hz[lane].value();
            let attack = state.lane_attack[lane].value();
            let decay = state.lane_decay[lane].value();
            let vol = state.lane_vol[lane].value();
            let sample = self.voices[lane].tick(pitch, attack, decay, &mut self.rng, sr);
            out += sample * vol;
        }
        out * state.master_vol.value() / DRUM_LANES as f32
    }
}

impl Default for DrumDspState {
    fn default() -> Self {
        Self::new()
    }
}
