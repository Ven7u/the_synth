/// Abstraction over the synth parameter write path.
///
/// Implemented by `SynthEngineHandle` (standalone) and `PluginParamWriter`
/// (nih-plug). Panels call these methods on user interaction; the host
/// decides where the value actually goes.
pub trait ParamWriter {
    // ── Oscillators ──────────────────────────────────────────────────────────
    fn set_osc_wave(&mut self, osc: u8, wave: u8);
    fn set_osc_freq_mult(&mut self, osc: u8, v: f32);
    fn set_osc_vol(&mut self, osc: u8, v: f32);
    fn set_osc_pulse_width(&mut self, osc: u8, v: f32);
    fn set_osc_unison_detune(&mut self, osc: u8, copy: u8, v: f32);
    fn set_osc_unison_vol(&mut self, osc: u8, copy: u8, v: f32);
    fn set_hard_sync_enabled(&mut self, on: bool);
    fn set_fm_depth(&mut self, v: f32);
    fn set_ring_depth(&mut self, v: f32);
    fn set_noise_vol(&mut self, v: f32);

    // ── Filter ───────────────────────────────────────────────────────────────
    fn set_filter_cutoff(&mut self, hz: f32);
    fn set_filter_resonance(&mut self, v: f32);
    fn set_filter_drive(&mut self, v: f32);
    fn set_filter_key_track(&mut self, v: f32);
    fn set_filter_env_amount(&mut self, v: f32);
    fn set_fenv_attack(&mut self, s: f32);
    fn set_fenv_decay(&mut self, s: f32);
    fn set_fenv_sustain(&mut self, v: f32);
    fn set_fenv_release(&mut self, s: f32);

    // ── LFO 1 ────────────────────────────────────────────────────────────────
    fn set_lfo_rate(&mut self, hz: f32);
    fn set_lfo_depth(&mut self, v: f32);
    fn set_lfo_shape(&mut self, s: u8);
    fn set_lfo_dest(&mut self, d: u8);

    // ── LFO 2 ────────────────────────────────────────────────────────────────
    fn set_lfo2_rate(&mut self, hz: f32);
    fn set_lfo2_depth(&mut self, v: f32);
    fn set_lfo2_shape(&mut self, s: u8);
    fn set_lfo2_dest(&mut self, d: u8);

    // ── Amp / global ─────────────────────────────────────────────────────────
    fn set_amp_attack(&mut self, s: f32);
    fn set_amp_decay(&mut self, s: f32);
    fn set_amp_sustain(&mut self, v: f32);
    fn set_amp_release(&mut self, s: f32);
    fn set_glide_time(&mut self, s: f32);
    fn set_master_volume(&mut self, v: f32);
    fn set_global_volume(&mut self, v: f32);
    fn set_limiter_enabled(&mut self, on: bool);
    fn set_limiter_threshold(&mut self, v: f32);

    // ── FX ───────────────────────────────────────────────────────────────────
    fn set_fx_overdrive_drive(&mut self, v: f32);
    fn set_fx_overdrive_mix(&mut self, v: f32);
    fn set_fx_overdrive_tone(&mut self, v: f32);
    fn set_fx_overdrive_asym(&mut self, v: f32);
    fn set_fx_distortion_drive(&mut self, v: f32);
    fn set_fx_distortion_mix(&mut self, v: f32);
    fn set_fx_distortion_tone(&mut self, v: f32);
    fn set_fx_distortion_pre(&mut self, v: f32);
    fn set_fx_chorus_rate(&mut self, hz: f32);
    fn set_fx_chorus_depth(&mut self, s: f32);
    fn set_fx_chorus_mix(&mut self, v: f32);
    fn set_fx_delay_time(&mut self, s: f32);
    fn set_fx_delay_feedback(&mut self, v: f32);
    fn set_fx_delay_mix(&mut self, v: f32);
    fn set_fx_reverb_size(&mut self, v: f32);
    fn set_fx_reverb_damp(&mut self, v: f32);
    fn set_fx_reverb_mix(&mut self, v: f32);
    fn set_fx_reverb_predelay(&mut self, ms: f32);
    fn set_fx_reverb_type(&mut self, t: u8);
    fn set_stereo_spread(&mut self, v: f32);
    fn set_stereo_width(&mut self, v: f32);

    // ── Shimmer ──────────────────────────────────────────────────────────────
    fn set_shimmer_size(&mut self, v: f32);
    fn set_shimmer_damp(&mut self, v: f32);
    fn set_shimmer_mix(&mut self, v: f32);
    fn set_shimmer_amount(&mut self, v: f32);
    fn set_shimmer_width(&mut self, v: f32);
    fn set_shimmer_spread(&mut self, v: f32);
    fn set_shimmer_pitch(&mut self, p: u8);

    // ── Crystallizer ─────────────────────────────────────────────────────────
    fn set_crystal_grain(&mut self, ms: f32);
    fn set_crystal_scatter(&mut self, v: f32);
    fn set_crystal_feedback(&mut self, v: f32);
    fn set_crystal_delay(&mut self, ms: f32);
    fn set_crystal_mix(&mut self, v: f32);
    fn set_crystal_pitch(&mut self, p: u8);
}
