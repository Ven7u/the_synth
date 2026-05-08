//! Wire-ready control protocol.
//!
//! Three types make up the portable contract between the engine and any
//! frontend (egui, Bevy, Swift/FFI, OSC, WebSocket, CLAP shell, …):
//!
//! * [`ParamId`] — a stable identifier for every live engine parameter.
//! * [`ParamDescriptor`] — static metadata (name, min/max/default/unit/kind)
//!   used to render UIs and format values.
//! * [`Command`] — every operation the engine can perform, as a
//!   serialisable value.
//!
//! Indexed variant convention: `(osc_idx, copy_idx)` for unison, `(osc_idx)`
//! for per-osc. Osc indices run 0..=2 (three oscillators), unison copies run
//! 0..=4 (five copies per oscillator).
//!
//! Both enums are `#[non_exhaustive]` so new variants can be added without
//! breaking external `match` statements.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ParamId — stable identifier for every UI-writable parameter
// ---------------------------------------------------------------------------

/// Stable identifier for every UI-writable engine parameter.
///
/// Covers ~65 enum variants, representing ~120 individual parameter slots
/// (indexed variants like `OscVol(u8)` collapse three per-oscillator params
/// into one variant).
///
/// The five variants `FilterCutoff`, `FilterResonance`, `LfoDepth`,
/// `MasterVolume`, `LfoPitchMult` predate the rest of this enum and must
/// keep their names — existing `forma-bevy` and `forma` callers match
/// on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub enum ParamId {
    // --- Oscillator bank (osc_idx 0..=2) ---
    OscWave(u8),
    OscFreqMult(u8),
    OscVol(u8),
    OscPulseWidth(u8),
    /// (osc_idx, copy_idx), copies 0..=4.
    OscUnisonDetune(u8, u8),
    /// (osc_idx, copy_idx), copies 0..=4.
    OscUnisonVol(u8, u8),
    HardSyncEnabled,
    FmDepth,
    RingDepth,
    NoiseVol,

    // --- Filter ---
    FilterCutoff,
    FilterResonance,
    FilterEnvAmount,
    FenvAttack,
    FenvDecay,
    FenvSustain,
    FenvRelease,

    // --- LFO 1 ---
    LfoRate,
    LfoDepth,
    LfoShape,
    LfoDest,
    LfoSync,
    LfoDivision,
    LfoPitchMult,

    // --- LFO 2 ---
    Lfo2Rate,
    Lfo2Depth,
    Lfo2Shape,
    Lfo2Dest,

    // --- Gate lane: amp ducker ("Pulse") ---
    GateAenvEnabled,
    /// 16-bit step mask, transmitted as f32 (0.0..=65535.0).
    GateAenvPattern,
    GateAenvLength,
    GateAenvDivision,
    GateAenvRate,
    GateAenvDepth,

    // --- Gate lane: LFO1 retrigger ---
    GateLfo1Enabled,
    GateLfo1Pattern,
    GateLfo1Length,
    GateLfo1Division,
    GateLfo1Rate,

    // --- Gate lane: LFO2 retrigger ---
    GateLfo2Enabled,
    GateLfo2Pattern,
    GateLfo2Length,
    GateLfo2Division,
    GateLfo2Rate,

    // --- Amp envelope + glide + master ---
    AmpAttack,
    AmpDecay,
    AmpSustain,
    AmpRelease,
    GlideTime,
    MasterVolume,
    GlobalVolume,
    LimiterEnabled,
    LimiterThreshold,

    // --- FX chain ---
    FxOverdriveDrive,
    FxOverdriveMix,
    FxOverdriveTone,
    FxOverdriveAsym,
    FxDistortionDrive,
    FxDistortionMix,
    FxDistortionTone,
    FxDistortionPre,
    FxChorusRate,
    FxChorusDepth,
    FxChorusMix,
    FxDelayTime,
    FxDelayFeedback,
    FxDelayMix,
    FxDelaySync,
    FxDelayDivision,
    FxReverbSize,
    FxReverbDamp,
    FxReverbMix,
    FxReverbPredelay,
    FxReverbType,
    StereoSpread,
    StereoWidth,

    // --- Shimmer (nested struct fx_shimmer) ---
    ShimmerSize,
    ShimmerDamp,
    ShimmerMix,
    ShimmerAmount,
    ShimmerWidth,
    ShimmerSpread,
    ShimmerPitch,

    // --- Crystallizer (nested struct fx_crystal) ---
    CrystalGrain,
    CrystalScatter,
    CrystalFeedback,
    CrystalDelay,
    CrystalMix,
    CrystalPitch,

    // --- Arpeggiator ---
    ArpEnabled,
    ArpMode,
    ArpDivision,
    ArpOctaveRange,
    ArpGate,
    ArpHold,
    ArpBpm,

    // --- Scale walker ---
    WalkerEnabled,
    WalkerScale,
    WalkerRoot,
    WalkerOctaveRange,
    WalkerDivision,
    WalkerGate,
    WalkerBpm,
}

// ---------------------------------------------------------------------------
// ParamKind — how a value is interpreted
// ---------------------------------------------------------------------------

/// How a parameter value should be interpreted by the handle's generic
/// `apply(Command::SetParam)` path and by rendering UIs.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ParamKind {
    /// Numeric, linear slider.
    Linear,
    /// Numeric, logarithmic slider (e.g. Hz, ms).
    Log,
    /// Integer-valued choice with `count` legal values (0..count).
    /// The handle's dispatch rounds + clamps to `[0, count-1]`.
    Discrete(u8),
    /// Boolean. 0.0 = false, any non-zero = true.
    Bool,
}

// ---------------------------------------------------------------------------
// ParamDescriptor — static metadata for one parameter
// ---------------------------------------------------------------------------

/// Static metadata used to render UIs and format values.
#[derive(Clone, Copy, Debug)]
pub struct ParamDescriptor {
    pub id: ParamId,
    pub name: &'static str,
    pub path: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
    pub kind: ParamKind,
}

impl ParamDescriptor {
    /// Format a raw parameter value for display, dispatching on kind + unit.
    pub fn format(&self, v: f32) -> String {
        match (self.kind, self.unit) {
            (ParamKind::Bool, _) => {
                if v != 0.0 {
                    "on".into()
                } else {
                    "off".into()
                }
            }
            (ParamKind::Discrete(_), _) => format!("{}", v.round() as u8),
            (_, "Hz") if v.abs() >= 1000.0 => format!("{:.2} kHz", v / 1000.0),
            (_, "Hz") => format!("{:.0} Hz", v),
            (_, "s") if v.abs() < 1.0 => format!("{:.0} ms", v * 1000.0),
            (_, "s") => format!("{:.2} s", v),
            (_, "dB") => format!("{:+.1} dB", v),
            (_, "") => format!("{:.0}%", v * 100.0),
            (_, u) => format!("{:.2} {}", v, u),
        }
    }
}

// ---------------------------------------------------------------------------
// Descriptor table
// ---------------------------------------------------------------------------

/// Returns the static table of all descriptors. One entry per `ParamId`
/// variant instance (indexed variants expand to one entry per index).
pub fn all_params() -> &'static [ParamDescriptor] {
    TABLE
}

// Helpers to keep the table terse.
const fn d_linear(
    id: ParamId,
    name: &'static str,
    path: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: &'static str,
) -> ParamDescriptor {
    ParamDescriptor {
        id,
        name,
        path,
        min,
        max,
        default,
        unit,
        kind: ParamKind::Linear,
    }
}
const fn d_log(
    id: ParamId,
    name: &'static str,
    path: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: &'static str,
) -> ParamDescriptor {
    ParamDescriptor {
        id,
        name,
        path,
        min,
        max,
        default,
        unit,
        kind: ParamKind::Log,
    }
}
const fn d_discrete(
    id: ParamId,
    name: &'static str,
    path: &'static str,
    count: u8,
    default: f32,
) -> ParamDescriptor {
    ParamDescriptor {
        id,
        name,
        path,
        min: 0.0,
        max: (count as f32 - 1.0).max(0.0),
        default,
        unit: "",
        kind: ParamKind::Discrete(count),
    }
}
const fn d_bool(
    id: ParamId,
    name: &'static str,
    path: &'static str,
    default: bool,
) -> ParamDescriptor {
    ParamDescriptor {
        id,
        name,
        path,
        min: 0.0,
        max: 1.0,
        default: if default { 1.0 } else { 0.0 },
        unit: "",
        kind: ParamKind::Bool,
    }
}

static TABLE: &[ParamDescriptor] = &[
    // -- Oscillator bank --
    d_discrete(ParamId::OscWave(0), "Osc 1 Wave", "osc/1/wave", 4, 1.0),
    d_discrete(ParamId::OscWave(1), "Osc 2 Wave", "osc/2/wave", 4, 0.0),
    d_discrete(ParamId::OscWave(2), "Osc 3 Wave", "osc/3/wave", 4, 0.0),
    d_linear(
        ParamId::OscFreqMult(0),
        "Osc 1 Freq Mult",
        "osc/1/freq_mult",
        0.25,
        4.0,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscFreqMult(1),
        "Osc 2 Freq Mult",
        "osc/2/freq_mult",
        0.25,
        4.0,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscFreqMult(2),
        "Osc 3 Freq Mult",
        "osc/3/freq_mult",
        0.25,
        4.0,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscVol(0),
        "Osc 1 Volume",
        "osc/1/vol",
        0.0,
        1.0,
        0.4,
        "",
    ),
    d_linear(
        ParamId::OscVol(1),
        "Osc 2 Volume",
        "osc/2/vol",
        0.0,
        1.0,
        0.3,
        "",
    ),
    d_linear(
        ParamId::OscVol(2),
        "Osc 3 Volume",
        "osc/3/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscPulseWidth(0),
        "Osc 1 PW",
        "osc/1/pulse_width",
        0.01,
        0.99,
        0.5,
        "",
    ),
    d_linear(
        ParamId::OscPulseWidth(1),
        "Osc 2 PW",
        "osc/2/pulse_width",
        0.01,
        0.99,
        0.5,
        "",
    ),
    d_linear(
        ParamId::OscPulseWidth(2),
        "Osc 3 PW",
        "osc/3/pulse_width",
        0.01,
        0.99,
        0.5,
        "",
    ),
    // Unison detune: 3 osc × 5 copies = 15 entries
    d_linear(
        ParamId::OscUnisonDetune(0, 0),
        "Osc 1 Unison Detune 1",
        "osc/1/unison/1/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(0, 1),
        "Osc 1 Unison Detune 2",
        "osc/1/unison/2/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(0, 2),
        "Osc 1 Unison Detune 3",
        "osc/1/unison/3/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(0, 3),
        "Osc 1 Unison Detune 4",
        "osc/1/unison/4/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(0, 4),
        "Osc 1 Unison Detune 5",
        "osc/1/unison/5/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(1, 0),
        "Osc 2 Unison Detune 1",
        "osc/2/unison/1/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(1, 1),
        "Osc 2 Unison Detune 2",
        "osc/2/unison/2/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(1, 2),
        "Osc 2 Unison Detune 3",
        "osc/2/unison/3/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(1, 3),
        "Osc 2 Unison Detune 4",
        "osc/2/unison/4/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(1, 4),
        "Osc 2 Unison Detune 5",
        "osc/2/unison/5/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(2, 0),
        "Osc 3 Unison Detune 1",
        "osc/3/unison/1/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(2, 1),
        "Osc 3 Unison Detune 2",
        "osc/3/unison/2/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(2, 2),
        "Osc 3 Unison Detune 3",
        "osc/3/unison/3/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(2, 3),
        "Osc 3 Unison Detune 4",
        "osc/3/unison/4/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonDetune(2, 4),
        "Osc 3 Unison Detune 5",
        "osc/3/unison/5/detune",
        0.9,
        1.1,
        1.0,
        "",
    ),
    // Unison vol: 3 × 5 = 15
    d_linear(
        ParamId::OscUnisonVol(0, 0),
        "Osc 1 Unison Vol 1",
        "osc/1/unison/1/vol",
        0.0,
        1.0,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(0, 1),
        "Osc 1 Unison Vol 2",
        "osc/1/unison/2/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(0, 2),
        "Osc 1 Unison Vol 3",
        "osc/1/unison/3/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(0, 3),
        "Osc 1 Unison Vol 4",
        "osc/1/unison/4/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(0, 4),
        "Osc 1 Unison Vol 5",
        "osc/1/unison/5/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(1, 0),
        "Osc 2 Unison Vol 1",
        "osc/2/unison/1/vol",
        0.0,
        1.0,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(1, 1),
        "Osc 2 Unison Vol 2",
        "osc/2/unison/2/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(1, 2),
        "Osc 2 Unison Vol 3",
        "osc/2/unison/3/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(1, 3),
        "Osc 2 Unison Vol 4",
        "osc/2/unison/4/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(1, 4),
        "Osc 2 Unison Vol 5",
        "osc/2/unison/5/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(2, 0),
        "Osc 3 Unison Vol 1",
        "osc/3/unison/1/vol",
        0.0,
        1.0,
        1.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(2, 1),
        "Osc 3 Unison Vol 2",
        "osc/3/unison/2/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(2, 2),
        "Osc 3 Unison Vol 3",
        "osc/3/unison/3/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(2, 3),
        "Osc 3 Unison Vol 4",
        "osc/3/unison/4/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::OscUnisonVol(2, 4),
        "Osc 3 Unison Vol 5",
        "osc/3/unison/5/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_bool(
        ParamId::HardSyncEnabled,
        "Hard Sync",
        "osc/hard_sync",
        false,
    ),
    d_linear(
        ParamId::FmDepth,
        "FM Depth",
        "osc/fm_depth",
        0.0,
        2.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::RingDepth,
        "Ring Depth",
        "osc/ring_depth",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::NoiseVol,
        "Noise Vol",
        "noise/vol",
        0.0,
        1.0,
        0.0,
        "",
    ),
    // -- Filter --
    d_log(
        ParamId::FilterCutoff,
        "Filter Cutoff",
        "filter/cutoff",
        80.0,
        18000.0,
        3000.0,
        "Hz",
    ),
    d_linear(
        ParamId::FilterResonance,
        "Filter Resonance",
        "filter/resonance",
        0.1,
        20.0,
        0.3,
        "",
    ),
    d_linear(
        ParamId::FilterEnvAmount,
        "Filter Env Amount",
        "filter/env_amount",
        0.0,
        1.0,
        0.3,
        "",
    ),
    d_log(
        ParamId::FenvAttack,
        "Fenv Attack",
        "filter/env/attack",
        0.001,
        5.0,
        0.01,
        "s",
    ),
    d_log(
        ParamId::FenvDecay,
        "Fenv Decay",
        "filter/env/decay",
        0.001,
        5.0,
        0.3,
        "s",
    ),
    d_linear(
        ParamId::FenvSustain,
        "Fenv Sustain",
        "filter/env/sustain",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_log(
        ParamId::FenvRelease,
        "Fenv Release",
        "filter/env/release",
        0.001,
        10.0,
        0.2,
        "s",
    ),
    // -- LFO 1 --
    d_log(
        ParamId::LfoRate,
        "LFO 1 Rate",
        "lfo1/rate",
        0.1,
        20.0,
        2.0,
        "Hz",
    ),
    d_linear(
        ParamId::LfoDepth,
        "LFO 1 Depth",
        "lfo1/depth",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_discrete(ParamId::LfoShape, "LFO 1 Shape", "lfo1/shape", 3, 0.0),
    d_discrete(ParamId::LfoDest, "LFO 1 Dest", "lfo1/dest", 3, 1.0),
    d_discrete(ParamId::LfoSync, "LFO 1 Sync", "lfo1/sync", 2, 0.0),
    d_discrete(
        ParamId::LfoDivision,
        "LFO 1 Division",
        "lfo1/division",
        16,
        2.0,
    ),
    d_linear(
        ParamId::LfoPitchMult,
        "LFO 1 Pitch Mult",
        "lfo1/pitch_mult",
        0.5,
        2.0,
        1.0,
        "",
    ),
    // -- LFO 2 --
    d_log(
        ParamId::Lfo2Rate,
        "LFO 2 Rate",
        "lfo2/rate",
        0.01,
        20.0,
        0.3,
        "Hz",
    ),
    d_linear(
        ParamId::Lfo2Depth,
        "LFO 2 Depth",
        "lfo2/depth",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_discrete(ParamId::Lfo2Shape, "LFO 2 Shape", "lfo2/shape", 3, 0.0),
    d_discrete(ParamId::Lfo2Dest, "LFO 2 Dest", "lfo2/dest", 3, 2.0),
    // -- Gate lane: amp ducker ("Pulse") --
    d_bool(
        ParamId::GateAenvEnabled,
        "Pulse Enable",
        "pulse/enabled",
        false,
    ),
    d_linear(
        ParamId::GateAenvPattern,
        "Pulse Pattern",
        "pulse/pattern",
        0.0,
        65535.0,
        0.0,
        "",
    ),
    d_discrete(
        ParamId::GateAenvLength,
        "Pulse Length",
        "pulse/length",
        16,
        15.0,
    ),
    d_discrete(
        ParamId::GateAenvDivision,
        "Pulse Division",
        "pulse/division",
        14,
        3.0,
    ),
    d_log(
        ParamId::GateAenvRate,
        "Pulse Rate",
        "pulse/rate",
        0.1,
        40.0,
        4.0,
        "Hz",
    ),
    d_linear(
        ParamId::GateAenvDepth,
        "Pulse Depth",
        "pulse/depth",
        0.0,
        1.0,
        0.0,
        "",
    ),
    // -- Gate lane: LFO1 retrigger --
    d_bool(
        ParamId::GateLfo1Enabled,
        "LFO1 Gate Enable",
        "gate/lfo1/enabled",
        false,
    ),
    d_linear(
        ParamId::GateLfo1Pattern,
        "LFO1 Gate Pattern",
        "gate/lfo1/pattern",
        0.0,
        65535.0,
        0.0,
        "",
    ),
    d_discrete(
        ParamId::GateLfo1Length,
        "LFO1 Gate Length",
        "gate/lfo1/length",
        16,
        15.0,
    ),
    d_discrete(
        ParamId::GateLfo1Division,
        "LFO1 Gate Division",
        "gate/lfo1/division",
        14,
        3.0,
    ),
    d_log(
        ParamId::GateLfo1Rate,
        "LFO1 Gate Rate",
        "gate/lfo1/rate",
        0.1,
        40.0,
        4.0,
        "Hz",
    ),
    // -- Gate lane: LFO2 retrigger --
    d_bool(
        ParamId::GateLfo2Enabled,
        "LFO2 Gate Enable",
        "gate/lfo2/enabled",
        false,
    ),
    d_linear(
        ParamId::GateLfo2Pattern,
        "LFO2 Gate Pattern",
        "gate/lfo2/pattern",
        0.0,
        65535.0,
        0.0,
        "",
    ),
    d_discrete(
        ParamId::GateLfo2Length,
        "LFO2 Gate Length",
        "gate/lfo2/length",
        16,
        15.0,
    ),
    d_discrete(
        ParamId::GateLfo2Division,
        "LFO2 Gate Division",
        "gate/lfo2/division",
        14,
        3.0,
    ),
    d_log(
        ParamId::GateLfo2Rate,
        "LFO2 Gate Rate",
        "gate/lfo2/rate",
        0.1,
        40.0,
        4.0,
        "Hz",
    ),
    // -- Amp envelope + glide + master --
    d_log(
        ParamId::AmpAttack,
        "Amp Attack",
        "amp/attack",
        0.001,
        5.0,
        0.01,
        "s",
    ),
    d_log(
        ParamId::AmpDecay,
        "Amp Decay",
        "amp/decay",
        0.001,
        5.0,
        0.15,
        "s",
    ),
    d_linear(
        ParamId::AmpSustain,
        "Amp Sustain",
        "amp/sustain",
        0.0,
        1.0,
        0.7,
        "",
    ),
    d_log(
        ParamId::AmpRelease,
        "Amp Release",
        "amp/release",
        0.001,
        10.0,
        0.4,
        "s",
    ),
    d_log(
        ParamId::GlideTime,
        "Glide Time",
        "glide/time",
        0.0,
        0.5,
        0.0,
        "s",
    ),
    d_linear(
        ParamId::MasterVolume,
        "Master Volume",
        "master/vol",
        0.0,
        1.0,
        0.8,
        "",
    ),
    d_linear(
        ParamId::GlobalVolume,
        "Global Volume",
        "master/global_vol",
        0.0,
        1.0,
        0.8,
        "",
    ),
    d_bool(
        ParamId::LimiterEnabled,
        "Limiter Enabled",
        "master/limiter/enabled",
        true,
    ),
    d_linear(
        ParamId::LimiterThreshold,
        "Limiter Threshold",
        "master/limiter/threshold",
        0.5,
        1.0,
        0.95,
        "",
    ),
    // -- FX: Overdrive --
    d_linear(
        ParamId::FxOverdriveDrive,
        "OD Drive",
        "fx/overdrive/drive",
        1.0,
        10.0,
        3.0,
        "",
    ),
    d_linear(
        ParamId::FxOverdriveMix,
        "OD Mix",
        "fx/overdrive/mix",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::FxOverdriveTone,
        "OD Tone",
        "fx/overdrive/tone",
        0.0,
        1.0,
        0.8,
        "",
    ),
    d_linear(
        ParamId::FxOverdriveAsym,
        "OD Asym",
        "fx/overdrive/asym",
        0.0,
        1.0,
        0.0,
        "",
    ),
    // -- FX: Distortion --
    d_linear(
        ParamId::FxDistortionDrive,
        "Dist Drive",
        "fx/distortion/drive",
        1.0,
        20.0,
        8.0,
        "",
    ),
    d_linear(
        ParamId::FxDistortionMix,
        "Dist Mix",
        "fx/distortion/mix",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::FxDistortionTone,
        "Dist Tone",
        "fx/distortion/tone",
        0.0,
        1.0,
        0.8,
        "",
    ),
    d_linear(
        ParamId::FxDistortionPre,
        "Dist Pre",
        "fx/distortion/pre",
        0.0,
        1.0,
        0.0,
        "",
    ),
    // -- FX: Chorus --
    d_log(
        ParamId::FxChorusRate,
        "Chorus Rate",
        "fx/chorus/rate",
        0.1,
        5.0,
        0.8,
        "Hz",
    ),
    d_linear(
        ParamId::FxChorusDepth,
        "Chorus Depth",
        "fx/chorus/depth",
        0.0,
        0.02,
        0.008,
        "s",
    ),
    d_linear(
        ParamId::FxChorusMix,
        "Chorus Mix",
        "fx/chorus/mix",
        0.0,
        1.0,
        0.0,
        "",
    ),
    // -- FX: Delay --
    d_log(
        ParamId::FxDelayTime,
        "Delay Time",
        "fx/delay/time",
        0.0,
        1.0,
        0.35,
        "s",
    ),
    d_linear(
        ParamId::FxDelayFeedback,
        "Delay Feedback",
        "fx/delay/feedback",
        0.0,
        0.95,
        0.4,
        "",
    ),
    d_linear(
        ParamId::FxDelayMix,
        "Delay Mix",
        "fx/delay/mix",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_discrete(ParamId::FxDelaySync, "Delay Sync", "fx/delay/sync", 2, 0.0),
    d_discrete(
        ParamId::FxDelayDivision,
        "Delay Division",
        "fx/delay/division",
        16,
        8.0,
    ),
    // -- FX: Reverb --
    d_linear(
        ParamId::FxReverbSize,
        "Reverb Size",
        "fx/reverb/size",
        0.0,
        1.0,
        0.6,
        "",
    ),
    d_linear(
        ParamId::FxReverbDamp,
        "Reverb Damp",
        "fx/reverb/damp",
        0.0,
        1.0,
        0.5,
        "",
    ),
    d_linear(
        ParamId::FxReverbMix,
        "Reverb Mix",
        "fx/reverb/mix",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::FxReverbPredelay,
        "Reverb Predelay",
        "fx/reverb/predelay",
        0.0,
        0.1,
        0.0,
        "s",
    ),
    d_discrete(
        ParamId::FxReverbType,
        "Reverb Type",
        "fx/reverb/type",
        3,
        0.0,
    ),
    // -- Stereo --
    d_linear(
        ParamId::StereoSpread,
        "Stereo Spread",
        "stereo/spread",
        0.0,
        0.012,
        0.0,
        "s",
    ),
    d_linear(
        ParamId::StereoWidth,
        "Stereo Width",
        "stereo/width",
        0.0,
        2.0,
        1.0,
        "",
    ),
    // -- Shimmer --
    d_linear(
        ParamId::ShimmerSize,
        "Shimmer Size",
        "fx/shimmer/size",
        0.0,
        1.0,
        0.6,
        "",
    ),
    d_linear(
        ParamId::ShimmerDamp,
        "Shimmer Damp",
        "fx/shimmer/damp",
        0.0,
        1.0,
        0.5,
        "",
    ),
    d_linear(
        ParamId::ShimmerMix,
        "Shimmer Mix",
        "fx/shimmer/mix",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::ShimmerAmount,
        "Shimmer Amount",
        "fx/shimmer/shimmer",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::ShimmerWidth,
        "Shimmer Width",
        "fx/shimmer/width",
        0.5,
        2.0,
        1.0,
        "",
    ),
    d_linear(
        ParamId::ShimmerSpread,
        "Shimmer Spread",
        "fx/shimmer/spread",
        0.0,
        0.3,
        0.0,
        "",
    ),
    d_discrete(
        ParamId::ShimmerPitch,
        "Shimmer Pitch",
        "fx/shimmer/pitch",
        3,
        0.0,
    ),
    // -- Crystallizer --
    d_linear(
        ParamId::CrystalGrain,
        "Crystal Grain",
        "fx/crystal/grain",
        10.0,
        500.0,
        80.0,
        "ms",
    ),
    d_linear(
        ParamId::CrystalScatter,
        "Crystal Scatter",
        "fx/crystal/scatter",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_linear(
        ParamId::CrystalFeedback,
        "Crystal Feedback",
        "fx/crystal/feedback",
        0.0,
        0.95,
        0.0,
        "",
    ),
    d_linear(
        ParamId::CrystalDelay,
        "Crystal Delay",
        "fx/crystal/delay",
        0.0,
        500.0,
        0.0,
        "ms",
    ),
    d_linear(
        ParamId::CrystalMix,
        "Crystal Mix",
        "fx/crystal/mix",
        0.0,
        1.0,
        0.0,
        "",
    ),
    d_discrete(
        ParamId::CrystalPitch,
        "Crystal Pitch",
        "fx/crystal/pitch",
        5,
        0.0,
    ),
    // -- Arp --
    d_bool(ParamId::ArpEnabled, "Arp Enabled", "arp/enabled", false),
    d_discrete(ParamId::ArpMode, "Arp Mode", "arp/mode", 5, 0.0),
    d_discrete(
        ParamId::ArpDivision,
        "Arp Division",
        "arp/division",
        16,
        2.0,
    ),
    d_discrete(
        ParamId::ArpOctaveRange,
        "Arp Octave Range",
        "arp/octave_range",
        4,
        1.0,
    ),
    d_linear(ParamId::ArpGate, "Arp Gate", "arp/gate", 0.05, 1.0, 0.5, ""),
    d_bool(ParamId::ArpHold, "Arp Hold", "arp/hold", false),
    d_log(
        ParamId::ArpBpm,
        "Arp BPM",
        "arp/bpm",
        20.0,
        300.0,
        120.0,
        "BPM",
    ),
    // -- Walker --
    d_bool(
        ParamId::WalkerEnabled,
        "Walker Enabled",
        "walker/enabled",
        false,
    ),
    d_discrete(ParamId::WalkerScale, "Walker Scale", "walker/scale", 8, 0.0),
    d_discrete(ParamId::WalkerRoot, "Walker Root", "walker/root", 128, 60.0),
    d_discrete(
        ParamId::WalkerOctaveRange,
        "Walker Octave Range",
        "walker/octave_range",
        3,
        1.0,
    ),
    d_discrete(
        ParamId::WalkerDivision,
        "Walker Division",
        "walker/division",
        16,
        2.0,
    ),
    d_linear(
        ParamId::WalkerGate,
        "Walker Gate",
        "walker/gate",
        0.05,
        1.0,
        0.5,
        "",
    ),
    d_log(
        ParamId::WalkerBpm,
        "Walker BPM",
        "walker/bpm",
        20.0,
        300.0,
        120.0,
        "BPM",
    ),
];

// ---------------------------------------------------------------------------
// Command — every operation the engine can perform
// ---------------------------------------------------------------------------

/// Every operation the engine can perform, as a serialisable value.
///
/// Track-less for Stage 1. The handle's `apply(Command)` internally emits
/// `ControlEvent::*` with `track: 0`. Multi-track support can be added as a
/// non-breaking addition (e.g., `NoteOnTracked`) later without disturbing
/// existing callers.
///
/// `ApplyPatch` intentionally omitted in Stage 1 — `Patch` lives in
/// `forma` crate today and moves in Stage 3.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub enum Command {
    SetParam { id: ParamId, value: f32 },
    NoteOn { pitch: u8, velocity: u8 },
    NoteOff { pitch: u8 },
    AllNotesOff,
    ChordHold(Vec<u8>),
    ArpRestart,
    WalkerRestart,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn descriptor_table_invariants() {
        let params = all_params();
        assert!(!params.is_empty());

        let mut ids: HashSet<ParamId> = HashSet::new();
        for desc in params {
            assert!(
                ids.insert(desc.id),
                "duplicate ParamId in descriptor table: {:?}",
                desc.id
            );
            assert!(
                desc.min <= desc.default,
                "{:?}: min {} > default {}",
                desc.id,
                desc.min,
                desc.default
            );
            assert!(
                desc.default <= desc.max,
                "{:?}: default {} > max {}",
                desc.id,
                desc.default,
                desc.max
            );
        }
    }

    #[test]
    fn format_covers_common_units() {
        let desc_hz = d_log(ParamId::FilterCutoff, "", "", 0.0, 20000.0, 0.0, "Hz");
        assert_eq!(desc_hz.format(500.0), "500 Hz");
        assert_eq!(desc_hz.format(1500.0), "1.50 kHz");

        let desc_s = d_log(ParamId::AmpAttack, "", "", 0.0, 10.0, 0.0, "s");
        assert_eq!(desc_s.format(0.25), "250 ms");
        assert_eq!(desc_s.format(2.5), "2.50 s");

        let desc_bool = d_bool(ParamId::ArpEnabled, "", "", false);
        assert_eq!(desc_bool.format(0.0), "off");
        assert_eq!(desc_bool.format(1.0), "on");

        let desc_pct = d_linear(ParamId::MasterVolume, "", "", 0.0, 1.0, 0.0, "");
        assert_eq!(desc_pct.format(0.5), "50%");
    }
}
