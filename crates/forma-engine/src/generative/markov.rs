//! Markov Music System — Phase 8.3
//!
//! See `doc/markov-music-system.md` for full design rationale.
//!
//! # Module structure
//! - `Lcg`            — RT-safe RNG, no heap, no std rand
//! - `Scale`          — scale intervals, degree→pitch resolution
//! - `HarmonicChain`  — global 7-state chord-function chain
//! - `RhythmicChain`  — per-voice 5-state rhythmic pattern chain
//! - `MelodicChain`   — per-voice 7-state scale-degree chain
//! - `VoiceRole`      — Bass/Pad/Melody/Texture constraints
//! - `MoodSet`        — named triple of matrices + blend helpers
//! - `PhraseCounter`  — bar counter, phrase boundary events
//! - `MarkovVoice`    — combines rhythmic + melodic chain for one voice
//! - `MarkovEngine`   — N voices + global harmonic + phrase counter
//!
//! # RT safety
//! All shared config uses `Arc<AtomicXxx>` or `fundsp::Shared` (atomic f32).
//! Mutable state (`MarkovVoice`, `HarmonicChain`, etc.) lives on the audio thread only.
//! `MarkovEngineShared` is `Clone + Send` and can be held by the UI / Bevy thread.

use fundsp::prelude32::{shared, Shared};
use std::sync::{
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    Arc,
};

// ---------------------------------------------------------------------------
// LCG — identical to forma-engine's private copy; duplicated to avoid coupling
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

    /// Sample a row of a probability table. `row` must be length N and sum to ~1.0.
    /// Returns the chosen index.
    fn sample_row(&mut self, row: &[f32]) -> usize {
        let r = (self.next_u32() as f64 / u32::MAX as f64) as f32;
        let mut acc = 0.0f32;
        for (i, &p) in row.iter().enumerate() {
            acc += p;
            if r < acc {
                return i;
            }
        }
        row.len().saturating_sub(1)
    }
}

// ---------------------------------------------------------------------------
// Scale — degree → semitone offset, pitch resolution
// ---------------------------------------------------------------------------

/// Musical scales, tonality-agnostic (intervals only).
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scale {
    #[default]
    Major = 0,
    Minor = 1, // natural minor
    Dorian = 2,
    Phrygian = 3,
    Lydian = 4,
    Mixolydian = 5,
    HarmonicMinor = 6,
}

impl Scale {
    /// Semitone offsets for degrees 0–6 (scale degrees 1–7).
    pub fn intervals(self) -> &'static [u8; 7] {
        match self {
            Self::Major => &[0, 2, 4, 5, 7, 9, 11],
            Self::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Self::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Self::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Self::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Self::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Self::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
        }
    }

    pub const ALL: &'static [Self] = &[
        Self::Major,
        Self::Minor,
        Self::Dorian,
        Self::Phrygian,
        Self::Lydian,
        Self::Mixolydian,
        Self::HarmonicMinor,
    ];
    pub const LABELS: &'static [&'static str] = &[
        "Major",
        "Minor",
        "Dorian",
        "Phrygian",
        "Lydian",
        "Mixolyd.",
        "Harm.Minor",
    ];

    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Minor,
            2 => Self::Dorian,
            3 => Self::Phrygian,
            4 => Self::Lydian,
            5 => Self::Mixolydian,
            6 => Self::HarmonicMinor,
            _ => Self::Major,
        }
    }

    /// Resolve a scale degree (0-based, 0=tonic) to a MIDI pitch.
    /// `root` is the MIDI pitch of the tonic. `octave_offset` shifts register.
    pub fn degree_to_midi(self, root: u8, degree: usize, octave_offset: i8) -> u8 {
        let semitone = self.intervals()[degree % 7];
        let extra_octave = (degree / 7) as i8;
        let raw = root as i32 + semitone as i32 + (octave_offset + extra_octave) as i32 * 12;
        raw.clamp(0, 127) as u8
    }
}

// ---------------------------------------------------------------------------
// Harmonic function — chord roles (7 states)
// ---------------------------------------------------------------------------

/// Harmonic function: roman numeral chord role, scale-relative.
/// The mapping to actual chord tones depends on the current Scale.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HarmonicFunction {
    #[default]
    Tonic = 0, // I / i
    Supertonic = 1,  // II / ii
    Mediant = 2,     // III / iii / bIII
    Subdominant = 3, // IV / iv
    Dominant = 4,    // V / V7
    Submediant = 5,  // VI / vi / bVI
    LeadingTone = 6, // VII / vii / bVII
}

impl HarmonicFunction {
    pub const N: usize = 7;

    /// Chord tones (scale degrees, 0-based) for this function in the given scale.
    /// Returns root, third, fifth of the chord.
    pub fn chord_degrees(self) -> [usize; 3] {
        let root = self as usize;
        [root, (root + 2) % 7, (root + 4) % 7]
    }

    pub fn from_usize(v: usize) -> Self {
        match v {
            1 => Self::Supertonic,
            2 => Self::Mediant,
            3 => Self::Subdominant,
            4 => Self::Dominant,
            5 => Self::Submediant,
            6 => Self::LeadingTone,
            _ => Self::Tonic,
        }
    }

    pub const LABELS: &'static [&'static str] = &["I", "ii", "iii", "IV", "V", "vi", "vii"];
}

// ---------------------------------------------------------------------------
// Transition matrix helpers
// ---------------------------------------------------------------------------

pub const HARMONIC_STATES: usize = HarmonicFunction::N; // 7
pub const RHYTHMIC_STATES: usize = 5;
pub const MELODIC_STATES: usize = 7; // scale degrees 1–7

pub type HarmonicMatrix = [[f32; HARMONIC_STATES]; HARMONIC_STATES];
pub type RhythmicMatrix = [[f32; RHYTHMIC_STATES]; RHYTHMIC_STATES];
pub type MelodicMatrix = [[f32; MELODIC_STATES]; MELODIC_STATES];

/// Blend two matrices element-wise: `a * (1-t) + b * t`.
/// Blend two matrices: used by the training phase (8.4) to interpolate learned matrices.
#[allow(dead_code)]
pub fn blend_harmonic(a: &HarmonicMatrix, b: &HarmonicMatrix, t: f32) -> HarmonicMatrix {
    let mut out = [[0.0f32; HARMONIC_STATES]; HARMONIC_STATES];
    for i in 0..HARMONIC_STATES {
        for j in 0..HARMONIC_STATES {
            out[i][j] = a[i][j] * (1.0 - t) + b[i][j] * t;
        }
    }
    out
}

#[allow(dead_code)]
pub fn blend_rhythmic(a: &RhythmicMatrix, b: &RhythmicMatrix, t: f32) -> RhythmicMatrix {
    let mut out = [[0.0f32; RHYTHMIC_STATES]; RHYTHMIC_STATES];
    for i in 0..RHYTHMIC_STATES {
        for j in 0..RHYTHMIC_STATES {
            out[i][j] = a[i][j] * (1.0 - t) + b[i][j] * t;
        }
    }
    out
}

#[allow(dead_code)]
pub fn blend_melodic(a: &MelodicMatrix, b: &MelodicMatrix, t: f32) -> MelodicMatrix {
    let mut out = [[0.0f32; MELODIC_STATES]; MELODIC_STATES];
    for i in 0..MELODIC_STATES {
        for j in 0..MELODIC_STATES {
            out[i][j] = a[i][j] * (1.0 - t) + b[i][j] * t;
        }
    }
    out
}

/// Apply a bias vector to a matrix row and renormalize.
/// `bias[j]` is a positive multiplier for column j. Zero = forbidden.
fn apply_bias_and_normalize(
    row: &[f32; MELODIC_STATES],
    bias: &[f32; MELODIC_STATES],
) -> [f32; MELODIC_STATES] {
    let mut out = [0.0f32; MELODIC_STATES];
    let mut total = 0.0f32;
    for j in 0..MELODIC_STATES {
        out[j] = (row[j] * bias[j]).max(0.0);
        total += out[j];
    }
    if total > 0.0 {
        for j in 0..MELODIC_STATES {
            out[j] /= total;
        }
    } else {
        // fallback: uniform
        for j in 0..MELODIC_STATES {
            out[j] = 1.0 / MELODIC_STATES as f32;
        }
    }
    out
}

/// Apply density to a rhythmic row: scale down the Rest column, renormalize.
fn apply_density(row: &[f32; RHYTHMIC_STATES], density: f32) -> [f32; RHYTHMIC_STATES] {
    let mut out = *row;
    // density=0 → keep matrix as-is; density=1 → rest column → 0
    out[RhythmicState::Rest as usize] *= 1.0 - density.clamp(0.0, 1.0);
    let total: f32 = out.iter().sum();
    if total > 0.0 {
        for v in &mut out {
            *v /= total;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// RhythmicState
// ---------------------------------------------------------------------------

#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RhythmicState {
    #[default]
    Rest = 0,
    Hold = 1,
    Single = 2,
    Double = 3,
    Accent = 4,
}

impl RhythmicState {
    pub fn from_usize(v: usize) -> Self {
        match v {
            1 => Self::Hold,
            2 => Self::Single,
            3 => Self::Double,
            4 => Self::Accent,
            _ => Self::Rest,
        }
    }
    pub const LABELS: &'static [&'static str] = &["Rest", "Hold", "Single", "Double", "Accent"];
}

// ---------------------------------------------------------------------------
// MoodSet — named triple of matrices
// ---------------------------------------------------------------------------

/// A named mood: three matrices, a display label, and behavioural hints.
#[derive(Clone)]
pub struct MoodSet {
    pub name: &'static str,
    pub harmonic: HarmonicMatrix,
    pub rhythmic: RhythmicMatrix,
    pub melodic: MelodicMatrix,
    /// How long notes sustain relative to the step interval (0.0=1 subdiv, 1.0=16 subdivs).
    pub gate_length: f32,
    /// Rhythmic speed multiplier. 1.0 = use role's base divisor as-is.
    /// <1.0 = slower (e.g. 0.5 = half speed, every voice steps half as often).
    /// >1.0 = faster (e.g. 2.0 = double speed).
    /// Blended across moods, then applied to each role's rhythmic_divisor.
    pub rhythmic_speed: f32,
    /// Probability that the harmonic chain advances on each bar (0.0–1.0).
    /// 1.0 = always advance (current behaviour). 0.0 = chord never changes.
    /// At 0.3, the chord holds ~3 bars on average before changing.
    /// Blended across moods, then rolled per bar in on_bar().
    pub chord_change_prob: f32,
    /// Random variation applied to gate_length on each note-on (±variance).
    /// 0.0 = perfectly consistent gate (mechanical). High values = unpredictable articulation.
    /// Blended across moods, then sampled per note-on in MarkovEngine::on_subdivision.
    pub gate_length_variance: f32,
}

// ---------------------------------------------------------------------------
// Built-in moods
// ---------------------------------------------------------------------------

/// Calm — inspired by Satie (Gymnopédies), Debussy, Brian Eno (Music for Airports).
/// Harmonic signature: I↔IV plagal pendulum, V is rare and resolves immediately.
/// Tonic sits for long stretches (I self-loop 0.40). Harmony barely moves.
/// Melodic signature: Satie-like stepwise 1→2→3→2→1 pendulum, very narrow range.
/// Rhythmic signature: extremely sparse, Rest/Hold dominate, Double/Accent near-zero.
pub const MOOD_CALM: MoodSet = MoodSet {
    name: "Calm",
    gate_length: 0.85,          // long, breathing sustains
    rhythmic_speed: 0.7,        // slower pulse — Satie's unhurried tempo
    chord_change_prob: 0.4,     // chords linger — I↔IV pendulum is slow
    gate_length_variance: 0.05, // very consistent — meditative, predictable sustains
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.40, 0.05, 0.02, 0.35, 0.05, 0.10, 0.03], // I   — sits, then → IV
        [0.10, 0.10, 0.05, 0.15, 0.40, 0.15, 0.05], // ii  → V (when it appears)
        [0.10, 0.05, 0.05, 0.35, 0.10, 0.30, 0.05], // iii → IV or vi
        [0.55, 0.05, 0.03, 0.15, 0.10, 0.10, 0.02], // IV  → I (plagal resolution)
        [0.65, 0.05, 0.03, 0.10, 0.05, 0.10, 0.02], // V   → I (immediate resolve)
        [0.15, 0.10, 0.05, 0.40, 0.10, 0.15, 0.05], // vi  → IV
        [0.50, 0.05, 0.05, 0.15, 0.10, 0.10, 0.05], // vii → I
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.50, 0.20, 0.25, 0.03, 0.02], // Rest  — stays silent
        [0.10, 0.55, 0.30, 0.03, 0.02], // Hold  — long sustains
        [0.30, 0.25, 0.35, 0.06, 0.04], // Single
        [0.45, 0.15, 0.30, 0.07, 0.03], // Double → rest (transient)
        [0.45, 0.20, 0.28, 0.05, 0.02], // Accent → rest (transient)
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.25, 0.40, 0.15, 0.03, 0.12, 0.03, 0.02], // 1 → 2 (stepwise)
        [0.30, 0.15, 0.35, 0.10, 0.05, 0.03, 0.02], // 2 → 1 or 3
        [0.10, 0.35, 0.20, 0.20, 0.10, 0.03, 0.02], // 3 → 2 (stepwise back)
        [0.05, 0.10, 0.35, 0.15, 0.30, 0.03, 0.02], // 4 → 3 or 5
        [0.20, 0.05, 0.10, 0.15, 0.25, 0.15, 0.10], // 5
        [0.08, 0.05, 0.05, 0.05, 0.35, 0.20, 0.22], // 6 → 5
        [0.55, 0.05, 0.05, 0.03, 0.12, 0.10, 0.10], // 7 → 1 (resolve)
    ],
};

/// Tense — inspired by Wagner (Tristan und Isolde), Herrmann (Vertigo), Penderecki.
/// Harmonic signature: V is a black hole that never resolves. ii→V→V→vii→V loops.
/// I has minimal self-loop (stability impossible). The "Tristan" dominant hangs.
/// Melodic signature: tritone-prone (4↔7), avoids resolution to 1, wide leaps.
/// Rhythmic signature: bursts — Rest explodes into Double/Accent, then collapses.
pub const MOOD_TENSE: MoodSet = MoodSet {
    name: "Tense",
    gate_length: 0.20,          // short, stabby, agitated
    rhythmic_speed: 1.5,        // frantic pace — Herrmann's relentless tension
    chord_change_prob: 0.9,     // harmony shifts restlessly — unresolved Wagner chromaticism
    gate_length_variance: 0.30, // wildly unpredictable — the uncertainty IS the tension
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.05, 0.15, 0.05, 0.10, 0.40, 0.10, 0.15], // I   — destabilizes to V/vii
        [0.03, 0.08, 0.05, 0.10, 0.55, 0.10, 0.09], // ii  → V (dominant pull)
        [0.05, 0.12, 0.05, 0.15, 0.35, 0.15, 0.13], // iii → V
        [0.08, 0.10, 0.05, 0.05, 0.45, 0.12, 0.15], // IV  → V
        [0.15, 0.12, 0.05, 0.08, 0.30, 0.15, 0.15], // V   — hangs (self-loop 0.30)
        [0.05, 0.25, 0.08, 0.12, 0.30, 0.08, 0.12], // vi  → ii or V
        [0.12, 0.15, 0.08, 0.10, 0.30, 0.10, 0.15], // vii — circles with V
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.15, 0.03, 0.30, 0.32, 0.20], // Rest  → burst (Double/Accent)
        [0.15, 0.10, 0.35, 0.25, 0.15], // Hold  → breaks into attack
        [0.12, 0.05, 0.30, 0.33, 0.20], // Single → Double/Accent
        [0.25, 0.03, 0.30, 0.27, 0.15], // Double → Rest (collapse)
        [0.30, 0.03, 0.30, 0.22, 0.15], // Accent → Rest (collapse)
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.05, 0.08, 0.12, 0.20, 0.25, 0.15, 0.15], // 1 → 4/5/7 (leaps out)
        [0.08, 0.05, 0.12, 0.20, 0.15, 0.25, 0.15], // 2 → 4/6 (avoid tonic)
        [0.10, 0.08, 0.05, 0.12, 0.25, 0.15, 0.25], // 3 → 5/7 (tritone)
        [0.08, 0.10, 0.15, 0.05, 0.30, 0.12, 0.20], // 4 → 5/7 (tritone pair)
        [0.10, 0.05, 0.10, 0.25, 0.10, 0.20, 0.20], // 5 → 4/6/7 (destabilize)
        [0.08, 0.05, 0.08, 0.15, 0.25, 0.10, 0.29], // 6 → 7 (leading to nowhere)
        [0.12, 0.08, 0.10, 0.20, 0.25, 0.15, 0.10], // 7 → 4/5 (avoids 1!)
    ],
};

/// Dark — inspired by Radiohead (Exit Music), Pink Floyd (Breathe), Andalusian cadence.
/// Harmonic signature: vi and vii are home bases. Aeolian cadence I→vii→vi→V→vi.
/// V→vi deceptive cadence (0.40) denies resolution. bVI/bVII modal character.
/// Melodic signature: descending, minor 3rd emphasis, b6/b7 prominent, 7→6→5 chains.
/// Rhythmic signature: sparse with sudden stabs — silence, then violent Accent, then silence.
pub const MOOD_DARK: MoodSet = MoodSet {
    name: "Dark",
    gate_length: 0.90,          // heavy, drone-like sustains when notes appear
    rhythmic_speed: 0.6,        // slow, brooding — Radiohead's sparse pacing
    chord_change_prob: 0.35,    // chords hang in darkness — Aeolian cadence is unhurried
    gate_length_variance: 0.20, // moderate variance — sparse notes with unpredictable decay
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.10, 0.03, 0.08, 0.12, 0.10, 0.35, 0.22], // I   → vi/vii (darkens)
        [0.05, 0.08, 0.05, 0.12, 0.30, 0.30, 0.10], // ii  → V or vi
        [0.08, 0.05, 0.05, 0.25, 0.10, 0.30, 0.17], // iii → vi
        [0.15, 0.05, 0.05, 0.10, 0.15, 0.25, 0.25], // IV  → vi/vii
        [0.15, 0.05, 0.03, 0.07, 0.10, 0.40, 0.20], // V   → vi (deceptive!)
        [0.10, 0.05, 0.08, 0.25, 0.12, 0.12, 0.28], // vi  → IV/vii (Andalusian)
        [0.30, 0.05, 0.05, 0.10, 0.12, 0.28, 0.10], // vii → I or vi
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.50, 0.18, 0.15, 0.03, 0.14], // Rest  — silence, then sudden stab
        [0.12, 0.55, 0.18, 0.03, 0.12], // Hold  — long drone sustains
        [0.35, 0.15, 0.25, 0.08, 0.17], // Single → often back to Rest
        [0.40, 0.10, 0.25, 0.10, 0.15], // Double → Rest (collapses)
        [0.55, 0.10, 0.20, 0.05, 0.10], // Accent → Rest (jump-scare dies)
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.15, 0.10, 0.20, 0.05, 0.12, 0.15, 0.23], // 1 → 3/7 (descending feel)
        [0.15, 0.10, 0.30, 0.12, 0.08, 0.15, 0.10], // 2 → 3 (minor 3rd)
        [0.18, 0.15, 0.15, 0.22, 0.12, 0.10, 0.08], // 3 → 4 (descending)
        [0.05, 0.08, 0.30, 0.12, 0.25, 0.12, 0.08], // 4 → 3/5
        [0.12, 0.05, 0.10, 0.15, 0.15, 0.25, 0.18], // 5 → 6 (descending)
        [0.10, 0.05, 0.12, 0.08, 0.25, 0.15, 0.25], // 6 → 5/7 (b6 oscillation)
        [0.25, 0.05, 0.08, 0.05, 0.10, 0.30, 0.17], // 7 → 6 (descends, not resolves)
    ],
};

/// Euphoric — inspired by Pachelbel (Canon), Sigur Rós (Hoppípolla), EDM/trance builds.
/// Harmonic signature: I→V→vi→IV cycle (Pachelbel/pop). IV→I plagal lift (0.55).
/// V→I fast joyful resolution (0.55). Low self-loops — harmony always moves forward.
/// Melodic signature: strongly ascending 1→3→5→6→7→1. Pentatonic feel.
/// Rhythmic signature: steady energetic pulse, Single dominates, Rest is rare.
pub const MOOD_EUPHORIC: MoodSet = MoodSet {
    name: "Euphoric",
    gate_length: 0.30,          // short, bright, bouncy
    rhythmic_speed: 1.8,        // fast energetic pulse — EDM build energy
    chord_change_prob: 1.0,     // harmony always moves forward — Pachelbel never stops
    gate_length_variance: 0.10, // slight bounce — energetic but not chaotic
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.08, 0.05, 0.05, 0.25, 0.40, 0.12, 0.05], // I   → V/IV (moves forward)
        [0.05, 0.05, 0.05, 0.15, 0.50, 0.15, 0.05], // ii  → V
        [0.08, 0.05, 0.05, 0.45, 0.15, 0.15, 0.07], // iii → IV (Pachelbel)
        [0.55, 0.03, 0.03, 0.05, 0.25, 0.05, 0.04], // IV  → I (plagal lift!)
        [0.55, 0.03, 0.03, 0.05, 0.05, 0.25, 0.04], // V   → I or vi
        [0.08, 0.05, 0.15, 0.45, 0.12, 0.08, 0.07], // vi  → IV (the pop cycle)
        [0.50, 0.05, 0.05, 0.15, 0.15, 0.05, 0.05], // vii → I
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.05, 0.03, 0.50, 0.27, 0.15], // Rest  → Single (immediate pulse)
        [0.05, 0.08, 0.50, 0.25, 0.12], // Hold  → Single
        [0.08, 0.05, 0.40, 0.30, 0.17], // Single → Single/Double (steady)
        [0.10, 0.05, 0.40, 0.28, 0.17], // Double
        [0.10, 0.05, 0.45, 0.25, 0.15], // Accent
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.10, 0.25, 0.25, 0.05, 0.25, 0.05, 0.05], // 1 → 2/3/5 (ascend)
        [0.10, 0.08, 0.40, 0.10, 0.20, 0.07, 0.05], // 2 → 3 (ascending)
        [0.08, 0.08, 0.10, 0.10, 0.45, 0.12, 0.07], // 3 → 5 (leap up)
        [0.05, 0.05, 0.15, 0.05, 0.50, 0.12, 0.08], // 4 → 5
        [0.10, 0.03, 0.08, 0.05, 0.12, 0.42, 0.20], // 5 → 6 (ascending)
        [0.08, 0.03, 0.05, 0.03, 0.10, 0.12, 0.59], // 6 → 7 (climbing)
        [0.60, 0.05, 0.08, 0.03, 0.12, 0.07, 0.05], // 7 → 1 (triumphant resolve)
    ],
};

/// Cosmic — inspired by Zimmer (Interstellar organ), Vangelis (Blade Runner), Tangerine Dream.
/// Harmonic signature: I↔IV plagal oscillation, harmonic time nearly stops.
/// I self-loop 0.45 (just sits). V almost never appears. vi is rare Vangelis color.
/// Melodic signature: near-static drone. 1→1 self-loop, 1↔5 organ 5th oscillation.
/// Rhythmic signature: extremely sparse — a note event is a rare cosmic occurrence.
pub const MOOD_COSMIC: MoodSet = MoodSet {
    name: "Cosmic",
    gate_length: 0.95,          // near-infinite sustain, drone-like
    rhythmic_speed: 0.4,        // glacial — Vangelis/Tangerine Dream timelessness
    chord_change_prob: 0.15,    // chords barely change — Interstellar organ sits for minutes
    gate_length_variance: 0.03, // near-zero — drone-like, unchanging sustain
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.45, 0.02, 0.03, 0.35, 0.02, 0.10, 0.03], // I   — sits, then → IV
        [0.15, 0.10, 0.05, 0.30, 0.10, 0.25, 0.05], // ii  → IV/vi
        [0.10, 0.05, 0.10, 0.30, 0.05, 0.35, 0.05], // iii → IV/vi
        [0.50, 0.03, 0.03, 0.20, 0.04, 0.15, 0.05], // IV  → I (plagal return)
        [0.40, 0.05, 0.05, 0.30, 0.05, 0.10, 0.05], // V   → I/IV (V is lost here)
        [0.20, 0.05, 0.08, 0.40, 0.05, 0.15, 0.07], // vi  → IV
        [0.35, 0.05, 0.05, 0.25, 0.10, 0.15, 0.05], // vii → I
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.60, 0.25, 0.12, 0.02, 0.01], // Rest  — vast silence
        [0.05, 0.70, 0.20, 0.03, 0.02], // Hold  — infinite sustain
        [0.40, 0.25, 0.25, 0.06, 0.04], // Single → back to Rest/Hold
        [0.50, 0.15, 0.25, 0.07, 0.03], // Double → Rest (transient)
        [0.50, 0.20, 0.22, 0.05, 0.03], // Accent → Rest (transient)
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.40, 0.20, 0.08, 0.05, 0.20, 0.04, 0.03], // 1 → 1/2/5 (drone + 5th)
        [0.30, 0.20, 0.22, 0.10, 0.10, 0.05, 0.03], // 2 → 1 (return to drone)
        [0.12, 0.28, 0.22, 0.18, 0.12, 0.05, 0.03], // 3 → 2 (stepwise)
        [0.08, 0.10, 0.25, 0.22, 0.25, 0.07, 0.03], // 4 → 3/5
        [0.30, 0.05, 0.08, 0.12, 0.30, 0.10, 0.05], // 5 → 1/5 (organ 5th)
        [0.15, 0.05, 0.05, 0.08, 0.30, 0.22, 0.15], // 6 → 5
        [0.40, 0.05, 0.05, 0.05, 0.18, 0.15, 0.12], // 7 → 1 (resolve)
    ],
};

/// Gravity — inspired by Philip Glass (Koyaanisqatsi), Zimmer (Interstellar docking),
/// Michael Nyman (The Piano). Minimalist ostinato, relentless repetition.
/// Harmonic signature: tight I→V→vi→IV→I loop. V→vi deceptive cadence (0.55)
/// keeps deflecting resolution. Low self-loops — always moving, but in circles.
/// Melodic signature: 1↔5↔3 ostinato pendulum, Glass-like arpeggiation.
/// Rhythmic signature: mechanical Single pulse, clock-like, the machine never stops.
pub const MOOD_GRAVITY: MoodSet = MoodSet {
    name: "Gravity",
    gate_length: 0.55,          // medium — deliberate, mechanical
    rhythmic_speed: 1.4,        // mechanical clock-pulse — Glass's relentless arpeggiation
    chord_change_prob: 0.85,    // tight chord cycles — minimalist loops advance steadily
    gate_length_variance: 0.04, // very consistent — mechanical precision, Glass-like
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.08, 0.05, 0.03, 0.12, 0.50, 0.15, 0.07], // I   → V (pushes forward)
        [0.05, 0.05, 0.03, 0.10, 0.50, 0.20, 0.07], // ii  → V
        [0.08, 0.05, 0.05, 0.30, 0.20, 0.25, 0.07], // iii → IV/vi
        [0.45, 0.05, 0.03, 0.05, 0.20, 0.17, 0.05], // IV  → I (return)
        [0.15, 0.05, 0.03, 0.05, 0.08, 0.55, 0.09], // V   → vi (deceptive!)
        [0.08, 0.08, 0.05, 0.50, 0.15, 0.08, 0.06], // vi  → IV (the cycle)
        [0.20, 0.05, 0.05, 0.15, 0.35, 0.15, 0.05], // vii → V
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.08, 0.10, 0.55, 0.15, 0.12], // Rest  → Single (machine starts)
        [0.08, 0.15, 0.50, 0.15, 0.12], // Hold  → Single
        [0.10, 0.12, 0.45, 0.20, 0.13], // Single → Single (steady pulse)
        [0.12, 0.08, 0.45, 0.22, 0.13], // Double
        [0.15, 0.08, 0.45, 0.20, 0.12], // Accent
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.15, 0.12, 0.25, 0.05, 0.35, 0.05, 0.03], // 1 → 3/5 (ostinato)
        [0.25, 0.10, 0.30, 0.10, 0.15, 0.07, 0.03], // 2 → 1/3
        [0.15, 0.15, 0.12, 0.15, 0.30, 0.08, 0.05], // 3 → 5 (arpeggio up)
        [0.08, 0.10, 0.25, 0.10, 0.35, 0.08, 0.04], // 4 → 3/5
        [0.35, 0.08, 0.15, 0.10, 0.15, 0.10, 0.07], // 5 → 1 (arpeggio down)
        [0.15, 0.05, 0.10, 0.08, 0.30, 0.15, 0.17], // 6 → 5
        [0.40, 0.08, 0.10, 0.05, 0.20, 0.10, 0.07], // 7 → 1 (resolve)
    ],
};

pub const ALL_MOODS: &[&MoodSet] = &[
    &MOOD_CALM,
    &MOOD_TENSE,
    &MOOD_DARK,
    &MOOD_EUPHORIC,
    &MOOD_COSMIC,
    &MOOD_GRAVITY,
];
pub const N_MOODS: usize = 6;

// ---------------------------------------------------------------------------
// Phrase-boundary harmonic matrix (wider jumps allowed)
// ---------------------------------------------------------------------------

pub const PHRASE_BOUNDARY_HARMONIC: HarmonicMatrix = [
    //  I      ii     iii    IV     V      vi     vii
    [0.10, 0.10, 0.10, 0.15, 0.15, 0.25, 0.15], // I   → vi (relative shift)
    [0.10, 0.05, 0.10, 0.15, 0.25, 0.25, 0.10], // ii
    [0.10, 0.10, 0.05, 0.20, 0.15, 0.25, 0.15], // iii
    [0.15, 0.10, 0.10, 0.05, 0.20, 0.25, 0.15], // IV
    [0.25, 0.10, 0.10, 0.15, 0.05, 0.25, 0.10], // V
    [0.25, 0.15, 0.10, 0.15, 0.15, 0.10, 0.10], // vi → I (relative shift)
    [0.20, 0.10, 0.10, 0.15, 0.20, 0.15, 0.10], // vii
];

// ---------------------------------------------------------------------------
// VoiceRole
// ---------------------------------------------------------------------------

/// Role of a voice in the ensemble. Determines register and melodic degree bias.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VoiceRole {
    #[default]
    Bass = 0,
    Pad = 1,
    Melody = 2,
    Texture = 3,
    /// Rhythmic pulse voice — fast, root-locked, designed for percussive/sequencer patches.
    /// Use with short-envelope patches (saw stabs, FM hits, noise bursts) to add
    /// rhythmic drive to a scene without breaking the ambient harmonic framework.
    Pulse = 4,
}

impl VoiceRole {
    /// MIDI note range [low, high] for this role.
    pub fn register(self) -> (u8, u8) {
        match self {
            Self::Bass => (24, 47),
            Self::Pad => (48, 71),
            Self::Melody => (60, 83),
            Self::Texture => (72, 95),
            Self::Pulse => (36, 60), // low-mid: rhythmic stabs and bass hits
        }
    }

    /// Multiplier applied to each chord tone after role bias, to pull notes
    /// toward the active harmony. Higher = more chord-locked.
    pub fn chord_attraction(self) -> f32 {
        match self {
            Self::Bass => 3.5,    // almost always root/fifth
            Self::Pad => 2.5,     // chord tones strongly preferred
            Self::Melody => 1.6,  // mild pull; passing tones still happen
            Self::Texture => 1.2, // mostly free, slight harmonic gravity
            Self::Pulse => 4.0,   // very chord-locked — rhythmic parts need harmonic clarity
        }
    }

    /// Per-degree bias multipliers (index = scale degree 0-based).
    /// 0.0 = forbidden, 1.0 = neutral, >1.0 = preferred.
    pub fn degree_bias(self) -> [f32; MELODIC_STATES] {
        match self {
            Self::Bass => [3.0, 0.8, 0.0, 0.0, 2.5, 0.0, 0.0],
            Self::Pad => [2.0, 0.5, 2.0, 0.3, 2.0, 0.5, 0.3],
            Self::Melody => [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            Self::Texture => [0.2, 1.5, 1.2, 0.5, 0.5, 2.0, 1.8],
            Self::Pulse => [4.0, 0.3, 0.0, 0.0, 3.0, 0.0, 0.3], // root + fifth only
        }
    }

    /// How many subdivisions between rhythmic steps (1 = every subdiv, 2 = every other).
    pub fn rhythmic_divisor(self) -> u32 {
        match self {
            Self::Bass => 2,
            Self::Pad => 2,
            Self::Melody => 1,
            Self::Texture => 4,
            Self::Pulse => 1, // fastest — mood rhythmic_speed controls actual rate
        }
    }

    pub const ALL: &'static [Self] = &[
        Self::Bass,
        Self::Pad,
        Self::Melody,
        Self::Texture,
        Self::Pulse,
    ];
    pub const LABELS: &'static [&'static str] = &["Bass", "Pad", "Melody", "Texture", "Pulse"];

    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Pad,
            2 => Self::Melody,
            3 => Self::Texture,
            4 => Self::Pulse,
            _ => Self::Bass,
        }
    }
}

// ---------------------------------------------------------------------------
// HarmonicChain — global, advances per bar / phrase boundary
// ---------------------------------------------------------------------------

/// Global harmonic chain state. Owned by the audio thread.
pub struct HarmonicChain {
    pub state: HarmonicFunction,
    rng: Lcg,
}

impl HarmonicChain {
    pub fn new(seed: u64) -> Self {
        Self {
            state: HarmonicFunction::Tonic,
            rng: Lcg::new(seed),
        }
    }

    /// Advance the chain using the blended mood matrix.
    /// Call once per bar (normal) or once per phrase boundary (use phrase matrix).
    pub fn advance(&mut self, matrix: &HarmonicMatrix) {
        let row = &matrix[self.state as usize];
        let next = self.rng.sample_row(row);
        self.state = HarmonicFunction::from_usize(next);
    }

    /// Chord tone scale degrees for the current state.
    pub fn chord_degrees(&self) -> [usize; 3] {
        self.state.chord_degrees()
    }
}

// ---------------------------------------------------------------------------
// RhythmicChain — per voice, advances per subdivision (divided by role)
// ---------------------------------------------------------------------------

/// Per-voice rhythmic chain state. Owned by the audio thread.
pub struct RhythmicChain {
    pub state: RhythmicState,
    rng: Lcg,
    subdiv_counter: u32,
}

impl RhythmicChain {
    pub fn new(seed: u64) -> Self {
        Self {
            state: RhythmicState::Rest,
            rng: Lcg::new(seed),
            subdiv_counter: 0,
        }
    }

    /// Call once per subdivision. Returns the new state only when the role's
    /// divisor threshold is reached (i.e. not every subdivision triggers a step).
    /// Returns `None` when this subdivision is skipped for this role.
    ///
    /// `rhythmic_speed`: mood-blended speed multiplier (1.0 = normal).
    /// >1.0 = faster (lower effective divisor), <1.0 = slower (higher effective divisor).
    pub fn on_subdivision(
        &mut self,
        matrix: &RhythmicMatrix,
        density: f32,
        role: VoiceRole,
        rhythmic_speed: f32,
    ) -> Option<RhythmicState> {
        self.subdiv_counter += 1;
        // Scale the role's base divisor by the inverse of rhythmic_speed.
        // speed=2.0 → effective_divisor = base/2 (steps twice as often).
        // speed=0.5 → effective_divisor = base*2 (steps half as often).
        let effective_divisor = (role.rhythmic_divisor() as f32 / rhythmic_speed.max(0.1))
            .round()
            .max(1.0) as u32;
        if self.subdiv_counter < effective_divisor {
            return None;
        }
        self.subdiv_counter = 0;

        let row = apply_density(&matrix[self.state as usize], density);
        let next = self.rng.sample_row(&row);
        self.state = RhythmicState::from_usize(next);
        Some(self.state)
    }
}

// ---------------------------------------------------------------------------
// MelodicChain — per voice, advances when rhythmic fires an attack
// ---------------------------------------------------------------------------

/// Per-voice melodic chain state. Owned by the audio thread.
pub struct MelodicChain {
    pub degree: usize, // 0-based scale degree (0=tonic)
    rng: Lcg,
}

impl MelodicChain {
    pub fn new(seed: u64, start_degree: usize) -> Self {
        Self {
            degree: start_degree % MELODIC_STATES,
            rng: Lcg::new(seed),
        }
    }

    /// Advance the melodic chain and resolve to a MIDI pitch.
    /// Call only when the rhythmic chain fires `Single`, `Double`, or `Accent`.
    ///
    /// `force_root`: if true, snap to the harmonic root degree (used for Bass downbeats).
    pub fn advance_and_resolve(
        &mut self,
        matrix: &MelodicMatrix,
        role: VoiceRole,
        harmonic: &HarmonicChain,
        root: u8,
        scale: Scale,
        octave_offset: i8,
        force_root: bool,
        chord_attraction: f32,
    ) -> u8 {
        if force_root {
            // Snap to the chord root degree (the harmonic function's own scale degree).
            self.degree = harmonic.state as usize;
        } else {
            // 1. Apply role degree bias
            let biased = apply_bias_and_normalize(&matrix[self.degree], &role.degree_bias());

            // 2. Apply chord-tone attraction scaled by global param and role weight.
            //    attraction=0 → no boost; attraction=1 → full role multiplier.
            let role_mult = role.chord_attraction(); // max multiplier for this role
            let effective = 1.0 + (role_mult - 1.0) * chord_attraction;
            let chord_degs = harmonic.chord_degrees();
            let mut attracted = biased;
            for &cd in &chord_degs {
                attracted[cd] *= effective;
            }
            // Renormalize after attraction boost
            let total: f32 = attracted.iter().sum();
            if total > 0.0 {
                for v in &mut attracted {
                    *v /= total;
                }
            }

            self.degree = self.rng.sample_row(&attracted);
        }

        // Resolve degree to MIDI, clamped to role register
        let raw = scale.degree_to_midi(root, self.degree, octave_offset);
        let (lo, hi) = role.register();

        // If out of register, shift by octaves
        let mut midi = raw;
        while midi < lo && midi + 12 <= hi {
            midi += 12;
        }
        while midi > hi && midi >= lo + 12 {
            midi -= 12;
        }

        midi.clamp(lo, hi)
    }
}

// ---------------------------------------------------------------------------
// PhraseCounter — global bar + phrase tracking
// ---------------------------------------------------------------------------

/// Tracks bar position and fires phrase boundary events.
pub struct PhraseCounter {
    pub bar: u64,
    pub bars_per_phrase: u32,
    bars_in_phrase: u32,
}

/// Events emitted by the phrase counter at bar boundaries.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhraseEvents {
    /// A new bar started.
    pub new_bar: bool,
    /// A phrase boundary was crossed — use wide harmonic matrix.
    pub phrase_boundary: bool,
}

impl PhraseCounter {
    pub fn new(bars_per_phrase: u32) -> Self {
        Self {
            bar: 0,
            bars_per_phrase: bars_per_phrase.max(1),
            bars_in_phrase: 0,
        }
    }

    /// Call when `BeatEvents::bar` fires.
    pub fn on_bar(&mut self) -> PhraseEvents {
        self.bar += 1;
        self.bars_in_phrase += 1;
        let phrase_boundary = self.bars_in_phrase >= self.bars_per_phrase;
        if phrase_boundary {
            self.bars_in_phrase = 0;
        }
        PhraseEvents {
            new_bar: true,
            phrase_boundary,
        }
    }

    pub fn reset(&mut self) {
        self.bar = 0;
        self.bars_in_phrase = 0;
    }
}

// ---------------------------------------------------------------------------
// MoodBlend — runtime-mutable mood interpolation weights
// ---------------------------------------------------------------------------

/// Thread-safe mood blend weights. One `Shared` per mood.
/// All weights are kept normalized (sum to 1.0) by the setter.
#[derive(Clone)]
pub struct MoodBlend {
    weights: [Shared; N_MOODS],
}

impl MoodBlend {
    pub fn new() -> Self {
        // Default: 100% Calm
        let weights = std::array::from_fn(|i| shared(if i == 0 { 1.0 } else { 0.0 }));
        Self { weights }
    }

    /// Set blend weights. Vector is normalized to sum=1.0.
    pub fn set(&self, w: &[f32; N_MOODS]) {
        let total: f32 = w.iter().sum();
        let norm = if total > 0.0 { total } else { 1.0 };
        for (i, s) in self.weights.iter().enumerate() {
            s.set_value(w[i] / norm);
        }
    }

    pub fn weight(&self, i: usize) -> f32 {
        self.weights[i].value()
    }

    /// Blend all mood matrices into a single active set.
    /// Called from audio thread on each chain transition.
    pub fn blend_harmonic(&self, moods: &[&MoodSet; N_MOODS]) -> HarmonicMatrix {
        let w0 = self.weight(0);
        let mut out = scale_harmonic(&moods[0].harmonic, w0);
        for i in 1..N_MOODS {
            let wi = self.weight(i);
            if wi > 0.0 {
                let contribution = scale_harmonic(&moods[i].harmonic, wi);
                for r in 0..HARMONIC_STATES {
                    for c in 0..HARMONIC_STATES {
                        out[r][c] += contribution[r][c];
                    }
                }
            }
        }
        out
    }

    pub fn blend_rhythmic(&self, moods: &[&MoodSet; N_MOODS]) -> RhythmicMatrix {
        let w0 = self.weight(0);
        let mut out = scale_rhythmic(&moods[0].rhythmic, w0);
        for i in 1..N_MOODS {
            let wi = self.weight(i);
            if wi > 0.0 {
                let contribution = scale_rhythmic(&moods[i].rhythmic, wi);
                for r in 0..RHYTHMIC_STATES {
                    for c in 0..RHYTHMIC_STATES {
                        out[r][c] += contribution[r][c];
                    }
                }
            }
        }
        out
    }

    /// Blend gate_length from mood weights. Returns value in [0.0, 1.0].
    pub fn blend_gate_length(&self, moods: &[&MoodSet; N_MOODS]) -> f32 {
        moods
            .iter()
            .enumerate()
            .map(|(i, m)| self.weight(i) * m.gate_length)
            .sum::<f32>()
            .clamp(0.0, 1.0)
    }

    /// Blend rhythmic_speed from mood weights. Returns multiplier (>0).
    pub fn blend_rhythmic_speed(&self, moods: &[&MoodSet; N_MOODS]) -> f32 {
        moods
            .iter()
            .enumerate()
            .map(|(i, m)| self.weight(i) * m.rhythmic_speed)
            .sum::<f32>()
            .max(0.1) // never fully stop
    }

    /// Blend chord_change_prob from mood weights. Returns probability in [0.0, 1.0].
    pub fn blend_chord_change_prob(&self, moods: &[&MoodSet; N_MOODS]) -> f32 {
        moods
            .iter()
            .enumerate()
            .map(|(i, m)| self.weight(i) * m.chord_change_prob)
            .sum::<f32>()
            .clamp(0.0, 1.0)
    }

    /// Blend gate_length_variance from mood weights. Returns value >= 0.
    pub fn blend_gate_length_variance(&self, moods: &[&MoodSet; N_MOODS]) -> f32 {
        moods
            .iter()
            .enumerate()
            .map(|(i, m)| self.weight(i) * m.gate_length_variance)
            .sum::<f32>()
            .max(0.0)
    }

    pub fn blend_melodic(&self, moods: &[&MoodSet; N_MOODS]) -> MelodicMatrix {
        let w0 = self.weight(0);
        let mut out = scale_melodic(&moods[0].melodic, w0);
        for i in 1..N_MOODS {
            let wi = self.weight(i);
            if wi > 0.0 {
                let contribution = scale_melodic(&moods[i].melodic, wi);
                for r in 0..MELODIC_STATES {
                    for c in 0..MELODIC_STATES {
                        out[r][c] += contribution[r][c];
                    }
                }
            }
        }
        out
    }
}

impl Default for MoodBlend {
    fn default() -> Self {
        Self::new()
    }
}

fn scale_harmonic(m: &HarmonicMatrix, w: f32) -> HarmonicMatrix {
    let mut out = [[0.0f32; HARMONIC_STATES]; HARMONIC_STATES];
    for r in 0..HARMONIC_STATES {
        for c in 0..HARMONIC_STATES {
            out[r][c] = m[r][c] * w;
        }
    }
    out
}
fn scale_rhythmic(m: &RhythmicMatrix, w: f32) -> RhythmicMatrix {
    let mut out = [[0.0f32; RHYTHMIC_STATES]; RHYTHMIC_STATES];
    for r in 0..RHYTHMIC_STATES {
        for c in 0..RHYTHMIC_STATES {
            out[r][c] = m[r][c] * w;
        }
    }
    out
}
fn scale_melodic(m: &MelodicMatrix, w: f32) -> MelodicMatrix {
    let mut out = [[0.0f32; MELODIC_STATES]; MELODIC_STATES];
    for r in 0..MELODIC_STATES {
        for c in 0..MELODIC_STATES {
            out[r][c] = m[r][c] * w;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// MarkovEngineShared — thread-safe config, Clone + Send
// ---------------------------------------------------------------------------

/// Number of step columns in the Launchpad display buffer.
pub const LAUNCHPAD_COLS: usize = 16;

/// Thread-safe runtime parameters shared between audio thread and UI/Bevy.
#[derive(Clone)]
pub struct MarkovEngineShared {
    /// MIDI root note (0-127).
    pub root: Arc<AtomicU8>,
    /// Scale (Scale enum as u8).
    pub scale: Arc<AtomicU8>,
    /// Mood blend weights.
    pub mood: MoodBlend,
    /// Global density (0.0-1.0). Per-voice density can add on top.
    pub density: Shared,
    /// Bars per phrase.
    pub bars_per_phrase: Arc<std::sync::atomic::AtomicU32>,
    /// Per-voice role (VoiceRole as u8).
    pub roles: Vec<Arc<AtomicU8>>,
    /// Per-voice density override (0.0 = use global, >0.0 = override).
    pub voice_density: Vec<Shared>,
    /// Per-voice enabled flag.
    pub voice_enabled: Vec<Arc<std::sync::atomic::AtomicBool>>,
    /// Per-voice octave offset (i8 stored as u8 with +64 bias).
    pub voice_octave: Vec<Arc<AtomicU8>>,
    /// Launchpad display buffer: [voice][col] = RhythmicState as u8.
    /// Written by audio thread on each subdivision; read-only for UI.
    pub launchpad: Arc<Vec<[AtomicU8; LAUNCHPAD_COLS]>>,
    /// Current write column in the launchpad ring buffer (0..LAUNCHPAD_COLS).
    pub launchpad_col: Arc<AtomicUsize>,

    // ── Harmonic behaviour controls ──────────────────────────────────────────
    /// Chord-tone attraction strength (0.0–1.0).
    pub chord_attraction: Shared,
    /// When true, Bass voices snap to the chord root on each bar downbeat.
    pub bass_lock: Arc<AtomicBool>,
    /// When true, dissonant intervals between simultaneous note-ons are nudged apart.
    pub dissonance_resolve: Arc<AtomicBool>,
    /// How aggressively dissonance is resolved.
    /// 0 = semitones only, 1 = semitones + tritones, 2 = semitones + tritones + minor 7ths.
    pub dissonance_threshold: Arc<AtomicU8>,
    /// Per-phrase register drift probability override (0.0–1.0).
    pub register_drift: Shared,

    // ── Motif lock (per-voice) ───────────────────────────────────────────────
    /// When true for voice i, the engine switches to motif-lock mode.
    /// Setting to true triggers the Capturing phase; false returns to Off.
    pub motif_lock: Vec<Arc<AtomicBool>>,
    /// Capture length in steps (4, 8, 16, or 32). Clamped to MOTIF_BUF_MAX.
    pub motif_length: Vec<Arc<AtomicU8>>,
    /// Read-only flag written by the audio thread: true when voice is in Replaying phase.
    pub motif_active: Vec<Arc<AtomicBool>>,

    // ── Clock division ───────────────────────────────────────────────────────
    /// How many BeatClock subdivisions pass between Markov steps.
    /// 1 = every 16th note, 2 = every 8th note, 4 = every beat, 8 = every 2 beats.
    pub clock_div: Arc<AtomicU8>,

    // ── Phrase epoch (for Timeline synchronization) ────────────────────────
    /// Monotonically increasing counter, incremented on each phrase boundary
    /// by the audio thread. The control thread polls this to drive the Timeline.
    pub phrase_epoch: Arc<AtomicUsize>,
    /// Incremented by the audio thread on each bar boundary.
    pub bar_epoch: Arc<AtomicUsize>,
    /// Incremented by the audio thread on each 16th-note subdivision.
    pub subdivision_epoch: Arc<AtomicUsize>,
    /// Set to true by the control thread to request an immediate timeline
    /// section advance (hard cut, ignores remaining phrase count).
    /// Cleared by the control thread after processing.
    pub force_timeline_advance: Arc<AtomicBool>,

    // ── Harmonic sequence ────────────────────────────────────────────────────
    /// Number of active chord slots in the sequence (1–8). 1 = static key (legacy).
    pub seq_len: Arc<AtomicU8>,
    /// Root MIDI note for each sequence slot (8 slots max).
    pub seq_roots: Vec<Arc<AtomicU8>>,
    /// Scale (Scale enum as u8) for each sequence slot.
    pub seq_scales: Vec<Arc<AtomicU8>>,
    /// Number of phrases each slot lasts before advancing (1–16).
    pub seq_phrases: Vec<Arc<AtomicU8>>,
}

impl MarkovEngineShared {
    pub const SEQ_MAX: usize = 8;

    pub fn new(n_voices: usize) -> Self {
        Self {
            root: Arc::new(AtomicU8::new(60)), // C4
            scale: Arc::new(AtomicU8::new(Scale::Minor as u8)),
            mood: MoodBlend::new(),
            density: shared(0.5),
            bars_per_phrase: Arc::new(std::sync::atomic::AtomicU32::new(4)),
            roles: (0..n_voices)
                .map(|i| {
                    Arc::new(AtomicU8::new(match i % 4 {
                        0 => VoiceRole::Bass as u8,
                        1 => VoiceRole::Pad as u8,
                        2 => VoiceRole::Melody as u8,
                        _ => VoiceRole::Texture as u8,
                    }))
                })
                .collect(),
            voice_density: (0..n_voices).map(|_| shared(0.0)).collect(),
            voice_enabled: (0..n_voices)
                .map(|_| Arc::new(AtomicBool::new(true)))
                .collect(),
            voice_octave: (0..n_voices).map(|_| Arc::new(AtomicU8::new(64))).collect(),
            launchpad: Arc::new(
                (0..n_voices)
                    .map(|_| std::array::from_fn(|_| AtomicU8::new(0)))
                    .collect(),
            ),
            launchpad_col: Arc::new(AtomicUsize::new(0)),
            chord_attraction: shared(0.5),
            bass_lock: Arc::new(AtomicBool::new(true)),
            dissonance_resolve: Arc::new(AtomicBool::new(true)),
            dissonance_threshold: Arc::new(AtomicU8::new(1)),
            register_drift: shared(0.2),
            motif_lock: (0..n_voices)
                .map(|_| Arc::new(AtomicBool::new(false)))
                .collect(),
            motif_length: (0..n_voices).map(|_| Arc::new(AtomicU8::new(16))).collect(),
            motif_active: (0..n_voices)
                .map(|_| Arc::new(AtomicBool::new(false)))
                .collect(),
            clock_div: Arc::new(AtomicU8::new(4)), // one step per beat by default
            phrase_epoch: Arc::new(AtomicUsize::new(0)),
            bar_epoch: Arc::new(AtomicUsize::new(0)),
            subdivision_epoch: Arc::new(AtomicUsize::new(0)),
            force_timeline_advance: Arc::new(AtomicBool::new(false)),
            seq_len: Arc::new(AtomicU8::new(1)),
            seq_roots: (0..Self::SEQ_MAX)
                .map(|_| Arc::new(AtomicU8::new(60)))
                .collect(),
            seq_scales: (0..Self::SEQ_MAX)
                .map(|_| Arc::new(AtomicU8::new(0)))
                .collect(),
            seq_phrases: (0..Self::SEQ_MAX)
                .map(|_| Arc::new(AtomicU8::new(4)))
                .collect(),
        }
    }

    pub fn root(&self) -> u8 {
        self.root.load(Ordering::Relaxed)
    }
    pub fn scale(&self) -> Scale {
        Scale::from_u8(self.scale.load(Ordering::Relaxed))
    }
    pub fn density(&self) -> f32 {
        self.density.value()
    }
    pub fn chord_attraction(&self) -> f32 {
        self.chord_attraction.value().clamp(0.0, 1.0)
    }
    pub fn bass_lock(&self) -> bool {
        self.bass_lock.load(Ordering::Relaxed)
    }
    pub fn dissonance_resolve(&self) -> bool {
        self.dissonance_resolve.load(Ordering::Relaxed)
    }
    pub fn dissonance_threshold(&self) -> u8 {
        self.dissonance_threshold.load(Ordering::Relaxed)
    }
    pub fn register_drift(&self) -> f32 {
        self.register_drift.value().clamp(0.0, 1.0)
    }

    pub fn role(&self, i: usize) -> VoiceRole {
        VoiceRole::from_u8(self.roles[i].load(Ordering::Relaxed))
    }

    pub fn voice_density(&self, i: usize) -> f32 {
        let vd = self.voice_density[i].value();
        if vd > 0.0 {
            vd
        } else {
            self.density()
        }
    }

    pub fn voice_enabled(&self, i: usize) -> bool {
        self.voice_enabled[i].load(Ordering::Relaxed)
    }

    pub fn octave_offset(&self, i: usize) -> i8 {
        self.voice_octave[i].load(Ordering::Relaxed) as i8 - 64
    }

    pub fn bars_per_phrase(&self) -> u32 {
        self.bars_per_phrase.load(Ordering::Relaxed).max(1)
    }

    pub fn clock_div(&self) -> u8 {
        self.clock_div.load(Ordering::Relaxed).max(1)
    }

    pub fn seq_len(&self) -> usize {
        (self.seq_len.load(Ordering::Relaxed) as usize).clamp(1, Self::SEQ_MAX)
    }

    pub fn seq_root(&self, slot: usize) -> u8 {
        self.seq_roots[slot.min(Self::SEQ_MAX - 1)].load(Ordering::Relaxed)
    }

    pub fn seq_scale(&self, slot: usize) -> Scale {
        Scale::from_u8(self.seq_scales[slot.min(Self::SEQ_MAX - 1)].load(Ordering::Relaxed))
    }

    pub fn seq_phrases(&self, slot: usize) -> u8 {
        self.seq_phrases[slot.min(Self::SEQ_MAX - 1)]
            .load(Ordering::Relaxed)
            .max(1)
    }
}

// ---------------------------------------------------------------------------
// MotifPhase — internal state machine for motif lock
// ---------------------------------------------------------------------------

/// Phase of the per-voice motif lock state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MotifPhase {
    /// Normal Markov mode — motif lock is off.
    Off,
    /// Observing: lock was just enabled; we run the chain normally and record
    /// output into the buffer until `capture_steps` steps have been collected.
    Capturing { steps_remaining: u8 },
    /// Replaying the captured buffer in a loop.
    Replaying,
}

// ---------------------------------------------------------------------------
// MarkovVoice — one voice, audio thread only
// ---------------------------------------------------------------------------

/// Maximum motif buffer length (steps). A 32-step buffer at 1/16 = 2 bars.
pub const MOTIF_BUF_MAX: usize = 32;

/// Audio-thread-only state for one voice.
pub struct MarkovVoice {
    pub rhythmic: RhythmicChain,
    pub melodic: MelodicChain,
    pub current_note: Option<u8>,
    /// Set by on_bar for Bass voices; consumed on the next attack to force root snap.
    pending_root_snap: bool,
    /// Per-phrase register drift: added to octave_offset from shared (-1, 0, or +1).
    pub octave_drift: i8,
    /// Subdivisions since last note-on. Used for mood-driven gate length.
    note_age_subdivs: u32,
    // ── Motif lock ────────────────────────────────────────────────────────────
    motif_phase: MotifPhase,
    motif_buf: [RhythmicState; MOTIF_BUF_MAX],
    motif_len: u8, // captured length (4–32 steps)
    motif_pos: u8, // current replay cursor
}

/// Events emitted by a single voice per subdivision.
#[derive(Clone, Copy, Debug, Default)]
pub struct VoiceEvent {
    pub note_on: Option<u8>,
    pub note_off: Option<u8>,
    /// True if this was an Accent (caller can use for velocity).
    pub accent: bool,
    /// True if this was a Double (caller should fire two rapid NoteOns).
    pub double: bool,
    /// The rhythmic state that produced this event (used by Launchpad display).
    pub rhythmic: RhythmicState,
}

impl MarkovVoice {
    pub fn new(seed: u64) -> Self {
        Self {
            rhythmic: RhythmicChain::new(seed),
            melodic: MelodicChain::new(seed ^ 0xDEAD_BEEF, 0),
            current_note: None,
            pending_root_snap: false,
            octave_drift: 0,
            note_age_subdivs: 0,
            motif_phase: MotifPhase::Off,
            motif_buf: [RhythmicState::Rest; MOTIF_BUF_MAX],
            motif_len: 16,
            motif_pos: 0,
        }
    }

    /// Sample the rhythmic state for this step, applying motif lock if active.
    ///
    /// Returns the `RhythmicState` to use for this step, and records it into
    /// the motif buffer if we're in the Capturing phase.
    fn rhythmic_step(
        &mut self,
        rhythmic_matrix: &RhythmicMatrix,
        density: f32,
        role: VoiceRole,
        rhythmic_speed: f32,
        shared: &MarkovEngineShared,
        voice_idx: usize,
    ) -> Option<RhythmicState> {
        let lock_requested = shared.motif_lock[voice_idx].load(Ordering::Relaxed);
        let capture_len = (shared.motif_length[voice_idx].load(Ordering::Relaxed) as usize)
            .clamp(4, MOTIF_BUF_MAX) as u8;

        // State transitions driven by lock_requested flag.
        match self.motif_phase {
            MotifPhase::Off => {
                if lock_requested {
                    // Start capturing.
                    self.motif_len = capture_len;
                    self.motif_pos = 0;
                    self.motif_phase = MotifPhase::Capturing {
                        steps_remaining: capture_len,
                    };
                    shared.motif_active[voice_idx].store(false, Ordering::Relaxed);
                }
            }
            MotifPhase::Capturing { .. } | MotifPhase::Replaying => {
                if !lock_requested {
                    // Lock released — return to normal Markov.
                    self.motif_phase = MotifPhase::Off;
                    shared.motif_active[voice_idx].store(false, Ordering::Relaxed);
                }
            }
        }

        match self.motif_phase {
            MotifPhase::Off => {
                // Normal Markov path.
                self.rhythmic
                    .on_subdivision(rhythmic_matrix, density, role, rhythmic_speed)
            }
            MotifPhase::Capturing {
                ref mut steps_remaining,
            } => {
                // Run chain normally and record the output.
                let state =
                    self.rhythmic
                        .on_subdivision(rhythmic_matrix, density, role, rhythmic_speed)?;
                let idx = (self.motif_len - *steps_remaining) as usize;
                self.motif_buf[idx.min(MOTIF_BUF_MAX - 1)] = state;
                *steps_remaining -= 1;
                if *steps_remaining == 0 {
                    // Capture complete — switch to replaying.
                    self.motif_pos = 0;
                    self.motif_phase = MotifPhase::Replaying;
                    shared.motif_active[voice_idx].store(true, Ordering::Relaxed);
                }
                Some(state)
            }
            MotifPhase::Replaying => {
                // Read from buffer, advance cursor.
                let state = self.motif_buf[self.motif_pos as usize];
                self.motif_pos = (self.motif_pos + 1) % self.motif_len;
                Some(state)
            }
        }
    }

    /// Call once per Markov step (after clock division is applied by the engine).
    /// `gate_subdivs`: maximum number of steps a note sustains before auto-off.
    ///   1 = staccato (note cut after one step), 16 = let patch ADSR decay naturally.
    /// `rhythmic_speed`: mood-blended speed multiplier (1.0 = normal).
    pub fn on_subdivision(
        &mut self,
        rhythmic_matrix: &RhythmicMatrix,
        melodic_matrix: &MelodicMatrix,
        harmonic: &HarmonicChain,
        shared: &MarkovEngineShared,
        voice_idx: usize,
        gate_subdivs: u32,
        rhythmic_speed: f32,
    ) -> VoiceEvent {
        if !shared.voice_enabled(voice_idx) {
            let note_off = self.current_note.take();
            return VoiceEvent {
                note_off,
                ..Default::default()
            };
        }

        let role = shared.role(voice_idx);
        let density = shared.voice_density(voice_idx);

        // Increment age of any sustained note and auto-off if gate expired.
        if self.current_note.is_some() {
            self.note_age_subdivs += 1;
            if self.note_age_subdivs >= gate_subdivs {
                return VoiceEvent {
                    note_off: self.current_note.take(),
                    rhythmic: RhythmicState::Rest,
                    ..Default::default()
                };
            }
        }

        let Some(rhythmic_state) = self.rhythmic_step(
            rhythmic_matrix,
            density,
            role,
            rhythmic_speed,
            shared,
            voice_idx,
        ) else {
            return VoiceEvent::default();
        };

        let mut ev = VoiceEvent {
            rhythmic: rhythmic_state,
            ..Default::default()
        };

        match rhythmic_state {
            RhythmicState::Rest => {
                ev.note_off = self.current_note.take();
            }
            RhythmicState::Hold => {
                // Continue sounding — age already incremented above.
            }
            RhythmicState::Single | RhythmicState::Double | RhythmicState::Accent => {
                ev.note_off = self.current_note.take();

                let force_root = self.pending_root_snap && role == VoiceRole::Bass;
                self.pending_root_snap = false;

                let octave = shared.octave_offset(voice_idx) + self.octave_drift;
                let pitch = self.melodic.advance_and_resolve(
                    melodic_matrix,
                    role,
                    harmonic,
                    shared.root(),
                    shared.scale(),
                    octave,
                    force_root,
                    shared.chord_attraction(),
                );
                self.current_note = Some(pitch);
                self.note_age_subdivs = 0;
                ev.note_on = Some(pitch);
                ev.accent = rhythmic_state == RhythmicState::Accent;
                ev.double = rhythmic_state == RhythmicState::Double;
            }
        }

        ev
    }
}

// ---------------------------------------------------------------------------
// MarkovEngine — N voices + global harmonic + phrase counter
// ---------------------------------------------------------------------------

/// Full Markov engine, audio thread only.
pub struct MarkovEngine {
    pub voices: Vec<MarkovVoice>,
    pub harmonic: HarmonicChain,
    pub phrase: PhraseCounter,
    moods: [&'static MoodSet; N_MOODS],
    /// RNG used for phrase-level decisions (register drift, chord change).
    phrase_rng: Lcg,
    /// RNG used for per-note gate length variance.
    gate_rng: Lcg,
    /// Current slot index in the harmonic sequence.
    pub seq_slot: usize,
    /// Number of phrases elapsed in the current sequence slot.
    phrases_in_slot: u32,
}

impl MarkovEngine {
    pub fn new(n_voices: usize, seed: u64) -> Self {
        Self {
            voices: (0..n_voices)
                .map(|i| MarkovVoice::new(seed ^ (i as u64 * 0x1111_1111)))
                .collect(),
            harmonic: HarmonicChain::new(seed ^ 0xFEED_FACE),
            phrase: PhraseCounter::new(4),
            moods: [
                &MOOD_CALM,
                &MOOD_TENSE,
                &MOOD_DARK,
                &MOOD_EUPHORIC,
                &MOOD_COSMIC,
                &MOOD_GRAVITY,
            ],
            phrase_rng: Lcg::new(seed ^ 0xA5A5_B6B6),
            gate_rng: Lcg::new(seed ^ 0x7777_CAFE),
            seq_slot: 0,
            phrases_in_slot: 0,
        }
    }

    /// Call once per Markov step (after clock division gating in the audio callback).
    /// Returns one `VoiceEvent` per voice.
    pub fn on_subdivision(&mut self, shared: &MarkovEngineShared) -> Vec<VoiceEvent> {
        let rhythmic = shared.mood.blend_rhythmic(&self.moods);
        let melodic = shared.mood.blend_melodic(&self.moods);

        // Blend gate length from moods: 0.0 → 1 step, 1.0 → 16 steps.
        let gate_blend = shared.mood.blend_gate_length(&self.moods).clamp(0.0, 1.0);
        let gate_variance = shared.mood.blend_gate_length_variance(&self.moods);

        // Blend rhythmic speed from moods.
        let rhythmic_speed = shared.mood.blend_rhythmic_speed(&self.moods);

        // Pre-compute per-voice gate_subdivs with random variance.
        let n = self.voices.len();
        let mut gate_per_voice = Vec::with_capacity(n);
        for _ in 0..n {
            let offset = if gate_variance > 0.0 {
                let r = (self.gate_rng.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0;
                r * gate_variance
            } else {
                0.0
            };
            let effective_gate = (gate_blend + offset).clamp(0.0, 1.0);
            gate_per_voice.push(((effective_gate * 15.0) as u32 + 1).max(1));
        }

        let mut events: Vec<VoiceEvent> = self
            .voices
            .iter_mut()
            .enumerate()
            .map(|(i, v)| {
                v.on_subdivision(
                    &rhythmic,
                    &melodic,
                    &self.harmonic,
                    shared,
                    i,
                    gate_per_voice[i],
                    rhythmic_speed,
                )
            })
            .collect();

        // ── Dissonance resolution post-pass ──────────────────────────────────
        if shared.dissonance_resolve() {
            let threshold = shared.dissonance_threshold(); // 0=semitones, 1=+tritone, 2=+minor7th
            let n = events.len();
            for i in 0..n {
                for j in (i + 1)..n {
                    let (pi, pj) = match (events[i].note_on, events[j].note_on) {
                        (Some(a), Some(b)) => (a, b),
                        _ => continue,
                    };
                    let interval = (pi as i16 - pj as i16).unsigned_abs() % 12;
                    let is_dissonant = interval == 1            // minor 2nd (always)
                        || (threshold >= 1 && interval == 6)   // tritone
                        || (threshold >= 2 && interval == 10); // minor 7th
                    if is_dissonant {
                        let (lo, hi) = shared.role(j).register();
                        let nudged = pj.saturating_add(1).clamp(lo, hi);
                        events[j].note_on = Some(nudged);
                        self.voices[j].current_note = Some(nudged);
                    }
                }
            }
        }

        // Write launchpad display buffer (lock-free, best-effort).
        let col = shared.launchpad_col.fetch_add(1, Ordering::Relaxed) % LAUNCHPAD_COLS;
        for (i, ev) in events.iter().enumerate() {
            if let Some(row) = shared.launchpad.get(i) {
                row[col].store(ev.rhythmic as u8, Ordering::Relaxed);
            }
        }

        events
    }

    /// Call when `BeatEvents::bar` fires. Advances phrase counter and harmonic chain.
    pub fn on_bar(&mut self, shared: &MarkovEngineShared) -> PhraseEvents {
        let harmonic_matrix = shared.mood.blend_harmonic(&self.moods);
        let phrase_ev = self.phrase.on_bar();
        // Update bars_per_phrase from shared in case it changed.
        self.phrase.bars_per_phrase = shared.bars_per_phrase();

        if phrase_ev.phrase_boundary {
            // Signal the control thread that a phrase boundary occurred.
            shared.phrase_epoch.fetch_add(1, Ordering::Relaxed);

            // Advance harmonic sequence slot if sequence length > 1.
            let seq_len = shared.seq_len();
            if seq_len > 1 {
                self.phrases_in_slot += 1;
                if self.phrases_in_slot >= shared.seq_phrases(self.seq_slot) as u32 {
                    self.phrases_in_slot = 0;
                    self.seq_slot = (self.seq_slot + 1) % seq_len;
                    // Update the active root + scale atomics so voices pick them up.
                    shared
                        .root
                        .store(shared.seq_root(self.seq_slot), Ordering::Relaxed);
                    shared.scale.store(
                        shared.seq_scales[self.seq_slot].load(Ordering::Relaxed),
                        Ordering::Relaxed,
                    );
                }
            }
            self.harmonic.advance(&PHRASE_BOUNDARY_HARMONIC);
        } else {
            // ── Harmonic rhythm control ───────────────────────────────────────
            // Roll against blended chord_change_prob to decide whether the
            // harmonic chain actually advances this bar. At 1.0 it always
            // advances (previous behaviour). At 0.15 (Cosmic) the chord holds
            // ~6–7 bars on average.
            let chord_prob = shared.mood.blend_chord_change_prob(&self.moods);
            let r = (self.phrase_rng.next_u32() as f32) / (u32::MAX as f32);
            if r < chord_prob {
                self.harmonic.advance(&harmonic_matrix);
            }
            // else: chord holds — harmonic chain stays on current state.
        }

        // Flag Bass and Pulse voices to snap to root on their next attack (if bass_lock enabled).
        if shared.bass_lock() {
            for (i, voice) in self.voices.iter_mut().enumerate() {
                let role = shared.role(i);
                if role == VoiceRole::Bass || role == VoiceRole::Pulse {
                    voice.pending_root_snap = true;
                }
            }
        }

        // ── Per-phrase register drift ─────────────────────────────────────────
        // Bass and Pulse excluded. Probability = register_drift knob (0=off, 1=always).
        if phrase_ev.phrase_boundary {
            let drift_prob = shared.register_drift();
            if drift_prob > 0.0 {
                for (i, voice) in self.voices.iter_mut().enumerate() {
                    let role = shared.role(i);
                    if role == VoiceRole::Bass || role == VoiceRole::Pulse {
                        continue;
                    }
                    let r = (self.phrase_rng.next_u32() as f32) / (u32::MAX as f32);
                    if r < drift_prob {
                        let dir: i8 = if self.phrase_rng.next_u32() & 1 == 0 {
                            1
                        } else {
                            -1
                        };
                        voice.octave_drift = (voice.octave_drift + dir).clamp(-1, 1);
                    }
                }
            }
        }

        phrase_ev
    }

    pub fn n_voices(&self) -> usize {
        self.voices.len()
    }
}

// ---------------------------------------------------------------------------
// Timeline — song-level temporal structure (Phase 8.7)
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};

/// Optional per-section effects overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EffectsTargets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shimmer_mix: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shimmer_amount: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shimmer_size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crystal_mix: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crystal_feedback: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crystal_delay_ms: Option<f32>,
}

/// One section in a Timeline: a target state the engine interpolates toward.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSection {
    /// Display name ("Intro", "Build", "Peak", …).
    pub name: String,
    /// How many phrases this section lasts before advancing.
    pub phrases: u32,
    /// How many phrases to spend crossfading from the previous section (≤ phrases).
    #[serde(default)]
    pub transition_phrases: u32,

    // ── Target values (all optional — omitted = carry forward from previous) ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mood: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bars_per_phrase: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_enabled: Option<Vec<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<EffectsTargets>,
}

/// Fully resolved parameter snapshot (no Options). Used for interpolation endpoints.
#[derive(Debug, Clone)]
pub struct ResolvedState {
    pub mood: [f32; N_MOODS],
    pub density: f32,
    pub root: u8,
    pub scale: u8,
    pub bars_per_phrase: u32,
    pub voice_enabled: [bool; 4],
    pub shimmer_mix: f32,
    pub shimmer_amount: f32,
    pub shimmer_size: f32,
    pub crystal_mix: f32,
    pub crystal_feedback: f32,
    pub crystal_delay_ms: f32,
}

impl Default for ResolvedState {
    fn default() -> Self {
        Self {
            mood: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0], // 100% Calm
            density: 0.3,
            root: 60,
            scale: 0,
            bars_per_phrase: 4,
            voice_enabled: [true; 4],
            shimmer_mix: 0.5,
            shimmer_amount: 0.5,
            shimmer_size: 0.5,
            crystal_mix: 0.2,
            crystal_feedback: 0.3,
            crystal_delay_ms: 400.0,
        }
    }
}

impl ResolvedState {
    /// Create a resolved state by reading current values from shared atomics.
    pub fn snapshot_from_shared(shared: &MarkovEngineShared) -> Self {
        let mut mood = [0.0f32; N_MOODS];
        for i in 0..N_MOODS {
            mood[i] = shared.mood.weight(i);
        }
        let mut voice_enabled = [true; 4];
        for i in 0..4.min(shared.voice_enabled.len()) {
            voice_enabled[i] = shared.voice_enabled[i].load(Ordering::Relaxed);
        }
        Self {
            mood,
            density: shared.density(),
            root: shared.root(),
            scale: shared.scale.load(Ordering::Relaxed),
            bars_per_phrase: shared.bars_per_phrase(),
            voice_enabled,
            // Effects are not in MarkovEngineShared — use defaults, will be overwritten
            // by the actual scene globals when first resolved.
            shimmer_mix: 0.5,
            shimmer_amount: 0.5,
            shimmer_size: 0.5,
            crystal_mix: 0.2,
            crystal_feedback: 0.3,
            crystal_delay_ms: 400.0,
        }
    }

    /// Apply a section's optional fields on top of this state, returning a new resolved state.
    pub fn with_section(&self, section: &TimelineSection) -> Self {
        let mut out = self.clone();
        if let Some(ref mood) = section.mood {
            for (i, &w) in mood.iter().enumerate().take(N_MOODS) {
                out.mood[i] = w;
            }
        }
        if let Some(d) = section.density {
            out.density = d;
        }
        if let Some(r) = section.root {
            out.root = r;
        }
        if let Some(s) = section.scale {
            out.scale = s;
        }
        if let Some(b) = section.bars_per_phrase {
            out.bars_per_phrase = b;
        }
        if let Some(ref ve) = section.voice_enabled {
            for (i, &v) in ve.iter().enumerate().take(4) {
                out.voice_enabled[i] = v;
            }
        }
        if let Some(ref fx) = section.effects {
            if let Some(v) = fx.shimmer_mix {
                out.shimmer_mix = v;
            }
            if let Some(v) = fx.shimmer_amount {
                out.shimmer_amount = v;
            }
            if let Some(v) = fx.shimmer_size {
                out.shimmer_size = v;
            }
            if let Some(v) = fx.crystal_mix {
                out.crystal_mix = v;
            }
            if let Some(v) = fx.crystal_feedback {
                out.crystal_feedback = v;
            }
            if let Some(v) = fx.crystal_delay_ms {
                out.crystal_delay_ms = v;
            }
        }
        out
    }

    /// Linearly interpolate between two resolved states.
    /// `t` ranges from 0.0 (= self) to 1.0 (= other).
    /// Continuous params are lerped. Discrete params (root, scale, voice_enabled) snap at t=0.5.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let snap = t >= 0.5;

        let mut mood = [0.0f32; N_MOODS];
        for i in 0..N_MOODS {
            mood[i] = self.mood[i] * (1.0 - t) + other.mood[i] * t;
        }

        Self {
            mood,
            density: self.density * (1.0 - t) + other.density * t,
            root: if snap { other.root } else { self.root },
            scale: if snap { other.scale } else { self.scale },
            bars_per_phrase: if snap {
                other.bars_per_phrase
            } else {
                self.bars_per_phrase
            },
            voice_enabled: if snap {
                other.voice_enabled
            } else {
                self.voice_enabled
            },
            shimmer_mix: self.shimmer_mix * (1.0 - t) + other.shimmer_mix * t,
            shimmer_amount: self.shimmer_amount * (1.0 - t) + other.shimmer_amount * t,
            shimmer_size: self.shimmer_size * (1.0 - t) + other.shimmer_size * t,
            crystal_mix: self.crystal_mix * (1.0 - t) + other.crystal_mix * t,
            crystal_feedback: self.crystal_feedback * (1.0 - t) + other.crystal_feedback * t,
            crystal_delay_ms: self.crystal_delay_ms * (1.0 - t) + other.crystal_delay_ms * t,
        }
    }
}

/// Song-level temporal structure that modulates engine parameters over time.
/// Lives on the control thread. Writes to `MarkovEngineShared` atomics.
pub struct Timeline {
    pub sections: Vec<TimelineSection>,
    pub cursor: usize,
    pub phrase_in_sect: u32,
    pub loop_mode: bool,
    pub active: bool,

    /// Previous section's final state (interpolation start point).
    pub prev_state: ResolvedState,
    /// Current section's target state (interpolation end point).
    pub target_state: ResolvedState,
}

/// Snapshot of timeline status for UI display.
#[derive(Debug, Clone)]
pub struct TimelineStatus {
    pub active: bool,
    pub cursor: usize,
    pub section_count: usize,
    pub section_name: String,
    pub phrase_in_sect: u32,
    pub section_phrases: u32,
    pub transition_phrases: u32,
    /// 0.0 = start of section, 1.0 = end of section.
    pub section_progress: f32,
    /// true if currently in the crossfade window.
    pub in_transition: bool,
    /// The interpolated state currently being applied.
    pub current_state: ResolvedState,
}

impl Timeline {
    /// Create a new Timeline from section definitions.
    /// `base_state` is the resolved state from the scene's base markov config.
    pub fn new(sections: Vec<TimelineSection>, loop_mode: bool, base_state: ResolvedState) -> Self {
        let target_state = if let Some(first) = sections.first() {
            base_state.with_section(first)
        } else {
            base_state.clone()
        };
        Self {
            sections,
            cursor: 0,
            phrase_in_sect: 0,
            loop_mode,
            active: true,
            prev_state: base_state,
            target_state,
        }
    }

    /// Create an inactive (empty) timeline.
    pub fn inactive() -> Self {
        Self {
            sections: Vec::new(),
            cursor: 0,
            phrase_in_sect: 0,
            loop_mode: false,
            active: false,
            prev_state: ResolvedState::default(),
            target_state: ResolvedState::default(),
        }
    }

    /// Total number of phrases across all sections.
    pub fn total_phrases(&self) -> u32 {
        self.sections.iter().map(|s| s.phrases).sum()
    }

    /// Number of phrases elapsed from the beginning of the timeline.
    pub fn elapsed_phrases(&self) -> u32 {
        let prior: u32 = self.sections[..self.cursor].iter().map(|s| s.phrases).sum();
        prior + self.phrase_in_sect
    }

    /// Call on each phrase boundary. Returns the interpolated state to apply.
    pub fn on_phrase_boundary(&mut self) -> ResolvedState {
        if !self.active || self.sections.is_empty() {
            return self.target_state.clone();
        }

        self.phrase_in_sect += 1;

        let current_section = &self.sections[self.cursor];

        // Check if section is exhausted.
        if self.phrase_in_sect >= current_section.phrases {
            if self.cursor + 1 < self.sections.len() {
                // Advance to next section.
                self.prev_state = self.target_state.clone();
                self.cursor += 1;
                self.phrase_in_sect = 0;
                self.target_state = self.prev_state.with_section(&self.sections[self.cursor]);
            } else if self.loop_mode {
                // Loop back to first section.
                self.prev_state = self.target_state.clone();
                self.cursor = 0;
                self.phrase_in_sect = 0;
                self.target_state = self.prev_state.with_section(&self.sections[0]);
            }
            // else: hold on final section indefinitely.
        }

        self.interpolated_state()
    }

    /// Get the current interpolated state without advancing.
    pub fn interpolated_state(&self) -> ResolvedState {
        if !self.active || self.sections.is_empty() {
            return self.target_state.clone();
        }

        let section = &self.sections[self.cursor];
        let transition = section.transition_phrases.min(section.phrases);

        if transition == 0 || self.phrase_in_sect >= transition {
            // Past transition window — hold at target.
            self.target_state.clone()
        } else {
            // In transition window — interpolate.
            let t = self.phrase_in_sect as f32 / transition as f32;
            self.prev_state.lerp(&self.target_state, t)
        }
    }

    /// Force-advance to the next section immediately, ignoring remaining phrase count.
    /// Use for Bevy-controlled or visual-driven transitions.
    /// Returns the new section name (empty string if already on last section and not looping).
    pub fn force_advance(&mut self) -> String {
        if !self.active || self.sections.is_empty() {
            return String::new();
        }
        // Jump phrase_in_sect to the section boundary to trigger advance logic.
        self.phrase_in_sect = self.sections[self.cursor].phrases.saturating_sub(1);
        self.on_phrase_boundary();
        self.sections
            .get(self.cursor)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }

    /// Write the current interpolated state to the engine's shared atomics.
    pub fn apply_to_shared(&self, shared: &MarkovEngineShared) {
        if !self.active || self.sections.is_empty() {
            return;
        }

        let state = self.interpolated_state();

        // Mood blend.
        shared.mood.set(&state.mood);

        // Density.
        shared.density.set_value(state.density);

        // Root + scale.
        shared.root.store(state.root, Ordering::Relaxed);
        shared.scale.store(state.scale, Ordering::Relaxed);

        // Bars per phrase.
        shared
            .bars_per_phrase
            .store(state.bars_per_phrase, Ordering::Relaxed);

        // Voice enabled.
        for (i, &v) in state.voice_enabled.iter().enumerate() {
            if i < shared.voice_enabled.len() {
                shared.voice_enabled[i].store(v, Ordering::Relaxed);
            }
        }

        // Effects are written separately by the UI (they live in scene globals,
        // not in MarkovEngineShared). The UI reads `current_state` from
        // `TimelineStatus` and applies effects there.
    }

    /// Get a status snapshot for UI display.
    pub fn status(&self) -> TimelineStatus {
        let (section_name, section_phrases, transition_phrases) = if self.sections.is_empty() {
            ("(none)".to_string(), 0u32, 0u32)
        } else {
            let s = &self.sections[self.cursor];
            (s.name.clone(), s.phrases, s.transition_phrases)
        };

        let section_progress = if section_phrases > 0 {
            self.phrase_in_sect as f32 / section_phrases as f32
        } else {
            0.0
        };

        let in_transition = self.phrase_in_sect < transition_phrases;

        TimelineStatus {
            active: self.active && !self.sections.is_empty(),
            cursor: self.cursor,
            section_count: self.sections.len(),
            section_name,
            phrase_in_sect: self.phrase_in_sect,
            section_phrases,
            transition_phrases,
            section_progress,
            in_transition,
            current_state: self.interpolated_state(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine(n: usize) -> (MarkovEngine, MarkovEngineShared) {
        (
            MarkovEngine::new(n, 0xABCD_1234),
            MarkovEngineShared::new(n),
        )
    }

    #[test]
    fn harmonic_chain_advances() {
        let mut chain = HarmonicChain::new(42);
        let initial = chain.state;
        // After many advances, state should have changed at least once.
        let mut changed = false;
        for _ in 0..100 {
            chain.advance(&MOOD_CALM.harmonic);
            if chain.state != initial {
                changed = true;
                break;
            }
        }
        assert!(changed, "harmonic chain should advance");
    }

    #[test]
    fn rhythmic_chain_respects_density_zero() {
        // density=0 → use matrix as-is, rest probability should be non-zero.
        let mut chain = RhythmicChain::new(99);
        chain.state = RhythmicState::Single;
        let mut rest_count = 0u32;
        for _ in 0..200 {
            if let Some(st) = chain.on_subdivision(&MOOD_CALM.rhythmic, 0.0, VoiceRole::Melody, 1.0)
            {
                if st == RhythmicState::Rest {
                    rest_count += 1;
                }
            }
        }
        assert!(rest_count > 0, "some rests expected at density=0");
    }

    #[test]
    fn rhythmic_chain_density_one_suppresses_rest() {
        let mut chain = RhythmicChain::new(99);
        chain.state = RhythmicState::Single;
        let mut rest_count = 0u32;
        for _ in 0..200 {
            if let Some(st) = chain.on_subdivision(&MOOD_CALM.rhythmic, 1.0, VoiceRole::Melody, 1.0)
            {
                if st == RhythmicState::Rest {
                    rest_count += 1;
                }
            }
        }
        assert_eq!(rest_count, 0, "no rests at density=1");
    }

    #[test]
    fn melodic_chain_stays_in_register() {
        let harmonic = HarmonicChain::new(0);
        let mut melodic = MelodicChain::new(7, 0);
        for _ in 0..200 {
            let pitch = melodic.advance_and_resolve(
                &MOOD_CALM.melodic,
                VoiceRole::Melody,
                &harmonic,
                60,
                Scale::Major,
                0,
                false,
                0.5,
            );
            let (lo, hi) = VoiceRole::Melody.register();
            assert!(
                pitch >= lo && pitch <= hi,
                "pitch {pitch} out of melody register {lo}-{hi}"
            );
        }
    }

    #[test]
    fn bass_role_avoids_forbidden_degrees() {
        // Bias for Bass: degrees 2 (index 2) and 3 (index 3) have bias 0.0.
        let bias = VoiceRole::Bass.degree_bias();
        assert_eq!(
            bias[2], 0.0,
            "degree 3 (index 2) should be forbidden for bass"
        );
        assert_eq!(
            bias[3], 0.0,
            "degree 4 (index 3) should be forbidden for bass"
        );
    }

    #[test]
    fn phrase_counter_fires_at_boundary() {
        let mut counter = PhraseCounter::new(4);
        let mut fired = false;
        for _ in 0..4 {
            let ev = counter.on_bar();
            if ev.phrase_boundary {
                fired = true;
            }
        }
        assert!(fired, "phrase boundary should fire after 4 bars");
    }

    #[test]
    fn mood_blend_uniform_gives_average() {
        let blend = MoodBlend::new();
        blend.set(&[0.166, 0.166, 0.166, 0.166, 0.166, 0.17]);
        let moods = [
            &MOOD_CALM,
            &MOOD_TENSE,
            &MOOD_DARK,
            &MOOD_EUPHORIC,
            &MOOD_COSMIC,
            &MOOD_GRAVITY,
        ];
        let mat = blend.blend_rhythmic(&moods);
        // Each row should still sum to ~1.0
        for row in &mat {
            let sum: f32 = row.iter().sum();
            assert!((sum - 1.0).abs() < 0.01, "blended row sum {sum} ≠ 1.0");
        }
    }

    #[test]
    fn engine_produces_events_per_voice() {
        let (mut eng, shared) = make_engine(4);
        // Run 64 subdivisions — at least some voices should fire.
        let mut total_note_ons = 0usize;
        for _ in 0..64 {
            let evs = eng.on_subdivision(&shared);
            assert_eq!(evs.len(), 4);
            for ev in &evs {
                if ev.note_on.is_some() {
                    total_note_ons += 1;
                }
            }
        }
        assert!(
            total_note_ons > 0,
            "at least some note_ons expected over 64 subdivisions"
        );
    }

    #[test]
    fn engine_bar_advances_harmonic() {
        let (mut eng, shared) = make_engine(2);
        let initial = eng.harmonic.state;
        let mut changed = false;
        for _ in 0..32 {
            eng.on_bar(&shared);
            if eng.harmonic.state != initial {
                changed = true;
                break;
            }
        }
        assert!(changed, "harmonic state should change over bars");
    }

    #[test]
    fn scale_degree_resolution_in_range() {
        // C major, degree 0 → C4 (60), degree 6 → B4 (71)
        assert_eq!(Scale::Major.degree_to_midi(60, 0, 0), 60); // C
        assert_eq!(Scale::Major.degree_to_midi(60, 4, 0), 67); // G
        assert_eq!(Scale::Major.degree_to_midi(60, 6, 0), 71); // B
    }

    #[test]
    fn mood_blend_normalization() {
        let blend = MoodBlend::new();
        // Unnormalized input — should be auto-normalized.
        blend.set(&[2.0, 2.0, 0.0, 0.0, 0.0, 0.0]);
        assert!((blend.weight(0) - 0.5).abs() < 0.001);
        assert!((blend.weight(1) - 0.5).abs() < 0.001);
    }

    #[test]
    fn markov_voice_event_sequence_is_consistent() {
        let mut voice = MarkovVoice::new(0);
        let shared = MarkovEngineShared::new(1);
        let harmonic = HarmonicChain::new(0);
        let mut prev_note = voice.current_note;

        for _ in 0..256 {
            let ev = voice.on_subdivision(
                &MOOD_CALM.rhythmic,
                &MOOD_CALM.melodic,
                &harmonic,
                &shared,
                0,
                8,
                1.0,
            );
            if let Some(off) = ev.note_off {
                assert_eq!(prev_note, Some(off));
            }
            if let Some(on) = ev.note_on {
                assert_eq!(voice.current_note, Some(on));
                assert_eq!(voice.note_age_subdivs, 0);
            }
            prev_note = voice.current_note;
        }
    }

    #[test]
    fn markov_engine_long_running_generates_note_events() {
        let (mut eng, shared) = make_engine(4);
        shared.clock_div.store(1, Ordering::Relaxed);

        let mut total_note_ons = 0;
        let mut total_note_offs = 0;

        for _ in 0..512 {
            let events = eng.on_subdivision(&shared);
            assert_eq!(events.len(), 4);
            for (i, ev) in events.into_iter().enumerate() {
                if let Some(off) = ev.note_off {
                    total_note_offs += 1;
                    assert!(off < 128);
                }
                if let Some(on) = ev.note_on {
                    total_note_ons += 1;
                    assert_eq!(eng.voices[i].current_note, Some(on));
                }
            }
        }

        assert!(total_note_ons > 0, "expected some note_on events");
        assert!(total_note_offs > 0, "expected some note_off events");
    }
}
