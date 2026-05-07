//! `Patch` — serialisable snapshot of the synth's sound-generating parameters.
//!
//! Covers oscillator bank, noise, LFOs, filter, filter ADSR, amp ADSR, glide,
//! master / global volume, limiter, and the entire FX chain. Excludes
//! sequencer patterns, keyboard octave, MIDI device, voice state.
//!
//! `Patch` lives in `synth-engine` so that any frontend — egui, Bevy, Swift,
//! WebSocket bridge, DAW plugin — can round-trip patches without
//! re-implementing the schema. The handle has `apply_patch(&Patch)` and
//! `snapshot_patch()` methods that treat this struct as the canonical
//! engine-state snapshot.
//!
//! A handful of fields (`osc_enabled`, `fm_enabled`, `filter_enabled`,
//! `*_on`, `osc_pw_enabled`, …) are UI-side "bypass" flags: the engine has
//! no separate enable bits — it bypasses by muting volume / zeroing depth /
//! maxing filter cutoff / zeroing mix. `apply_patch` interprets these
//! flags when writing to the engine.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Patch struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    pub name: String,
    pub category: String,
    #[serde(default)]
    pub synth_model: String,

    // OSC bank (3 oscillators)
    pub osc_wave: [usize; 3],
    pub osc_octave: [i32; 3],
    pub osc_detune: [f32; 3],
    pub osc_vol: [f32; 3],
    pub osc_enabled: [bool; 3],
    pub osc_pulse_width: [f32; 3],
    pub osc_pw_enabled: [bool; 3],
    pub osc_unison_enabled: [bool; 3],
    pub osc_unison_count: [usize; 3],
    pub osc_unison_spread: [f32; 3],
    pub hard_sync: bool,
    pub fm_enabled: bool,
    pub fm_depth: f32,
    pub ring_enabled: bool,
    pub ring_depth: f32,

    // Noise
    pub noise_vol: f32,

    // LFO 1
    pub lfo_enabled: bool,
    pub lfo_rate: f32,
    pub lfo_depth: f32,
    pub lfo_shape: usize,
    pub lfo_dest: usize,

    #[serde(default)]
    pub lfo_sync: bool,
    #[serde(default = "default_lfo_division")]
    pub lfo_division: usize,

    // LFO 2
    #[serde(default)]
    pub lfo2_enabled: bool,
    #[serde(default = "default_lfo2_rate")]
    pub lfo2_rate: f32,
    #[serde(default)]
    pub lfo2_depth: f32,
    #[serde(default)]
    pub lfo2_shape: usize,
    #[serde(default = "default_lfo2_dest")]
    pub lfo2_dest: usize,

    // Gate lanes — tempo-synced 16-step gate sequencers per modulation source.
    //   `gate_aenv_*`: master ducker ("Pulse") — fires a fast duck on the master output.
    //   `gate_lfo1_*` / `gate_lfo2_*`: retrigger LFO1 / LFO2 phase to 0 on each "on" step.
    // All fields default to "off" so legacy scenes load unchanged.
    #[serde(default)]
    pub gate_aenv_enabled: bool,
    #[serde(default)]
    pub gate_aenv_pattern: u16,
    #[serde(default = "default_gate_length")]
    pub gate_aenv_length: u8,
    #[serde(default = "default_gate_division")]
    pub gate_aenv_division: usize,
    #[serde(default)]
    pub gate_aenv_depth: f32,
    #[serde(default)]
    pub gate_lfo1_enabled: bool,
    #[serde(default)]
    pub gate_lfo1_pattern: u16,
    #[serde(default = "default_gate_length")]
    pub gate_lfo1_length: u8,
    #[serde(default = "default_gate_division")]
    pub gate_lfo1_division: usize,
    #[serde(default)]
    pub gate_lfo2_enabled: bool,
    #[serde(default)]
    pub gate_lfo2_pattern: u16,
    #[serde(default = "default_gate_length")]
    pub gate_lfo2_length: u8,
    #[serde(default = "default_gate_division")]
    pub gate_lfo2_division: usize,

    // Filter
    pub filter_enabled: bool,
    pub filter_cutoff: f32,
    pub filter_q: f32,
    #[serde(default = "default_filter_drive")]
    pub filter_drive: f32,
    #[serde(default)]
    pub filter_key_track: f32,
    pub filter_env_amount: f32,
    pub fenv_adsr: [f32; 4],

    // Amp
    pub amp_adsr: [f32; 4],

    // Global
    pub glide_time: f32,
    pub master_vol: f32,
    #[serde(default = "default_global_vol")]
    pub global_vol: f32,
    #[serde(default = "default_limiter_enabled")]
    pub limiter_enabled: bool,
    #[serde(default = "default_limiter_threshold")]
    pub limiter_threshold: f32,

    // FX chain (all default to bypass)
    #[serde(default)]
    pub fx_overdrive_on: bool,
    #[serde(default = "default_overdrive_drive")]
    pub fx_overdrive_drive: f32,
    #[serde(default)]
    pub fx_overdrive_mix: f32,
    #[serde(default = "default_tone")]
    pub fx_overdrive_tone: f32,
    #[serde(default)]
    pub fx_overdrive_asym: f32,
    #[serde(default)]
    pub fx_distortion_on: bool,
    #[serde(default = "default_distortion_drive")]
    pub fx_distortion_drive: f32,
    #[serde(default)]
    pub fx_distortion_mix: f32,
    #[serde(default = "default_tone")]
    pub fx_distortion_tone: f32,
    #[serde(default)]
    pub fx_distortion_pre: f32,
    #[serde(default)]
    pub fx_chorus_on: bool,
    #[serde(default = "default_chorus_rate")]
    pub fx_chorus_rate: f32,
    #[serde(default = "default_chorus_depth")]
    pub fx_chorus_depth: f32,
    #[serde(default)]
    pub fx_chorus_mix: f32,
    #[serde(default)]
    pub fx_delay_on: bool,
    #[serde(default = "default_delay_time")]
    pub fx_delay_time: f32,
    #[serde(default = "default_delay_fb")]
    pub fx_delay_feedback: f32,
    #[serde(default)]
    pub fx_delay_mix: f32,
    #[serde(default)]
    pub fx_delay_sync: bool,
    #[serde(default = "default_delay_division")]
    pub fx_delay_division: usize,
    #[serde(default)]
    pub fx_reverb_on: bool,
    #[serde(default = "default_reverb_size")]
    pub fx_reverb_size: f32,
    #[serde(default = "default_reverb_damp")]
    pub fx_reverb_damp: f32,
    #[serde(default)]
    pub fx_reverb_mix: f32,
    #[serde(default)]
    pub fx_reverb_predelay: f32,
    #[serde(default)]
    pub fx_reverb_type: u8, // 0=Freeverb, 1=Plate, 2=FDN Hall
    #[serde(default)]
    pub stereo_spread: f32,
    #[serde(default = "default_stereo_width")]
    pub stereo_width: f32,

    // Shimmer reverb (independent from plain reverb)
    #[serde(default)]
    pub fx_shimmer_on: bool,
    #[serde(default = "default_shimmer_size")]
    pub fx_shimmer_size: f32,
    #[serde(default = "default_shimmer_damp")]
    pub fx_shimmer_damp: f32,
    #[serde(default = "default_shimmer_mix")]
    pub fx_shimmer_mix: f32,
    #[serde(default = "default_shimmer_amt")]
    pub fx_shimmer_amt: f32,
    #[serde(default = "default_shimmer_width")]
    pub fx_shimmer_width: f32,
    #[serde(default = "default_shimmer_spread")]
    pub fx_shimmer_spread: f32,
    #[serde(default = "default_shimmer_pitch")]
    pub fx_shimmer_pitch: u8,
    #[serde(default)]
    pub fx_crystal_on: bool,
    #[serde(default = "default_crystal_mix")]
    pub fx_crystal_mix: f32,
    #[serde(default = "default_crystal_grain")]
    pub fx_crystal_grain_ms: f32,
    #[serde(default = "default_crystal_scatter")]
    pub fx_crystal_scatter: f32,
    #[serde(default = "default_crystal_feedback")]
    pub fx_crystal_feedback: f32,
    #[serde(default = "default_crystal_delay")]
    pub fx_crystal_delay_ms: f32,
    #[serde(default = "default_crystal_pitch")]
    pub fx_crystal_pitch: u8,
}

fn default_filter_drive() -> f32 {
    1.0
}
fn default_stereo_width() -> f32 {
    1.0
}
fn default_lfo_division() -> usize {
    4
}
fn default_gate_length() -> u8 {
    16
}
fn default_gate_division() -> usize {
    // ClockDivision::Eighth = 3
    3
}
fn default_lfo2_rate() -> f32 {
    0.3
}
fn default_lfo2_dest() -> usize {
    2
}
fn default_global_vol() -> f32 {
    0.8
}
fn default_limiter_enabled() -> bool {
    true
}
fn default_limiter_threshold() -> f32 {
    0.95
}
fn default_delay_division() -> usize {
    2
}
fn default_overdrive_drive() -> f32 {
    3.0
}
fn default_distortion_drive() -> f32 {
    8.0
}
fn default_tone() -> f32 {
    0.8
}
fn default_chorus_rate() -> f32 {
    0.8
}
fn default_chorus_depth() -> f32 {
    0.008
}
fn default_delay_time() -> f32 {
    0.35
}
fn default_delay_fb() -> f32 {
    0.4
}
fn default_reverb_size() -> f32 {
    0.6
}
fn default_reverb_damp() -> f32 {
    0.5
}
fn default_shimmer_size() -> f32 {
    0.7
}
fn default_shimmer_damp() -> f32 {
    0.4
}
fn default_shimmer_mix() -> f32 {
    0.4
}
fn default_shimmer_amt() -> f32 {
    0.5
}
fn default_shimmer_width() -> f32 {
    1.35
}
fn default_shimmer_spread() -> f32 {
    0.10
}
fn default_shimmer_pitch() -> u8 {
    1
}
fn default_crystal_mix() -> f32 {
    0.35
}
fn default_crystal_grain() -> f32 {
    120.0
}
fn default_crystal_scatter() -> f32 {
    0.25
}
fn default_crystal_feedback() -> f32 {
    0.35
}
fn default_crystal_delay() -> f32 {
    260.0
}
fn default_crystal_pitch() -> u8 {
    2
}
