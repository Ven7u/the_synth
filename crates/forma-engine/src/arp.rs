//! Chord-responsive arpeggiator and scale walker.
//!
//! # Design
//! - `ArpShared` / `ScaleWalkerShared` — atomic config structs shared between the UI thread
//!   and the audio callback. Mirror the same pattern as `AudioState`.
//! - `ArpState` / `ScaleWalker` — mutable algorithm state owned by the audio callback closure.
//!   No atomics needed: audio thread only.
//! - `tick(cfg, frames, sr)` — called once per audio buffer. Returns an `ArpEvents` pair
//!   (optional note_on + optional note_off). Zero heap allocation.
//!
//! # Integration
//! In the audio callback:
//! 1. Route `ControlEvent::NoteOn/Off` to `arp.note_on/off()` when arp is enabled,
//!    or directly to voice allocation when disabled.
//! 2. `ControlEvent::ChordHold` → `arp.set_chord()` regardless of enabled state.
//! 3. Call `arp.tick(cfg, frames, sr)` once per buffer; handle returned events as if they
//!    were direct `NoteOn/Off` events hitting the voice allocator.
//! 4. Same pattern for `ScaleWalker` (no input, just tick).

use fundsp::prelude32::{shared, Shared};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Euclidean rhythm generator
// ---------------------------------------------------------------------------

/// Generate a Euclidean rhythm bitmask: distribute `k` hits across `n` steps
/// as evenly as possible (Bjorklund / Toussaint algorithm).
/// `rotation` shifts the pattern forward by that many steps.
/// Bit `i` of the result = step `i` is active.
pub fn euclidean_pattern(k: usize, n: usize, rotation: usize) -> u32 {
    if n == 0 || n > 32 {
        return 0;
    }
    let k = k.min(n);
    let rotation = rotation % n;
    let mut pattern = 0u32;
    for i in 0..n {
        let base_i = (i + n - rotation) % n;
        if (base_i * k) % n < k {
            pattern |= 1 << i;
        }
    }
    pattern
}

/// Named rhythmic presets: (label, k, n, rotation).
/// All are musically useful as arp gate patterns.
pub const RING_PRESETS: &[(&str, u8, u8, u8)] = &[
    ("Full", 8, 8, 0),      // all steps active — normal arp
    ("Tresillo", 3, 8, 0),  // E(3,8) — Afro-Cuban syncopation
    ("Clave", 3, 8, 1),     // E(3,8) rotated — habanera feel
    ("Cinquillo", 5, 8, 0), // E(5,8) — denser Cuban pattern
    ("Bossa", 5, 16, 0),    // E(5,16) — bossa nova phrasing
    ("Sparse", 3, 16, 0),   // E(3,16) — wide spacious gaps
    ("Offbeat", 4, 8, 1),   // E(4,8) rotated — syncopated push
];

// ---------------------------------------------------------------------------
// Simple RT-safe LCG — no std RNG, no heap
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
    fn next_usize(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        self.next_u32() as usize % n
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArpMode {
    Up = 0,
    Down = 1,
    UpDown = 2,
    Random = 3,
    AsPlayed = 4,
    ThirdsWalk = 5, // +2 −1 delta: 1,3,2,4,3,5,...
    Alberti = 6,    // low, high, mid, high loop
    Pendulum = 7,   // walking idx alternates with highest: 1,N,2,N,3,N,...
}

impl ArpMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Down,
            2 => Self::UpDown,
            3 => Self::Random,
            4 => Self::AsPlayed,
            5 => Self::ThirdsWalk,
            6 => Self::Alberti,
            7 => Self::Pendulum,
            _ => Self::Up,
        }
    }
    pub const ALL: &'static [ArpMode] = &[
        Self::Up,
        Self::Down,
        Self::UpDown,
        Self::Random,
        Self::AsPlayed,
        Self::ThirdsWalk,
        Self::Alberti,
        Self::Pendulum,
    ];
    pub const LABELS: &'static [&'static str] = &[
        "Up", "Down", "UpDn", "Rnd", "Played", "3rds", "Alberti", "Pendulum",
    ];
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClockDiv {
    Quarter = 0,
    Eighth = 1,
    Sixteenth = 2,
    Thirtysecond = 3,
    EighthTriplet = 4,
    SixteenthTriplet = 5,
}

impl ClockDiv {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Eighth,
            2 => Self::Sixteenth,
            3 => Self::Thirtysecond,
            4 => Self::EighthTriplet,
            5 => Self::SixteenthTriplet,
            _ => Self::Quarter,
        }
    }
    /// Step duration as a fraction of one beat.
    pub fn beats_per_step(self) -> f32 {
        match self {
            Self::Quarter => 1.0,
            Self::Eighth => 0.5,
            Self::Sixteenth => 0.25,
            Self::Thirtysecond => 0.125,
            Self::EighthTriplet => 1.0 / 3.0,
            Self::SixteenthTriplet => 1.0 / 6.0,
        }
    }
    pub const ALL: &'static [ClockDiv] = &[
        Self::Quarter,
        Self::Eighth,
        Self::Sixteenth,
        Self::Thirtysecond,
        Self::EighthTriplet,
        Self::SixteenthTriplet,
    ];
    pub const LABELS: &'static [&'static str] = &["1/4", "1/8", "1/16", "1/32", "1/8T", "1/16T"];
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    Major = 0,
    Minor = 1,
    Dorian = 2,
    Phrygian = 3,
    Lydian = 4,
    Mixolydian = 5,
    Locrian = 6,
    Pentatonic = 7,
    MinorPenta = 8,
    Blues = 9,
    Chromatic = 10,
}

impl Scale {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Minor,
            2 => Self::Dorian,
            3 => Self::Phrygian,
            4 => Self::Lydian,
            5 => Self::Mixolydian,
            6 => Self::Locrian,
            7 => Self::Pentatonic,
            8 => Self::MinorPenta,
            9 => Self::Blues,
            10 => Self::Chromatic,
            _ => Self::Major,
        }
    }
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Self::Major => &[0, 2, 4, 5, 7, 9, 11],
            Self::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Self::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Self::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Self::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Self::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Self::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            Self::Pentatonic => &[0, 2, 4, 7, 9],
            Self::MinorPenta => &[0, 3, 5, 7, 10],
            Self::Blues => &[0, 3, 5, 6, 7, 10],
            Self::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        }
    }
    pub const ALL: &'static [Scale] = &[
        Self::Major,
        Self::Minor,
        Self::Dorian,
        Self::Phrygian,
        Self::Lydian,
        Self::Mixolydian,
        Self::Locrian,
        Self::Pentatonic,
        Self::MinorPenta,
        Self::Blues,
        Self::Chromatic,
    ];
    pub const LABELS: &'static [&'static str] = &[
        "Major",
        "Minor",
        "Dorian",
        "Phrygian",
        "Lydian",
        "Mixolyd.",
        "Locrian",
        "Penta",
        "m.Penta",
        "Blues",
        "Chromatic",
    ];
}

// ---------------------------------------------------------------------------
// ArpEvents — zero-alloc output from tick()
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
pub struct ArpEvents {
    pub note_on: Option<u8>,
    pub note_off: Option<u8>,
}

// ---------------------------------------------------------------------------
// ArpShared — UI-accessible config (atomics, safe to share across threads)
// ---------------------------------------------------------------------------

pub struct ArpShared {
    pub enabled: Arc<AtomicBool>,
    pub mode: Arc<AtomicU8>,         // ArpMode as u8
    pub division: Arc<AtomicU8>,     // ClockDiv as u8
    pub octave_range: Arc<AtomicU8>, // 1-4
    pub gate: Shared,                // 0.05-1.0  (fraction of step note is held)
    pub hold: Arc<AtomicBool>,       // latch: keep chord after keys released
    pub bpm: Shared,                 // 20-300
    // Ring gate sequencer
    pub ring_enabled: Arc<AtomicBool>,
    pub ring_steps: Arc<AtomicU8>,    // N: 2-16
    pub ring_pattern: Arc<AtomicU32>, // bitmask: bit i = step i active
    pub ring_pos: Arc<AtomicU8>,      // current step (written by audio, read by UI)
}

impl ArpShared {
    pub fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            mode: Arc::new(AtomicU8::new(ArpMode::Up as u8)),
            division: Arc::new(AtomicU8::new(ClockDiv::Eighth as u8)),
            octave_range: Arc::new(AtomicU8::new(1)),
            gate: shared(0.7),
            hold: Arc::new(AtomicBool::new(false)),
            bpm: shared(120.0),
            ring_enabled: Arc::new(AtomicBool::new(false)),
            ring_steps: Arc::new(AtomicU8::new(8)),
            ring_pattern: Arc::new(AtomicU32::new(0xFF)),
            ring_pos: Arc::new(AtomicU8::new(0)),
        }
    }
}

impl Default for ArpShared {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ArpState — mutable algorithm state, audio callback only
// ---------------------------------------------------------------------------

pub struct ArpState {
    // Held notes — insertion order (for AsPlayed) and sorted (for Up/Down/UpDown)
    held: [u8; 32],
    held_sorted: [u8; 32],
    held_count: usize,

    // Sequencer
    step_idx: usize,
    current: Option<u8>,
    phase: f32, // 0..1 within current step
    gate_fired: bool,
    direction: i8, // +1 or -1, for UpDown mode

    // Ring gate sequencer position (independent cycle from note sequence)
    ring_step: usize,

    // Pattern-mode state (ThirdsWalk, Alberti, Pendulum)
    pattern_pos: usize,  // position within the mode's formula cycle
    pattern_base: usize, // ThirdsWalk: accumulated absolute index

    rng: Lcg,
    prev_enabled: bool,
    restart_pending: bool,
}

impl ArpState {
    pub fn new() -> Self {
        Self {
            held: [0; 32],
            held_sorted: [0; 32],
            held_count: 0,
            step_idx: 0,
            current: None,
            phase: 0.0,
            gate_fired: false,
            direction: 1,
            ring_step: 0,
            pattern_pos: 0,
            pattern_base: 0,
            rng: Lcg::new(0xDEAD_BEEF_1234_5678),
            prev_enabled: false,
            restart_pending: false,
        }
    }

    /// Call from callback when arp is enabled and a NoteOn arrives.
    pub fn note_on(&mut self, pitch: u8) {
        if self.held_count >= 32 {
            return;
        }
        // Avoid duplicates
        if self.held[..self.held_count].contains(&pitch) {
            return;
        }
        let was_empty = self.held_count == 0;
        self.held[self.held_count] = pitch;
        self.held_count += 1;
        self.rebuild_sorted();
        // First note of a fresh chord: rewind and force an immediate step boundary
        // so the first arp note fires on the very next audio buffer, not after a
        // full step delay.
        if was_empty {
            self.step_idx = 0;
            self.direction = 1;
            self.pattern_pos = 0;
            self.pattern_base = 0;
            self.phase = 1.0; // trigger step boundary on next tick
            self.gate_fired = false;
            self.restart_pending = true;
        }
    }

    /// Call from callback when arp is enabled and a NoteOff arrives.
    pub fn note_off(&mut self, pitch: u8, hold: bool) {
        if hold {
            return;
        }
        let Some(pos) = self.held[..self.held_count]
            .iter()
            .position(|&n| n == pitch)
        else {
            return;
        };
        for i in pos..self.held_count - 1 {
            self.held[i] = self.held[i + 1];
        }
        self.held_count -= 1;
        if self.held_count > 0 {
            self.step_idx %= self.held_count;
        }
        self.rebuild_sorted();
    }

    /// Latch a full chord (e.g. from ControlEvent::ChordHold or programmatic use).
    pub fn set_chord(&mut self, notes: &[u8]) {
        let n = notes.len().min(32);
        self.held[..n].copy_from_slice(&notes[..n]);
        self.held_count = n;
        self.step_idx = 0;
        self.restart_pending = true;
        self.rebuild_sorted();
    }

    /// Clear all held notes. Fires NoteOff for the current sounding note.
    pub fn clear(&mut self) -> Option<u8> {
        self.held_count = 0;
        self.step_idx = 0;
        self.current.take()
    }

    /// Restart arp sequencing without clearing the held chord.
    /// Returns a note_off for the currently sounding note (if any).
    pub fn restart(&mut self) -> Option<u8> {
        let off = self.current.take();
        self.step_idx = 0;
        self.ring_step = 0;
        self.pattern_pos = 0;
        self.pattern_base = 0;
        self.phase = 1.0; // force immediate retrigger on next tick
        self.gate_fired = false;
        self.direction = 1;
        self.restart_pending = true;
        off
    }

    /// Advance the arpeggiator by `frames` samples. Call once per audio buffer.
    /// Returns optional note_off (gate end or step boundary) and note_on (new step).
    pub fn tick(&mut self, cfg: &ArpShared, frames: usize, sr: f64) -> ArpEvents {
        let enabled = cfg.enabled.load(Ordering::Relaxed);

        // Transition: just disabled -> fire NoteOff and fully reset arp state.
        // Without this, stale held notes can survive a disable/enable cycle.
        if !enabled && self.prev_enabled {
            let off = self.current.take();
            self.held_count = 0;
            self.step_idx = 0;
            self.ring_step = 0;
            self.pattern_pos = 0;
            self.pattern_base = 0;
            self.phase = 0.0;
            self.gate_fired = false;
            self.direction = 1;
            self.restart_pending = false;
            self.prev_enabled = false;
            return ArpEvents {
                note_on: None,
                note_off: off,
            };
        }

        // Transition: just enabled -> restart from a clean step boundary.
        if enabled && !self.prev_enabled {
            self.step_idx = 0;
            self.ring_step = 0;
            self.pattern_pos = 0;
            self.pattern_base = 0;
            self.phase = 1.0; // force immediate first step note_on on this tick
            self.gate_fired = false;
            self.direction = 1;
            self.restart_pending = true;
        }
        self.prev_enabled = enabled;

        if !enabled || self.held_count == 0 {
            // Fire NoteOff for any still-sounding note before going silent
            if let Some(note) = self.current.take() {
                return ArpEvents {
                    note_on: None,
                    note_off: Some(note),
                };
            }
            return ArpEvents::default();
        }

        let bpm = cfg.bpm.value().max(1.0) as f64;
        let division = ClockDiv::from_u8(cfg.division.load(Ordering::Relaxed));
        let gate = cfg.gate.value().clamp(0.05, 0.99);

        let step_secs = 60.0 / bpm * division.beats_per_step() as f64;
        let delta = frames as f32 / (step_secs * sr) as f32;

        self.phase += delta;

        let mut ev = ArpEvents::default();

        // Gate off: NoteOff fires at `gate` fraction through the step
        if !self.gate_fired && self.phase >= gate {
            if let Some(note) = self.current.take() {
                ev.note_off = Some(note);
            }
            self.gate_fired = true;
        }

        // Step boundary: advance pattern, fire NoteOn
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.gate_fired = false;

            // NoteOff if gate=1.0 (current wasn't cleared above)
            if let Some(note) = self.current.take() {
                ev.note_off = Some(note);
            }

            if self.held_count > 0 {
                let mode = ArpMode::from_u8(cfg.mode.load(Ordering::Relaxed));
                let octave_range = cfg.octave_range.load(Ordering::Relaxed).clamp(1, 4) as usize;

                // Ring gate: check if this step should fire a note.
                // ring_step advances every clock step; note sequence only advances on hits.
                let ring_on = cfg.ring_enabled.load(Ordering::Relaxed);
                let fire = if ring_on {
                    let n = (cfg.ring_steps.load(Ordering::Relaxed) as usize).clamp(2, 16);
                    let pattern = cfg.ring_pattern.load(Ordering::Relaxed);
                    let idx = self.ring_step % n;
                    let active = (pattern >> idx) & 1 == 1;
                    cfg.ring_pos.store(idx as u8, Ordering::Relaxed);
                    self.ring_step = idx + 1;
                    active
                } else {
                    true
                };

                if fire {
                    let note = if self.restart_pending {
                        self.restart_pending = false;
                        self.note_at_index(mode, octave_range)
                    } else {
                        self.advance(mode, octave_range)
                    };
                    self.current = Some(note);
                    ev.note_on = Some(note);
                }
                // On a rest step: note sequence doesn't advance, restart_pending stays set.
            }
        }

        ev
    }

    fn rebuild_sorted(&mut self) {
        self.held_sorted[..self.held_count].copy_from_slice(&self.held[..self.held_count]);
        self.held_sorted[..self.held_count].sort_unstable();
    }

    fn note_at_index(&mut self, mode: ArpMode, octave_range: usize) -> u8 {
        // Pattern modes always start at the first note (pattern_pos/base reset to 0)
        match mode {
            ArpMode::ThirdsWalk | ArpMode::Alberti | ArpMode::Pendulum => {
                return self.held_sorted[0];
            }
            _ => {}
        }
        let total = (self.held_count * octave_range).max(1);
        self.step_idx %= total;
        let octave = self.step_idx / self.held_count;
        let note_idx = self.step_idx % self.held_count;
        let base = match mode {
            ArpMode::AsPlayed => self.held[note_idx],
            _ => self.held_sorted[note_idx],
        };
        base.saturating_add(octave as u8 * 12)
    }

    fn advance(&mut self, mode: ArpMode, octave_range: usize) -> u8 {
        let n = self.held_count.max(1);
        let total = (n * octave_range).max(1);

        match mode {
            ArpMode::ThirdsWalk => {
                // Delta pattern [+2, −1] applied to an absolute index mod total.
                // Produces: 1,3,2,4,3,5,... across the note set.
                const DELTAS: [i32; 2] = [2, -1];
                let delta = DELTAS[self.pattern_pos % DELTAS.len()];
                self.pattern_pos += 1;
                self.pattern_base =
                    ((self.pattern_base as i32 + delta).rem_euclid(total as i32)) as usize;
                let octave = self.pattern_base / n;
                let note_idx = self.pattern_base % n;
                return self.held_sorted[note_idx].saturating_add(octave as u8 * 12);
            }
            ArpMode::Alberti => {
                // 4-step cycle: lowest, highest, middle, highest.
                self.pattern_pos += 1;
                let idx = match self.pattern_pos % 4 {
                    0 => 0,
                    1 => total - 1,
                    2 => total / 2,
                    _ => total - 1,
                };
                let octave = idx / n;
                let note_idx = idx % n;
                return self.held_sorted[note_idx].saturating_add(octave as u8 * 12);
            }
            ArpMode::Pendulum => {
                // Even steps: walk forward through indices.
                // Odd steps: always play the highest note (pivot).
                self.pattern_pos += 1;
                let idx = if self.pattern_pos % 2 == 0 {
                    (self.pattern_pos / 2) % total
                } else {
                    total - 1
                };
                let octave = idx / n;
                let note_idx = idx % n;
                return self.held_sorted[note_idx].saturating_add(octave as u8 * 12);
            }
            _ => {}
        }

        self.step_idx = match mode {
            ArpMode::Up | ArpMode::AsPlayed => (self.step_idx + 1) % total,
            ArpMode::Down => {
                if self.step_idx == 0 {
                    total - 1
                } else {
                    self.step_idx - 1
                }
            }
            ArpMode::UpDown => {
                if total <= 1 {
                    0
                } else {
                    let next = self.step_idx as i32 + self.direction as i32;
                    if next >= total as i32 - 1 {
                        self.direction = -1;
                        (total - 1).saturating_sub(1)
                    } else if next <= 0 {
                        self.direction = 1;
                        1.min(total - 1)
                    } else {
                        next as usize
                    }
                }
            }
            ArpMode::Random => self.rng.next_usize(total),
            _ => self.step_idx, // unreachable — pattern modes returned above
        };

        self.note_at_index(mode, octave_range)
    }
}

impl Default for ArpState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ScaleWalkerShared — UI-accessible config
// ---------------------------------------------------------------------------

pub struct ScaleWalkerShared {
    pub enabled: Arc<AtomicBool>,
    pub scale: Arc<AtomicU8>,        // Scale as u8
    pub root: Arc<AtomicU8>,         // MIDI root note (0-127; typical: 48-72)
    pub octave_range: Arc<AtomicU8>, // 1-3
    pub division: Arc<AtomicU8>,     // ClockDiv as u8
    pub gate: Shared,                // 0.05-1.0
    pub bpm: Shared,                 // 20-300
}

impl ScaleWalkerShared {
    pub fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            scale: Arc::new(AtomicU8::new(Scale::Major as u8)),
            root: Arc::new(AtomicU8::new(60)), // middle C
            octave_range: Arc::new(AtomicU8::new(2)),
            division: Arc::new(AtomicU8::new(ClockDiv::Eighth as u8)),
            gate: shared(0.6),
            bpm: shared(120.0),
        }
    }
}

impl Default for ScaleWalkerShared {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ScaleWalker — autonomous random walk within a scale, audio callback only
// ---------------------------------------------------------------------------

pub struct ScaleWalker {
    scale_notes: [u8; 128],
    scale_count: usize,
    current_idx: usize,
    current: Option<u8>,
    phase: f32,
    gate_fired: bool,
    rng: Lcg,
    prev_enabled: bool,
    restart_pending: bool,
    // cached to detect config changes
    prev_scale: u8,
    prev_root: u8,
    prev_oct: u8,
}

impl ScaleWalker {
    pub fn new() -> Self {
        let mut w = Self {
            scale_notes: [0; 128],
            scale_count: 0,
            current_idx: 0,
            current: None,
            phase: 0.0,
            gate_fired: false,
            rng: Lcg::new(0xFEED_FACE_CAFE_BABE),
            prev_enabled: false,
            restart_pending: false,
            prev_scale: 255,
            prev_root: 255,
            prev_oct: 255,
        };
        w.rebuild(Scale::Major as u8, 60, 2);
        w
    }

    /// Advance the walker by `frames` samples. Call once per audio buffer.
    pub fn tick(&mut self, cfg: &ScaleWalkerShared, frames: usize, sr: f64) -> ArpEvents {
        let enabled = cfg.enabled.load(Ordering::Relaxed);

        // Rebuild scale notes if config changed
        let scale = cfg.scale.load(Ordering::Relaxed);
        let root = cfg.root.load(Ordering::Relaxed);
        let oct = cfg.octave_range.load(Ordering::Relaxed).clamp(1, 3);
        if scale != self.prev_scale || root != self.prev_root || oct != self.prev_oct {
            self.rebuild(scale, root, oct);
            self.prev_scale = scale;
            self.prev_root = root;
            self.prev_oct = oct;
        }

        // Transition: just disabled -> fire NoteOff and reset walker state.
        if !enabled && self.prev_enabled {
            let off = self.current.take();
            self.phase = 0.0;
            self.gate_fired = false;
            self.current_idx = 0;
            self.restart_pending = false;
            self.prev_enabled = false;
            return ArpEvents {
                note_on: None,
                note_off: off,
            };
        }

        // Transition: just enabled -> restart from clean step boundary.
        if enabled && !self.prev_enabled {
            self.phase = 1.0; // force immediate first step note_on on this tick
            self.gate_fired = false;
            self.current_idx = 0;
            self.restart_pending = true;
        }
        self.prev_enabled = enabled;

        if !enabled || self.scale_count == 0 {
            return ArpEvents::default();
        }

        let bpm = cfg.bpm.value().max(1.0) as f64;
        let division = ClockDiv::from_u8(cfg.division.load(Ordering::Relaxed));
        let gate = cfg.gate.value().clamp(0.05, 0.99);

        let step_secs = 60.0 / bpm * division.beats_per_step() as f64;
        let delta = frames as f32 / (step_secs * sr) as f32;

        self.phase += delta;

        let mut ev = ArpEvents::default();

        if !self.gate_fired && self.phase >= gate {
            if let Some(note) = self.current.take() {
                ev.note_off = Some(note);
            }
            self.gate_fired = true;
        }

        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.gate_fired = false;

            if let Some(note) = self.current.take() {
                ev.note_off = Some(note);
            }

            let note = if self.restart_pending {
                self.restart_pending = false;
                self.scale_notes[self.current_idx]
            } else {
                self.walk_step()
            };
            self.current = Some(note);
            ev.note_on = Some(note);
        }

        ev
    }

    /// Restart walker sequencing at the first index.
    /// Returns a note_off for the currently sounding note (if any).
    pub fn restart(&mut self) -> Option<u8> {
        let off = self.current.take();
        self.phase = 1.0; // force immediate retrigger on next tick
        self.gate_fired = false;
        self.current_idx = 0;
        self.restart_pending = true;
        off
    }

    fn rebuild(&mut self, scale: u8, root: u8, octave_range: u8) {
        self.scale_count = 0;
        let intervals = Scale::from_u8(scale).intervals();
        for oct in 0..octave_range {
            for &interval in intervals {
                let note = root.saturating_add(oct * 12).saturating_add(interval);
                if note < 128 && self.scale_count < 128 {
                    self.scale_notes[self.scale_count] = note;
                    self.scale_count += 1;
                }
            }
        }
        // Reset index so we don't start out-of-bounds
        if self.scale_count > 0 {
            self.current_idx %= self.scale_count;
        }
    }

    fn walk_step(&mut self) -> u8 {
        if self.scale_count == 0 {
            return 60;
        }
        // Random walk: step ±1 or ±2, wrap around the scale
        let r = self.rng.next_u32() % 6;
        let delta: i32 = match r {
            0 => -2,
            1 | 2 => -1,
            3 | 4 => 1,
            _ => 2,
        };
        let new_idx =
            ((self.current_idx as i32 + delta).rem_euclid(self.scale_count as i32)) as usize;
        self.current_idx = new_idx;
        self.scale_notes[new_idx]
    }
}

impl Default for ScaleWalker {
    fn default() -> Self {
        Self::new()
    }
}
