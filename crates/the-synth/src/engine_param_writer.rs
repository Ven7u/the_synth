//! Bridges `SynthEngineHandle` to the `synth_ui::ParamWriter` trait.

use synth_engine::SynthEngineHandle;
use synth_ui::ParamWriter;

pub struct EngineParamWriter<'a>(pub &'a SynthEngineHandle);

impl<'a> ParamWriter for EngineParamWriter<'a> {
    fn set_osc_wave(&mut self, osc: u8, wave: u8) {
        self.0.set_osc_wave(osc, wave);
    }
    fn set_osc_freq_mult(&mut self, osc: u8, v: f32) {
        self.0.set_osc_freq_mult(osc, v);
    }
    fn set_osc_vol(&mut self, osc: u8, v: f32) {
        self.0.set_osc_vol(osc, v);
    }
    fn set_osc_pulse_width(&mut self, osc: u8, v: f32) {
        self.0.set_osc_pulse_width(osc, v);
    }
    fn set_osc_unison_detune(&mut self, osc: u8, copy: u8, v: f32) {
        self.0.set_osc_unison_detune(osc, copy, v);
    }
    fn set_osc_unison_vol(&mut self, osc: u8, copy: u8, v: f32) {
        self.0.set_osc_unison_vol(osc, copy, v);
    }
    fn set_hard_sync_enabled(&mut self, on: bool) {
        self.0.set_hard_sync_enabled(on);
    }
    fn set_fm_depth(&mut self, v: f32) {
        self.0.set_fm_depth(v);
    }
    fn set_ring_depth(&mut self, v: f32) {
        self.0.set_ring_depth(v);
    }
    fn set_noise_vol(&mut self, v: f32) {
        self.0.set_noise_vol(v);
    }

    fn set_filter_cutoff(&mut self, hz: f32) {
        self.0.set_filter_cutoff(hz);
    }
    fn set_filter_resonance(&mut self, v: f32) {
        self.0.set_filter_resonance(v);
    }
    fn set_filter_drive(&mut self, v: f32) {
        self.0.set_filter_drive(v);
    }
    fn set_filter_key_track(&mut self, v: f32) {
        self.0.set_filter_key_track(v);
    }
    fn set_filter_env_amount(&mut self, v: f32) {
        self.0.set_filter_env_amount(v);
    }
    fn set_fenv_attack(&mut self, s: f32) {
        self.0.set_fenv_attack(s);
    }
    fn set_fenv_decay(&mut self, s: f32) {
        self.0.set_fenv_decay(s);
    }
    fn set_fenv_sustain(&mut self, v: f32) {
        self.0.set_fenv_sustain(v);
    }
    fn set_fenv_release(&mut self, s: f32) {
        self.0.set_fenv_release(s);
    }

    fn set_lfo_rate(&mut self, hz: f32) {
        self.0.set_lfo_rate(hz);
    }
    fn set_lfo_depth(&mut self, v: f32) {
        self.0.set_lfo_depth(v);
    }
    fn set_lfo_shape(&mut self, s: u8) {
        self.0.set_lfo_shape(s);
    }
    fn set_lfo_dest(&mut self, d: u8) {
        self.0.set_lfo_dest(d);
    }

    fn set_lfo2_rate(&mut self, hz: f32) {
        self.0.set_lfo2_rate(hz);
    }
    fn set_lfo2_depth(&mut self, v: f32) {
        self.0.set_lfo2_depth(v);
    }
    fn set_lfo2_shape(&mut self, s: u8) {
        self.0.set_lfo2_shape(s);
    }
    fn set_lfo2_dest(&mut self, d: u8) {
        self.0.set_lfo2_dest(d);
    }

    fn set_amp_attack(&mut self, s: f32) {
        self.0.set_amp_attack(s);
    }
    fn set_amp_decay(&mut self, s: f32) {
        self.0.set_amp_decay(s);
    }
    fn set_amp_sustain(&mut self, v: f32) {
        self.0.set_amp_sustain(v);
    }
    fn set_amp_release(&mut self, s: f32) {
        self.0.set_amp_release(s);
    }
    fn set_glide_time(&mut self, s: f32) {
        self.0.set_glide_time(s);
    }
    fn set_master_volume(&mut self, v: f32) {
        self.0.set_master_volume(v);
    }
    fn set_global_volume(&mut self, v: f32) {
        self.0.set_global_volume(v);
    }
    fn set_limiter_enabled(&mut self, on: bool) {
        self.0.set_limiter_enabled(on);
    }
    fn set_limiter_threshold(&mut self, v: f32) {
        self.0.set_limiter_threshold(v);
    }

    fn set_fx_overdrive_drive(&mut self, v: f32) {
        self.0.set_fx_overdrive_drive(v);
    }
    fn set_fx_overdrive_mix(&mut self, v: f32) {
        self.0.set_fx_overdrive_mix(v);
    }
    fn set_fx_overdrive_tone(&mut self, v: f32) {
        self.0.set_fx_overdrive_tone(v);
    }
    fn set_fx_overdrive_asym(&mut self, v: f32) {
        self.0.set_fx_overdrive_asym(v);
    }
    fn set_fx_distortion_drive(&mut self, v: f32) {
        self.0.set_fx_distortion_drive(v);
    }
    fn set_fx_distortion_mix(&mut self, v: f32) {
        self.0.set_fx_distortion_mix(v);
    }
    fn set_fx_distortion_tone(&mut self, v: f32) {
        self.0.set_fx_distortion_tone(v);
    }
    fn set_fx_distortion_pre(&mut self, v: f32) {
        self.0.set_fx_distortion_pre(v);
    }
    fn set_fx_chorus_rate(&mut self, hz: f32) {
        self.0.set_fx_chorus_rate(hz);
    }
    fn set_fx_chorus_depth(&mut self, s: f32) {
        self.0.set_fx_chorus_depth(s);
    }
    fn set_fx_chorus_mix(&mut self, v: f32) {
        self.0.set_fx_chorus_mix(v);
    }
    fn set_fx_delay_time(&mut self, s: f32) {
        self.0.set_fx_delay_time(s);
    }
    fn set_fx_delay_feedback(&mut self, v: f32) {
        self.0.set_fx_delay_feedback(v);
    }
    fn set_fx_delay_mix(&mut self, v: f32) {
        self.0.set_fx_delay_mix(v);
    }
    fn set_fx_reverb_size(&mut self, v: f32) {
        self.0.set_fx_reverb_size(v);
    }
    fn set_fx_reverb_damp(&mut self, v: f32) {
        self.0.set_fx_reverb_damp(v);
    }
    fn set_fx_reverb_mix(&mut self, v: f32) {
        self.0.set_fx_reverb_mix(v);
    }
    fn set_fx_reverb_predelay(&mut self, ms: f32) {
        self.0.set_fx_reverb_predelay(ms);
    }
    fn set_fx_reverb_type(&mut self, t: u8) {
        self.0.set_fx_reverb_type(t);
    }
    fn set_stereo_spread(&mut self, v: f32) {
        self.0.set_stereo_spread(v);
    }
    fn set_stereo_width(&mut self, v: f32) {
        self.0.set_stereo_width(v);
    }

    fn set_shimmer_size(&mut self, v: f32) {
        self.0.set_shimmer_size(v);
    }
    fn set_shimmer_damp(&mut self, v: f32) {
        self.0.set_shimmer_damp(v);
    }
    fn set_shimmer_mix(&mut self, v: f32) {
        self.0.set_shimmer_mix(v);
    }
    fn set_shimmer_amount(&mut self, v: f32) {
        self.0.set_shimmer_amount(v);
    }
    fn set_shimmer_width(&mut self, v: f32) {
        self.0.set_shimmer_width(v);
    }
    fn set_shimmer_spread(&mut self, v: f32) {
        self.0.set_shimmer_spread(v);
    }
    fn set_shimmer_pitch(&mut self, p: u8) {
        self.0.set_shimmer_pitch(p);
    }

    fn set_crystal_grain(&mut self, ms: f32) {
        self.0.set_crystal_grain(ms);
    }
    fn set_crystal_scatter(&mut self, v: f32) {
        self.0.set_crystal_scatter(v);
    }
    fn set_crystal_feedback(&mut self, v: f32) {
        self.0.set_crystal_feedback(v);
    }
    fn set_crystal_delay(&mut self, ms: f32) {
        self.0.set_crystal_delay(ms);
    }
    fn set_crystal_mix(&mut self, v: f32) {
        self.0.set_crystal_mix(v);
    }
    fn set_crystal_pitch(&mut self, p: u8) {
        self.0.set_crystal_pitch(p);
    }
}
