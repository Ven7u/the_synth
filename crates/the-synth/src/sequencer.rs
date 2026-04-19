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
// Chord types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ChordType {
    Triad,   // 1-3-5
    Maj7,    // 1-3-5-7 (major seventh)
    Min7,    // 1-b3-5-b7
    Dom7,    // 1-3-5-b7
    Sus2,    // 1-2-5
    Sus4,    // 1-4-5
    Add9,    // 1-3-5-9
    Power,   // 1-5
}

impl ChordType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Triad => "Triad",
            Self::Maj7  => "Maj7",
            Self::Min7  => "Min7",
            Self::Dom7  => "Dom7",
            Self::Sus2  => "Sus2",
            Self::Sus4  => "Sus4",
            Self::Add9  => "Add9",
            Self::Power => "Power",
        }
    }

    pub fn all() -> &'static [ChordType] {
        &[Self::Triad, Self::Maj7, Self::Min7, Self::Dom7,
          Self::Sus2, Self::Sus4, Self::Add9, Self::Power]
    }
}

/// Compute MIDI notes for a pad config. Returns up to 4 notes (unused slots = 255).
pub fn chord_notes_typed(root: u8, scale: ScaleType, degree: usize, octave: i32, chord_type: ChordType) -> Vec<u8> {
    let intervals = scale_intervals(scale);
    let base = root as i32 + octave * 12;

    // Scale degree root (with wrapping octave)
    let oct_bump = (degree / 7) as i32;
    let deg_root = base + intervals[degree % 7] as i32 + oct_bump * 12;

    // Is this scale degree minor or diminished?
    let quality = chord_quality(scale, degree);
    let is_minor = quality == "m" || quality == "°";

    let clamp = |s: i32| s.clamp(0, 127) as u8;

    match chord_type {
        ChordType::Triad => {
            // Use diatonic triad (existing logic)
            let n = |d: usize| -> u8 {
                let ob = (d / 7) as i32;
                clamp(base + intervals[d % 7] as i32 + ob * 12)
            };
            vec![n(degree), n(degree + 2), n(degree + 4)]
        }
        ChordType::Maj7 => vec![
            clamp(deg_root),
            clamp(deg_root + 4),
            clamp(deg_root + 7),
            clamp(deg_root + 11),
        ],
        ChordType::Min7 => vec![
            clamp(deg_root),
            clamp(deg_root + 3),
            clamp(deg_root + 7),
            clamp(deg_root + 10),
        ],
        ChordType::Dom7 => vec![
            clamp(deg_root),
            clamp(deg_root + 4),
            clamp(deg_root + 7),
            clamp(deg_root + 10),
        ],
        ChordType::Sus2 => vec![
            clamp(deg_root),
            clamp(deg_root + 2),
            clamp(deg_root + 7),
        ],
        ChordType::Sus4 => vec![
            clamp(deg_root),
            clamp(deg_root + 5),
            clamp(deg_root + 7),
        ],
        ChordType::Add9 => vec![
            clamp(deg_root),
            clamp(deg_root + if is_minor { 3 } else { 4 }),
            clamp(deg_root + 7),
            clamp(deg_root + 14),
        ],
        ChordType::Power => vec![
            clamp(deg_root),
            clamp(deg_root + 7),
        ],
    }
}

/// Per-pad configuration in the chord keyboard grid.
#[derive(Clone, Copy)]
pub struct PadConfig {
    pub chord_type: ChordType,
    pub custom_root: Option<u8>, // None = follow scale degree
}

impl PadConfig {
    pub fn new(chord_type: ChordType) -> Self {
        Self { chord_type, custom_root: None }
    }
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

pub const CHORD_KB_ROWS: usize = 3;
pub const CHORD_KB_COLS: usize = 7;

/// Default chord type for each row.
fn default_row_chord_type(row: usize) -> ChordType {
    match row {
        0 => ChordType::Dom7,
        1 => ChordType::Triad,
        2 => ChordType::Sus2,
        _ => ChordType::Triad,
    }
}

pub struct ChordKbState {
    pub root: u8,
    pub scale: ScaleType,
    pub octave: i32,
    /// 3×7 grid of pad configs.
    pub pads: [[PadConfig; CHORD_KB_COLS]; CHORD_KB_ROWS],
    /// (row, col) held by mouse, if any.
    pub held_pad: Option<(usize, usize)>,
    /// (row, col) pads held by keyboard keys.
    pub kb_held: std::collections::HashSet<(usize, usize)>,
    /// Edit mode: show chord-type picker on click.
    pub edit_mode: bool,
    /// Which pad's popover is open (row, col).
    pub editing_pad: Option<(usize, usize)>,
    /// Show the piano preview strip below the grid.
    pub show_piano_preview: bool,
}

impl ChordKbState {
    pub fn new() -> Self {
        let pads = std::array::from_fn(|row| {
            std::array::from_fn(|_col| PadConfig::new(default_row_chord_type(row)))
        });
        Self {
            root: 0,
            scale: ScaleType::Major,
            octave: 4,
            pads,
            held_pad: None,
            kb_held: std::collections::HashSet::new(),
            edit_mode: false,
            editing_pad: None,
            show_piano_preview: true,
        }
    }

    pub fn chord_notes_for(&self, row: usize, col: usize) -> Vec<u8> {
        let pad = &self.pads[row][col];
        chord_notes_typed(self.root, self.scale, col, self.octave, pad.chord_type)
    }

    /// Reset a row's chord types to defaults (called when scale changes).
    pub fn reset_row(&mut self, row: usize) {
        let ct = default_row_chord_type(row);
        for col in 0..CHORD_KB_COLS {
            self.pads[row][col].chord_type = ct;
        }
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
