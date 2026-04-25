//! AU / CLAP / VST3 plugin shell for the synth engine.
//!
//! `synth-engine` is completely cpal-free, so this crate simply replaces the
//! cpal stream callback from `the-synth` with nih-plug's `process()` method.
//! The fundsp graph, AudioState, VoiceAllocator, LFO logic, DC blocker, and
//! lookahead limiter are all ported verbatim from `the-synth/src/audio.rs`.

#![allow(clippy::precedence)]

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::num::NonZeroU32;
use std::sync::atomic::Ordering;
use std::sync::Arc; // needed for struct fields; nih_plug::prelude::* also exports Arc, causing an unused-import warning on some rustc versions

use fundsp::prelude32::*;
use synth_control::{make_control_channel, ControlEvent, ControlReceiver, ControlSender};
use synth_dsp::LookaheadLimiter;
use synth_engine::audio::{build_synth_graph, AudioState};
use synth_engine::{enable_ftz_on_current_thread, VoiceAllocator, VOICE_COUNT};

mod editor;
mod params;

use params::TheSynthParams;

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

struct TheSynthPlugin {
    params: Arc<TheSynthParams>,
    editor_state: Arc<EguiState>,

    state: Arc<AudioState>,
    graph: Option<Box<dyn AudioUnit + Send>>,
    voice_alloc: VoiceAllocator,
    ctrl_tx: ControlSender,
    ctrl_rx: ControlReceiver,

    sample_rate: f64,

    // LFO phase accumulators (0..1)
    lfo_phase: f32,
    lfo2_phase: f32,

    // Per-voice glide smoothing (callback-owned, matches the standalone's smoothed_freqs)
    smoothed_freqs: Vec<f32>,

    // DC blocker state
    dc_x_prev_l: f32,
    dc_x_prev_r: f32,
    dc_y_prev_l: f32,
    dc_y_prev_r: f32,
    dc_coeff: f32,

    // Lookahead limiter
    lookahead_lim: Option<LookaheadLimiter>,

    // Smoothed global volume (10ms one-pole)
    global_vol_smooth: f32,
    global_vol_coeff: f32,

    // Voice gain staging (20ms one-pole)
    voice_gain_smooth: f32,
    vgs_coeff: f32,

    // Key tracking: last freq of highest sounding voice, persists across buffers
    last_keyed_freq: f32,
}

impl Default for TheSynthPlugin {
    fn default() -> Self {
        let state = Arc::new(AudioState::new());
        let (ctrl_tx, ctrl_rx) = make_control_channel(1024);
        Self {
            params: Arc::new(TheSynthParams::default()),
            editor_state: editor::default_state(),
            state,
            graph: None,
            voice_alloc: VoiceAllocator::new(),
            ctrl_tx,
            ctrl_rx,
            sample_rate: 44100.0,
            lfo_phase: 0.0,
            lfo2_phase: 0.25,
            smoothed_freqs: vec![440.0; VOICE_COUNT],
            dc_x_prev_l: 0.0,
            dc_x_prev_r: 0.0,
            dc_y_prev_l: 0.0,
            dc_y_prev_r: 0.0,
            dc_coeff: 0.9972,
            lookahead_lim: None,
            global_vol_smooth: 1.0,
            global_vol_coeff: 0.9993,
            voice_gain_smooth: 1.0,
            vgs_coeff: 0.9994,
            last_keyed_freq: 261.63,
        }
    }
}

// ---------------------------------------------------------------------------
// Param → AudioState sync (runs once at the top of every process() call)
// ---------------------------------------------------------------------------

impl TheSynthPlugin {
    fn sync_params(&self) {
        let p = &self.params;
        let s = &self.state;

        macro_rules! set {
            ($shared:expr, $val:expr) => {
                $shared.set($val)
            };
        }
        macro_rules! store {
            ($atomic:expr, $val:expr) => {
                $atomic.store($val, Ordering::Relaxed)
            };
        }

        // OSC 1
        store!(s.osc_wave[0], p.osc1.wave.value().idx());
        set!(s.osc_freq_mult[0], p.osc1.freq_mult.value());
        set!(s.osc_vol[0], p.osc1.vol.value());
        set!(s.osc_pulse_width[0], p.osc1.pulse_width.value());
        set!(s.osc_unison_detune[0][0], p.osc1.unison_detune_0.value());
        set!(s.osc_unison_detune[0][1], p.osc1.unison_detune_1.value());
        set!(s.osc_unison_detune[0][2], p.osc1.unison_detune_2.value());
        set!(s.osc_unison_detune[0][3], p.osc1.unison_detune_3.value());
        set!(s.osc_unison_detune[0][4], p.osc1.unison_detune_4.value());
        set!(s.osc_unison_vol[0][0], p.osc1.unison_vol_0.value());
        set!(s.osc_unison_vol[0][1], p.osc1.unison_vol_1.value());
        set!(s.osc_unison_vol[0][2], p.osc1.unison_vol_2.value());
        set!(s.osc_unison_vol[0][3], p.osc1.unison_vol_3.value());
        set!(s.osc_unison_vol[0][4], p.osc1.unison_vol_4.value());

        // OSC 2
        store!(s.osc_wave[1], p.osc2.wave.value().idx());
        set!(s.osc_freq_mult[1], p.osc2.freq_mult.value());
        set!(s.osc_vol[1], p.osc2.vol.value());
        set!(s.osc_pulse_width[1], p.osc2.pulse_width.value());
        set!(s.osc_unison_detune[1][0], p.osc2.unison_detune_0.value());
        set!(s.osc_unison_detune[1][1], p.osc2.unison_detune_1.value());
        set!(s.osc_unison_detune[1][2], p.osc2.unison_detune_2.value());
        set!(s.osc_unison_detune[1][3], p.osc2.unison_detune_3.value());
        set!(s.osc_unison_detune[1][4], p.osc2.unison_detune_4.value());
        set!(s.osc_unison_vol[1][0], p.osc2.unison_vol_0.value());
        set!(s.osc_unison_vol[1][1], p.osc2.unison_vol_1.value());
        set!(s.osc_unison_vol[1][2], p.osc2.unison_vol_2.value());
        set!(s.osc_unison_vol[1][3], p.osc2.unison_vol_3.value());
        set!(s.osc_unison_vol[1][4], p.osc2.unison_vol_4.value());

        // OSC 3
        store!(s.osc_wave[2], p.osc3.wave.value().idx());
        set!(s.osc_freq_mult[2], p.osc3.freq_mult.value());
        set!(s.osc_vol[2], p.osc3.vol.value());
        set!(s.osc_pulse_width[2], p.osc3.pulse_width.value());
        set!(s.osc_unison_detune[2][0], p.osc3.unison_detune_0.value());
        set!(s.osc_unison_detune[2][1], p.osc3.unison_detune_1.value());
        set!(s.osc_unison_detune[2][2], p.osc3.unison_detune_2.value());
        set!(s.osc_unison_detune[2][3], p.osc3.unison_detune_3.value());
        set!(s.osc_unison_detune[2][4], p.osc3.unison_detune_4.value());
        set!(s.osc_unison_vol[2][0], p.osc3.unison_vol_0.value());
        set!(s.osc_unison_vol[2][1], p.osc3.unison_vol_1.value());
        set!(s.osc_unison_vol[2][2], p.osc3.unison_vol_2.value());
        set!(s.osc_unison_vol[2][3], p.osc3.unison_vol_3.value());
        set!(s.osc_unison_vol[2][4], p.osc3.unison_vol_4.value());

        // Mod sources
        store!(
            s.hard_sync_enabled,
            p.hard_sync.value()
        );
        set!(s.fm_depth, p.fm_depth.value());
        set!(s.ring_depth, p.ring_depth.value());
        set!(s.noise_vol, p.noise_vol.value());

        // Filter
        set!(s.cutoff, p.filter.cutoff.value());
        set!(s.resonance, p.filter.resonance.value());
        set!(s.filter_drive, p.filter.drive.value());
        set!(s.filter_key_track, p.filter.key_track.value());
        set!(s.filter_env_amount, p.filter.env_amount.value());
        set!(s.fenv_attack, p.filter.attack.value());
        set!(s.fenv_decay, p.filter.decay.value());
        set!(s.fenv_sustain, p.filter.sustain.value());
        set!(s.fenv_release, p.filter.release.value());

        // LFO 1
        set!(s.lfo_rate, p.lfo1.rate.value());
        set!(s.lfo_depth, p.lfo1.depth.value());
        store!(s.lfo_shape, p.lfo1.shape.value().idx());
        store!(s.lfo_dest, p.lfo1.dest.value().idx());

        // LFO 2
        set!(s.lfo2_rate, p.lfo2.rate.value());
        set!(s.lfo2_depth, p.lfo2.depth.value());
        store!(s.lfo2_shape, p.lfo2.shape.value().idx());
        store!(s.lfo2_dest, p.lfo2.dest.value().idx());

        // Amp
        set!(s.adsr_attack, p.amp.attack.value());
        set!(s.adsr_decay, p.amp.decay.value());
        set!(s.adsr_sustain, p.amp.sustain.value());
        set!(s.adsr_release, p.amp.release.value());
        set!(s.glide_time, p.amp.glide_time.value());

        // Master
        set!(s.master_vol, p.master.master_vol.value());
        set!(s.global_vol, p.master.global_vol.value());
        store!(
            s.limiter_enabled,
            p.master.limiter_enabled.value()
        );
        set!(s.limiter_threshold, p.master.limiter_threshold.value());

        // FX
        set!(s.fx_overdrive_drive, p.fx.overdrive_drive.value());
        set!(s.fx_overdrive_mix, p.fx.overdrive_mix.value());
        set!(s.fx_overdrive_tone, p.fx.overdrive_tone.value());
        set!(s.fx_overdrive_asym, p.fx.overdrive_asym.value());
        set!(s.fx_distortion_drive, p.fx.distortion_drive.value());
        set!(s.fx_distortion_mix, p.fx.distortion_mix.value());
        set!(s.fx_distortion_tone, p.fx.distortion_tone.value());
        set!(s.fx_distortion_pre, p.fx.distortion_pre.value());
        set!(s.fx_chorus_rate, p.fx.chorus_rate.value());
        set!(s.fx_chorus_depth, p.fx.chorus_depth.value());
        set!(s.fx_chorus_mix, p.fx.chorus_mix.value());
        set!(s.fx_delay_time, p.fx.delay_time.value());
        set!(s.fx_delay_feedback, p.fx.delay_feedback.value());
        set!(s.fx_delay_mix, p.fx.delay_mix.value());
        set!(s.fx_reverb_size, p.fx.reverb_size.value());
        set!(s.fx_reverb_damp, p.fx.reverb_damp.value());
        set!(s.fx_reverb_mix, p.fx.reverb_mix.value());
        set!(s.fx_reverb_predelay, p.fx.reverb_predelay.value());
        store!(s.fx_reverb_type, p.fx.reverb_type.value().idx());
        set!(s.stereo_spread, p.fx.stereo_spread.value());
        set!(s.stereo_width, p.fx.stereo_width.value());

        // Shimmer
        set!(s.fx_shimmer.size, p.fx.shimmer_size.value());
        set!(s.fx_shimmer.damp, p.fx.shimmer_damp.value());
        set!(s.fx_shimmer.mix, p.fx.shimmer_mix.value());
        set!(s.fx_shimmer.shimmer, p.fx.shimmer_amount.value());
        set!(s.fx_shimmer.width, p.fx.shimmer_width.value());
        set!(s.fx_shimmer.spread, p.fx.shimmer_spread.value());

        // Crystallizer
        set!(s.fx_crystal.grain_ms, p.fx.crystal_grain.value());
        set!(s.fx_crystal.scatter, p.fx.crystal_scatter.value());
        set!(s.fx_crystal.feedback, p.fx.crystal_feedback.value());
        set!(s.fx_crystal.delay_ms, p.fx.crystal_delay.value());
        set!(s.fx_crystal.mix, p.fx.crystal_mix.value());
    }
}

// ---------------------------------------------------------------------------
// nih-plug Plugin impl
// ---------------------------------------------------------------------------

impl Plugin for TheSynthPlugin {
    const NAME: &'static str = "The Synth";
    const VENDOR: &'static str = "Francesco Ventura";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create_editor(self.params.clone(), self.editor_state.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let sr = buffer_config.sample_rate as f64;
        self.sample_rate = sr;

        enable_ftz_on_current_thread();
        self.state
            .sample_rate
            .store(sr as u32, Ordering::Relaxed);

        // Build graph once; never rebuilt (all params live in AudioState atomics)
        let mut graph = build_synth_graph(&self.state, sr);
        graph.set_sample_rate(sr);
        graph.allocate();
        self.graph = Some(graph);

        self.lookahead_lim = Some(LookaheadLimiter::new(sr as f32, 1.5, 80.0));
        self.dc_coeff = 1.0 - (std::f32::consts::TAU * 20.0 / sr as f32);
        self.global_vol_coeff = (-1.0_f64 / (0.010 * sr)).exp() as f32;
        self.vgs_coeff = (-1.0_f64 / (0.020 * sr)).exp() as f32;

        true
    }

    fn reset(&mut self) {
        self.lfo_phase = 0.0;
        self.lfo2_phase = 0.25;
        self.dc_x_prev_l = 0.0;
        self.dc_x_prev_r = 0.0;
        self.dc_y_prev_l = 0.0;
        self.dc_y_prev_r = 0.0;
        self.global_vol_smooth = self.state.global_vol.value() as f32;
        self.voice_gain_smooth = 1.0;
        self.last_keyed_freq = 261.63;
        self.smoothed_freqs.iter_mut().for_each(|f| *f = 440.0);
        if let Some(lim) = &mut self.lookahead_lim {
            lim.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        if self.graph.is_none() {
            return ProcessStatus::Normal;
        }

        let frames = buffer.samples();
        let sr = self.sample_rate;

        // Phase A: push nih-plug param values into AudioState atomics
        self.sync_params();

        // Phase B: convert MIDI events to ControlEvents for the VoiceAllocator
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { note, velocity, .. } => {
                    let _ = self.ctrl_tx.try_send(ControlEvent::NoteOn {
                        pitch: note,
                        velocity: (velocity * 127.0) as u8,
                        track: 0,
                    });
                }
                NoteEvent::NoteOff { note, .. } => {
                    let _ = self.ctrl_tx.try_send(ControlEvent::NoteOff {
                        pitch: note,
                        track: 0,
                    });
                }
                _ => {}
            }
        }

        // Phase C: audio processing — ported from the-synth/src/audio.rs callback
        // SAFETY: checked is_none() at the top of process(), so this is always Some.
        let graph = self.graph.as_mut().unwrap();

        self.voice_alloc
            .begin_buffer(&self.state, &self.ctrl_rx, frames, sr);

        // Voice gain staging: smooth 1/sqrt(active_voices) to prevent polyphony loudness jumps
        {
            let count = self
                .state
                .amp_cursors
                .iter()
                .filter(|c| c.value() > 0.01)
                .count();
            let n_active = if count < 1 { 1 } else { count };
            let target_scale = 1.0_f32 / (n_active as f32).sqrt();
            self.voice_gain_smooth =
                target_scale + self.vgs_coeff * (self.voice_gain_smooth - target_scale);
            self.state.voice_gain_scale.set(self.voice_gain_smooth);
        }

        let sr_f = sr as f32;

        // Read LFO params once per buffer (avoids per-sample atomic loads)
        let lfo_rate = self.state.lfo_rate.value();
        let lfo_depth = self.state.lfo_depth.value();
        let lfo_shape = self.state.lfo_shape.load(Ordering::Relaxed);
        let lfo_dest = self.state.lfo_dest.load(Ordering::Relaxed);
        let lfo_dt = lfo_rate / sr_f;
        let lfo2_rate = self.state.lfo2_rate.value();
        let lfo2_depth = self.state.lfo2_depth.value();
        let lfo2_shape = self.state.lfo2_shape.load(Ordering::Relaxed);
        let lfo2_dest = self.state.lfo2_dest.load(Ordering::Relaxed);
        let lfo2_dt = lfo2_rate / sr_f;
        let base_cutoff = self.state.cutoff.value().clamp(80.0, 18000.0);

        // Key tracking: pick the highest sounding voice for cutoff tracking
        let key_track = self.state.filter_key_track.value();
        if key_track > 0.001 {
            let mut top_freq: f32 = 0.0;
            for vi in 0..VOICE_COUNT {
                if self.state.amp_cursors[vi].value() > 0.5 {
                    let f = self.state.voice_freq_targets[vi].value();
                    if f > top_freq {
                        top_freq = f;
                    }
                }
            }
            if top_freq > 0.0 {
                self.last_keyed_freq = top_freq;
            }
        }
        let key_mult = if key_track > 0.001 {
            (self.last_keyed_freq / 261.63_f32).powf(key_track * 2.0)
        } else {
            1.0
        };
        let keyed_cutoff = base_cutoff * key_mult;

        // Glide: smooth voice_freq_targets → voice_freqs once per buffer
        let glide_time = self.state.glide_time.value();
        for vi in 0..VOICE_COUNT {
            let target = self.state.voice_freq_targets[vi].value();
            if glide_time < 0.001 {
                self.smoothed_freqs[vi] = target;
            } else {
                let coeff = (-(frames as f32) / (glide_time * sr_f)).exp();
                self.smoothed_freqs[vi] =
                    coeff * self.smoothed_freqs[vi] + (1.0 - coeff) * target;
            }
            self.state.voice_freqs[vi].set(self.smoothed_freqs[vi]);
        }

        let limiter_on = self.state.limiter_enabled.load(Ordering::Relaxed);
        let threshold = self.state.limiter_threshold.value();
        let mut peak_l_local: f32 = 0.0;
        let mut peak_r_local: f32 = 0.0;

        let output = buffer.as_slice();

        for i in 0..frames {
            // LFO 1
            self.lfo_phase += lfo_dt;
            if self.lfo_phase >= 1.0 {
                self.lfo_phase -= 1.0;
            }
            let lfo_raw = match lfo_shape {
                1 => {
                    if self.lfo_phase < 0.5 {
                        4.0 * self.lfo_phase - 1.0
                    } else {
                        3.0 - 4.0 * self.lfo_phase
                    }
                }
                2 => 2.0 * self.lfo_phase - 1.0,
                _ => (self.lfo_phase * std::f32::consts::TAU).sin(),
            };

            // LFO 2
            self.lfo2_phase += lfo2_dt;
            if self.lfo2_phase >= 1.0 {
                self.lfo2_phase -= 1.0;
            }
            let lfo2_raw = match lfo2_shape {
                1 => {
                    if self.lfo2_phase < 0.5 {
                        4.0 * self.lfo2_phase - 1.0
                    } else {
                        3.0 - 4.0 * self.lfo2_phase
                    }
                }
                2 => 2.0 * self.lfo2_phase - 1.0,
                _ => (self.lfo2_phase * std::f32::consts::TAU).sin(),
            };

            // Accumulate pitch, filter, amp contributions from both LFOs
            let mut pitch_mod: f32 = 0.0;
            let mut filter_mod: f32 = 0.0;
            let mut amp_mod: f32 = 1.0;
            for (raw, depth, dest) in [
                (lfo_raw, lfo_depth, lfo_dest),
                (lfo2_raw, lfo2_depth, lfo2_dest),
            ] {
                match dest {
                    0 => pitch_mod += raw * depth,
                    2 => amp_mod *= 1.0 - depth * (1.0 - raw) * 0.5,
                    _ => filter_mod += raw * depth,
                }
            }

            self.state
                .lfo_pitch_mult
                .set(2_f32.powf(pitch_mod * 2.0 / 12.0));
            self.state.effective_cutoff.set(
                (keyed_cutoff + filter_mod * keyed_cutoff * 0.5).clamp(80.0, 18000.0),
            );

            // Retrigger countdown (4-sample gate gap for click-free retriggering)
            self.voice_alloc.tick_sample(&self.state);

            let (raw_l_pre, raw_r_pre) = graph.get_stereo();

            // DC blocker: 1-pole high-pass at ~20 Hz
            let dc_l = raw_l_pre - self.dc_x_prev_l + self.dc_coeff * self.dc_y_prev_l;
            let dc_r = raw_r_pre - self.dc_x_prev_r + self.dc_coeff * self.dc_y_prev_r;
            self.dc_x_prev_l = raw_l_pre;
            self.dc_y_prev_l = dc_l;
            self.dc_x_prev_r = raw_r_pre;
            self.dc_y_prev_r = dc_r;
            let (mut raw_l, mut raw_r) = (dc_l, dc_r);

            // Lookahead limiter
            if limiter_on {
                if let Some(lim) = &mut self.lookahead_lim {
                    let (lim_l, lim_r) = lim.process_stereo(raw_l, raw_r, threshold);
                    raw_l = lim_l;
                    raw_r = lim_r;
                }
            }

            // Soft clip + tremolo (amp LFO mod) + smoothed global volume
            let target_global = self.state.global_vol.value() as f32;
            self.global_vol_smooth =
                target_global + self.global_vol_coeff * (self.global_vol_smooth - target_global);
            let l =
                if raw_l.is_finite() { raw_l.tanh() } else { 0.0 } * amp_mod * self.global_vol_smooth;
            let r =
                if raw_r.is_finite() { raw_r.tanh() } else { 0.0 } * amp_mod * self.global_vol_smooth;

            if l.abs() > peak_l_local {
                peak_l_local = l.abs();
            }
            if r.abs() > peak_r_local {
                peak_r_local = r.abs();
            }

            output[0][i] = l;
            output[1][i] = r;
        }

        self.state
            .peak_l
            .store(peak_l_local.to_bits(), Ordering::Relaxed);
        self.state
            .peak_r
            .store(peak_r_local.to_bits(), Ordering::Relaxed);

        ProcessStatus::Normal
    }
}

// ---------------------------------------------------------------------------
// Plugin exports
// ---------------------------------------------------------------------------

impl ClapPlugin for TheSynthPlugin {
    const CLAP_ID: &'static str = "io.github.francescoventura.the-synth";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("MiniMoog-inspired subtractive synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for TheSynthPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"TheSynthFrancesc";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_clap!(TheSynthPlugin);
nih_export_vst3!(TheSynthPlugin);
