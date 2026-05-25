//! `Patch` — serialisable snapshot of the synth's sound-generating parameters.
//!
//! Covers oscillator bank, noise, LFOs, filter, filter ADSR, amp ADSR, glide,
//! master / global volume, limiter, and the entire FX chain. Excludes
//! sequencer patterns, keyboard octave, MIDI device, voice state.
//!
//! `Patch` lives in `forma-engine` so that any frontend — egui, Bevy, Swift,
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
    /// Free-form labels: character (warm, dark, evolving…), timbre (analog, fm, bell…),
    /// attribution (eno, pink-floyd…). A patch can carry any number.
    #[serde(default)]
    pub tags: Vec<String>,

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
    /// How much velocity scales amplitude. 0 = always full, 1 = full velocity range.
    #[serde(default = "default_vel_amp")]
    pub vel_amp: f32,
    /// How much velocity opens the filter (adds up to 8 kHz at 1.0). 0 = off.
    #[serde(default)]
    pub vel_filter: f32,
    /// Voice mode: 0 = poly, 1 = mono, 2 = legato.
    #[serde(default)]
    pub mono_mode: u8,
    /// Mod wheel destination: 0=Off, 1=Filter, 2=LFO Depth, 3=Amp.
    #[serde(default = "default_mod_wheel_dest")]
    pub mod_wheel_dest: u8,
    /// Mod wheel depth 0–1.
    #[serde(default = "default_mod_wheel_depth")]
    pub mod_wheel_depth: f32,
    /// Aftertouch destination: 0=Off, 1=Filter, 2=LFO Depth, 3=Amp.
    #[serde(default = "default_aftertouch_dest")]
    pub aftertouch_dest: u8,
    /// Aftertouch depth 0–1.
    #[serde(default = "default_aftertouch_depth")]
    pub aftertouch_depth: f32,
    /// Mod matrix: 4 slots × (source, destination, depth).
    ///   src  0=Off 1=LFO1 2=LFO2 3=ModWheel 4=Aftertouch
    ///   dst  0=Off 1=Filter 2=Amp 3=Pitch
    ///   depth -1..+1
    #[serde(default)]
    pub mat_src: [u8; 4],
    #[serde(default)]
    pub mat_dst: [u8; 4],
    #[serde(default)]
    pub mat_depth: [f32; 4],
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

    // Arp ring gate sequencer
    #[serde(default)]
    pub arp_ring_enabled: bool,
    #[serde(default = "default_arp_ring_steps")]
    pub arp_ring_steps: u8,
    #[serde(default = "default_arp_ring_pattern")]
    pub arp_ring_pattern: u32,

    // Per-sequencer clock division (index into SeqClockDiv::LABELS)
    #[serde(default = "default_note_seq_div")]
    pub note_seq_div: u8,
    #[serde(default = "default_chord_seq_div")]
    pub chord_seq_div: u8,

    // Bit crusher
    #[serde(default)]
    pub fx_bitcrush_on: bool,
    #[serde(default = "default_bitcrush_bits")]
    pub fx_bitcrush_bits: f32,
    #[serde(default = "default_bitcrush_rate")]
    pub fx_bitcrush_rate: f32,
    #[serde(default)]
    pub fx_bitcrush_mix: f32,

    // Tape saturation
    #[serde(default)]
    pub fx_tape_on: bool,
    #[serde(default = "default_tape_drive")]
    pub fx_tape_drive: f32,
    #[serde(default = "default_tape_tone")]
    pub fx_tape_tone: f32,
    #[serde(default = "default_tape_bias")]
    pub fx_tape_bias: f32,
    #[serde(default)]
    pub fx_tape_mix: f32,

    // Phaser
    #[serde(default)]
    pub fx_phaser_on: bool,
    #[serde(default = "default_phaser_rate")]
    pub fx_phaser_rate: f32,
    #[serde(default = "default_phaser_depth")]
    pub fx_phaser_depth: f32,
    #[serde(default = "default_phaser_feedback")]
    pub fx_phaser_feedback: f32,
    #[serde(default = "default_phaser_center")]
    pub fx_phaser_center: f32,
    #[serde(default = "default_phaser_stages")]
    pub fx_phaser_stages: u8,
    #[serde(default)]
    pub fx_phaser_mix: f32,
}

fn default_filter_drive() -> f32 {
    1.0
}
fn default_vel_amp() -> f32 {
    1.0
}
fn default_mod_wheel_dest() -> u8 {
    1
}
fn default_mod_wheel_depth() -> f32 {
    0.5
}
fn default_aftertouch_dest() -> u8 {
    1
}
fn default_aftertouch_depth() -> f32 {
    0.3
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
fn default_arp_ring_steps() -> u8 {
    8
}
fn default_arp_ring_pattern() -> u32 {
    0xFF // all 8 steps active
}
fn default_note_seq_div() -> u8 {
    1 // 1/8 note
}
fn default_chord_seq_div() -> u8 {
    4 // 1 bar
}
fn default_bitcrush_bits() -> f32 {
    16.0
}
fn default_bitcrush_rate() -> f32 {
    1.0
}
fn default_tape_drive() -> f32 {
    0.5
}
fn default_tape_tone() -> f32 {
    0.7
}
fn default_tape_bias() -> f32 {
    0.2
}
fn default_phaser_rate() -> f32 {
    0.5
}
fn default_phaser_depth() -> f32 {
    0.7
}
fn default_phaser_feedback() -> f32 {
    0.5
}
fn default_phaser_center() -> f32 {
    1200.0
}
fn default_phaser_stages() -> u8 {
    8
}

impl Default for Patch {
    fn default() -> Self {
        Self {
            name: "Init".into(),
            category: "User".into(),
            synth_model: String::new(),
            tags: Vec::new(),
            // OSC bank: OSC1=saw, OSC2=sine on; OSC3 off
            osc_wave: [1, 0, 0],
            osc_octave: [0, 0, 0],
            osc_detune: [0.0, 0.0, 0.0],
            osc_vol: [0.4, 0.3, 0.5],
            osc_enabled: [true, true, false],
            osc_pulse_width: [0.5, 0.5, 0.5],
            osc_pw_enabled: [false, false, false],
            osc_unison_enabled: [false, false, false],
            osc_unison_count: [2, 2, 2],
            osc_unison_spread: [20.0, 20.0, 20.0],
            hard_sync: false,
            fm_enabled: false,
            fm_depth: 1.0,
            ring_enabled: false,
            ring_depth: 1.0,
            noise_vol: 0.0,
            lfo_enabled: false,
            lfo_rate: 2.0,
            lfo_depth: 0.0,
            lfo_shape: 0,
            lfo_dest: 1,
            lfo_sync: false,
            lfo_division: default_lfo_division(),
            lfo2_enabled: false,
            lfo2_rate: default_lfo2_rate(),
            lfo2_depth: 0.0,
            lfo2_shape: 0,
            lfo2_dest: default_lfo2_dest(),
            gate_aenv_enabled: false,
            gate_aenv_pattern: 0,
            gate_aenv_length: default_gate_length(),
            gate_aenv_division: default_gate_division(),
            gate_aenv_depth: 0.0,
            gate_lfo1_enabled: false,
            gate_lfo1_pattern: 0,
            gate_lfo1_length: default_gate_length(),
            gate_lfo1_division: default_gate_division(),
            gate_lfo2_enabled: false,
            gate_lfo2_pattern: 0,
            gate_lfo2_length: default_gate_length(),
            gate_lfo2_division: default_gate_division(),
            filter_enabled: true,
            filter_cutoff: 3000.0,
            filter_q: 0.3,
            filter_drive: default_filter_drive(),
            filter_key_track: 0.0,
            filter_env_amount: 0.0,
            vel_amp: default_vel_amp(),
            vel_filter: 0.0,
            mono_mode: 0,
            mod_wheel_dest: default_mod_wheel_dest(),
            mod_wheel_depth: default_mod_wheel_depth(),
            aftertouch_dest: default_aftertouch_dest(),
            aftertouch_depth: default_aftertouch_depth(),
            mat_src: [0; 4],
            mat_dst: [0; 4],
            mat_depth: [0.0; 4],
            fenv_adsr: [0.01, 0.3, 0.0, 0.2],
            amp_adsr: [0.01, 0.15, 0.7, 0.4],
            glide_time: 0.0,
            master_vol: 0.8,
            global_vol: default_global_vol(),
            limiter_enabled: default_limiter_enabled(),
            limiter_threshold: default_limiter_threshold(),
            fx_overdrive_on: false,
            fx_overdrive_drive: default_overdrive_drive(),
            fx_overdrive_mix: 0.0,
            fx_overdrive_tone: default_tone(),
            fx_overdrive_asym: 0.0,
            fx_distortion_on: false,
            fx_distortion_drive: default_distortion_drive(),
            fx_distortion_mix: 0.0,
            fx_distortion_tone: default_tone(),
            fx_distortion_pre: 0.0,
            fx_chorus_on: false,
            fx_chorus_rate: default_chorus_rate(),
            fx_chorus_depth: default_chorus_depth(),
            fx_chorus_mix: 0.0,
            fx_delay_on: false,
            fx_delay_time: default_delay_time(),
            fx_delay_feedback: default_delay_fb(),
            fx_delay_mix: 0.0,
            fx_delay_sync: false,
            fx_delay_division: default_delay_division(),
            fx_reverb_on: false,
            fx_reverb_size: default_reverb_size(),
            fx_reverb_damp: default_reverb_damp(),
            fx_reverb_mix: 0.0,
            fx_reverb_predelay: 0.0,
            fx_reverb_type: 0,
            stereo_spread: 0.0,
            stereo_width: default_stereo_width(),
            fx_shimmer_on: false,
            fx_shimmer_size: default_shimmer_size(),
            fx_shimmer_damp: default_shimmer_damp(),
            fx_shimmer_mix: default_shimmer_mix(),
            fx_shimmer_amt: default_shimmer_amt(),
            fx_shimmer_width: default_shimmer_width(),
            fx_shimmer_spread: default_shimmer_spread(),
            fx_shimmer_pitch: default_shimmer_pitch(),
            fx_crystal_on: false,
            fx_crystal_mix: default_crystal_mix(),
            fx_crystal_grain_ms: default_crystal_grain(),
            fx_crystal_scatter: default_crystal_scatter(),
            fx_crystal_feedback: default_crystal_feedback(),
            fx_crystal_delay_ms: default_crystal_delay(),
            fx_crystal_pitch: default_crystal_pitch(),
            arp_ring_enabled: false,
            arp_ring_steps: default_arp_ring_steps(),
            arp_ring_pattern: default_arp_ring_pattern(),
            note_seq_div: default_note_seq_div(),
            chord_seq_div: default_chord_seq_div(),
            fx_bitcrush_on: false,
            fx_bitcrush_bits: default_bitcrush_bits(),
            fx_bitcrush_rate: default_bitcrush_rate(),
            fx_bitcrush_mix: 0.0,
            fx_tape_on: false,
            fx_tape_drive: default_tape_drive(),
            fx_tape_tone: default_tape_tone(),
            fx_tape_bias: default_tape_bias(),
            fx_tape_mix: 0.0,
            fx_phaser_on: false,
            fx_phaser_rate: default_phaser_rate(),
            fx_phaser_depth: default_phaser_depth(),
            fx_phaser_feedback: default_phaser_feedback(),
            fx_phaser_center: default_phaser_center(),
            fx_phaser_stages: default_phaser_stages(),
            fx_phaser_mix: 0.0,
        }
    }
}
