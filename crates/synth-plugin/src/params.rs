//! nih-plug Params structs mirroring the engine's AudioState / ParamDescriptor table.
//!
//! Enums for discrete selectors (waveform, LFO shape, etc.) are defined here
//! as local types — they can't re-use fundsp's WaveShape because nih-plug's
//! `Enum` derive must run on a type in this crate.

use nih_plug::prelude::*;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Discrete selector enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum WaveShapeParam {
    Sine,
    Saw,
    Square,
    Triangle,
}

impl WaveShapeParam {
    pub fn idx(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum LfoShapeParam {
    Sine,
    Triangle,
    Saw,
}

impl LfoShapeParam {
    pub fn idx(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum LfoDestParam {
    Pitch,
    Filter,
    Amp,
}

impl LfoDestParam {
    pub fn idx(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ReverbTypeParam {
    Freeverb,
    Plate,
    FdnHall,
}

impl ReverbTypeParam {
    pub fn idx(self) -> u8 {
        self as u8
    }
}

// ---------------------------------------------------------------------------
// Per-oscillator params (instantiated 3× with id_prefix)
// ---------------------------------------------------------------------------

#[derive(Params)]
pub struct OscParams {
    #[id = "wave"]
    pub wave: EnumParam<WaveShapeParam>,

    #[id = "vol"]
    pub vol: FloatParam,

    #[id = "freq_mult"]
    pub freq_mult: FloatParam,

    #[id = "pw"]
    pub pulse_width: FloatParam,

    // Unison detune: 5 copies (copy 0 = no detune, copies 1-4 = spread)
    #[id = "ud0"]
    pub unison_detune_0: FloatParam,
    #[id = "ud1"]
    pub unison_detune_1: FloatParam,
    #[id = "ud2"]
    pub unison_detune_2: FloatParam,
    #[id = "ud3"]
    pub unison_detune_3: FloatParam,
    #[id = "ud4"]
    pub unison_detune_4: FloatParam,

    // Unison volume: 5 copies
    #[id = "uv0"]
    pub unison_vol_0: FloatParam,
    #[id = "uv1"]
    pub unison_vol_1: FloatParam,
    #[id = "uv2"]
    pub unison_vol_2: FloatParam,
    #[id = "uv3"]
    pub unison_vol_3: FloatParam,
    #[id = "uv4"]
    pub unison_vol_4: FloatParam,
}

fn osc_params(wave_default: WaveShapeParam, vol_default: f32) -> OscParams {
    OscParams {
        wave: EnumParam::new("Wave", wave_default),
        vol: FloatParam::new(
            "Volume",
            vol_default,
            FloatRange::Linear { min: 0.0, max: 1.0 },
        )
        .with_unit("%")
        .with_value_to_string(formatters::v2s_f32_percentage(1)),
        freq_mult: FloatParam::new(
            "Freq Mult",
            1.0,
            FloatRange::Linear {
                min: 0.25,
                max: 4.0,
            },
        ),
        pulse_width: FloatParam::new(
            "Pulse Width",
            0.5,
            FloatRange::Linear {
                min: 0.01,
                max: 0.99,
            },
        )
        .with_unit("%")
        .with_value_to_string(formatters::v2s_f32_percentage(1)),
        unison_detune_0: FloatParam::new(
            "Unison Detune 1",
            1.0,
            FloatRange::Linear { min: 0.9, max: 1.1 },
        ),
        unison_detune_1: FloatParam::new(
            "Unison Detune 2",
            1.0,
            FloatRange::Linear { min: 0.9, max: 1.1 },
        ),
        unison_detune_2: FloatParam::new(
            "Unison Detune 3",
            1.0,
            FloatRange::Linear { min: 0.9, max: 1.1 },
        ),
        unison_detune_3: FloatParam::new(
            "Unison Detune 4",
            1.0,
            FloatRange::Linear { min: 0.9, max: 1.1 },
        ),
        unison_detune_4: FloatParam::new(
            "Unison Detune 5",
            1.0,
            FloatRange::Linear { min: 0.9, max: 1.1 },
        ),
        unison_vol_0: FloatParam::new(
            "Unison Vol 1",
            1.0,
            FloatRange::Linear { min: 0.0, max: 1.0 },
        ),
        unison_vol_1: FloatParam::new(
            "Unison Vol 2",
            0.0,
            FloatRange::Linear { min: 0.0, max: 1.0 },
        ),
        unison_vol_2: FloatParam::new(
            "Unison Vol 3",
            0.0,
            FloatRange::Linear { min: 0.0, max: 1.0 },
        ),
        unison_vol_3: FloatParam::new(
            "Unison Vol 4",
            0.0,
            FloatRange::Linear { min: 0.0, max: 1.0 },
        ),
        unison_vol_4: FloatParam::new(
            "Unison Vol 5",
            0.0,
            FloatRange::Linear { min: 0.0, max: 1.0 },
        ),
    }
}

// ---------------------------------------------------------------------------
// Filter params
// ---------------------------------------------------------------------------

#[derive(Params)]
pub struct FilterParams {
    #[id = "cutoff"]
    pub cutoff: FloatParam,

    #[id = "res"]
    pub resonance: FloatParam,

    #[id = "drive"]
    pub drive: FloatParam,

    #[id = "key_track"]
    pub key_track: FloatParam,

    #[id = "env_amt"]
    pub env_amount: FloatParam,

    #[id = "atk"]
    pub attack: FloatParam,

    #[id = "dec"]
    pub decay: FloatParam,

    #[id = "sus"]
    pub sustain: FloatParam,

    #[id = "rel"]
    pub release: FloatParam,
}

impl Default for FilterParams {
    fn default() -> Self {
        Self {
            cutoff: FloatParam::new(
                "Cutoff",
                3000.0,
                FloatRange::Skewed {
                    min: 80.0,
                    max: 18000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(2)),
            resonance: FloatParam::new(
                "Resonance",
                0.3,
                FloatRange::Linear {
                    min: 0.1,
                    max: 20.0,
                },
            ),
            drive: FloatParam::new(
                "Filter Drive",
                1.0,
                FloatRange::Linear {
                    min: 1.0,
                    max: 10.0,
                },
            ),
            key_track: FloatParam::new("Key Track", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(1)),
            env_amount: FloatParam::new(
                "Env Amount",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
            attack: FloatParam::new(
                "Attack",
                0.01,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 5.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" s"),
            decay: FloatParam::new(
                "Decay",
                0.3,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 5.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" s"),
            sustain: FloatParam::new("Sustain", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(1)),
            release: FloatParam::new(
                "Release",
                0.2,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" s"),
        }
    }
}

// ---------------------------------------------------------------------------
// LFO params
// ---------------------------------------------------------------------------

#[derive(Params)]
pub struct LfoParams {
    #[id = "rate"]
    pub rate: FloatParam,

    #[id = "depth"]
    pub depth: FloatParam,

    #[id = "shape"]
    pub shape: EnumParam<LfoShapeParam>,

    #[id = "dest"]
    pub dest: EnumParam<LfoDestParam>,
}

fn lfo_params() -> LfoParams {
    LfoParams {
        rate: FloatParam::new(
            "Rate",
            1.0,
            FloatRange::Skewed {
                min: 0.1,
                max: 20.0,
                factor: FloatRange::skew_factor(-1.0),
            },
        )
        .with_unit(" Hz"),
        depth: FloatParam::new("Depth", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
        shape: EnumParam::new("Shape", LfoShapeParam::Sine),
        dest: EnumParam::new("Dest", LfoDestParam::Pitch),
    }
}

// ---------------------------------------------------------------------------
// Amp envelope + glide
// ---------------------------------------------------------------------------

#[derive(Params)]
pub struct AmpParams {
    #[id = "atk"]
    pub attack: FloatParam,

    #[id = "dec"]
    pub decay: FloatParam,

    #[id = "sus"]
    pub sustain: FloatParam,

    #[id = "rel"]
    pub release: FloatParam,

    #[id = "glide"]
    pub glide_time: FloatParam,
}

impl Default for AmpParams {
    fn default() -> Self {
        Self {
            attack: FloatParam::new(
                "Attack",
                0.005,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 2.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" s"),
            decay: FloatParam::new(
                "Decay",
                0.1,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 2.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" s"),
            sustain: FloatParam::new("Sustain", 0.7, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(1)),
            release: FloatParam::new(
                "Release",
                0.15,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 4.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" s"),
            glide_time: FloatParam::new("Glide", 0.0, FloatRange::Linear { min: 0.0, max: 0.5 })
                .with_unit(" s"),
        }
    }
}

// ---------------------------------------------------------------------------
// FX params
// ---------------------------------------------------------------------------

#[derive(Params)]
pub struct FxParams {
    // Overdrive
    #[id = "od_drive"]
    pub overdrive_drive: FloatParam,
    #[id = "od_mix"]
    pub overdrive_mix: FloatParam,
    #[id = "od_tone"]
    pub overdrive_tone: FloatParam,
    #[id = "od_asym"]
    pub overdrive_asym: FloatParam,

    // Distortion
    #[id = "dist_drive"]
    pub distortion_drive: FloatParam,
    #[id = "dist_mix"]
    pub distortion_mix: FloatParam,
    #[id = "dist_tone"]
    pub distortion_tone: FloatParam,
    #[id = "dist_pre"]
    pub distortion_pre: FloatParam,

    // Chorus
    #[id = "ch_rate"]
    pub chorus_rate: FloatParam,
    #[id = "ch_depth"]
    pub chorus_depth: FloatParam,
    #[id = "ch_mix"]
    pub chorus_mix: FloatParam,

    // Delay
    #[id = "dly_time"]
    pub delay_time: FloatParam,
    #[id = "dly_fb"]
    pub delay_feedback: FloatParam,
    #[id = "dly_mix"]
    pub delay_mix: FloatParam,

    // Reverb
    #[id = "rv_size"]
    pub reverb_size: FloatParam,
    #[id = "rv_damp"]
    pub reverb_damp: FloatParam,
    #[id = "rv_mix"]
    pub reverb_mix: FloatParam,
    #[id = "rv_pre"]
    pub reverb_predelay: FloatParam,
    #[id = "rv_type"]
    pub reverb_type: EnumParam<ReverbTypeParam>,

    // Stereo
    #[id = "st_spread"]
    pub stereo_spread: FloatParam,
    #[id = "st_width"]
    pub stereo_width: FloatParam,

    // Shimmer
    #[id = "sh_size"]
    pub shimmer_size: FloatParam,
    #[id = "sh_damp"]
    pub shimmer_damp: FloatParam,
    #[id = "sh_mix"]
    pub shimmer_mix: FloatParam,
    #[id = "sh_amt"]
    pub shimmer_amount: FloatParam,
    #[id = "sh_width"]
    pub shimmer_width: FloatParam,
    #[id = "sh_spread"]
    pub shimmer_spread: FloatParam,

    // Crystallizer
    #[id = "cr_grain"]
    pub crystal_grain: FloatParam,
    #[id = "cr_scatter"]
    pub crystal_scatter: FloatParam,
    #[id = "cr_fb"]
    pub crystal_feedback: FloatParam,
    #[id = "cr_delay"]
    pub crystal_delay: FloatParam,
    #[id = "cr_mix"]
    pub crystal_mix: FloatParam,
}

impl Default for FxParams {
    fn default() -> Self {
        Self {
            overdrive_drive: FloatParam::new(
                "OD Drive",
                1.0,
                FloatRange::Linear {
                    min: 1.0,
                    max: 10.0,
                },
            ),
            overdrive_mix: FloatParam::new(
                "OD Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
            overdrive_tone: FloatParam::new(
                "OD Tone",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            overdrive_asym: FloatParam::new(
                "OD Asym",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            distortion_drive: FloatParam::new(
                "Dist Drive",
                1.0,
                FloatRange::Linear {
                    min: 1.0,
                    max: 20.0,
                },
            ),
            distortion_mix: FloatParam::new(
                "Dist Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
            distortion_tone: FloatParam::new(
                "Dist Tone",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            distortion_pre: FloatParam::new(
                "Dist Pre",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            chorus_rate: FloatParam::new(
                "Chorus Rate",
                0.5,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 5.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" Hz"),
            chorus_depth: FloatParam::new(
                "Chorus Depth",
                0.005,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.02,
                },
            )
            .with_unit(" s"),
            chorus_mix: FloatParam::new(
                "Chorus Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
            delay_time: FloatParam::new(
                "Delay Time",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit(" s"),
            delay_feedback: FloatParam::new(
                "Delay Feedback",
                0.3,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.95,
                },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
            delay_mix: FloatParam::new("Delay Mix", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(1)),
            reverb_size: FloatParam::new(
                "Reverb Size",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            reverb_damp: FloatParam::new(
                "Reverb Damp",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            reverb_mix: FloatParam::new(
                "Reverb Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
            reverb_predelay: FloatParam::new(
                "Reverb Predelay",
                0.0,
                FloatRange::Linear { min: 0.0, max: 0.1 },
            )
            .with_unit(" s"),
            reverb_type: EnumParam::new("Reverb Type", ReverbTypeParam::Freeverb),
            stereo_spread: FloatParam::new(
                "Stereo Spread",
                0.002,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.012,
                },
            )
            .with_unit(" s"),
            stereo_width: FloatParam::new(
                "Stereo Width",
                1.0,
                FloatRange::Linear { min: 0.0, max: 2.0 },
            ),
            shimmer_size: FloatParam::new(
                "Shimmer Size",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            shimmer_damp: FloatParam::new(
                "Shimmer Damp",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            shimmer_mix: FloatParam::new(
                "Shimmer Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
            shimmer_amount: FloatParam::new(
                "Shimmer Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
            shimmer_width: FloatParam::new(
                "Shimmer Width",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            shimmer_spread: FloatParam::new(
                "Shimmer Spread",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            crystal_grain: FloatParam::new(
                "Crystal Grain",
                80.0,
                FloatRange::Linear {
                    min: 10.0,
                    max: 400.0,
                },
            )
            .with_unit(" ms"),
            crystal_scatter: FloatParam::new(
                "Crystal Scatter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            crystal_feedback: FloatParam::new(
                "Crystal Feedback",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.95,
                },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
            crystal_delay: FloatParam::new(
                "Crystal Delay",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 500.0,
                },
            )
            .with_unit(" ms"),
            crystal_mix: FloatParam::new(
                "Crystal Mix",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
        }
    }
}

// ---------------------------------------------------------------------------
// Master params
// ---------------------------------------------------------------------------

#[derive(Params)]
pub struct MasterParams {
    #[id = "vol"]
    pub master_vol: FloatParam,

    #[id = "gvol"]
    pub global_vol: FloatParam,

    #[id = "lim_on"]
    pub limiter_enabled: BoolParam,

    #[id = "lim_thr"]
    pub limiter_threshold: FloatParam,
}

impl Default for MasterParams {
    fn default() -> Self {
        Self {
            master_vol: FloatParam::new(
                "Master Volume",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
            global_vol: FloatParam::new(
                "Global Volume",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.5 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(1)),
            limiter_enabled: BoolParam::new("Limiter", true),
            limiter_threshold: FloatParam::new(
                "Limiter Threshold",
                0.9,
                FloatRange::Linear { min: 0.5, max: 1.0 },
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level TheSynthParams
// ---------------------------------------------------------------------------

#[derive(Params)]
pub struct TheSynthParams {
    #[nested(group = "OSC 1", id_prefix = "o1_")]
    pub osc1: OscParams,

    #[nested(group = "OSC 2", id_prefix = "o2_")]
    pub osc2: OscParams,

    #[nested(group = "OSC 3", id_prefix = "o3_")]
    pub osc3: OscParams,

    // Modulation sources shared across OSCs
    #[id = "hard_sync"]
    pub hard_sync: BoolParam,

    #[id = "fm_depth"]
    pub fm_depth: FloatParam,

    #[id = "ring_depth"]
    pub ring_depth: FloatParam,

    #[id = "noise_vol"]
    pub noise_vol: FloatParam,

    #[nested(group = "Filter")]
    pub filter: FilterParams,

    #[nested(group = "LFO 1", id_prefix = "l1_")]
    pub lfo1: LfoParams,

    #[nested(group = "LFO 2", id_prefix = "l2_")]
    pub lfo2: LfoParams,

    #[nested(group = "Amp")]
    pub amp: AmpParams,

    #[nested(group = "FX")]
    pub fx: FxParams,

    #[nested(group = "Master")]
    pub master: MasterParams,
}

impl Default for TheSynthParams {
    fn default() -> Self {
        Self {
            osc1: osc_params(WaveShapeParam::Saw, 0.4),
            osc2: osc_params(WaveShapeParam::Sine, 0.3),
            osc3: osc_params(WaveShapeParam::Sine, 0.0),
            hard_sync: BoolParam::new("Hard Sync", false),
            fm_depth: FloatParam::new("FM Depth", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 }),
            ring_depth: FloatParam::new(
                "Ring Depth",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            noise_vol: FloatParam::new("Noise Vol", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(1)),
            filter: FilterParams::default(),
            lfo1: lfo_params(),
            lfo2: lfo_params(),
            amp: AmpParams::default(),
            fx: FxParams::default(),
            master: MasterParams::default(),
        }
    }
}
