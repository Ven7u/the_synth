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

    // LFO 1
    pub lfo_enabled: bool,
    pub lfo_rate:    f32,
    pub lfo_depth:   f32,
    pub lfo_shape:   usize,
    pub lfo_dest:    usize,

    // LFO 2
    #[serde(default)] pub lfo2_enabled: bool,
    #[serde(default = "default_lfo2_rate")]  pub lfo2_rate:  f32,
    #[serde(default)] pub lfo2_depth:  f32,
    #[serde(default)] pub lfo2_shape:  usize,
    #[serde(default = "default_lfo2_dest")]  pub lfo2_dest:  usize,

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
    #[serde(default = "default_reverb_size")] pub fx_reverb_size:     f32,
    #[serde(default = "default_reverb_damp")] pub fx_reverb_damp:     f32,
    #[serde(default)] pub fx_reverb_mix:      f32,
    #[serde(default)] pub fx_reverb_predelay: f32,

    // Shimmer reverb (independent from plain reverb)
    #[serde(default)] pub fx_shimmer_on:    bool,
    #[serde(default = "default_shimmer_size")]  pub fx_shimmer_size:  f32,
    #[serde(default = "default_shimmer_damp")]  pub fx_shimmer_damp:  f32,
    #[serde(default = "default_shimmer_mix")]   pub fx_shimmer_mix:   f32,
    #[serde(default = "default_shimmer_amt")]   pub fx_shimmer_amt:   f32,
    #[serde(default = "default_shimmer_width")] pub fx_shimmer_width: f32,
    #[serde(default = "default_shimmer_spread")] pub fx_shimmer_spread: f32,
    #[serde(default = "default_shimmer_pitch")] pub fx_shimmer_pitch: u8,
    #[serde(default)] pub fx_crystal_on: bool,
    #[serde(default = "default_crystal_mix")] pub fx_crystal_mix: f32,
    #[serde(default = "default_crystal_grain")] pub fx_crystal_grain_ms: f32,
    #[serde(default = "default_crystal_scatter")] pub fx_crystal_scatter: f32,
    #[serde(default = "default_crystal_feedback")] pub fx_crystal_feedback: f32,
    #[serde(default = "default_crystal_delay")] pub fx_crystal_delay_ms: f32,
    #[serde(default = "default_crystal_pitch")] pub fx_crystal_pitch: u8,
}

fn default_lfo2_rate() -> f32 { 0.3 }
fn default_lfo2_dest() -> usize { 2 }
fn default_overdrive_drive()  -> f32 { 3.0 }
fn default_distortion_drive() -> f32 { 8.0 }
fn default_tone()             -> f32 { 0.8 }
fn default_chorus_rate()      -> f32 { 0.8 }
fn default_chorus_depth()     -> f32 { 0.008 }
fn default_delay_time()       -> f32 { 0.35 }
fn default_delay_fb()         -> f32 { 0.4 }
fn default_reverb_size()      -> f32 { 0.6 }
fn default_reverb_damp()      -> f32 { 0.5 }
fn default_shimmer_size()     -> f32 { 0.7 }
fn default_shimmer_damp()     -> f32 { 0.4 }
fn default_shimmer_mix()      -> f32 { 0.4 }
fn default_shimmer_amt()      -> f32 { 0.5 }
fn default_shimmer_width()    -> f32 { 1.35 }
fn default_shimmer_spread()   -> f32 { 0.10 }
fn default_shimmer_pitch()    -> u8  { 1 }
fn default_crystal_mix()      -> f32 { 0.35 }
fn default_crystal_grain()    -> f32 { 120.0 }
fn default_crystal_scatter()  -> f32 { 0.25 }
fn default_crystal_feedback() -> f32 { 0.35 }
fn default_crystal_delay()    -> f32 { 260.0 }
fn default_crystal_pitch()    -> u8  { 2 }

// ---------------------------------------------------------------------------
// Default patches — scanned from assets/patches/**/*.json at runtime.
// Drop any .json file into a subfolder and it appears in both apps automatically.
// ---------------------------------------------------------------------------

fn collect_patch_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for ent in entries.flatten() {
        let p = ent.path();
        if p.is_dir() {
            collect_patch_files(&p, out);
        } else if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
            out.push(p);
        }
    }
}

pub fn default_patches() -> Vec<Patch> {
    let mut files = Vec::new();
    collect_patch_files(std::path::Path::new("assets/patches"), &mut files);
    files.sort();
    files.iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|s| serde_json::from_str(&s).ok())
        .collect()
}
