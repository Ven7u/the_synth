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

use std::sync::{
    atomic::{AtomicU8, AtomicUsize, Ordering},
    Arc,
};
use fundsp::prelude32::{shared, Shared};

// ---------------------------------------------------------------------------
// LCG — identical to synth-engine's private copy; duplicated to avoid coupling
// ---------------------------------------------------------------------------

struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self { Self(seed | 1) }

    fn next_u32(&mut self) -> u32 {
        self.0 = self.0
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
            if r < acc { return i; }
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
    Major      = 0,
    Minor      = 1,  // natural minor
    Dorian     = 2,
    Phrygian   = 3,
    Lydian     = 4,
    Mixolydian = 5,
    HarmonicMinor = 6,
}

impl Scale {
    /// Semitone offsets for degrees 0–6 (scale degrees 1–7).
    pub fn intervals(self) -> &'static [u8; 7] {
        match self {
            Self::Major         => &[0, 2, 4, 5, 7, 9, 11],
            Self::Minor         => &[0, 2, 3, 5, 7, 8, 10],
            Self::Dorian        => &[0, 2, 3, 5, 7, 9, 10],
            Self::Phrygian      => &[0, 1, 3, 5, 7, 8, 10],
            Self::Lydian        => &[0, 2, 4, 6, 7, 9, 11],
            Self::Mixolydian    => &[0, 2, 4, 5, 7, 9, 10],
            Self::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
        }
    }

    pub const ALL: &'static [Self] = &[
        Self::Major, Self::Minor, Self::Dorian, Self::Phrygian,
        Self::Lydian, Self::Mixolydian, Self::HarmonicMinor,
    ];
    pub const LABELS: &'static [&'static str] = &[
        "Major", "Minor", "Dorian", "Phrygian", "Lydian", "Mixolyd.", "Harm.Minor",
    ];

    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Minor, 2 => Self::Dorian, 3 => Self::Phrygian,
            4 => Self::Lydian, 5 => Self::Mixolydian, 6 => Self::HarmonicMinor,
            _ => Self::Major,
        }
    }

    /// Resolve a scale degree (0-based, 0=tonic) to a MIDI pitch.
    /// `root` is the MIDI pitch of the tonic. `octave_offset` shifts register.
    pub fn degree_to_midi(self, root: u8, degree: usize, octave_offset: i8) -> u8 {
        let semitone = self.intervals()[degree % 7];
        let extra_octave = (degree / 7) as i8;
        let raw = root as i32
            + semitone as i32
            + (octave_offset + extra_octave) as i32 * 12;
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
    Tonic         = 0, // I / i
    Supertonic    = 1, // II / ii
    Mediant       = 2, // III / iii / bIII
    Subdominant   = 3, // IV / iv
    Dominant      = 4, // V / V7
    Submediant    = 5, // VI / vi / bVI
    LeadingTone   = 6, // VII / vii / bVII
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
            1 => Self::Supertonic, 2 => Self::Mediant, 3 => Self::Subdominant,
            4 => Self::Dominant,   5 => Self::Submediant, 6 => Self::LeadingTone,
            _ => Self::Tonic,
        }
    }

    pub const LABELS: &'static [&'static str] =
        &["I", "ii", "iii", "IV", "V", "vi", "vii"];
}

// ---------------------------------------------------------------------------
// Transition matrix helpers
// ---------------------------------------------------------------------------

pub const HARMONIC_STATES: usize = HarmonicFunction::N; // 7
pub const RHYTHMIC_STATES: usize = 5;
pub const MELODIC_STATES:  usize = 7; // scale degrees 1–7

pub type HarmonicMatrix = [[f32; HARMONIC_STATES]; HARMONIC_STATES];
pub type RhythmicMatrix = [[f32; RHYTHMIC_STATES]; RHYTHMIC_STATES];
pub type MelodicMatrix  = [[f32; MELODIC_STATES];  MELODIC_STATES];

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
fn apply_bias_and_normalize(row: &[f32; MELODIC_STATES], bias: &[f32; MELODIC_STATES])
    -> [f32; MELODIC_STATES]
{
    let mut out = [0.0f32; MELODIC_STATES];
    let mut total = 0.0f32;
    for j in 0..MELODIC_STATES {
        out[j] = (row[j] * bias[j]).max(0.0);
        total += out[j];
    }
    if total > 0.0 {
        for j in 0..MELODIC_STATES { out[j] /= total; }
    } else {
        // fallback: uniform
        for j in 0..MELODIC_STATES { out[j] = 1.0 / MELODIC_STATES as f32; }
    }
    out
}

/// Apply density to a rhythmic row: scale down the Rest column, renormalize.
fn apply_density(row: &[f32; RHYTHMIC_STATES], density: f32) -> [f32; RHYTHMIC_STATES] {
    let mut out = *row;
    // density=0 → keep matrix as-is; density=1 → rest column → 0
    out[RhythmicState::Rest as usize] *= 1.0 - density.clamp(0.0, 1.0);
    let total: f32 = out.iter().sum();
    if total > 0.0 { for v in &mut out { *v /= total; } }
    out
}

// ---------------------------------------------------------------------------
// RhythmicState
// ---------------------------------------------------------------------------

#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RhythmicState {
    #[default]
    Rest   = 0,
    Hold   = 1,
    Single = 2,
    Double = 3,
    Accent = 4,
}

impl RhythmicState {
    pub fn from_usize(v: usize) -> Self {
        match v { 1 => Self::Hold, 2 => Self::Single, 3 => Self::Double, 4 => Self::Accent, _ => Self::Rest }
    }
    pub const LABELS: &'static [&'static str] = &["Rest", "Hold", "Single", "Double", "Accent"];
}

// ---------------------------------------------------------------------------
// MoodSet — named triple of matrices
// ---------------------------------------------------------------------------

/// A named mood: three matrices and a display label.
#[derive(Clone)]
pub struct MoodSet {
    pub name:     &'static str,
    pub harmonic: HarmonicMatrix,
    pub rhythmic: RhythmicMatrix,
    pub melodic:  MelodicMatrix,
}

// ---------------------------------------------------------------------------
// Built-in moods
// ---------------------------------------------------------------------------

/// Calm: tonic-heavy, sparse rhythm, stepwise melody, strong tonic pull.
pub const MOOD_CALM: MoodSet = MoodSet {
    name: "Calm",
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.25,  0.10,  0.05,  0.25,  0.20,  0.10,  0.05], // I
        [0.05,  0.15,  0.05,  0.15,  0.40,  0.15,  0.05], // ii
        [0.10,  0.10,  0.10,  0.25,  0.20,  0.20,  0.05], // iii
        [0.25,  0.10,  0.05,  0.20,  0.25,  0.10,  0.05], // IV
        [0.45,  0.05,  0.05,  0.10,  0.15,  0.15,  0.05], // V
        [0.15,  0.20,  0.05,  0.20,  0.25,  0.10,  0.05], // vi
        [0.35,  0.10,  0.05,  0.15,  0.20,  0.10,  0.05], // vii
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.40,  0.20,  0.35,  0.03,  0.02], // Rest
        [0.15,  0.45,  0.35,  0.03,  0.02], // Hold
        [0.25,  0.20,  0.40,  0.10,  0.05], // Single
        [0.30,  0.10,  0.45,  0.10,  0.05], // Double
        [0.35,  0.15,  0.40,  0.08,  0.02], // Accent
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.25,  0.30,  0.15,  0.05,  0.15,  0.05,  0.05], // 1
        [0.20,  0.20,  0.30,  0.15,  0.08,  0.05,  0.02], // 2
        [0.10,  0.25,  0.20,  0.25,  0.12,  0.05,  0.03], // 3
        [0.05,  0.15,  0.20,  0.20,  0.25,  0.10,  0.05], // 4
        [0.20,  0.08,  0.12,  0.15,  0.25,  0.15,  0.05], // 5
        [0.10,  0.05,  0.10,  0.10,  0.20,  0.25,  0.20], // 6
        [0.30,  0.05,  0.05,  0.05,  0.15,  0.15,  0.25], // 7
    ],
};

/// Tense: dominant-heavy, dense/accented rhythm, leaping dissonant melody.
pub const MOOD_TENSE: MoodSet = MoodSet {
    name: "Tense",
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.10,  0.15,  0.05,  0.15,  0.35,  0.10,  0.10], // I
        [0.05,  0.10,  0.05,  0.10,  0.50,  0.10,  0.10], // ii
        [0.08,  0.12,  0.08,  0.20,  0.30,  0.12,  0.10], // iii
        [0.15,  0.10,  0.05,  0.10,  0.40,  0.10,  0.10], // IV
        [0.30,  0.08,  0.05,  0.10,  0.25,  0.12,  0.10], // V
        [0.10,  0.20,  0.05,  0.15,  0.35,  0.08,  0.07], // vi
        [0.25,  0.10,  0.05,  0.10,  0.30,  0.10,  0.10], // vii
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.15,  0.05,  0.40,  0.25,  0.15], // Rest
        [0.10,  0.20,  0.40,  0.20,  0.10], // Hold
        [0.10,  0.10,  0.35,  0.30,  0.15], // Single
        [0.15,  0.05,  0.35,  0.35,  0.10], // Double
        [0.20,  0.05,  0.35,  0.25,  0.15], // Accent
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.15,  0.10,  0.20,  0.05,  0.30,  0.10,  0.10], // 1
        [0.10,  0.10,  0.15,  0.10,  0.25,  0.20,  0.10], // 2
        [0.20,  0.10,  0.10,  0.10,  0.25,  0.15,  0.10], // 3
        [0.15,  0.10,  0.15,  0.10,  0.30,  0.10,  0.10], // 4
        [0.25,  0.05,  0.15,  0.10,  0.15,  0.15,  0.15], // 5
        [0.20,  0.05,  0.10,  0.10,  0.25,  0.15,  0.15], // 6
        [0.35,  0.05,  0.10,  0.05,  0.25,  0.10,  0.10], // 7
    ],
};

/// Dark: minor-leaning, modal, sparse with sudden accents, low-register wandering.
pub const MOOD_DARK: MoodSet = MoodSet {
    name: "Dark",
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.30,  0.05,  0.05,  0.10,  0.15,  0.25,  0.10], // I   → vi (modal)
        [0.05,  0.20,  0.05,  0.15,  0.30,  0.20,  0.05], // ii
        [0.10,  0.10,  0.15,  0.15,  0.15,  0.25,  0.10], // iii
        [0.20,  0.05,  0.05,  0.25,  0.20,  0.20,  0.05], // IV
        [0.35,  0.05,  0.05,  0.10,  0.20,  0.20,  0.05], // V
        [0.20,  0.10,  0.10,  0.15,  0.15,  0.20,  0.10], // vi
        [0.25,  0.08,  0.05,  0.15,  0.22,  0.15,  0.10], // vii
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.45,  0.25,  0.20,  0.02,  0.08], // Rest  — mostly stays silent
        [0.10,  0.55,  0.25,  0.02,  0.08], // Hold  — long sustains
        [0.30,  0.15,  0.35,  0.05,  0.15], // Single
        [0.35,  0.10,  0.40,  0.08,  0.07], // Double
        [0.40,  0.10,  0.35,  0.05,  0.10], // Accent — accents lead to rest
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.30,  0.15,  0.10,  0.08,  0.15,  0.12,  0.10], // 1
        [0.20,  0.20,  0.15,  0.15,  0.10,  0.10,  0.10], // 2
        [0.15,  0.20,  0.25,  0.15,  0.10,  0.10,  0.05], // 3 (b3 in minor)
        [0.08,  0.15,  0.20,  0.20,  0.20,  0.12,  0.05], // 4
        [0.20,  0.10,  0.10,  0.15,  0.25,  0.10,  0.10], // 5
        [0.15,  0.10,  0.15,  0.10,  0.15,  0.25,  0.10], // 6 (b6 in minor)
        [0.35,  0.08,  0.08,  0.05,  0.15,  0.12,  0.17], // 7 → resolve to 1
    ],
};

/// Euphoric: fast major resolutions, IV→I lifts, high-energy ascending.
pub const MOOD_EUPHORIC: MoodSet = MoodSet {
    name: "Euphoric",
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.20,  0.05,  0.05,  0.35,  0.25,  0.05,  0.05], // I   → IV strong lift
        [0.10,  0.10,  0.05,  0.20,  0.45,  0.05,  0.05], // ii  → V
        [0.15,  0.10,  0.08,  0.25,  0.30,  0.07,  0.05], // iii
        [0.40,  0.05,  0.05,  0.15,  0.25,  0.05,  0.05], // IV  → I (plagal lift)
        [0.50,  0.05,  0.05,  0.15,  0.15,  0.05,  0.05], // V   → I (strong resolve)
        [0.20,  0.15,  0.05,  0.25,  0.25,  0.05,  0.05], // vi
        [0.40,  0.08,  0.05,  0.15,  0.22,  0.05,  0.05], // vii → I
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.10,  0.05,  0.45,  0.25,  0.15], // Rest  — rarely stays resting
        [0.08,  0.15,  0.45,  0.22,  0.10], // Hold
        [0.10,  0.08,  0.35,  0.32,  0.15], // Single — lots of doubles/accents
        [0.12,  0.05,  0.38,  0.32,  0.13], // Double
        [0.15,  0.05,  0.40,  0.28,  0.12], // Accent
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.15,  0.15,  0.25,  0.05,  0.25,  0.10,  0.05], // 1 → 3 or 5 (ascending)
        [0.10,  0.10,  0.35,  0.10,  0.20,  0.10,  0.05], // 2 → 3
        [0.15,  0.10,  0.15,  0.10,  0.30,  0.15,  0.05], // 3 → 5
        [0.10,  0.10,  0.20,  0.10,  0.35,  0.10,  0.05], // 4 → 5
        [0.20,  0.05,  0.15,  0.05,  0.20,  0.25,  0.10], // 5 → 6 (ascending)
        [0.15,  0.05,  0.10,  0.05,  0.20,  0.20,  0.25], // 6 → 7
        [0.40,  0.05,  0.10,  0.05,  0.20,  0.10,  0.10], // 7 → 1 (resolve up)
    ],
};

/// Cosmic (Interstellar-inspired): long harmonic cycles, IV↔I plagal motion,
/// vi as a resting point, very sparse rhythm, slow stepwise melody.
/// Inspired by Hans Zimmer's organ-driven space feel.
pub const MOOD_COSMIC: MoodSet = MoodSet {
    name: "Cosmic",
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.20,  0.05,  0.05,  0.30,  0.10,  0.25,  0.05], // I   → IV or vi (plagal)
        [0.08,  0.12,  0.05,  0.20,  0.30,  0.20,  0.05], // ii
        [0.10,  0.08,  0.12,  0.20,  0.15,  0.30,  0.05], // iii → vi
        [0.35,  0.08,  0.05,  0.18,  0.15,  0.14,  0.05], // IV  → I (plagal cadence)
        [0.25,  0.10,  0.05,  0.25,  0.10,  0.20,  0.05], // V   → IV or I
        [0.25,  0.10,  0.10,  0.25,  0.12,  0.12,  0.06], // vi  → I or IV
        [0.30,  0.08,  0.05,  0.20,  0.20,  0.12,  0.05], // vii
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.55,  0.30,  0.12,  0.02,  0.01], // Rest  — very sparse
        [0.08,  0.65,  0.22,  0.03,  0.02], // Hold  — long sustained tones
        [0.35,  0.25,  0.30,  0.06,  0.04], // Single
        [0.40,  0.15,  0.35,  0.07,  0.03], // Double
        [0.45,  0.20,  0.28,  0.05,  0.02], // Accent
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.30,  0.25,  0.12,  0.08,  0.12,  0.08,  0.05], // 1 → stepwise
        [0.22,  0.22,  0.28,  0.12,  0.08,  0.05,  0.03], // 2 → 3
        [0.12,  0.22,  0.22,  0.22,  0.12,  0.06,  0.04], // 3
        [0.08,  0.12,  0.22,  0.22,  0.22,  0.10,  0.04], // 4
        [0.18,  0.08,  0.10,  0.18,  0.24,  0.16,  0.06], // 5
        [0.18,  0.06,  0.08,  0.10,  0.18,  0.26,  0.14], // 6
        [0.32,  0.06,  0.06,  0.08,  0.14,  0.18,  0.16], // 7 → resolve to 1
    ],
};

/// Gravity (Interstellar docking scene): minor, dense sustained harmonics,
/// vi–IV–I–V cycle, mechanical rhythm with sudden silences, narrow melodic range.
pub const MOOD_GRAVITY: MoodSet = MoodSet {
    name: "Gravity",
    harmonic: [
        //  I      ii     iii    IV     V      vi     vii
        [0.12,  0.08,  0.05,  0.20,  0.28,  0.22,  0.05], // I
        [0.08,  0.10,  0.05,  0.18,  0.40,  0.14,  0.05], // ii → V
        [0.10,  0.10,  0.08,  0.22,  0.22,  0.22,  0.06], // iii
        [0.28,  0.08,  0.05,  0.12,  0.22,  0.20,  0.05], // IV → I
        [0.22,  0.10,  0.05,  0.18,  0.12,  0.28,  0.05], // V  → vi (deceptive)
        [0.20,  0.12,  0.08,  0.30,  0.18,  0.08,  0.04], // vi → IV
        [0.28,  0.10,  0.05,  0.15,  0.28,  0.10,  0.04], // vii
    ],
    rhythmic: [
        //  Rest   Hold   Single Double Accent
        [0.30,  0.18,  0.32,  0.12,  0.08], // Rest
        [0.12,  0.40,  0.32,  0.10,  0.06], // Hold — sustained with occasional attacks
        [0.18,  0.15,  0.35,  0.20,  0.12], // Single
        [0.22,  0.08,  0.35,  0.25,  0.10], // Double
        [0.28,  0.10,  0.35,  0.18,  0.09], // Accent → rest (abrupt)
    ],
    melodic: [
        //  1      2      3      4      5      6      7
        [0.28,  0.20,  0.18,  0.08,  0.12,  0.08,  0.06], // 1 — narrow stepwise
        [0.22,  0.22,  0.25,  0.14,  0.08,  0.06,  0.03], // 2
        [0.18,  0.22,  0.22,  0.20,  0.10,  0.05,  0.03], // 3 (b3 in minor)
        [0.10,  0.15,  0.22,  0.22,  0.18,  0.08,  0.05], // 4
        [0.22,  0.10,  0.12,  0.18,  0.22,  0.12,  0.04], // 5
        [0.20,  0.08,  0.10,  0.12,  0.18,  0.24,  0.08], // 6
        [0.30,  0.08,  0.08,  0.08,  0.18,  0.14,  0.14], // 7
    ],
};

pub const ALL_MOODS: &[&MoodSet] = &[
    &MOOD_CALM, &MOOD_TENSE, &MOOD_DARK, &MOOD_EUPHORIC,
    &MOOD_COSMIC, &MOOD_GRAVITY,
];
pub const N_MOODS: usize = 6;

// ---------------------------------------------------------------------------
// Phrase-boundary harmonic matrix (wider jumps allowed)
// ---------------------------------------------------------------------------

pub const PHRASE_BOUNDARY_HARMONIC: HarmonicMatrix = [
    //  I      ii     iii    IV     V      vi     vii
    [0.10,  0.10,  0.10,  0.15,  0.15,  0.25,  0.15], // I   → vi (relative shift)
    [0.10,  0.05,  0.10,  0.15,  0.25,  0.25,  0.10], // ii
    [0.10,  0.10,  0.05,  0.20,  0.15,  0.25,  0.15], // iii
    [0.15,  0.10,  0.10,  0.05,  0.20,  0.25,  0.15], // IV
    [0.25,  0.10,  0.10,  0.15,  0.05,  0.25,  0.10], // V
    [0.25,  0.15,  0.10,  0.15,  0.15,  0.10,  0.10], // vi → I (relative shift)
    [0.20,  0.10,  0.10,  0.15,  0.20,  0.15,  0.10], // vii
];

// ---------------------------------------------------------------------------
// VoiceRole
// ---------------------------------------------------------------------------

/// Role of a voice in the ensemble. Determines register and melodic degree bias.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VoiceRole {
    #[default]
    Bass    = 0,
    Pad     = 1,
    Melody  = 2,
    Texture = 3,
}

impl VoiceRole {
    /// MIDI note range [low, high] for this role.
    pub fn register(self) -> (u8, u8) {
        match self {
            Self::Bass    => (24, 47),
            Self::Pad     => (48, 71),
            Self::Melody  => (60, 83),
            Self::Texture => (72, 95),
        }
    }

    /// Multiplier applied to each chord tone after role bias, to pull notes
    /// toward the active harmony. Higher = more chord-locked.
    pub fn chord_attraction(self) -> f32 {
        match self {
            Self::Bass    => 3.5, // almost always root/fifth
            Self::Pad     => 2.5, // chord tones strongly preferred
            Self::Melody  => 1.6, // mild pull; passing tones still happen
            Self::Texture => 1.2, // mostly free, slight harmonic gravity
        }
    }

    /// Per-degree bias multipliers (index = scale degree 0-based).
    /// 0.0 = forbidden, 1.0 = neutral, >1.0 = preferred.
    pub fn degree_bias(self) -> [f32; MELODIC_STATES] {
        match self {
            Self::Bass    => [3.0, 0.8, 0.0, 0.0, 2.5, 0.0, 0.0],
            Self::Pad     => [2.0, 0.5, 2.0, 0.3, 2.0, 0.5, 0.3],
            Self::Melody  => [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            Self::Texture => [0.2, 1.5, 1.2, 0.5, 0.5, 2.0, 1.8],
        }
    }

    /// How many subdivisions between rhythmic steps (1 = every subdiv, 2 = every other).
    pub fn rhythmic_divisor(self) -> u32 {
        match self {
            Self::Bass    => 2,
            Self::Pad     => 2,
            Self::Melody  => 1,
            Self::Texture => 4,
        }
    }

    pub const ALL: &'static [Self] = &[Self::Bass, Self::Pad, Self::Melody, Self::Texture];
    pub const LABELS: &'static [&'static str] = &["Bass", "Pad", "Melody", "Texture"];

    pub fn from_u8(v: u8) -> Self {
        match v { 1 => Self::Pad, 2 => Self::Melody, 3 => Self::Texture, _ => Self::Bass }
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
        Self { state: HarmonicFunction::Tonic, rng: Lcg::new(seed) }
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
        Self { state: RhythmicState::Rest, rng: Lcg::new(seed), subdiv_counter: 0 }
    }

    /// Call once per subdivision. Returns the new state only when the role's
    /// divisor threshold is reached (i.e. not every subdivision triggers a step).
    /// Returns `None` when this subdivision is skipped for this role.
    pub fn on_subdivision(
        &mut self,
        matrix: &RhythmicMatrix,
        density: f32,
        role: VoiceRole,
    ) -> Option<RhythmicState> {
        self.subdiv_counter += 1;
        if self.subdiv_counter < role.rhythmic_divisor() {
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
        Self { degree: start_degree % MELODIC_STATES, rng: Lcg::new(seed) }
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
            if total > 0.0 { for v in &mut attracted { *v /= total; } }

            self.degree = self.rng.sample_row(&attracted);
        }

        // Resolve degree to MIDI, clamped to role register
        let raw = scale.degree_to_midi(root, self.degree, octave_offset);
        let (lo, hi) = role.register();

        // If out of register, shift by octaves
        let mut midi = raw;
        while midi < lo && midi + 12 <= hi { midi += 12; }
        while midi > hi && midi >= lo + 12 { midi -= 12; }

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
        Self { bar: 0, bars_per_phrase: bars_per_phrase.max(1), bars_in_phrase: 0 }
    }

    /// Call when `BeatEvents::bar` fires.
    pub fn on_bar(&mut self) -> PhraseEvents {
        self.bar += 1;
        self.bars_in_phrase += 1;
        let phrase_boundary = self.bars_in_phrase >= self.bars_per_phrase;
        if phrase_boundary { self.bars_in_phrase = 0; }
        PhraseEvents { new_bar: true, phrase_boundary }
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

impl Default for MoodBlend { fn default() -> Self { Self::new() } }

fn scale_harmonic(m: &HarmonicMatrix, w: f32) -> HarmonicMatrix {
    let mut out = [[0.0f32; HARMONIC_STATES]; HARMONIC_STATES];
    for r in 0..HARMONIC_STATES { for c in 0..HARMONIC_STATES { out[r][c] = m[r][c] * w; } }
    out
}
fn scale_rhythmic(m: &RhythmicMatrix, w: f32) -> RhythmicMatrix {
    let mut out = [[0.0f32; RHYTHMIC_STATES]; RHYTHMIC_STATES];
    for r in 0..RHYTHMIC_STATES { for c in 0..RHYTHMIC_STATES { out[r][c] = m[r][c] * w; } }
    out
}
fn scale_melodic(m: &MelodicMatrix, w: f32) -> MelodicMatrix {
    let mut out = [[0.0f32; MELODIC_STATES]; MELODIC_STATES];
    for r in 0..MELODIC_STATES { for c in 0..MELODIC_STATES { out[r][c] = m[r][c] * w; } }
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
    pub root:    Arc<AtomicU8>,
    /// Scale (Scale enum as u8).
    pub scale:   Arc<AtomicU8>,
    /// Mood blend weights.
    pub mood:    MoodBlend,
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
    /// 0 = voices wander by mood matrix only; 1 = strongly locked to chord tones.
    pub chord_attraction: Shared,
    /// When true, Bass voices snap to the chord root on each bar downbeat.
    pub bass_lock: Arc<std::sync::atomic::AtomicBool>,
    /// When true, dissonant intervals between simultaneous note-ons are nudged apart.
    pub dissonance_resolve: Arc<std::sync::atomic::AtomicBool>,
    /// How aggressively dissonance is resolved.
    /// 0 = semitones only, 1 = semitones + tritones, 2 = semitones + tritones + minor 7ths.
    pub dissonance_threshold: Arc<AtomicU8>,
    /// Per-phrase register drift probability override (0.0–1.0).
    /// 0 = no drift; 1 = always drift each phrase. Overrides mood-derived probability.
    pub register_drift: Shared,
}

impl MarkovEngineShared {
    pub fn new(n_voices: usize) -> Self {
        Self {
            root:    Arc::new(AtomicU8::new(60)), // C4
            scale:   Arc::new(AtomicU8::new(Scale::Minor as u8)),
            mood:    MoodBlend::new(),
            density: shared(0.5),
            bars_per_phrase: Arc::new(std::sync::atomic::AtomicU32::new(4)),
            roles:         (0..n_voices).map(|i| Arc::new(AtomicU8::new(match i % 4 {
                0 => VoiceRole::Bass as u8,
                1 => VoiceRole::Pad as u8,
                2 => VoiceRole::Melody as u8,
                _ => VoiceRole::Texture as u8,
            }))).collect(),
            voice_density:  (0..n_voices).map(|_| shared(0.0)).collect(),
            voice_enabled:  (0..n_voices).map(|_| Arc::new(std::sync::atomic::AtomicBool::new(true))).collect(),
            voice_octave:   (0..n_voices).map(|_| Arc::new(AtomicU8::new(64))).collect(), // 64 = offset 0
            launchpad: Arc::new(
                (0..n_voices).map(|_| std::array::from_fn(|_| AtomicU8::new(0))).collect()
            ),
            launchpad_col: Arc::new(AtomicUsize::new(0)),
            chord_attraction:     shared(0.5),
            bass_lock:            Arc::new(std::sync::atomic::AtomicBool::new(true)),
            dissonance_resolve:   Arc::new(std::sync::atomic::AtomicBool::new(true)),
            dissonance_threshold: Arc::new(AtomicU8::new(1)), // semitones + tritones
            register_drift:       shared(0.2),
        }
    }

    pub fn root(&self) -> u8 { self.root.load(Ordering::Relaxed) }
    pub fn scale(&self) -> Scale { Scale::from_u8(self.scale.load(Ordering::Relaxed)) }
    pub fn density(&self) -> f32 { self.density.value() }
    pub fn chord_attraction(&self) -> f32 { self.chord_attraction.value().clamp(0.0, 1.0) }
    pub fn bass_lock(&self) -> bool { self.bass_lock.load(Ordering::Relaxed) }
    pub fn dissonance_resolve(&self) -> bool { self.dissonance_resolve.load(Ordering::Relaxed) }
    pub fn dissonance_threshold(&self) -> u8 { self.dissonance_threshold.load(Ordering::Relaxed) }
    pub fn register_drift(&self) -> f32 { self.register_drift.value().clamp(0.0, 1.0) }

    pub fn role(&self, i: usize) -> VoiceRole {
        VoiceRole::from_u8(self.roles[i].load(Ordering::Relaxed))
    }

    pub fn voice_density(&self, i: usize) -> f32 {
        let vd = self.voice_density[i].value();
        if vd > 0.0 { vd } else { self.density() }
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
}

// ---------------------------------------------------------------------------
// MarkovVoice — one voice, audio thread only
// ---------------------------------------------------------------------------

/// Audio-thread-only state for one voice.
pub struct MarkovVoice {
    pub rhythmic: RhythmicChain,
    pub melodic:  MelodicChain,
    pub current_note: Option<u8>,
    /// Set by on_bar for Bass voices; consumed on the next attack to force root snap.
    pending_root_snap: bool,
    /// Per-phrase register drift: added to octave_offset from shared (-1, 0, or +1).
    pub octave_drift: i8,
}

/// Events emitted by a single voice per subdivision.
#[derive(Clone, Copy, Debug, Default)]
pub struct VoiceEvent {
    pub note_on:  Option<u8>,
    pub note_off: Option<u8>,
    /// True if this was an Accent (caller can use for velocity).
    pub accent:   bool,
    /// True if this was a Double (caller should fire two rapid NoteOns).
    pub double:   bool,
    /// The rhythmic state that produced this event (used by Launchpad display).
    pub rhythmic: RhythmicState,
}

impl MarkovVoice {
    pub fn new(seed: u64) -> Self {
        Self {
            rhythmic: RhythmicChain::new(seed),
            melodic:  MelodicChain::new(seed ^ 0xDEAD_BEEF, 0),
            current_note: None,
            pending_root_snap: false,
            octave_drift: 0,
        }
    }

    /// Call once per subdivision. Returns a `VoiceEvent` with any note changes.
    pub fn on_subdivision(
        &mut self,
        rhythmic_matrix: &RhythmicMatrix,
        melodic_matrix:  &MelodicMatrix,
        harmonic:        &HarmonicChain,
        shared:          &MarkovEngineShared,
        voice_idx:       usize,
    ) -> VoiceEvent {
        if !shared.voice_enabled(voice_idx) {
            let note_off = self.current_note.take();
            return VoiceEvent { note_off, ..Default::default() };
        }

        let role    = shared.role(voice_idx);
        let density = shared.voice_density(voice_idx);

        let Some(rhythmic_state) = self.rhythmic.on_subdivision(rhythmic_matrix, density, role)
        else {
            return VoiceEvent::default();
        };

        let mut ev = VoiceEvent { rhythmic: rhythmic_state, ..Default::default() };

        match rhythmic_state {
            RhythmicState::Rest => {
                // Fire note_off if something was sounding.
                ev.note_off = self.current_note.take();
            }
            RhythmicState::Hold => {
                // Continue sounding — no change.
            }
            RhythmicState::Single | RhythmicState::Double | RhythmicState::Accent => {
                // Release previous note.
                ev.note_off = self.current_note.take();

                // Bass voices snap to chord root on downbeats.
                let force_root = self.pending_root_snap && role == VoiceRole::Bass;
                self.pending_root_snap = false;

                // Advance melodic chain (combine shared octave offset with phrase drift).
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
                ev.note_on = Some(pitch);
                ev.accent  = rhythmic_state == RhythmicState::Accent;
                ev.double  = rhythmic_state == RhythmicState::Double;
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
    pub voices:   Vec<MarkovVoice>,
    pub harmonic: HarmonicChain,
    pub phrase:   PhraseCounter,
    moods:        [&'static MoodSet; N_MOODS],
    /// RNG used for phrase-level decisions (register drift).
    phrase_rng:   Lcg,
}

impl MarkovEngine {
    pub fn new(n_voices: usize, seed: u64) -> Self {
        Self {
            voices:      (0..n_voices).map(|i| MarkovVoice::new(seed ^ (i as u64 * 0x1111_1111))).collect(),
            harmonic:    HarmonicChain::new(seed ^ 0xFEED_FACE),
            phrase:      PhraseCounter::new(4),
            moods:       [&MOOD_CALM, &MOOD_TENSE, &MOOD_DARK, &MOOD_EUPHORIC, &MOOD_COSMIC, &MOOD_GRAVITY],
            phrase_rng:  Lcg::new(seed ^ 0xA5A5_B6B6),
        }
    }

    /// Call once per subdivision (when `BeatEvents::subdivision` fires).
    /// Returns one `VoiceEvent` per voice.
    pub fn on_subdivision(&mut self, shared: &MarkovEngineShared) -> Vec<VoiceEvent> {
        let rhythmic = shared.mood.blend_rhythmic(&self.moods);
        let melodic  = shared.mood.blend_melodic(&self.moods);

        let mut events: Vec<VoiceEvent> = self.voices
            .iter_mut()
            .enumerate()
            .map(|(i, v)| v.on_subdivision(&rhythmic, &melodic, &self.harmonic, shared, i))
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
            self.harmonic.advance(&PHRASE_BOUNDARY_HARMONIC);
        } else {
            self.harmonic.advance(&harmonic_matrix);
        }

        // Flag Bass voices to snap to root on their next attack (if bass_lock enabled).
        if shared.bass_lock() {
            for (i, voice) in self.voices.iter_mut().enumerate() {
                if shared.role(i) == VoiceRole::Bass {
                    voice.pending_root_snap = true;
                }
            }
        }

        // ── Per-phrase register drift ─────────────────────────────────────────
        // Bass excluded. Probability = register_drift knob (0=off, 1=always).
        if phrase_ev.phrase_boundary {
            let drift_prob = shared.register_drift();
            if drift_prob > 0.0 {
                for (i, voice) in self.voices.iter_mut().enumerate() {
                    if shared.role(i) == VoiceRole::Bass { continue; }
                    let r = (self.phrase_rng.next_u32() as f32) / (u32::MAX as f32);
                    if r < drift_prob {
                        let dir: i8 = if self.phrase_rng.next_u32() & 1 == 0 { 1 } else { -1 };
                        voice.octave_drift = (voice.octave_drift + dir).clamp(-1, 1);
                    }
                }
            }
        }

        phrase_ev
    }

    pub fn n_voices(&self) -> usize { self.voices.len() }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine(n: usize) -> (MarkovEngine, MarkovEngineShared) {
        (MarkovEngine::new(n, 0xABCD_1234), MarkovEngineShared::new(n))
    }

    #[test]
    fn harmonic_chain_advances() {
        let mut chain = HarmonicChain::new(42);
        let initial = chain.state;
        // After many advances, state should have changed at least once.
        let mut changed = false;
        for _ in 0..100 {
            chain.advance(&MOOD_CALM.harmonic);
            if chain.state != initial { changed = true; break; }
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
            if let Some(st) = chain.on_subdivision(&MOOD_CALM.rhythmic, 0.0, VoiceRole::Melody) {
                if st == RhythmicState::Rest { rest_count += 1; }
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
            if let Some(st) = chain.on_subdivision(&MOOD_CALM.rhythmic, 1.0, VoiceRole::Melody) {
                if st == RhythmicState::Rest { rest_count += 1; }
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
                60, Scale::Major, 0, false, 0.5,
            );
            let (lo, hi) = VoiceRole::Melody.register();
            assert!(pitch >= lo && pitch <= hi,
                "pitch {pitch} out of melody register {lo}-{hi}");
        }
    }

    #[test]
    fn bass_role_avoids_forbidden_degrees() {
        // Bias for Bass: degrees 2 (index 2) and 3 (index 3) have bias 0.0.
        let bias = VoiceRole::Bass.degree_bias();
        assert_eq!(bias[2], 0.0, "degree 3 (index 2) should be forbidden for bass");
        assert_eq!(bias[3], 0.0, "degree 4 (index 3) should be forbidden for bass");
    }

    #[test]
    fn phrase_counter_fires_at_boundary() {
        let mut counter = PhraseCounter::new(4);
        let mut fired = false;
        for _ in 0..4 {
            let ev = counter.on_bar();
            if ev.phrase_boundary { fired = true; }
        }
        assert!(fired, "phrase boundary should fire after 4 bars");
    }

    #[test]
    fn mood_blend_uniform_gives_average() {
        let blend = MoodBlend::new();
        blend.set(&[0.166, 0.166, 0.166, 0.166, 0.166, 0.17]);
        let moods = [&MOOD_CALM, &MOOD_TENSE, &MOOD_DARK, &MOOD_EUPHORIC, &MOOD_COSMIC, &MOOD_GRAVITY];
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
            for ev in &evs { if ev.note_on.is_some() { total_note_ons += 1; } }
        }
        assert!(total_note_ons > 0, "at least some note_ons expected over 64 subdivisions");
    }

    #[test]
    fn engine_bar_advances_harmonic() {
        let (mut eng, shared) = make_engine(2);
        let initial = eng.harmonic.state;
        let mut changed = false;
        for _ in 0..32 {
            eng.on_bar(&shared);
            if eng.harmonic.state != initial { changed = true; break; }
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
}
