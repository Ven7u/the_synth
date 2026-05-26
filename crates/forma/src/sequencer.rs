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
    Dorian,
    Pentatonic,
    PentatonicMinor,
}

impl ScaleType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Major => "Major",
            Self::Minor => "Minor",
            Self::Dorian => "Dorian",
            Self::Pentatonic => "Pent",
            Self::PentatonicMinor => "Pent m",
        }
    }

    /// All variants, in order for UI rendering.
    pub fn all_highlight() -> &'static [ScaleType] {
        &[
            ScaleType::Major,
            ScaleType::Minor,
            ScaleType::Dorian,
            ScaleType::Pentatonic,
            ScaleType::PentatonicMinor,
        ]
    }
}

/// Semitone intervals for each scale degree (0–6) relative to root.
/// Only valid for diatonic (7-note) scales used by the chord keyboard.
pub fn scale_intervals(scale: ScaleType) -> [u8; 7] {
    match scale {
        ScaleType::Major => [0, 2, 4, 5, 7, 9, 11],
        ScaleType::Minor | ScaleType::Dorian => [0, 2, 3, 5, 7, 8, 10],
        ScaleType::Pentatonic => [0, 2, 4, 7, 9, 11, 0],
        ScaleType::PentatonicMinor => [0, 3, 5, 7, 10, 12, 0],
    }
}

/// Semitone offsets (relative to root) for every tone in the scale.
pub fn scale_tones(scale: ScaleType) -> &'static [u8] {
    match scale {
        ScaleType::Major => &[0, 2, 4, 5, 7, 9, 11],
        ScaleType::Minor => &[0, 2, 3, 5, 7, 8, 10],
        ScaleType::Dorian => &[0, 2, 3, 5, 7, 9, 10],
        ScaleType::Pentatonic => &[0, 2, 4, 7, 9],
        ScaleType::PentatonicMinor => &[0, 3, 5, 7, 10],
    }
}

/// Returns a 12-element bool array: `arr[pitch_class]` is true if that pitch class is in the scale.
pub fn scale_pitch_classes(scale: ScaleType, root: u8) -> [bool; 12] {
    let mut arr = [false; 12];
    for &t in scale_tones(scale) {
        arr[((root as u16 + t as u16) % 12) as usize] = true;
    }
    arr
}

/// Roman numeral label for a scale degree (0-indexed).
pub const DEGREE_LABELS: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];

impl SeqMode {
    pub fn to_u8(self) -> u8 {
        match self {
            Self::NoteSeq => 0,
            Self::ChordSeq => 1,
            Self::ChordKb => 2,
        }
    }
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::ChordSeq,
            2 => Self::ChordKb,
            _ => Self::NoteSeq,
        }
    }
}

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
        ScaleType::Minor
        | ScaleType::Dorian
        | ScaleType::Pentatonic
        | ScaleType::PentatonicMinor => match degree % 7 {
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

pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

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
// Voicing
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum VoicingType {
    #[default]
    Root, // close position, root in bass
    First,  // 1st inversion — 3rd in bass
    Second, // 2nd inversion — 5th in bass
    Open,   // spread: root + 5th + 3rd (octave up)
    Full,   // root-position + root doubled an octave above (denser, 4–5 notes)
}

impl VoicingType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Root => "Root",
            Self::First => "1st",
            Self::Second => "2nd",
            Self::Open => "Open",
            Self::Full => "Full",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::Root => "R",
            Self::First => "1",
            Self::Second => "2",
            Self::Open => "O",
            Self::Full => "F",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Root => Self::First,
            Self::First => Self::Second,
            Self::Second => Self::Open,
            Self::Open => Self::Full,
            Self::Full => Self::Root,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Root => Self::Full,
            Self::First => Self::Root,
            Self::Second => Self::First,
            Self::Open => Self::Second,
            Self::Full => Self::Open,
        }
    }

    pub fn all() -> &'static [VoicingType] {
        &[
            Self::Root,
            Self::First,
            Self::Second,
            Self::Open,
            Self::Full,
        ]
    }
}

/// Apply a voicing to a set of root-position notes (ascending pitch order).
/// Returns the re-voiced notes in the same count.
pub fn apply_voicing(mut notes: Vec<u8>, voicing: VoicingType) -> Vec<u8> {
    let n = notes.len();
    if n < 2 {
        return notes;
    }
    match voicing {
        VoicingType::Root => notes,
        VoicingType::First => {
            let first = notes.remove(0);
            notes.push((first as i32 + 12).clamp(0, 127) as u8);
            notes
        }
        VoicingType::Second => {
            if n < 3 {
                return notes;
            }
            let first = notes.remove(0);
            notes.push((first as i32 + 12).clamp(0, 127) as u8);
            let second = notes.remove(0);
            notes.push((second as i32 + 12).clamp(0, 127) as u8);
            notes
        }
        VoicingType::Open => {
            // Take index 1 (3rd), move it to the top an octave up: R–5–3↑
            let third = notes.remove(1);
            notes.push((third as i32 + 12).clamp(0, 127) as u8);
            notes
        }
        VoicingType::Full => {
            // Root-position + root doubled an octave above: R–3–5–(7)–R↑
            let root_up = (notes[0] as i32 + 12).clamp(0, 127) as u8;
            notes.push(root_up);
            notes
        }
    }
}

/// Pick the inversion of `notes` (root-position, ascending) that minimises
/// total semitone movement from `prev`.  Falls back to root position if
/// `prev` is empty.
pub fn best_voiced(notes: Vec<u8>, prev: &[u8]) -> Vec<u8> {
    if prev.is_empty() || notes.is_empty() {
        return notes;
    }
    let n = notes.len();
    let score = |candidate: &Vec<u8>| -> u32 {
        let len = candidate.len().min(prev.len());
        let mut a: Vec<u8> = candidate[..len].to_vec();
        let mut b: Vec<u8> = prev[..len].to_vec();
        a.sort_unstable();
        b.sort_unstable();
        a.iter()
            .zip(b.iter())
            .map(|(&x, &y)| x.abs_diff(y) as u32)
            .sum()
    };
    // Generate all n inversions
    let mut best = notes.clone();
    let mut best_score = score(&best);
    let mut current = notes;
    for _ in 1..n {
        current = apply_voicing(current, VoicingType::First);
        let s = score(&current);
        if s < best_score {
            best_score = s;
            best = current.clone();
        }
    }
    best
}

// ---------------------------------------------------------------------------
// Chord types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ChordType {
    Triad, // 1-3-5
    Maj7,  // 1-3-5-7 (major seventh)
    Min7,  // 1-b3-5-b7
    Dom7,  // 1-3-5-b7
    Sus2,  // 1-2-5
    Sus4,  // 1-4-5
    Add9,  // 1-3-5-9
    Power, // 1-5
}

impl ChordType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Triad => "Triad",
            Self::Maj7 => "Maj7",
            Self::Min7 => "Min7",
            Self::Dom7 => "Dom7",
            Self::Sus2 => "Sus2",
            Self::Sus4 => "Sus4",
            Self::Add9 => "Add9",
            Self::Power => "Power",
        }
    }

    pub fn all() -> &'static [ChordType] {
        &[
            Self::Triad,
            Self::Maj7,
            Self::Min7,
            Self::Dom7,
            Self::Sus2,
            Self::Sus4,
            Self::Add9,
            Self::Power,
        ]
    }
}

/// Compute MIDI notes for a pad config. Returns up to 4 notes (unused slots = 255).
pub fn chord_notes_typed(
    root: u8,
    scale: ScaleType,
    degree: usize,
    octave: i32,
    chord_type: ChordType,
) -> Vec<u8> {
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
        ChordType::Sus2 => vec![clamp(deg_root), clamp(deg_root + 2), clamp(deg_root + 7)],
        ChordType::Sus4 => vec![clamp(deg_root), clamp(deg_root + 5), clamp(deg_root + 7)],
        ChordType::Add9 => vec![
            clamp(deg_root),
            clamp(deg_root + if is_minor { 3 } else { 4 }),
            clamp(deg_root + 7),
            clamp(deg_root + 14),
        ],
        ChordType::Power => vec![clamp(deg_root), clamp(deg_root + 7)],
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
        Self {
            chord_type,
            custom_root: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Note sequencer state
// ---------------------------------------------------------------------------

pub struct NoteSeqState {
    pub steps: [bool; 24],
    pub notes: [u8; 24],
    pub velocities: [u8; 24],
    pub probabilities: [u8; 24],
    pub length: usize,
    pub drag_accum: [f32; 24],
}

impl NoteSeqState {
    pub fn new() -> Self {
        let mut steps = [false; 24];
        let mut notes = [60u8; 24];
        // Wish You Were Here – main arpeggio run (E3 G3 A3 G3 D4 C4 D4 E3)
        use forma_control::midi_note;
        for i in 0..8 {
            steps[i] = true;
        }
        for (i, &v) in [
            midi_note!(E, 3),
            midi_note!(G, 3),
            midi_note!(A, 3),
            midi_note!(G, 3),
            midi_note!(D, 4),
            midi_note!(C, 4),
            midi_note!(D, 4),
            midi_note!(E, 3),
        ]
        .iter()
        .enumerate()
        {
            notes[i] = v;
        }
        Self {
            steps,
            notes,
            velocities: [100u8; 24],
            probabilities: [100u8; 24],
            length: 8,
            drag_accum: [0.0; 24],
        }
    }
}

// ---------------------------------------------------------------------------
// Chord sequencer state
// ---------------------------------------------------------------------------

pub struct ChordSeqState {
    pub steps: [bool; 24],
    pub degrees: [usize; 24],         // 0–6 diatonic degree per step
    pub chord_types: [ChordType; 24], // per-step chord type
    pub voicings: [VoicingType; 24],  // per-step voicing (inversion / spread)
    pub octave_offsets: [i32; 24],    // per-step octave shift (−2 to +2)
    pub velocities: [u8; 24],
    pub probabilities: [u8; 24],
    pub length: usize,
    pub drag_accum: [f32; 24],
    pub root: u8, // 0=C … 11=B
    pub scale: ScaleType,
    pub octave: i32,      // base octave for chord voicing
    pub voice_lead: bool, // auto-pick best inversion for smooth voice movement
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
                for i in 0..8 {
                    a[i] = true;
                }
                a
            },
            degrees,
            chord_types: [ChordType::Triad; 24],
            voicings: [VoicingType::Root; 24],
            octave_offsets: [0i32; 24],
            velocities: [100u8; 24],
            probabilities: [100u8; 24],
            length: 8,
            drag_accum: [0.0; 24],
            root: 0, // C
            scale: ScaleType::Major,
            octave: 4,
            voice_lead: false,
        }
    }

    /// Root-position notes for step i (before any voicing is applied).
    fn step_notes_root(&self, i: usize) -> Vec<u8> {
        chord_notes_typed(
            self.root,
            self.scale,
            self.degrees[i],
            self.octave + self.octave_offsets[i],
            self.chord_types[i],
        )
    }

    /// Notes for step i with per-step voicing applied.
    pub fn step_notes(&self, i: usize) -> Vec<u8> {
        apply_voicing(self.step_notes_root(i), self.voicings[i])
    }

    /// Notes for step i with voice-leading: if voice_lead is on, picks the
    /// inversion closest to `prev`; otherwise uses the stored per-step voicing.
    pub fn step_notes_vl(&self, i: usize, prev: &[u8]) -> Vec<u8> {
        let root_pos = self.step_notes_root(i);
        if self.voice_lead && !prev.is_empty() {
            best_voiced(root_pos, prev)
        } else {
            apply_voicing(root_pos, self.voicings[i])
        }
    }
}

/// Map a MIDI note to the closest scale degree (0–6) within the given root + scale.
pub fn note_to_scale_degree(midi: u8, root: u8, scale: ScaleType) -> usize {
    let intervals = scale_intervals(scale);
    let note_class = (midi % 12) as i32;
    let root_class = (root % 12) as i32;
    let mut best = 0;
    let mut best_dist = 12i32;
    for (deg, &iv) in intervals.iter().enumerate() {
        let sc = (root_class + iv as i32) % 12;
        let d = (note_class - sc).abs();
        let d = d.min(12 - d);
        if d < best_dist {
            best_dist = d;
            best = deg;
        }
    }
    best
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
            Self::NoteSeq => "Note Seq",
            Self::ChordSeq => "Chord Seq",
            Self::ChordKb => "Chord KB",
        }
    }
}

// ---------------------------------------------------------------------------
// Sequencer handle — shared state between the sequencer thread and the UI
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Per-sequencer clock division
// ---------------------------------------------------------------------------

pub struct SeqClockDiv;

impl SeqClockDiv {
    /// Human-readable labels, index matches `beats_per_step`.
    pub const LABELS: &'static [&'static str] = &[
        "1/16", "1/8", "1/4", "1/2", "1", "2", "4", "8", "16", "32", "64",
    ];

    /// Quarter-note beats per sequencer step for division index `idx`.
    pub fn beats_per_step(idx: u8) -> f64 {
        match idx {
            0 => 0.25,
            1 => 0.5,
            2 => 1.0,
            3 => 2.0,
            4 => 4.0,
            5 => 8.0,
            6 => 16.0,
            7 => 32.0,
            8 => 64.0,
            9 => 128.0,
            10 => 256.0,
            _ => 0.5,
        }
    }

    pub fn step_dur_ms(idx: u8, bpm: u32) -> u64 {
        let beats = Self::beats_per_step(idx);
        ((beats * 60_000.0) / bpm.max(1) as f64).round() as u64
    }
}

pub struct SequencerHandle {
    /// UI writes, thread reads: is the sequencer running?
    pub playing: Arc<AtomicBool>,
    /// UI writes, thread reads: BPM (eigth-note grid).
    pub bpm: Arc<AtomicU32>,
    /// UI writes, thread reads: SeqMode encoded as u8.
    pub mode: Arc<AtomicU8>,
    /// UI writes, thread reads: align arp/walker restarts to bar boundaries.
    pub bar_quantize: Arc<AtomicBool>,
    /// UI writes+reads, thread reads: note sequencer pattern.
    pub note_seq: Arc<Mutex<NoteSeqState>>,
    /// UI writes+reads, thread reads: chord sequencer pattern.
    pub chord_seq: Arc<Mutex<ChordSeqState>>,
    /// Thread writes, UI reads: current playhead step.
    pub current_step: Arc<AtomicUsize>,
    /// UI sets true, thread swaps to false and fires ArpRestart at bar boundary.
    pub arp_restart: Arc<AtomicBool>,
    /// UI sets true, thread swaps to false and fires WalkerRestart at bar boundary.
    pub walker_restart: Arc<AtomicBool>,
    /// Clock division index for NoteSeq (index into SeqClockDiv::LABELS). Default: 1 (1/8 note).
    pub note_div: Arc<AtomicU8>,
    /// Clock division index for ChordSeq (index into SeqClockDiv::LABELS). Default: 4 (1 bar).
    pub chord_div: Arc<AtomicU8>,
    /// Step-entry / live-overdub recording active.
    pub recording: Arc<AtomicBool>,
    /// Step cursor used during step-entry recording (sequencer stopped).
    pub rec_step: Arc<AtomicUsize>,
    /// Timing humanization amount 0–100. Each step fires up to ±(humanize% × step_dur/2) ms off-grid.
    pub humanize: Arc<AtomicU8>,
    /// Gate length 1–100 (% of step duration notes are held). 100 = hold until next step.
    pub gate: Arc<AtomicU8>,
}

impl SequencerHandle {
    pub fn new() -> Self {
        Self {
            playing: Arc::new(AtomicBool::new(false)),
            bpm: Arc::new(AtomicU32::new(120)),
            mode: Arc::new(AtomicU8::new(SeqMode::NoteSeq.to_u8())),
            bar_quantize: Arc::new(AtomicBool::new(false)),
            note_seq: Arc::new(Mutex::new(NoteSeqState::new())),
            chord_seq: Arc::new(Mutex::new(ChordSeqState::new())),
            current_step: Arc::new(AtomicUsize::new(0)),
            arp_restart: Arc::new(AtomicBool::new(false)),
            walker_restart: Arc::new(AtomicBool::new(false)),
            note_div: Arc::new(AtomicU8::new(1)),
            chord_div: Arc::new(AtomicU8::new(4)),
            recording: Arc::new(AtomicBool::new(false)),
            rec_step: Arc::new(AtomicUsize::new(0)),
            humanize: Arc::new(AtomicU8::new(0)),
            gate: Arc::new(AtomicU8::new(90)),
        }
    }
}

/// Euclidean rhythm: distribute `hits` onsets evenly across `steps`, rotated by `offset`.
/// Uses Bresenham integer distribution (equivalent to Bjorklund for most hit counts).
pub fn bjorklund(hits: usize, steps: usize, offset: usize) -> Vec<bool> {
    let mut pattern = vec![false; steps];
    if steps == 0 || hits == 0 {
        return pattern;
    }
    let hits = hits.min(steps);
    for k in 0..hits {
        let pos = (k * steps / hits + offset) % steps;
        pattern[pos] = true;
    }
    pattern
}

/// Spawn the sequencer on a dedicated thread.
///
/// `engine` — typed engine handle. The sequencer calls `note_on` / `note_off`
/// / `arp_restart` / `walker_restart` on the handle; `note_on` also records
/// the latency-measurement timestamp internally.
pub fn spawn_sequencer(
    handle: Arc<SequencerHandle>,
    engine: forma_engine::SynthEngineHandle,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("sequencer".into())
        .spawn(move || {
            use std::time::{Duration, Instant};

            let mut prev_notes: Vec<u8> = Vec::new();
            let mut was_playing = false;
            let mut first_tick = true;
            let mut next_tick = Instant::now();
            // Cheap LCG for per-step probability rolls.
            let mut lcg: u32 = 0xdeadbeef;

            loop {
                let playing = handle.playing.load(Ordering::Relaxed);

                if !playing {
                    if was_playing {
                        for m in prev_notes.drain(..) {
                            engine.note_off(m);
                        }
                        was_playing = false;
                        first_tick = true;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }

                if !was_playing {
                    // Fire the first step immediately.
                    next_tick = Instant::now();
                    was_playing = true;
                }

                let bpm = handle.bpm.load(Ordering::Relaxed).max(1);
                let mode_now = SeqMode::from_u8(handle.mode.load(Ordering::Relaxed));
                let div_idx = match mode_now {
                    SeqMode::NoteSeq => handle.note_div.load(Ordering::Relaxed),
                    SeqMode::ChordSeq => handle.chord_div.load(Ordering::Relaxed),
                    SeqMode::ChordKb => 1,
                };
                let step_ms = SeqClockDiv::step_dur_ms(div_idx, bpm).max(1);
                let step_dur = Duration::from_millis(step_ms);

                // Humanization: compute a random ±offset, capped at 45% of step duration.
                let humanize = handle.humanize.load(Ordering::Relaxed) as i64;
                let fire_tick = if humanize > 0 {
                    lcg = lcg.wrapping_mul(1664525).wrapping_add(1013904223);
                    // Map LCG to signed [-1, +1] range, then scale.
                    let r = ((lcg >> 16) as i16) as i64; // -32768..32767
                    let max_offset_ms = step_ms as i64 * humanize * 45 / (100 * 100);
                    let offset_ms = r * max_offset_ms / 32768;
                    if offset_ms >= 0 {
                        next_tick + Duration::from_millis(offset_ms as u64)
                    } else {
                        next_tick
                            .checked_sub(Duration::from_millis((-offset_ms) as u64))
                            .unwrap_or(next_tick)
                    }
                } else {
                    next_tick
                };

                // Capped sleep to fire_tick so stopping is always responsive.
                loop {
                    let now = Instant::now();
                    if now >= fire_tick {
                        break;
                    }
                    let remaining = fire_tick - now;
                    std::thread::sleep(remaining.min(Duration::from_millis(50)));
                    if !handle.playing.load(Ordering::Relaxed) {
                        break;
                    }
                }

                // Re-check playing after sleep (user may have stopped).
                if !handle.playing.load(Ordering::Relaxed) {
                    continue;
                }

                next_tick += step_dur;

                // NoteOff previous notes.
                for m in prev_notes.drain(..) {
                    engine.note_off(m);
                }

                // Advance step. On the very first tick after Play we play the
                // stored current_step as-is so step 0 isn't skipped; subsequent
                // ticks advance by one.
                let mode = mode_now;
                let seq_length = match mode {
                    SeqMode::NoteSeq => handle.note_seq.lock().map(|g| g.length).unwrap_or(8),
                    SeqMode::ChordSeq => handle.chord_seq.lock().map(|g| g.length).unwrap_or(8),
                    SeqMode::ChordKb => continue,
                };

                let current = if first_tick {
                    first_tick = false;
                    handle.current_step.load(Ordering::Relaxed) % seq_length
                } else {
                    (handle.current_step.load(Ordering::Relaxed) + 1) % seq_length
                };
                handle.current_step.store(current, Ordering::Relaxed);
                let bar_boundary = current == 0;

                if bar_boundary {
                    if handle.arp_restart.swap(false, Ordering::Relaxed) {
                        engine.arp_restart();
                    }
                    if handle.walker_restart.swap(false, Ordering::Relaxed) {
                        engine.walker_restart();
                    }
                }

                // Roll probability for this step.
                lcg = lcg.wrapping_mul(1664525).wrapping_add(1013904223);
                let rand_pct = (lcg >> 25) as u8; // 0–127 mapped to 0–100 below

                // Collect notes (note, velocity) for this step.
                let notes_to_play: Vec<(u8, u8)> = match mode {
                    SeqMode::NoteSeq => handle
                        .note_seq
                        .lock()
                        .map(|ns| {
                            if ns.steps[current] {
                                let prob = ns.probabilities[current];
                                let passes =
                                    prob >= 100 || (rand_pct as u32 * 100 / 127 < prob as u32);
                                if passes {
                                    vec![(ns.notes[current], ns.velocities[current])]
                                } else {
                                    vec![]
                                }
                            } else {
                                vec![]
                            }
                        })
                        .unwrap_or_default(),
                    SeqMode::ChordSeq => handle
                        .chord_seq
                        .lock()
                        .map(|cs| {
                            if cs.steps[current] {
                                let prob = cs.probabilities[current];
                                let passes =
                                    prob >= 100 || (rand_pct as u32 * 100 / 127 < prob as u32);
                                if passes {
                                    let vel = cs.velocities[current];
                                    cs.step_notes_vl(current, &prev_notes)
                                        .into_iter()
                                        .map(|n| (n, vel))
                                        .collect()
                                } else {
                                    vec![]
                                }
                            } else {
                                vec![]
                            }
                        })
                        .unwrap_or_default(),
                    SeqMode::ChordKb => vec![],
                };

                // Send NoteOns. Record the actual fire timestamp AFTER sending so
                // gate timing is relative to when notes truly started, not the
                // scheduled fire_tick (which may already be slightly in the past).
                let mut this_step_notes: Vec<u8> = Vec::new();
                for (m, vel) in notes_to_play {
                    engine.note_on(m, vel);
                    this_step_notes.push(m);
                }
                let note_on_time = Instant::now();

                // Gate: if gate < 100%, fire note_off early inside the step,
                // then let the outer sleep carry us to the next step naturally.
                // Minimum 15 ms so very short gates still produce an audible transient
                // even at large audio buffer sizes (~11 ms at 44100/512).
                let gate = handle.gate.load(Ordering::Relaxed).max(1) as u64;
                if gate < 100 && !this_step_notes.is_empty() {
                    let gate_ms = (step_ms * gate / 100).max(15);
                    let gate_off = note_on_time + Duration::from_millis(gate_ms);
                    loop {
                        let now = Instant::now();
                        if now >= gate_off {
                            break;
                        }
                        std::thread::sleep((gate_off - now).min(Duration::from_millis(10)));
                    }
                    for m in this_step_notes.drain(..) {
                        engine.note_off(m);
                    }
                    // prev_notes stays empty — notes already silenced.
                } else {
                    // Gate 100% (or rest): carry notes forward, turn off at next step.
                    prev_notes = this_step_notes;
                }
            }
        })
        .expect("failed to spawn sequencer thread")
}
