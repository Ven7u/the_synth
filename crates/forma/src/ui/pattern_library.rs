//! Static preset libraries for the note and chord sequencers.
//!
//! All presets are key-agnostic:
//!   HarmonyPreset — degrees 0–6 (loaded into ChordSeqState; root/scale set by user)
//!   MelodyPreset  — semitone offsets from C4; transposed by user's current octave on load

use crate::sequencer::{ChordSeqState, ChordType, NoteSeqState};

// ---------------------------------------------------------------------------
// Harmony (chord) presets
// ---------------------------------------------------------------------------

pub struct HarmonyPreset {
    pub name: &'static str,
    pub category: &'static str,
    pub length: usize,
    /// Scale degrees, 0–6 (I–VII).  Must have exactly `length` entries.
    pub degrees: &'static [usize],
}

pub const HARMONY_PRESETS: &[HarmonyPreset] = &[
    // --- Pop ---
    HarmonyPreset {
        name: "I – V – vi – IV",
        category: "Pop",
        length: 4,
        degrees: &[0, 4, 5, 3],
    },
    HarmonyPreset {
        name: "I – vi – IV – V",
        category: "Pop",
        length: 4,
        degrees: &[0, 5, 3, 4],
    },
    HarmonyPreset {
        name: "I – IV – V – I",
        category: "Pop",
        length: 4,
        degrees: &[0, 3, 4, 0],
    },
    HarmonyPreset {
        name: "I–V–vi–IV (8)",
        category: "Pop",
        length: 8,
        degrees: &[0, 0, 4, 4, 5, 5, 3, 3],
    },
    HarmonyPreset {
        name: "IV – I – V – vi",
        category: "Pop",
        length: 4,
        degrees: &[3, 0, 4, 5],
    },
    // --- Jazz ---
    HarmonyPreset {
        name: "ii – V – I",
        category: "Jazz",
        length: 4,
        degrees: &[1, 1, 4, 0],
    },
    HarmonyPreset {
        name: "I – VI – ii – V",
        category: "Jazz",
        length: 4,
        degrees: &[0, 5, 1, 4],
    },
    HarmonyPreset {
        name: "iii – VI – ii – V",
        category: "Jazz",
        length: 4,
        degrees: &[2, 5, 1, 4],
    },
    HarmonyPreset {
        name: "I – IV – iii – VI",
        category: "Jazz",
        length: 4,
        degrees: &[0, 3, 2, 5],
    },
    HarmonyPreset {
        name: "ii–V–I–VI (8)",
        category: "Jazz",
        length: 8,
        degrees: &[1, 1, 4, 4, 0, 0, 5, 5],
    },
    // --- Blues ---
    HarmonyPreset {
        name: "12-bar Blues",
        category: "Blues",
        length: 12,
        degrees: &[0, 0, 0, 0, 3, 3, 0, 0, 4, 3, 0, 4],
    },
    HarmonyPreset {
        name: "8-bar Blues",
        category: "Blues",
        length: 8,
        degrees: &[0, 0, 3, 0, 4, 3, 0, 4],
    },
    HarmonyPreset {
        name: "Blues Turnaround",
        category: "Blues",
        length: 4,
        degrees: &[0, 3, 4, 3],
    },
    // --- Modal / Folk ---
    HarmonyPreset {
        name: "i – VII – VI – VII",
        category: "Modal",
        length: 4,
        degrees: &[0, 6, 5, 6],
    },
    HarmonyPreset {
        name: "i – VI – III – VII",
        category: "Modal",
        length: 4,
        degrees: &[0, 5, 2, 6],
    },
    HarmonyPreset {
        name: "I – II – IV – I",
        category: "Modal",
        length: 4,
        degrees: &[0, 1, 3, 0],
    },
    HarmonyPreset {
        name: "Andalusian Cadence",
        category: "Modal",
        length: 4,
        degrees: &[5, 4, 3, 4],
    },
    HarmonyPreset {
        name: "i – iv – i – V",
        category: "Modal",
        length: 4,
        degrees: &[0, 3, 0, 4],
    },
    // --- Ambient ---
    HarmonyPreset {
        name: "I – V – I – IV",
        category: "Ambient",
        length: 4,
        degrees: &[0, 4, 0, 3],
    },
    HarmonyPreset {
        name: "IV – V – iii – vi",
        category: "Ambient",
        length: 4,
        degrees: &[3, 4, 2, 5],
    },
    HarmonyPreset {
        name: "I (pedal, 8)",
        category: "Ambient",
        length: 8,
        degrees: &[0, 0, 0, 0, 0, 0, 0, 0],
    },
    HarmonyPreset {
        name: "I – IV (drone)",
        category: "Ambient",
        length: 4,
        degrees: &[0, 0, 3, 3],
    },
];

/// Load a harmony preset into `state`.  Degrees wrap to 0–6; chord types reset to Triad.
pub fn apply_harmony(state: &mut ChordSeqState, preset: &HarmonyPreset) {
    state.length = preset.length;
    for i in 0..preset.length {
        state.steps[i] = true;
        state.degrees[i] = preset.degrees[i] % 7;
        state.chord_types[i] = ChordType::Triad;
        state.octave_offsets[i] = 0;
        state.velocities[i] = 100;
        state.probabilities[i] = 100;
    }
    for i in preset.length..24 {
        state.steps[i] = false;
    }
}

// ---------------------------------------------------------------------------
// Melody (note) presets
// ---------------------------------------------------------------------------

pub struct MelodyPreset {
    pub name: &'static str,
    pub category: &'static str,
    pub length: usize,
    /// Semitone offsets from C4 (MIDI 60).  Must have exactly `length` entries.
    pub notes: &'static [i8],
    /// Which steps are active.  Must have exactly `length` entries.
    pub active: &'static [bool],
}

pub const MELODY_PRESETS: &[MelodyPreset] = &[
    // --- Arpeggios ---
    MelodyPreset {
        name: "Major Triad Up/Down",
        category: "Arpeggio",
        length: 8,
        notes: &[0, 4, 7, 12, 7, 4, 0, -12],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Minor Triad Roll",
        category: "Arpeggio",
        length: 8,
        notes: &[0, 3, 7, 12, 7, 3, 0, -12],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Dom7 Arpeggio",
        category: "Arpeggio",
        length: 8,
        notes: &[0, 4, 7, 10, 12, 10, 7, 4],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Broken Triad",
        category: "Arpeggio",
        length: 8,
        notes: &[0, 7, 4, 12, 0, 7, 4, 12],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Maj7 Cascade",
        category: "Arpeggio",
        length: 8,
        notes: &[0, 4, 7, 11, 12, 11, 7, 4],
        active: &[true; 8],
    },
    // --- Scale Runs ---
    MelodyPreset {
        name: "Major Scale Up",
        category: "Scale Run",
        length: 8,
        notes: &[0, 2, 4, 5, 7, 9, 11, 12],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Major Scale Down",
        category: "Scale Run",
        length: 8,
        notes: &[12, 11, 9, 7, 5, 4, 2, 0],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Pentatonic Up",
        category: "Scale Run",
        length: 8,
        notes: &[0, 2, 4, 7, 9, 12, 9, 7],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Minor Pentatonic Riff",
        category: "Scale Run",
        length: 8,
        notes: &[0, 3, 5, 7, 10, 7, 5, 3],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Ascending Thirds",
        category: "Scale Run",
        length: 8,
        notes: &[0, 4, 2, 5, 4, 7, 5, 9],
        active: &[true; 8],
    },
    // --- Bass Lines ---
    MelodyPreset {
        name: "Walking Bass",
        category: "Bass",
        length: 8,
        notes: &[-12, -10, -8, -7, -5, -7, -8, -10],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Octave Pump",
        category: "Bass",
        length: 8,
        notes: &[-12, -12, 0, -12, -12, 0, -12, 0],
        active: &[true, true, false, true, true, false, true, false],
    },
    MelodyPreset {
        name: "Alberti Bass",
        category: "Bass",
        length: 8,
        notes: &[-12, -5, -8, -5, -12, -5, -8, -5],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Blues Bass Riff",
        category: "Bass",
        length: 8,
        notes: &[-12, -12, -9, -12, -7, -6, -5, -7],
        active: &[true; 8],
    },
    // --- Motifs ---
    MelodyPreset {
        name: "Call & Response",
        category: "Motif",
        length: 8,
        notes: &[0, 4, 7, 12, 0, 7, 5, 4],
        active: &[true, true, true, true, false, true, true, true],
    },
    MelodyPreset {
        name: "Descending Hook",
        category: "Motif",
        length: 8,
        notes: &[12, 9, 7, 5, 4, 2, 0, -3],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Blues Lick",
        category: "Motif",
        length: 8,
        notes: &[0, 3, 5, 6, 7, 5, 3, 0],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Ostinato",
        category: "Motif",
        length: 8,
        notes: &[0, 0, 7, 0, 0, 7, 5, 7],
        active: &[true; 8],
    },
    MelodyPreset {
        name: "Syncopated Riff",
        category: "Motif",
        length: 8,
        notes: &[0, 5, 7, 5, 3, 5, 7, 12],
        active: &[true, false, true, true, false, true, true, true],
    },
    MelodyPreset {
        name: "Triad Bounce",
        category: "Motif",
        length: 8,
        notes: &[0, 4, 7, 4, 0, 4, 7, 4],
        active: &[true; 8],
    },
];

/// Load a melody preset into `state`, transposed so C4 maps to `base_midi`.
pub fn apply_melody(state: &mut NoteSeqState, preset: &MelodyPreset, base_midi: u8) {
    let base = base_midi as i32;
    state.length = preset.length;
    for i in 0..preset.length {
        state.steps[i] = preset.active[i];
        let midi = (base + preset.notes[i] as i32).clamp(21, 108) as u8;
        state.notes[i] = midi;
        state.velocities[i] = 100;
        state.probabilities[i] = 100;
    }
    for i in preset.length..24 {
        state.steps[i] = false;
    }
}

/// Unique category names for harmony presets (in declaration order, deduplicated).
pub fn harmony_categories() -> Vec<&'static str> {
    let mut seen = std::collections::HashSet::new();
    HARMONY_PRESETS
        .iter()
        .filter_map(|p| {
            if seen.insert(p.category) {
                Some(p.category)
            } else {
                None
            }
        })
        .collect()
}

/// Unique category names for melody presets.
pub fn melody_categories() -> Vec<&'static str> {
    let mut seen = std::collections::HashSet::new();
    MELODY_PRESETS
        .iter()
        .filter_map(|p| {
            if seen.insert(p.category) {
                Some(p.category)
            } else {
                None
            }
        })
        .collect()
}
