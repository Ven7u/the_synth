//! Patch — serialisable snapshot of the synth's sound-generating parameters.
//!
//! Covers: OSC bank, noise, LFO, filter, filter ADSR, amp ADSR, glide, master vol.
//! Excludes: sequencer patterns, keyboard octave, MIDI device, voice state.
//!
//! `Patch::from_app(app)` captures the current UI state.
//! `patch.apply(app)`    writes every field back to the UI state and AudioState Shareds.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Patch struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    pub name:     String,
    pub category: String,
    #[serde(default)]
    pub synth_model: String,

    // OSC bank (3 oscillators)
    pub osc_wave:          [usize; 3],
    pub osc_octave:        [i32;   3],
    pub osc_detune:        [f32;   3],
    pub osc_vol:           [f32;   3],
    pub osc_enabled:       [bool;  3],
    pub osc_pulse_width:   [f32;   3],
    pub osc_pw_enabled:    [bool;  3],
    pub osc_unison_enabled:[bool;  3],
    pub osc_unison_count:  [usize; 3],
    pub osc_unison_spread: [f32;   3],
    pub hard_sync:         bool,
    pub fm_enabled:        bool,
    pub fm_depth:          f32,
    pub ring_enabled:      bool,
    pub ring_depth:        f32,

    // Noise
    pub noise_vol: f32,

    // LFO
    pub lfo_enabled: bool,
    pub lfo_rate:    f32,
    pub lfo_depth:   f32,
    pub lfo_shape:   usize,
    pub lfo_dest:    usize,

    // Filter
    pub filter_enabled:    bool,
    pub filter_cutoff:     f32,
    pub filter_q:          f32,
    pub filter_env_amount: f32,
    pub fenv_adsr:         [f32; 4],

    // Amp
    pub amp_adsr: [f32; 4],

    // Global
    pub glide_time: f32,
    pub master_vol: f32,

    // FX chain (all default to bypass)
    #[serde(default)] pub fx_overdrive_on:    bool,
    #[serde(default = "default_overdrive_drive")] pub fx_overdrive_drive: f32,
    #[serde(default)] pub fx_overdrive_mix:   f32,
    #[serde(default = "default_tone")] pub fx_overdrive_tone: f32,
    #[serde(default)] pub fx_overdrive_asym:  f32,
    #[serde(default)] pub fx_distortion_on:   bool,
    #[serde(default = "default_distortion_drive")] pub fx_distortion_drive: f32,
    #[serde(default)] pub fx_distortion_mix:  f32,
    #[serde(default = "default_tone")] pub fx_distortion_tone: f32,
    #[serde(default)] pub fx_distortion_pre:  f32,
    #[serde(default)] pub fx_chorus_on:       bool,
    #[serde(default = "default_chorus_rate")]  pub fx_chorus_rate:  f32,
    #[serde(default = "default_chorus_depth")] pub fx_chorus_depth: f32,
    #[serde(default)] pub fx_chorus_mix:      f32,
    #[serde(default)] pub fx_delay_on:        bool,
    #[serde(default = "default_delay_time")]  pub fx_delay_time:     f32,
    #[serde(default = "default_delay_fb")]    pub fx_delay_feedback: f32,
    #[serde(default)] pub fx_delay_mix:       f32,
    #[serde(default)] pub fx_reverb_on:       bool,
    #[serde(default = "default_reverb_size")] pub fx_reverb_size:    f32,
    #[serde(default = "default_reverb_damp")] pub fx_reverb_damp:    f32,
    #[serde(default)] pub fx_reverb_mix:      f32,
}

fn default_overdrive_drive()  -> f32 { 3.0 }
fn default_distortion_drive() -> f32 { 8.0 }
fn default_tone()             -> f32 { 0.8 }
fn default_chorus_rate()      -> f32 { 0.8 }
fn default_chorus_depth()     -> f32 { 0.008 }
fn default_delay_time()       -> f32 { 0.35 }
fn default_delay_fb()         -> f32 { 0.4 }
fn default_reverb_size()      -> f32 { 0.6 }
fn default_reverb_damp()      -> f32 { 0.5 }

// ---------------------------------------------------------------------------
// Default patches — loaded from assets/patches/**/*.json at compile time
// ---------------------------------------------------------------------------

pub fn default_patches() -> Vec<Patch> {
    // Each entry is the raw JSON string embedded at compile time.
    // To add a new patch: drop a .json file in assets/patches/<Category>/
    // and add an include_str! line here. No code changes needed elsewhere.
    let embedded: &[&str] = &[
        // Bass
        include_str!("../assets/patches/Bass/Moog Bass.json"),
        include_str!("../assets/patches/Bass/Acid Bass.json"),
        include_str!("../assets/patches/Bass/Sub Bass.json"),
        include_str!("../assets/patches/Bass/Minimoog Bass.json"),
        include_str!("../assets/patches/Bass/Juno Bass.json"),
        include_str!("../assets/patches/Bass/OB Bass.json"),
        include_str!("../assets/patches/Bass/Prophet Bass.json"),
        include_str!("../assets/patches/Bass/ARP Bass.json"),
        include_str!("../assets/patches/Bass/Taurus Bass.json"),
        include_str!("../assets/patches/Bass/Sub 37 Bass.json"),
        include_str!("../assets/patches/Bass/Brute Bass.json"),
        include_str!("../assets/patches/Bass/Serum Bass.json"),
        include_str!("../assets/patches/Bass/Sub Wobble.json"),
        include_str!("../assets/patches/Bass/Vital Bass.json"),
        include_str!("../assets/patches/Bass/Prologue Bass.json"),
        include_str!("../assets/patches/Bass/DX7 Bass.json"),
        include_str!("../assets/patches/Bass/Bass Station Bass.json"),
        include_str!("../assets/patches/Bass/Midnight Crawler.json"),
        include_str!("../assets/patches/Bass/Voltage Storm.json"),
        // Lead
        include_str!("../assets/patches/Lead/Classic Lead.json"),
        include_str!("../assets/patches/Lead/Supersaw Lead.json"),
        include_str!("../assets/patches/Lead/Square Lead.json"),
        include_str!("../assets/patches/Lead/Sync Lead.json"),
        include_str!("../assets/patches/Lead/Minimoog Lead.json"),
        include_str!("../assets/patches/Lead/Juno Lead.json"),
        include_str!("../assets/patches/Lead/Juno Brass.json"),
        include_str!("../assets/patches/Lead/Acid Lead.json"),
        include_str!("../assets/patches/Lead/OB Lead.json"),
        include_str!("../assets/patches/Lead/Prophet Lead.json"),
        include_str!("../assets/patches/Lead/ARP Lead.json"),
        include_str!("../assets/patches/Lead/Bass Station Lead.json"),
        include_str!("../assets/patches/Lead/Sub 37 Lead.json"),
        include_str!("../assets/patches/Lead/Brute Lead.json"),
        include_str!("../assets/patches/Lead/Serum Lead.json"),
        include_str!("../assets/patches/Lead/Vital Lead.json"),
        include_str!("../assets/patches/Lead/Prologue Lead.json"),
        include_str!("../assets/patches/Lead/DX7 Lead.json"),
        include_str!("../assets/patches/Lead/Acid Rain.json"),
        include_str!("../assets/patches/Lead/Digital Ghost.json"),
        include_str!("../assets/patches/Lead/Neon Razor.json"),
        // Pad
        include_str!("../assets/patches/Pad/Warm Pad.json"),
        include_str!("../assets/patches/Pad/String Pad.json"),
        include_str!("../assets/patches/Pad/Dark Pad.json"),
        include_str!("../assets/patches/Pad/Minimoog Pad.json"),
        include_str!("../assets/patches/Pad/Juno Pad.json"),
        include_str!("../assets/patches/Pad/OB Strings.json"),
        include_str!("../assets/patches/Pad/Prophet Pad.json"),
        include_str!("../assets/patches/Pad/ARP Pad.json"),
        include_str!("../assets/patches/Pad/Sub 37 Pad.json"),
        include_str!("../assets/patches/Pad/Brute Pad.json"),
        include_str!("../assets/patches/Pad/Serum Hyper Saw.json"),
        include_str!("../assets/patches/Pad/Vital Pad.json"),
        include_str!("../assets/patches/Pad/Prologue Pad.json"),
        include_str!("../assets/patches/Pad/DX7 Pad.json"),
        include_str!("../assets/patches/Pad/Bass Station Pad.json"),
        include_str!("../assets/patches/Pad/Neon Drift.json"),
        include_str!("../assets/patches/Pad/Deep Space.json"),
        include_str!("../assets/patches/Pad/Aurora.json"),
        // Keys
        include_str!("../assets/patches/Keys/Electric Piano.json"),
        include_str!("../assets/patches/Keys/Organ.json"),
        include_str!("../assets/patches/Keys/Clav.json"),
        include_str!("../assets/patches/Keys/Juno Keys.json"),
        include_str!("../assets/patches/Keys/Prophet Keys.json"),
        include_str!("../assets/patches/Keys/Sub 37 Keys.json"),
        include_str!("../assets/patches/Keys/Serum Keys.json"),
        include_str!("../assets/patches/Keys/Vital Keys.json"),
        include_str!("../assets/patches/Keys/Prologue Keys.json"),
        include_str!("../assets/patches/Keys/DX7 Electric Piano.json"),
        include_str!("../assets/patches/Keys/Crystal Bell.json"),
        include_str!("../assets/patches/Keys/Spectral Keys.json"),
        // Pluck
        include_str!("../assets/patches/Pluck/Pluck.json"),
        include_str!("../assets/patches/Pluck/Acid Pluck.json"),
        include_str!("../assets/patches/Pluck/Minimoog Pluck.json"),
        include_str!("../assets/patches/Pluck/ARP Pluck.json"),
        include_str!("../assets/patches/Pluck/Serum Pluck.json"),
        include_str!("../assets/patches/Pluck/Vital Pluck.json"),
        include_str!("../assets/patches/Pluck/Glass Harp.json"),
        include_str!("../assets/patches/Pluck/Gravity Pluck.json"),
        // Brass
        include_str!("../assets/patches/Brass/Brass.json"),
        include_str!("../assets/patches/Brass/Mellow Brass.json"),
        include_str!("../assets/patches/Brass/Minimoog Brass.json"),
        include_str!("../assets/patches/Brass/OB Brass.json"),
        include_str!("../assets/patches/Brass/Prophet Brass.json"),
        include_str!("../assets/patches/Brass/DX7 Brass.json"),
        include_str!("../assets/patches/Brass/Iron Forest.json"),
        // FX
        include_str!("../assets/patches/FX/Laser.json"),
        include_str!("../assets/patches/FX/Noise Sweep.json"),
        include_str!("../assets/patches/FX/Ring Mod Bell.json"),
        include_str!("../assets/patches/FX/FM Metal.json"),
        include_str!("../assets/patches/FX/R2D2.json"),
        include_str!("../assets/patches/FX/Brute Metallic.json"),
        include_str!("../assets/patches/FX/Serum Growl.json"),
        include_str!("../assets/patches/FX/Solar Wind.json"),
        include_str!("../assets/patches/FX/Phantom Signal.json"),
    ];
    embedded.iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect()
}

