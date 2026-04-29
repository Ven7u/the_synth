/// All UI-local synthesis state shared across panels.
///
/// In `the-synth` this is embedded in `SynthApp` and synced to/from the engine
/// at load/save time. In `synth-plugin` it is populated from `TheSynthParams`
/// at the start of each editor frame and written back via `PluginParamWriter`.
#[derive(Clone)]
pub struct SynthUiState {
    // ── Oscillators ──────────────────────────────────────────────────────────
    pub osc_wave: [usize; 3],
    pub osc_octave: [i32; 3],
    pub osc_detune: [f32; 3],
    pub osc_vol: [f32; 3],
    pub osc_enabled: [bool; 3],
    pub osc_pulse_width: [f32; 3],
    pub osc_unison_enabled: [bool; 3],
    pub osc_unison_count: [usize; 3],
    pub osc_unison_spread: [f32; 3],
    pub hard_sync: bool,
    pub fm_enabled: bool,
    pub fm_depth: f32,
    pub ring_enabled: bool,
    pub ring_depth: f32,
    pub osc1_mod_view: bool,

    // ── Noise / master / global (mirrored from engine atomics) ───────────────
    pub noise_vol: f32,
    pub master_vol: f32,
    pub global_vol: f32,
    pub glide_time: f32,
    pub limiter_enabled: bool,
    pub limiter_threshold: f32,

    // ── Filter ───────────────────────────────────────────────────────────────
    pub filter_enabled: bool,
    pub filter_cutoff: f32,
    pub filter_q: f32,
    pub filter_drive: f32,
    pub filter_key_track: f32,
    pub filter_env_amount: f32,

    // ── Filter envelope ADSR ─────────────────────────────────────────────────
    pub fenv_attack: f32,
    pub fenv_decay: f32,
    pub fenv_sustain: f32,
    pub fenv_release: f32,

    // ── Amp envelope ADSR ────────────────────────────────────────────────────
    pub amp_attack: f32,
    pub amp_decay: f32,
    pub amp_sustain: f32,
    pub amp_release: f32,

    // ── LFO 1 ────────────────────────────────────────────────────────────────
    pub lfo_enabled: bool,
    pub lfo_rate: f32,
    pub lfo_depth: f32,
    pub lfo_shape: usize,
    pub lfo_dest: usize,
    pub lfo_sync: bool,
    pub lfo_division: usize,

    // ── LFO 2 ────────────────────────────────────────────────────────────────
    pub lfo2_enabled: bool,
    pub lfo2_rate: f32,
    pub lfo2_depth: f32,
    pub lfo2_shape: usize,
    pub lfo2_dest: usize,

    // ── Tempo / sync ─────────────────────────────────────────────────────────
    pub global_bpm: u32,
    pub global_sync: bool,

    // ── FX — overdrive ───────────────────────────────────────────────────────
    pub fx_overdrive_on: bool,
    pub fx_overdrive_drive: f32,
    pub fx_overdrive_mix: f32,
    pub fx_overdrive_tone: f32,
    pub fx_overdrive_asym: f32,

    // ── FX — distortion ──────────────────────────────────────────────────────
    pub fx_distortion_on: bool,
    pub fx_distortion_drive: f32,
    pub fx_distortion_mix: f32,
    pub fx_distortion_tone: f32,
    pub fx_distortion_pre: f32,

    // ── FX — chorus ──────────────────────────────────────────────────────────
    pub fx_chorus_on: bool,
    pub fx_chorus_rate: f32,
    pub fx_chorus_depth: f32,
    pub fx_chorus_mix: f32,

    // ── FX — delay ───────────────────────────────────────────────────────────
    pub fx_delay_on: bool,
    pub fx_delay_time: f32,
    pub fx_delay_feedback: f32,
    pub fx_delay_mix: f32,
    pub fx_delay_sync: bool,
    pub fx_delay_division: usize,

    // ── FX — reverb ──────────────────────────────────────────────────────────
    pub fx_reverb_on: bool,
    pub fx_reverb_size: f32,
    pub fx_reverb_damp: f32,
    pub fx_reverb_mix: f32,
    pub fx_reverb_predelay: f32,
    pub fx_reverb_type: u8,

    // ── FX — stereo ──────────────────────────────────────────────────────────
    pub stereo_spread: f32,
    pub stereo_width: f32,

    // ── FX — shimmer ─────────────────────────────────────────────────────────
    pub fx_shimmer_on: bool,
    pub fx_shimmer_size: f32,
    pub fx_shimmer_damp: f32,
    pub fx_shimmer_mix: f32,
    pub fx_shimmer_amt: f32,
    pub fx_shimmer_width: f32,
    pub fx_shimmer_spread: f32,
    pub fx_shimmer_pitch: u8,

    // ── FX — crystallizer ────────────────────────────────────────────────────
    pub fx_crystal_on: bool,
    pub fx_crystal_grain_ms: f32,
    pub fx_crystal_scatter: f32,
    pub fx_crystal_feedback: f32,
    pub fx_crystal_delay_ms: f32,
    pub fx_crystal_mix: f32,
    pub fx_crystal_pitch: u8,

    // ── Plugin browser UI state ───────────────────────────────────────────────
    pub browser_open: bool,
    pub browser_search: String,
    pub browser_category: usize, // 0 = All
    pub current_patch_name: String,
    pub patch_load_fx: bool,
}

impl Default for SynthUiState {
    fn default() -> Self {
        Self {
            osc_wave: [1, 0, 0],
            osc_octave: [0, 0, 0],
            osc_detune: [0.0, 0.0, 0.0],
            osc_vol: [0.4, 0.3, 0.0],
            osc_enabled: [true, true, false],
            osc_pulse_width: [0.5, 0.5, 0.5],
            osc_unison_enabled: [false, false, false],
            osc_unison_count: [2, 2, 2],
            osc_unison_spread: [10.0, 10.0, 10.0],
            hard_sync: false,
            fm_enabled: false,
            fm_depth: 1.0,
            ring_enabled: false,
            ring_depth: 1.0,
            osc1_mod_view: false,

            noise_vol: 0.0,
            master_vol: 0.7,
            global_vol: 0.8,
            glide_time: 0.0,
            limiter_enabled: true,
            limiter_threshold: 0.9,

            filter_enabled: true,
            filter_cutoff: 3000.0,
            filter_q: 0.0,
            filter_drive: 1.0,
            filter_key_track: 0.0,
            filter_env_amount: 0.0,

            fenv_attack: 0.01,
            fenv_decay: 0.3,
            fenv_sustain: 0.0,
            fenv_release: 0.1,

            amp_attack: 0.01,
            amp_decay: 0.3,
            amp_sustain: 0.7,
            amp_release: 0.3,

            lfo_enabled: false,
            lfo_rate: 2.0,
            lfo_depth: 0.3,
            lfo_shape: 0,
            lfo_dest: 1,
            lfo_sync: false,
            lfo_division: 2,

            lfo2_enabled: false,
            lfo2_rate: 1.0,
            lfo2_depth: 0.2,
            lfo2_shape: 0,
            lfo2_dest: 2,

            global_bpm: 120,
            global_sync: false,

            fx_overdrive_on: false,
            fx_overdrive_drive: 3.0,
            fx_overdrive_mix: 0.5,
            fx_overdrive_tone: 0.7,
            fx_overdrive_asym: 0.0,

            fx_distortion_on: false,
            fx_distortion_drive: 5.0,
            fx_distortion_mix: 0.5,
            fx_distortion_tone: 0.7,
            fx_distortion_pre: 0.0,

            fx_chorus_on: false,
            fx_chorus_rate: 0.5,
            fx_chorus_depth: 0.003,
            fx_chorus_mix: 0.4,

            fx_delay_on: false,
            fx_delay_time: 0.375,
            fx_delay_feedback: 0.4,
            fx_delay_mix: 0.3,
            fx_delay_sync: false,
            fx_delay_division: 2,

            fx_reverb_on: false,
            fx_reverb_size: 0.5,
            fx_reverb_damp: 0.5,
            fx_reverb_mix: 0.3,
            fx_reverb_predelay: 0.02,
            fx_reverb_type: 0,

            stereo_spread: 0.0,
            stereo_width: 1.0,

            fx_shimmer_on: false,
            fx_shimmer_size: 0.8,
            fx_shimmer_damp: 0.5,
            fx_shimmer_mix: 0.3,
            fx_shimmer_amt: 0.5,
            fx_shimmer_width: 1.0,
            fx_shimmer_spread: 0.1,
            fx_shimmer_pitch: 1,

            fx_crystal_on: false,
            fx_crystal_grain_ms: 80.0,
            fx_crystal_scatter: 0.3,
            fx_crystal_feedback: 0.4,
            fx_crystal_delay_ms: 200.0,
            fx_crystal_mix: 0.3,
            fx_crystal_pitch: 2,

            browser_open: false,
            browser_search: String::new(),
            browser_category: 0,
            current_patch_name: "Init".to_string(),
            patch_load_fx: false,
        }
    }
}

impl SynthUiState {
    pub fn lfo_sync_active(&self) -> bool {
        self.global_sync || self.lfo_sync
    }

    pub fn delay_sync_active(&self) -> bool {
        self.global_sync || self.fx_delay_sync
    }
}
