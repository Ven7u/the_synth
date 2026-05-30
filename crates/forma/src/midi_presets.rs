//! Named MIDI keyboard presets — default CC → ParamId mappings.
//!
//! Each preset maps the hardware's factory CC assignments to useful synth
//! parameters. Users can further customise via MIDI learn after loading.

use forma_control::ParamId;
use std::collections::HashMap;

pub struct KeyboardPreset {
    pub name: &'static str,
    pub description: &'static str,
    pub mappings: &'static [(u8, ParamId)],
}

// ── Arturia KeyLab MkIII ────────────────────────────────────────────────────
//
// Encoder row (Analog Lab V preset):
//   Enc 1 = CC 74  Enc 2 = CC 71  Enc 3 = CC 76  Enc 4 = CC 77
//   Enc 5 = CC 93  Enc 6 = CC 18  Enc 7 = CC 19  Enc 8 = CC 16
//
// Faders (Analog Lab V preset, faders 1-9):
//   F1 = CC 80  F2 = CC 81  F3 = CC 82  F4 = CC 83  F5 = CC 84
//   F6 = CC 85  F7 = CC 86  F8 = CC 87  F9 (master) = CC 7
//
// Already handled in the hardcoded CC switch (not duplicated here):
//   CC 1  = Mod wheel     CC 7  = Master volume
//   CC 64 = Sustain pedal CC 71 = Filter resonance   CC 74 = Filter cutoff

const KEYLAB_MK3: &[(u8, ParamId)] = &[
    // ── Encoders ────────────────────────────────────────────────────────────
    (74, ParamId::FilterCutoff),
    (71, ParamId::FilterResonance),
    (76, ParamId::LfoRate),
    (77, ParamId::LfoDepth),
    (93, ParamId::FxChorusMix),
    (18, ParamId::FxReverbMix),
    (19, ParamId::FxDelayMix),
    (16, ParamId::FxOverdriveMix),
    // ── Faders ──────────────────────────────────────────────────────────────
    (80, ParamId::OscVol(0)),       // Osc 1 level
    (81, ParamId::OscVol(1)),       // Osc 2 level
    (82, ParamId::OscVol(2)),       // Osc 3 level
    (83, ParamId::AmpAttack),       // Amp attack
    (84, ParamId::AmpDecay),        // Amp decay
    (85, ParamId::AmpSustain),      // Amp sustain
    (86, ParamId::AmpRelease),      // Amp release
    (87, ParamId::FilterEnvAmount), // Filter env depth
    (7, ParamId::MasterVolume),     // Master fader
];

// ── Arturia MiniLab MkIII ───────────────────────────────────────────────────
//
// 8 rotary encoders (Analog Lab V preset):
//   P1 = CC 74  P2 = CC 71  P3 = CC 76  P4 = CC 77
//   P5 = CC 93  P6 = CC 18  P7 = CC 19  P8 = CC 16

const MINILAB_MK3: &[(u8, ParamId)] = &[
    (74, ParamId::FilterCutoff),
    (71, ParamId::FilterResonance),
    (76, ParamId::LfoRate),
    (77, ParamId::LfoDepth),
    (93, ParamId::FxChorusMix),
    (18, ParamId::FxReverbMix),
    (19, ParamId::FxDelayMix),
    (16, ParamId::FxOverdriveMix),
    (7, ParamId::MasterVolume),
];

// ── Generic / minimal ───────────────────────────────────────────────────────
//
// Standard CC numbers found on most keyboards.

const GENERIC: &[(u8, ParamId)] = &[
    (1, ParamId::LfoDepth),         // Mod wheel → LFO depth
    (7, ParamId::MasterVolume),     // Volume fader
    (71, ParamId::FilterResonance), // Timbre / brightness
    (74, ParamId::FilterCutoff),    // Filter cutoff
];

// ── Catalogue ───────────────────────────────────────────────────────────────

pub const PRESETS: &[KeyboardPreset] = &[
    KeyboardPreset {
        name: "Arturia KeyLab MkIII",
        description: "9 encoders + 9 faders mapped to filter, LFO, FX, amp envelope and oscillator levels.\n\nPatch library (always active):\n  Wheel turn (CC 114) — browse all patches\n  Wheel press (CC 115) — pin current state to history\n  CC 60 — toggle favourite\n  CC 61 / 62 — prev / next favourite\n  CC 63 — randomize patch\n  Program Change — jump to patch by index",
        mappings: KEYLAB_MK3,
    },
    KeyboardPreset {
        name: "Arturia MiniLab MkIII",
        description: "8 encoders mapped to filter, LFO, and FX. Uses Analog Lab V factory preset.",
        mappings: MINILAB_MK3,
    },
    KeyboardPreset {
        name: "Generic",
        description: "Minimal mapping using standard CC numbers common to most keyboards.",
        mappings: GENERIC,
    },
];

/// Build a `HashMap` from a preset's mapping slice.
pub fn preset_bindings(preset: &KeyboardPreset) -> HashMap<u8, ParamId> {
    preset.mappings.iter().cloned().collect()
}
