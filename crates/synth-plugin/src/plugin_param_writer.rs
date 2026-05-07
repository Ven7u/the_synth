use nih_plug::prelude::*;
use synth_ui::ParamWriter;

use crate::params::{LfoDestParam, LfoShapeParam, ReverbTypeParam, TheSynthParams, WaveShapeParam};

pub struct PluginParamWriter<'a> {
    pub params: &'a TheSynthParams,
    pub setter: &'a ParamSetter<'a>,
}

fn to_wave(v: u8) -> WaveShapeParam {
    match v {
        1 => WaveShapeParam::Saw,
        2 => WaveShapeParam::Square,
        3 => WaveShapeParam::Triangle,
        _ => WaveShapeParam::Sine,
    }
}

fn to_lfo_shape(v: u8) -> LfoShapeParam {
    match v {
        1 => LfoShapeParam::Triangle,
        2 => LfoShapeParam::Saw,
        _ => LfoShapeParam::Sine,
    }
}

fn to_lfo_dest(v: u8) -> LfoDestParam {
    match v {
        1 => LfoDestParam::Filter,
        2 => LfoDestParam::Amp,
        _ => LfoDestParam::Pitch,
    }
}

fn to_reverb_type(v: u8) -> ReverbTypeParam {
    match v {
        1 => ReverbTypeParam::Plate,
        2 => ReverbTypeParam::FdnHall,
        _ => ReverbTypeParam::Freeverb,
    }
}

macro_rules! setf {
    ($self:ident, $param:expr, $val:expr) => {
        $self.setter.set_parameter($param, $val)
    };
}

macro_rules! set_osc {
    ($self:ident, osc: $osc:expr, $field:ident, $val:expr) => {
        match $osc {
            0 => setf!($self, &$self.params.osc1.$field, $val),
            1 => setf!($self, &$self.params.osc2.$field, $val),
            _ => setf!($self, &$self.params.osc3.$field, $val),
        }
    };
}

impl<'a> ParamWriter for PluginParamWriter<'a> {
    fn set_osc_wave(&mut self, osc: u8, wave: u8) {
        set_osc!(self, osc: osc, wave, to_wave(wave));
    }

    fn set_osc_freq_mult(&mut self, osc: u8, v: f32) {
        set_osc!(self, osc: osc, freq_mult, v);
    }

    fn set_osc_vol(&mut self, osc: u8, v: f32) {
        set_osc!(self, osc: osc, vol, v);
    }

    fn set_osc_pulse_width(&mut self, osc: u8, v: f32) {
        set_osc!(self, osc: osc, pulse_width, v);
    }

    fn set_osc_unison_detune(&mut self, osc: u8, copy: u8, v: f32) {
        macro_rules! set_ud {
            ($osc_params:expr) => {
                match copy {
                    0 => setf!(self, &$osc_params.unison_detune_0, v),
                    1 => setf!(self, &$osc_params.unison_detune_1, v),
                    2 => setf!(self, &$osc_params.unison_detune_2, v),
                    3 => setf!(self, &$osc_params.unison_detune_3, v),
                    _ => setf!(self, &$osc_params.unison_detune_4, v),
                }
            };
        }
        match osc {
            0 => set_ud!(self.params.osc1),
            1 => set_ud!(self.params.osc2),
            _ => set_ud!(self.params.osc3),
        }
    }

    fn set_osc_unison_vol(&mut self, osc: u8, copy: u8, v: f32) {
        macro_rules! set_uv {
            ($osc_params:expr) => {
                match copy {
                    0 => setf!(self, &$osc_params.unison_vol_0, v),
                    1 => setf!(self, &$osc_params.unison_vol_1, v),
                    2 => setf!(self, &$osc_params.unison_vol_2, v),
                    3 => setf!(self, &$osc_params.unison_vol_3, v),
                    _ => setf!(self, &$osc_params.unison_vol_4, v),
                }
            };
        }
        match osc {
            0 => set_uv!(self.params.osc1),
            1 => set_uv!(self.params.osc2),
            _ => set_uv!(self.params.osc3),
        }
    }

    fn set_hard_sync_enabled(&mut self, on: bool) {
        setf!(self, &self.params.hard_sync, on);
    }

    fn set_fm_depth(&mut self, v: f32) {
        setf!(self, &self.params.fm_depth, v);
    }

    fn set_ring_depth(&mut self, v: f32) {
        setf!(self, &self.params.ring_depth, v);
    }

    fn set_noise_vol(&mut self, v: f32) {
        setf!(self, &self.params.noise_vol, v);
    }

    fn set_filter_cutoff(&mut self, hz: f32) {
        setf!(self, &self.params.filter.cutoff, hz);
    }

    fn set_filter_resonance(&mut self, v: f32) {
        setf!(self, &self.params.filter.resonance, v);
    }

    fn set_filter_drive(&mut self, v: f32) {
        setf!(self, &self.params.filter.drive, v);
    }

    fn set_filter_key_track(&mut self, v: f32) {
        setf!(self, &self.params.filter.key_track, v);
    }

    fn set_filter_env_amount(&mut self, v: f32) {
        setf!(self, &self.params.filter.env_amount, v);
    }

    fn set_fenv_attack(&mut self, s: f32) {
        setf!(self, &self.params.filter.attack, s);
    }

    fn set_fenv_decay(&mut self, s: f32) {
        setf!(self, &self.params.filter.decay, s);
    }

    fn set_fenv_sustain(&mut self, v: f32) {
        setf!(self, &self.params.filter.sustain, v);
    }

    fn set_fenv_release(&mut self, s: f32) {
        setf!(self, &self.params.filter.release, s);
    }

    fn set_lfo_rate(&mut self, hz: f32) {
        setf!(self, &self.params.lfo1.rate, hz);
    }

    fn set_lfo_depth(&mut self, v: f32) {
        setf!(self, &self.params.lfo1.depth, v);
    }

    fn set_lfo_shape(&mut self, s: u8) {
        setf!(self, &self.params.lfo1.shape, to_lfo_shape(s));
    }

    fn set_lfo_dest(&mut self, d: u8) {
        setf!(self, &self.params.lfo1.dest, to_lfo_dest(d));
    }

    fn set_lfo2_rate(&mut self, hz: f32) {
        setf!(self, &self.params.lfo2.rate, hz);
    }

    fn set_lfo2_depth(&mut self, v: f32) {
        setf!(self, &self.params.lfo2.depth, v);
    }

    fn set_lfo2_shape(&mut self, s: u8) {
        setf!(self, &self.params.lfo2.shape, to_lfo_shape(s));
    }

    fn set_lfo2_dest(&mut self, d: u8) {
        setf!(self, &self.params.lfo2.dest, to_lfo_dest(d));
    }

    fn set_amp_attack(&mut self, s: f32) {
        setf!(self, &self.params.amp.attack, s);
    }

    fn set_amp_decay(&mut self, s: f32) {
        setf!(self, &self.params.amp.decay, s);
    }

    fn set_amp_sustain(&mut self, v: f32) {
        setf!(self, &self.params.amp.sustain, v);
    }

    fn set_amp_release(&mut self, s: f32) {
        setf!(self, &self.params.amp.release, s);
    }

    fn set_glide_time(&mut self, s: f32) {
        setf!(self, &self.params.amp.glide_time, s);
    }

    fn set_master_volume(&mut self, v: f32) {
        setf!(self, &self.params.master.master_vol, v);
    }

    fn set_global_volume(&mut self, v: f32) {
        setf!(self, &self.params.master.global_vol, v);
    }

    fn set_limiter_enabled(&mut self, on: bool) {
        setf!(self, &self.params.master.limiter_enabled, on);
    }

    fn set_limiter_threshold(&mut self, v: f32) {
        setf!(self, &self.params.master.limiter_threshold, v);
    }

    fn set_fx_overdrive_drive(&mut self, v: f32) {
        setf!(self, &self.params.fx.overdrive_drive, v);
    }

    fn set_fx_overdrive_mix(&mut self, v: f32) {
        setf!(self, &self.params.fx.overdrive_mix, v);
    }

    fn set_fx_overdrive_tone(&mut self, v: f32) {
        setf!(self, &self.params.fx.overdrive_tone, v);
    }

    fn set_fx_overdrive_asym(&mut self, v: f32) {
        setf!(self, &self.params.fx.overdrive_asym, v);
    }

    fn set_fx_distortion_drive(&mut self, v: f32) {
        setf!(self, &self.params.fx.distortion_drive, v);
    }

    fn set_fx_distortion_mix(&mut self, v: f32) {
        setf!(self, &self.params.fx.distortion_mix, v);
    }

    fn set_fx_distortion_tone(&mut self, v: f32) {
        setf!(self, &self.params.fx.distortion_tone, v);
    }

    fn set_fx_distortion_pre(&mut self, v: f32) {
        setf!(self, &self.params.fx.distortion_pre, v);
    }

    fn set_fx_chorus_rate(&mut self, hz: f32) {
        setf!(self, &self.params.fx.chorus_rate, hz);
    }

    fn set_fx_chorus_depth(&mut self, s: f32) {
        setf!(self, &self.params.fx.chorus_depth, s);
    }

    fn set_fx_chorus_mix(&mut self, v: f32) {
        setf!(self, &self.params.fx.chorus_mix, v);
    }

    fn set_fx_delay_time(&mut self, s: f32) {
        setf!(self, &self.params.fx.delay_time, s);
    }

    fn set_fx_delay_feedback(&mut self, v: f32) {
        setf!(self, &self.params.fx.delay_feedback, v);
    }

    fn set_fx_delay_mix(&mut self, v: f32) {
        setf!(self, &self.params.fx.delay_mix, v);
    }

    fn set_fx_reverb_size(&mut self, v: f32) {
        setf!(self, &self.params.fx.reverb_size, v);
    }

    fn set_fx_reverb_damp(&mut self, v: f32) {
        setf!(self, &self.params.fx.reverb_damp, v);
    }

    fn set_fx_reverb_mix(&mut self, v: f32) {
        setf!(self, &self.params.fx.reverb_mix, v);
    }

    fn set_fx_reverb_predelay(&mut self, ms: f32) {
        setf!(self, &self.params.fx.reverb_predelay, ms);
    }

    fn set_fx_reverb_type(&mut self, t: u8) {
        setf!(self, &self.params.fx.reverb_type, to_reverb_type(t));
    }

    fn set_stereo_spread(&mut self, v: f32) {
        setf!(self, &self.params.fx.stereo_spread, v);
    }

    fn set_stereo_width(&mut self, v: f32) {
        setf!(self, &self.params.fx.stereo_width, v);
    }

    fn set_shimmer_size(&mut self, v: f32) {
        setf!(self, &self.params.fx.shimmer_size, v);
    }

    fn set_shimmer_damp(&mut self, v: f32) {
        setf!(self, &self.params.fx.shimmer_damp, v);
    }

    fn set_shimmer_mix(&mut self, v: f32) {
        setf!(self, &self.params.fx.shimmer_mix, v);
    }

    fn set_shimmer_amount(&mut self, v: f32) {
        setf!(self, &self.params.fx.shimmer_amount, v);
    }

    fn set_shimmer_width(&mut self, v: f32) {
        setf!(self, &self.params.fx.shimmer_width, v);
    }

    fn set_shimmer_spread(&mut self, v: f32) {
        setf!(self, &self.params.fx.shimmer_spread, v);
    }

    fn set_shimmer_pitch(&mut self, _p: u8) {
        // shimmer_pitch is not a DAW-automatable param in v1
    }

    fn set_crystal_grain(&mut self, ms: f32) {
        setf!(self, &self.params.fx.crystal_grain, ms);
    }

    fn set_crystal_scatter(&mut self, v: f32) {
        setf!(self, &self.params.fx.crystal_scatter, v);
    }

    fn set_crystal_feedback(&mut self, v: f32) {
        setf!(self, &self.params.fx.crystal_feedback, v);
    }

    fn set_crystal_delay(&mut self, ms: f32) {
        setf!(self, &self.params.fx.crystal_delay, ms);
    }

    fn set_crystal_mix(&mut self, v: f32) {
        setf!(self, &self.params.fx.crystal_mix, v);
    }

    fn set_crystal_pitch(&mut self, _p: u8) {
        // crystal_pitch is not a DAW-automatable param in v1
    }
}
