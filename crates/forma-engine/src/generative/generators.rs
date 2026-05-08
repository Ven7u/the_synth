//! Generative pattern generators for ambient/game music.
//!
//! # Design
//! Unlike `ArpState` / `ScaleWalker` (which accumulate phase per sample buffer),
//! these generators are **event-driven**: they advance one step on each call to
//! `on_subdivision()`, which is called when `BeatClock::tick()` returns
//! `BeatEvents { subdivision: true }`.
//!
//! This keeps them decoupled from sample rate and buffer size — the caller
//! owns all timing and just notifies generators when a musical boundary fires.
//!
//! # Generators
//! - `EuclideanGen` — Bjorklund rhythm: N hits distributed evenly across M steps.
//! - `ProbTableGen` — Per-step (note, probability) table; `tension` biases density.
//!
//! # GenerativeMode
//! `GenerativeMode::ScaleWalk` delegates to the existing `ScaleWalker` in
//! `forma-engine`, which runs its own phase accumulator (BPM-driven, not BeatClock).
//! The remaining modes use the generators in this file.

use fundsp::prelude32::{shared, Shared};
use std::sync::{
    atomic::{AtomicBool, AtomicU8, Ordering},
    Arc,
};

// ---------------------------------------------------------------------------
// Minimal LCG — no_std-safe, zero heap, no rand dependency
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
    /// Uniform in [0, n).
    fn next_u8_in(&mut self, n: u8) -> u8 {
        if n == 0 {
            return 0;
        }
        (self.next_u32() % n as u32) as u8
    }
}

// ---------------------------------------------------------------------------
// GenEvent — zero-alloc output
// ---------------------------------------------------------------------------

/// What a generator emits for a single subdivision step.
#[derive(Clone, Copy, Debug, Default)]
pub struct GenEvent {
    /// New note to play (MIDI pitch 0-127). The caller must send NoteOn.
    pub note_on: Option<u8>,
    /// Note that was sounding and should be released. The caller must send NoteOff.
    pub note_off: Option<u8>,
}

// ---------------------------------------------------------------------------
// GenerativeMode
// ---------------------------------------------------------------------------

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GenerativeMode {
    #[default]
    Off = 0,
    Euclidean = 1,
    ProbTable = 2,
    /// Delegates to `forma_engine::ScaleWalker` (phase-accumulator driven).
    ScaleWalk = 3,
    /// Global Markov music engine (Phase 8.3). When set on track 0, all tracks
    /// are driven by `MarkovEngine`; per-track Euclidean/ProbTable modes are ignored.
    Markov = 4,
    /// Human-authored step sequence: a fixed loop of note/chord steps.
    Step = 5,
    /// Human-authored step sequence with per-step fire probability.
    StepProb = 6,
}

impl GenerativeMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Euclidean,
            2 => Self::ProbTable,
            3 => Self::ScaleWalk,
            4 => Self::Markov,
            5 => Self::Step,
            6 => Self::StepProb,
            _ => Self::Off,
        }
    }
    pub const ALL: &'static [Self] = &[
        Self::Off,
        Self::Euclidean,
        Self::ProbTable,
        Self::ScaleWalk,
        Self::Markov,
        Self::Step,
        Self::StepProb,
    ];
    pub const LABELS: &'static [&'static str] = &[
        "Off",
        "Euclidean",
        "Prob.Table",
        "ScaleWalk",
        "Markov",
        "Step",
        "Step+Prob",
    ];
}

// ===========================================================================
// EuclideanGen
// ===========================================================================

/// Maximum pattern length supported.
pub const EUCLIDEAN_MAX_STEPS: usize = 32;

// ---------------------------------------------------------------------------
// EuclideanShared — thread-safe config
// ---------------------------------------------------------------------------

/// Thread-safe configuration for the Euclidean generator.
/// Clone and hand copies to the UI / Bevy thread; the audio thread holds one too.
#[derive(Clone)]
pub struct EuclideanShared {
    pub enabled: Arc<AtomicBool>,
    /// N hits distributed across M steps (Bjorklund). Clamped to [1, steps].
    pub hits: Arc<AtomicU8>,
    /// M total steps. Clamped to [1, EUCLIDEAN_MAX_STEPS].
    pub steps: Arc<AtomicU8>,
    /// MIDI pitch played on each hit.
    pub root: Arc<AtomicU8>,
    /// Rotate the pattern by this many steps (0 = no rotation).
    pub rotation: Arc<AtomicU8>,
}

impl EuclideanShared {
    pub fn new(hits: u8, steps: u8, root: u8) -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(true)),
            hits: Arc::new(AtomicU8::new(hits.max(1).min(steps.max(1)))),
            steps: Arc::new(AtomicU8::new(steps.max(1).min(EUCLIDEAN_MAX_STEPS as u8))),
            root: Arc::new(AtomicU8::new(root)),
            rotation: Arc::new(AtomicU8::new(0)),
        }
    }
}

impl Default for EuclideanShared {
    fn default() -> Self {
        Self::new(4, 8, 60)
    }
}

// ---------------------------------------------------------------------------
// Bjorklund algorithm
// ---------------------------------------------------------------------------

/// Compute a Euclidean rhythm pattern: distribute `hits` hits across `steps` slots.
/// Uses the Bresenham / Bjorklund approach — O(n), no heap.
fn euclidean_pattern(hits: u8, steps: u8) -> [bool; EUCLIDEAN_MAX_STEPS] {
    let mut pattern = [false; EUCLIDEAN_MAX_STEPS];
    let n = (steps as usize).min(EUCLIDEAN_MAX_STEPS);
    let k = (hits as usize).min(n);
    if n == 0 || k == 0 {
        return pattern;
    }

    // Formula: step i fires iff (i * k) % n < k.
    // This distributes k hits evenly starting at step 0 (canonical Bjorklund result).
    for i in 0..n {
        pattern[i] = (i * k) % n < k;
    }
    pattern
}

/// Apply a rotation offset to a pattern (cyclic shift left by `rot` steps).
fn rotate_pattern(
    src: &[bool; EUCLIDEAN_MAX_STEPS],
    steps: usize,
    rot: usize,
) -> [bool; EUCLIDEAN_MAX_STEPS] {
    let mut dst = [false; EUCLIDEAN_MAX_STEPS];
    let rot = rot % steps.max(1);
    for i in 0..steps {
        dst[i] = src[(i + rot) % steps];
    }
    dst
}

// ---------------------------------------------------------------------------
// EuclideanGen — audio-thread state
// ---------------------------------------------------------------------------

/// Mutable Euclidean generator state. Lives on the audio thread.
pub struct EuclideanGen {
    pattern: [bool; EUCLIDEAN_MAX_STEPS],
    step: usize,
    current_note: Option<u8>,
    // Track config to detect changes and rebuild pattern.
    prev_hits: u8,
    prev_steps: u8,
    prev_rotation: u8,
    prev_enabled: bool,
}

impl EuclideanGen {
    pub fn new() -> Self {
        let default = EuclideanShared::default();
        let hits = default.hits.load(Ordering::Relaxed);
        let steps = default.steps.load(Ordering::Relaxed);
        Self {
            pattern: euclidean_pattern(hits, steps),
            step: 0,
            current_note: None,
            prev_hits: hits,
            prev_steps: steps,
            prev_rotation: 0,
            prev_enabled: true,
        }
    }

    /// Call once per subdivision boundary (when `BeatEvents::subdivision` is true).
    /// Returns a `GenEvent` with optional note_off (previous note) and note_on (new hit).
    pub fn on_subdivision(&mut self, cfg: &EuclideanShared) -> GenEvent {
        let enabled = cfg.enabled.load(Ordering::Relaxed);
        let hits = cfg.hits.load(Ordering::Relaxed);
        let steps = cfg
            .steps
            .load(Ordering::Relaxed)
            .max(1)
            .min(EUCLIDEAN_MAX_STEPS as u8);
        let root = cfg.root.load(Ordering::Relaxed);
        let rotation = cfg.rotation.load(Ordering::Relaxed);

        // Transition: disabled → fire note_off and reset.
        if !enabled {
            let off = self.current_note.take();
            if self.prev_enabled {
                self.step = 0;
                self.prev_enabled = false;
            }
            return GenEvent {
                note_on: None,
                note_off: off,
            };
        }
        self.prev_enabled = true;

        // Rebuild pattern if config changed.
        if hits != self.prev_hits || steps != self.prev_steps || rotation != self.prev_rotation {
            let base = euclidean_pattern(hits, steps);
            self.pattern = rotate_pattern(&base, steps as usize, rotation as usize);
            self.prev_hits = hits;
            self.prev_steps = steps;
            self.prev_rotation = rotation;
            // Keep step in range.
            self.step = self.step % steps as usize;
        }

        let n = steps as usize;
        let hit = self.pattern[self.step % n.max(1)];

        // Always fire note_off for the previous note first.
        let note_off = self.current_note.take();

        let note_on = if hit {
            self.current_note = Some(root);
            Some(root)
        } else {
            None
        };

        // Advance step.
        self.step = (self.step + 1) % n.max(1);

        GenEvent { note_on, note_off }
    }

    /// Reset step counter (e.g. on bar boundary for sync).
    pub fn reset(&mut self) {
        self.step = 0;
    }
}

impl Default for EuclideanGen {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// ProbTableGen
// ===========================================================================

/// Maximum number of steps in the probability table.
pub const PROB_TABLE_MAX_STEPS: usize = 16;

// ---------------------------------------------------------------------------
// ProbTableShared — thread-safe config
// ---------------------------------------------------------------------------

/// Thread-safe configuration for the probability table generator.
#[derive(Clone)]
pub struct ProbTableShared {
    pub enabled: Arc<AtomicBool>,
    /// Number of active steps (1-16).
    pub step_count: Arc<AtomicU8>,
    /// MIDI pitch per step (0-127).
    pub notes: [Arc<AtomicU8>; PROB_TABLE_MAX_STEPS],
    /// Probability per step encoded as 0-100 (i.e. 0% to 100%).
    pub probs: [Arc<AtomicU8>; PROB_TABLE_MAX_STEPS],
    /// Tension (0.0-2.0): 1.0 = as configured, 0.0 = nothing fires, 2.0 = always fires.
    pub tension: Shared,
}

impl ProbTableShared {
    pub fn new() -> Self {
        // Default: 8 steps, C-major pentatonic (C D E G A), moderate probability.
        const DEFAULT_NOTES: [u8; PROB_TABLE_MAX_STEPS] = [
            60, 62, 64, 67, 69, 60, 62, 64, // C4 D4 E4 G4 A4 ...
            67, 69, 60, 62, 64, 67, 69, 60,
        ];
        const DEFAULT_PROBS: [u8; PROB_TABLE_MAX_STEPS] = [
            80, 60, 70, 50, 65, 40, 75, 55, 60, 70, 50, 65, 80, 45, 70, 60,
        ];
        Self {
            enabled: Arc::new(AtomicBool::new(true)),
            step_count: Arc::new(AtomicU8::new(8)),
            notes: std::array::from_fn(|i| Arc::new(AtomicU8::new(DEFAULT_NOTES[i]))),
            probs: std::array::from_fn(|i| Arc::new(AtomicU8::new(DEFAULT_PROBS[i]))),
            tension: shared(1.0),
        }
    }

    pub fn set_step(&self, i: usize, note: u8, prob_pct: u8) {
        if i < PROB_TABLE_MAX_STEPS {
            self.notes[i].store(note, Ordering::Relaxed);
            self.probs[i].store(prob_pct.min(100), Ordering::Relaxed);
        }
    }
}

impl Default for ProbTableShared {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ProbTableGen — audio-thread state
// ---------------------------------------------------------------------------

/// Mutable probability table generator state. Lives on the audio thread.
pub struct ProbTableGen {
    step: usize,
    current_note: Option<u8>,
    rng: Lcg,
    prev_enabled: bool,
}

impl ProbTableGen {
    pub fn new(seed: u64) -> Self {
        Self {
            step: 0,
            current_note: None,
            rng: Lcg::new(seed),
            prev_enabled: true,
        }
    }

    /// Call once per subdivision boundary.
    pub fn on_subdivision(&mut self, cfg: &ProbTableShared) -> GenEvent {
        let enabled = cfg.enabled.load(Ordering::Relaxed);
        let step_count = (cfg.step_count.load(Ordering::Relaxed) as usize)
            .max(1)
            .min(PROB_TABLE_MAX_STEPS);

        // Transition: disabled → note_off and reset.
        if !enabled {
            let off = self.current_note.take();
            if self.prev_enabled {
                self.step = 0;
                self.prev_enabled = false;
            }
            return GenEvent {
                note_on: None,
                note_off: off,
            };
        }
        self.prev_enabled = true;

        let note = cfg.notes[self.step].load(Ordering::Relaxed);
        let prob = cfg.probs[self.step].load(Ordering::Relaxed) as f32 / 100.0;
        let tension = cfg.tension.value().clamp(0.0, 2.0);
        let adj_prob = (prob * tension).clamp(0.0, 1.0);

        // Always fire note_off first.
        let note_off = self.current_note.take();

        // Roll the dice: compare LCG output (0-99) against adjusted probability (0-100).
        let roll = self.rng.next_u8_in(100) as f32 / 100.0;
        let note_on = if roll < adj_prob {
            self.current_note = Some(note);
            Some(note)
        } else {
            None
        };

        self.step = (self.step + 1) % step_count;

        GenEvent { note_on, note_off }
    }

    /// Reset step counter (e.g. on bar boundary for sync).
    pub fn reset(&mut self) {
        self.step = 0;
    }
}

impl Default for ProbTableGen {
    fn default() -> Self {
        Self::new(0xABCD_EF01_2345_6789)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Bjorklund ----

    #[test]
    fn euclidean_4_in_8_is_tresillo_extended() {
        // E(4,8) = [1,0,1,0,1,0,1,0] — evenly spaced
        let p = euclidean_pattern(4, 8);
        assert!(p[0] && !p[1] && p[2] && !p[3] && p[4] && !p[5] && p[6] && !p[7]);
    }

    #[test]
    fn euclidean_3_in_8_is_tresillo() {
        // E(3,8) = [1,0,0,1,0,0,1,0]
        let p = euclidean_pattern(3, 8);
        let hits: Vec<usize> = (0..8).filter(|&i| p[i]).collect();
        assert_eq!(hits, vec![0, 3, 6]);
    }

    #[test]
    fn euclidean_all_hits_fills_all_steps() {
        let p = euclidean_pattern(8, 8);
        assert!(p[..8].iter().all(|&h| h));
    }

    #[test]
    fn euclidean_zero_hits_empty_pattern() {
        let p = euclidean_pattern(0, 8);
        assert!(p.iter().all(|&h| !h));
    }

    #[test]
    fn rotation_shifts_pattern() {
        let base = euclidean_pattern(3, 8); // hits at 0, 3, 6
        let rot = rotate_pattern(&base, 8, 1); // hits at 7, 2, 5 (shift left by 1)
        let hits: Vec<usize> = (0..8).filter(|&i| rot[i]).collect();
        assert_eq!(hits, vec![2, 5, 7]);
    }

    // ---- EuclideanGen ----

    #[test]
    fn euclidean_gen_fires_on_hits_only() {
        let cfg = EuclideanShared::new(3, 8, 60);
        let mut gen = EuclideanGen::new();
        // E(3,8): hits at steps 0, 3, 6
        let mut note_ons = vec![];
        for _ in 0..8 {
            let ev = gen.on_subdivision(&cfg);
            if ev.note_on.is_some() {
                note_ons.push(gen.step.wrapping_sub(1) % 8);
            }
        }
        assert_eq!(note_ons.len(), 3);
    }

    #[test]
    fn euclidean_gen_note_off_precedes_note_on() {
        // When a new hit fires on the step after a previous hit, note_off and note_on
        // are both present in the same event (note_off for previous, note_on for new).
        let cfg = EuclideanShared::new(8, 8, 60); // every step fires
        let mut gen = EuclideanGen::new();
        let ev0 = gen.on_subdivision(&cfg); // step 0: note_on=60, note_off=None
        assert!(ev0.note_on.is_some());
        assert!(ev0.note_off.is_none());
        let ev1 = gen.on_subdivision(&cfg); // step 1: note_off=60, note_on=60
        assert_eq!(ev1.note_off, Some(60));
        assert_eq!(ev1.note_on, Some(60));
    }

    #[test]
    fn euclidean_gen_disabled_fires_note_off() {
        let cfg = EuclideanShared::new(4, 8, 60);
        let mut gen = EuclideanGen::new();
        gen.on_subdivision(&cfg); // fires note_on
        cfg.enabled.store(false, Ordering::Relaxed);
        let ev = gen.on_subdivision(&cfg);
        assert!(ev.note_off.is_some());
        assert!(ev.note_on.is_none());
    }

    // ---- ProbTableGen ----

    #[test]
    fn prob_table_tension_zero_never_fires() {
        let cfg = ProbTableShared::new();
        cfg.tension.set_value(0.0);
        let mut gen = ProbTableGen::new(42);
        for _ in 0..100 {
            let ev = gen.on_subdivision(&cfg);
            assert!(ev.note_on.is_none(), "tension=0 should never fire");
        }
    }

    #[test]
    fn prob_table_tension_two_always_fires() {
        let cfg = ProbTableShared::new();
        // Set all probs to 50% so with tension=2 they become 100%.
        for i in 0..PROB_TABLE_MAX_STEPS {
            cfg.probs[i].store(50, Ordering::Relaxed);
        }
        cfg.tension.set_value(2.0);
        let mut gen = ProbTableGen::new(42);
        for _ in 0..16 {
            let ev = gen.on_subdivision(&cfg);
            assert!(
                ev.note_on.is_some(),
                "tension=2 + prob=50% should always fire"
            );
        }
    }

    #[test]
    fn prob_table_wraps_steps() {
        let cfg = ProbTableShared::new();
        cfg.step_count.store(4, Ordering::Relaxed);
        cfg.tension.set_value(2.0);
        for i in 0..4 {
            cfg.probs[i].store(100, Ordering::Relaxed);
        }
        let mut gen = ProbTableGen::new(0);
        // Run 8 steps — should cycle through 4 steps twice, always firing.
        let mut fired = 0usize;
        for _ in 0..8 {
            if gen.on_subdivision(&cfg).note_on.is_some() {
                fired += 1;
            }
        }
        assert_eq!(fired, 8);
    }

    #[test]
    fn prob_table_disabled_fires_note_off() {
        let cfg = ProbTableShared::new();
        cfg.tension.set_value(2.0);
        cfg.probs[0].store(100, Ordering::Relaxed);
        let mut gen = ProbTableGen::new(0);
        gen.on_subdivision(&cfg); // fires note_on
        cfg.enabled.store(false, Ordering::Relaxed);
        let ev = gen.on_subdivision(&cfg);
        assert!(ev.note_off.is_some());
        assert!(ev.note_on.is_none());
    }
}
