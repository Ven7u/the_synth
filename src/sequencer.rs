//! Sequencer and chord keyboard state + helpers.
//!
//! Three independent modes, each with its own state struct:
//!   NoteSeqState  — step sequencer with per-step chromatic note
//!   ChordSeqState — step sequencer with per-step diatonic chord
//!   ChordKbState  — live chord keyboard (no sequencer, mouse/click)
//!
//! Shared timing (BPM, current step, last tick) lives on SynthApp.

// ---------------------------------------------------------------------------
// Scale
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ScaleType {
    Major,
    Minor,
}

impl ScaleType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Major => "Major",
            Self::Minor => "Minor",
        }
    }
}

/// Semitone intervals for each scale degree (0–6) relative to root.
pub fn scale_intervals(scale: ScaleType) -> [u8; 7] {
    match scale {
        ScaleType::Major => [0, 2, 4, 5, 7, 9, 11],
        ScaleType::Minor => [0, 2, 3, 5, 7, 8, 10],
    }
}

/// Roman numeral label for a scale degree (0-indexed).
pub const DEGREE_LABELS: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];

/// Chord quality suffix for each degree in Major/Minor.
pub fn chord_quality(scale: ScaleType, degree: usize) -> &'static str {
    match scale {
        ScaleType::Major => match degree % 7 {
            0 => "",  // I   — major
            1 => "m", // II  — minor
            2 => "m", // III — minor
            3 => "",  // IV  — major
            4 => "",  // V   — major
            5 => "m", // VI  — minor
            6 => "°", // VII — diminished
            _ => "",
        },
        ScaleType::Minor => match degree % 7 {
            0 => "m", // I   — minor
            1 => "°", // II  — diminished
            2 => "",  // III — major
            3 => "m", // IV  — minor
            4 => "m", // V   — minor
            5 => "",  // VI  — major
            6 => "",  // VII — major
            _ => "",
        },
    }
}

pub const NOTE_NAMES: [&str; 12] = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];

/// Display name for a chord: root note + quality (e.g. "Cm", "F", "B°").
pub fn chord_name(root: u8, scale: ScaleType, degree: usize) -> String {
    let intervals = scale_intervals(scale);
    let note_idx = (root as usize + intervals[degree % 7] as usize) % 12;
    format!("{}{}", NOTE_NAMES[note_idx], chord_quality(scale, degree))
}

/// Compute the 3 MIDI notes for a triad.
/// `root`: MIDI semitone of root (0=C, 1=C#, …).
/// `degree`: 0–6 scale degree.
/// `octave`: base octave (4 = middle octave, so C4 = MIDI 60 when root=0).
pub fn chord_notes(root: u8, scale: ScaleType, degree: usize, octave: i32) -> [u8; 3] {
    let intervals = scale_intervals(scale);
    let base = root as i32 + octave * 12;
    let n = |d: usize| -> u8 {
        let oct_bump = (d / 7) as i32;
        let semitone = base + intervals[d % 7] as i32 + oct_bump * 12;
        semitone.clamp(0, 127) as u8
    };
    [n(degree), n(degree + 2), n(degree + 4)]
}

// ---------------------------------------------------------------------------
// Note sequencer state
// ---------------------------------------------------------------------------

pub struct NoteSeqState {
    pub steps: [bool; 24],
    pub notes: [u8; 24],
    pub length: usize,
    pub drag_accum: [f32; 24],
}

impl NoteSeqState {
    pub fn new() -> Self {
        let mut steps = [false; 24];
        let mut notes = [60u8; 24];
        for (i, &v) in [true, false, true, false, true, true, false, true].iter().enumerate() {
            steps[i] = v;
        }
        for (i, &v) in [60u8, 62, 64, 67, 69, 72, 67, 64].iter().enumerate() {
            notes[i] = v;
        }
        Self { steps, notes, length: 8, drag_accum: [0.0; 24] }
    }
}

// ---------------------------------------------------------------------------
// Chord sequencer state
// ---------------------------------------------------------------------------

pub struct ChordSeqState {
    pub steps: [bool; 24],
    pub degrees: [usize; 24], // 0–6 diatonic degree per step
    pub length: usize,
    pub drag_accum: [f32; 24],
    pub root: u8,        // 0=C … 11=B
    pub scale: ScaleType,
    pub octave: i32,     // base octave for chord voicing
}

impl ChordSeqState {
    pub fn new() -> Self {
        let mut degrees = [0usize; 24];
        // Default: I IV V IV I V VI IV — classic pop progression for 8 steps
        for (i, &d) in [0usize, 3, 4, 3, 0, 4, 5, 3].iter().enumerate() {
            degrees[i] = d;
        }
        Self {
            steps: {
                let mut a = [false; 24];
                for i in 0..8 { a[i] = true; }
                a
            },
            degrees,
            length: 8,
            drag_accum: [0.0; 24],
            root: 0,          // C
            scale: ScaleType::Major,
            octave: 4,
        }
    }

    /// Notes for step i.
    pub fn step_notes(&self, i: usize) -> [u8; 3] {
        chord_notes(self.root, self.scale, self.degrees[i], self.octave)
    }
}

// ---------------------------------------------------------------------------
// Chord keyboard state
// ---------------------------------------------------------------------------

pub struct ChordKbState {
    pub root: u8,
    pub scale: ScaleType,
    pub octave: i32,
    /// Which degree is currently held by the mouse (None = nothing held).
    pub held_degree: Option<usize>,
    /// Degrees currently held by keyboard keys (supports polyphonic chord playing).
    pub kb_held: std::collections::HashSet<usize>,
}

impl ChordKbState {
    pub fn new() -> Self {
        Self {
            root: 0,
            scale: ScaleType::Major,
            octave: 4,
            held_degree: None,
            kb_held: std::collections::HashSet::new(),
        }
    }

    pub fn chord_notes(&self, degree: usize) -> [u8; 3] {
        chord_notes(self.root, self.scale, degree, self.octave)
    }
}

// ---------------------------------------------------------------------------
// Mode selector
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum SeqMode {
    NoteSeq,
    ChordSeq,
    ChordKb,
}

impl SeqMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoteSeq  => "Note Seq",
            Self::ChordSeq => "Chord Seq",
            Self::ChordKb  => "Chord KB",
        }
    }
}
