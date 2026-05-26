//! cpal stream setup — multi-track mixer.
//!
//! Four independent synth engines run simultaneously. Each is a `TrackProcessor`
//! that owns its own DSP graph, voice allocator, LFO state, gate lanes, DC
//! blocker, and glide state. The audio callback processes all four tracks per
//! buffer, applies per-track volume/pan/mute from `TrackMixerAtomics`, sums
//! into a stereo mix bus, then applies the lookahead limiter and outputs.

#![allow(clippy::precedence)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use fundsp::prelude32::*;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::eq::{EqParams, ParametricEq};
use crate::recorder::Recorder;
use forma_control::{make_control_channel, ControlReceiver};
use forma_dsp::LookaheadLimiter;
use forma_engine::audio::build_synth_graph;
use forma_engine::{SynthEngineHandle, VoiceAllocator};

pub use forma_engine::audio::{AudioState, VOICE_COUNT};

pub const TRACK_COUNT: usize = 4;
pub const DRUM_CHANNELS: usize = 8;
pub const DRUM_STEP_COUNT: usize = 16;

type RecorderSink = Arc<Mutex<Option<Recorder>>>;

// ── Per-track mixer atomics (UI → audio thread) ──────────────────────────────

pub struct TrackMixerAtomics {
    volume: AtomicU32, // f32 bits, 0.0–1.0
    pan: AtomicU32,    // f32 bits, -1.0–+1.0
    pub muted: AtomicBool,
    pub solo: AtomicBool,
    /// Per-track peak level — written by audio thread, read by UI for VU meters.
    pub peak_l: AtomicU32, // f32 bits
    pub peak_r: AtomicU32, // f32 bits
}

impl TrackMixerAtomics {
    fn new(volume: f32, muted: bool) -> Arc<Self> {
        Arc::new(Self {
            volume: AtomicU32::new(volume.to_bits()),
            pan: AtomicU32::new(0.0f32.to_bits()),
            muted: AtomicBool::new(muted),
            solo: AtomicBool::new(false),
            peak_l: AtomicU32::new(0),
            peak_r: AtomicU32::new(0),
        })
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }
    pub fn pan(&self) -> f32 {
        f32::from_bits(self.pan.load(Ordering::Relaxed))
    }
    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }
    pub fn solo(&self) -> bool {
        self.solo.load(Ordering::Relaxed)
    }
    pub fn peak(&self) -> f32 {
        let l = f32::from_bits(self.peak_l.load(Ordering::Relaxed));
        let r = f32::from_bits(self.peak_r.load(Ordering::Relaxed));
        l.max(r)
    }
    pub fn set_volume(&self, v: f32) {
        self.volume.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn set_pan(&self, v: f32) {
        self.pan.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn set_muted(&self, v: bool) {
        self.muted.store(v, Ordering::Relaxed);
    }
    pub fn set_solo(&self, v: bool) {
        self.solo.store(v, Ordering::Relaxed);
    }
}

// ── Drum engine atomics (UI → audio thread) ──────────────────────────────────

pub struct DrumEngineAtomics {
    pub enabled: AtomicBool,
    pub bpm: AtomicU32,                            // f32 bits
    pub swing: AtomicU32,                          // f32 bits, 0.0–0.5
    pub step_patterns: [AtomicU16; DRUM_CHANNELS], // bit i = step i active
    pub channel_muted: [AtomicBool; DRUM_CHANNELS],
    pub channel_volume: [AtomicU32; DRUM_CHANNELS], // f32 bits
    pub current_step: AtomicUsize,                  // written by audio, read by UI
    /// UI sets true; audio thread resets step phase to bar 1 on next sample and clears.
    pub phase_reset: AtomicBool,
    // Per-step velocity (0–127) for the active pattern
    pub step_vel: [[std::sync::atomic::AtomicU8; DRUM_STEP_COUNT]; DRUM_CHANNELS],
    // Per-channel voice synthesis params (f32 bits)
    pub base_freq: [AtomicU32; DRUM_CHANNELS],   // Hz
    pub pitch_range: [AtomicU32; DRUM_CHANNELS], // Hz of pitch sweep
    pub amp_decay_s: [AtomicU32; DRUM_CHANNELS], // seconds
    pub noise_mix: [AtomicU32; DRUM_CHANNELS],   // 0.0–1.0
    // Drum bus mixer
    pub volume: AtomicU32, // f32 bits
    pub pan: AtomicU32,    // f32 bits, -1..+1
    pub muted: AtomicBool,
    pub peak_l: AtomicU32, // f32 bits
    pub peak_r: AtomicU32, // f32 bits
}

impl DrumEngineAtomics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(false),
            bpm: AtomicU32::new(120.0f32.to_bits()),
            swing: AtomicU32::new(0.0f32.to_bits()),
            step_patterns: std::array::from_fn(|_| AtomicU16::new(0)),
            channel_muted: std::array::from_fn(|_| AtomicBool::new(false)),
            channel_volume: std::array::from_fn(|_| AtomicU32::new(0.8f32.to_bits())),
            current_step: AtomicUsize::new(0),
            phase_reset: AtomicBool::new(false),
            step_vel: std::array::from_fn(|_| {
                std::array::from_fn(|_| std::sync::atomic::AtomicU8::new(100))
            }),
            base_freq: std::array::from_fn(|i| AtomicU32::new(DRUM_BASE_FREQ[i].to_bits())),
            pitch_range: std::array::from_fn(|i| AtomicU32::new(DRUM_PITCH_RANGE[i].to_bits())),
            amp_decay_s: std::array::from_fn(|i| AtomicU32::new(DRUM_AMP_DECAY_S[i].to_bits())),
            noise_mix: std::array::from_fn(|i| AtomicU32::new(DRUM_DEFAULT_NOISE_MIX[i].to_bits())),
            volume: AtomicU32::new(0.8f32.to_bits()),
            pan: AtomicU32::new(0.0f32.to_bits()),
            muted: AtomicBool::new(false),
            peak_l: AtomicU32::new(0),
            peak_r: AtomicU32::new(0),
        })
    }

    pub fn bpm(&self) -> f32 {
        f32::from_bits(self.bpm.load(Ordering::Relaxed))
    }
    pub fn swing(&self) -> f32 {
        f32::from_bits(self.swing.load(Ordering::Relaxed))
    }
    pub fn channel_volume(&self, ch: usize) -> f32 {
        f32::from_bits(self.channel_volume[ch].load(Ordering::Relaxed))
    }
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }
    pub fn pan(&self) -> f32 {
        f32::from_bits(self.pan.load(Ordering::Relaxed))
    }
    pub fn peak(&self) -> f32 {
        let l = f32::from_bits(self.peak_l.load(Ordering::Relaxed));
        let r = f32::from_bits(self.peak_r.load(Ordering::Relaxed));
        l.max(r)
    }
    pub fn set_bpm(&self, v: f32) {
        self.bpm.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn set_swing(&self, v: f32) {
        self.swing.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn set_volume(&self, v: f32) {
        self.volume.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn set_pan(&self, v: f32) {
        self.pan.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn set_channel_volume(&self, ch: usize, v: f32) {
        self.channel_volume[ch].store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn base_freq(&self, ch: usize) -> f32 {
        f32::from_bits(self.base_freq[ch].load(Ordering::Relaxed))
    }
    pub fn pitch_range(&self, ch: usize) -> f32 {
        f32::from_bits(self.pitch_range[ch].load(Ordering::Relaxed))
    }
    pub fn amp_decay_s(&self, ch: usize) -> f32 {
        f32::from_bits(self.amp_decay_s[ch].load(Ordering::Relaxed))
    }
    pub fn noise_mix(&self, ch: usize) -> f32 {
        f32::from_bits(self.noise_mix[ch].load(Ordering::Relaxed))
    }
    pub fn set_base_freq(&self, ch: usize, v: f32) {
        self.base_freq[ch].store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn set_pitch_range(&self, ch: usize, v: f32) {
        self.pitch_range[ch].store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn set_amp_decay_s(&self, ch: usize, v: f32) {
        self.amp_decay_s[ch].store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn set_noise_mix(&self, ch: usize, v: f32) {
        self.noise_mix[ch].store(v.to_bits(), Ordering::Relaxed);
    }
}

// ── Per-track processor (owns all per-track callback state) ──────────────────

struct TrackProcessor {
    state: Arc<AudioState>,
    rx: ControlReceiver,
    graph: BlockRateAdapter,
    voices: VoiceAllocator,

    // LFO phases
    lfo_phase: f32,
    lfo2_phase: f32,

    // Per-buffer LFO params (read once in begin_buffer)
    lfo_rate: f32,
    lfo_depth: f32,
    lfo_shape: u8,
    lfo_dest: u8,
    lfo_dt: f32,
    lfo2_rate: f32,
    lfo2_depth: f32,
    lfo2_shape: u8,
    lfo2_dest: u8,
    lfo2_dt: f32,

    // Per-buffer key-tracked cutoff
    keyed_cutoff: f32,
    last_keyed_freq: f32,

    // Per-buffer mod wheel / aftertouch routing
    mod_wheel_dest: u8,
    mod_wheel_depth: f32,
    aftertouch_dest: u8,
    aftertouch_depth: f32,

    // Per-buffer mod matrix (4 slots)
    mat_src: [u8; 4],
    mat_dst: [u8; 4],
    mat_depth: [f32; 4],

    // Gate lane "Pulse" (amp ducker)
    gate_aenv_enabled: bool,
    gate_aenv_pattern: u16,
    gate_aenv_length: u32,
    gate_aenv_dt: f32,
    gate_aenv_acc: f32,
    gate_aenv_step: u32,
    gate_aenv_was_enabled: bool,
    duck_env: f32,
    duck_attacking: bool,
    depth_smooth: f32,

    // Gate lane LFO1 retrigger
    gate_lfo1_enabled: bool,
    gate_lfo1_pattern: u16,
    gate_lfo1_length: u32,
    gate_lfo1_dt: f32,
    gate_lfo1_acc: f32,
    gate_lfo1_step: u32,
    gate_lfo1_was_enabled: bool,

    // Gate lane LFO2 retrigger
    gate_lfo2_enabled: bool,
    gate_lfo2_pattern: u16,
    gate_lfo2_length: u32,
    gate_lfo2_dt: f32,
    gate_lfo2_acc: f32,
    gate_lfo2_step: u32,
    gate_lfo2_was_enabled: bool,

    // DC blocker state
    dc_x_prev_l: f32,
    dc_x_prev_r: f32,
    dc_y_prev_l: f32,
    dc_y_prev_r: f32,

    // Glide
    smoothed_freqs: Vec<f32>,

    // Volume staging (smoothed 1/sqrt(n_voices))
    voice_gain_smooth: f32,

    // Smoothed global volume
    global_vol_smooth: f32,

    // Pre-computed smoothing coefficients (fixed per stream)
    vgs_coeff: f32,
    duck_attack_coeff: f32,
    duck_decay_coeff: f32,
    depth_smooth_coeff: f32,
    global_vol_coeff: f32,
}

impl TrackProcessor {
    fn new(state: Arc<AudioState>, rx: ControlReceiver, sr: f64) -> Self {
        let graph = BlockRateAdapter::new(build_synth_graph(&state, sr) as Box<dyn AudioUnit>);
        let sr_f = sr as f32;
        Self {
            state,
            rx,
            graph,
            voices: VoiceAllocator::new(),
            lfo_phase: 0.0,
            lfo2_phase: 0.25,
            lfo_rate: 0.0,
            lfo_depth: 0.0,
            lfo_shape: 0u8,
            lfo_dest: 1u8,
            lfo_dt: 0.0,
            lfo2_rate: 0.0,
            lfo2_depth: 0.0,
            lfo2_shape: 0u8,
            lfo2_dest: 2u8,
            lfo2_dt: 0.0,
            keyed_cutoff: 3000.0,
            last_keyed_freq: 261.63,
            mod_wheel_dest: 1,
            mod_wheel_depth: 0.5,
            aftertouch_dest: 1,
            aftertouch_depth: 0.3,
            mat_src: [0u8; 4],
            mat_dst: [0u8; 4],
            mat_depth: [0.0f32; 4],
            gate_aenv_enabled: false,
            gate_aenv_pattern: 0,
            gate_aenv_length: 16,
            gate_aenv_dt: 0.0,
            gate_aenv_acc: 1.0,
            gate_aenv_step: 0,
            gate_aenv_was_enabled: false,
            duck_env: 0.0,
            duck_attacking: false,
            depth_smooth: 0.0,
            gate_lfo1_enabled: false,
            gate_lfo1_pattern: 0,
            gate_lfo1_length: 16,
            gate_lfo1_dt: 0.0,
            gate_lfo1_acc: 1.0,
            gate_lfo1_step: 0,
            gate_lfo1_was_enabled: false,
            gate_lfo2_enabled: false,
            gate_lfo2_pattern: 0,
            gate_lfo2_length: 16,
            gate_lfo2_dt: 0.0,
            gate_lfo2_acc: 1.0,
            gate_lfo2_step: 0,
            gate_lfo2_was_enabled: false,
            dc_x_prev_l: 0.0,
            dc_x_prev_r: 0.0,
            dc_y_prev_l: 0.0,
            dc_y_prev_r: 0.0,
            smoothed_freqs: vec![440.0; VOICE_COUNT],
            voice_gain_smooth: 1.0,
            global_vol_smooth: 1.0,
            vgs_coeff: (-1.0_f64 / (0.020 * sr)).exp() as f32,
            duck_attack_coeff: (-1.0_f32 / (0.0015 * sr_f)).exp(),
            duck_decay_coeff: (-1.0_f32 / (0.150 * sr_f)).exp(),
            depth_smooth_coeff: (-1.0_f32 / (0.010 * sr_f)).exp(),
            global_vol_coeff: (-1.0_f64 / (0.010 * sr)).exp() as f32,
        }
    }

    /// Called once per buffer before the per-sample loop.
    fn begin_buffer(&mut self, frames: usize, sr: f64) {
        let sr_f = sr as f32;

        // Drain control events, tick arp/walker.
        self.voices.begin_buffer(&self.state, &self.rx, frames, sr);

        // Voice gain staging: smooth 1/sqrt(active_voices).
        let n_active = Ord::max(
            self.state
                .amp_cursors
                .iter()
                .filter(|c| c.value() > 0.01)
                .count(),
            1,
        );
        let target_scale = 1.0_f32 / (n_active as f32).sqrt();
        self.voice_gain_smooth =
            target_scale + self.vgs_coeff * (self.voice_gain_smooth - target_scale);
        self.state.voice_gain_scale.set(self.voice_gain_smooth);

        // Per-buffer LFO params.
        self.lfo_rate = self.state.lfo_rate.value();
        self.lfo_depth = self.state.lfo_depth.value();
        self.lfo_shape = self.state.lfo_shape.load(Ordering::Relaxed);
        self.lfo_dest = self.state.lfo_dest.load(Ordering::Relaxed);
        self.lfo_dt = self.lfo_rate / sr_f;
        self.lfo2_rate = self.state.lfo2_rate.value();
        self.lfo2_depth = self.state.lfo2_depth.value();
        self.lfo2_shape = self.state.lfo2_shape.load(Ordering::Relaxed);
        self.lfo2_dest = self.state.lfo2_dest.load(Ordering::Relaxed);
        self.lfo2_dt = self.lfo2_rate / sr_f;

        // Gate lane params (read once per buffer).
        macro_rules! read_gate {
            ($lane:ident, $enabled:ident, $pattern:ident, $length:ident, $dt:ident,
             $acc:ident, $step:ident, $was:ident) => {{
                let en = self.state.$lane.enabled.load(Ordering::Relaxed);
                let pat = self.state.$lane.pattern.load(Ordering::Relaxed);
                let len = {
                    let raw = self.state.$lane.length.load(Ordering::Relaxed);
                    if raw < 1 {
                        1u32
                    } else {
                        raw as u32
                    }
                };
                let dt = self.state.$lane.rate.value() / sr_f;
                if en && !self.$was {
                    self.$acc = 1.0;
                    self.$step = 0;
                }
                self.$was = en;
                self.$enabled = en;
                self.$pattern = pat;
                self.$length = len;
                self.$dt = dt;
            }};
        }
        read_gate!(
            gate_aenv,
            gate_aenv_enabled,
            gate_aenv_pattern,
            gate_aenv_length,
            gate_aenv_dt,
            gate_aenv_acc,
            gate_aenv_step,
            gate_aenv_was_enabled
        );
        read_gate!(
            gate_lfo1,
            gate_lfo1_enabled,
            gate_lfo1_pattern,
            gate_lfo1_length,
            gate_lfo1_dt,
            gate_lfo1_acc,
            gate_lfo1_step,
            gate_lfo1_was_enabled
        );
        read_gate!(
            gate_lfo2,
            gate_lfo2_enabled,
            gate_lfo2_pattern,
            gate_lfo2_length,
            gate_lfo2_dt,
            gate_lfo2_acc,
            gate_lfo2_step,
            gate_lfo2_was_enabled
        );

        // Read mod-wheel / aftertouch routing params once per buffer.
        self.mod_wheel_dest = self.state.mod_wheel_dest.load(Ordering::Relaxed);
        self.mod_wheel_depth = self.state.mod_wheel_depth.value();
        self.aftertouch_dest = self.state.aftertouch_dest.load(Ordering::Relaxed);
        self.aftertouch_depth = self.state.aftertouch_depth.value();
        for i in 0..4 {
            self.mat_src[i] = self.state.mat_src[i].load(Ordering::Relaxed);
            self.mat_dst[i] = self.state.mat_dst[i].load(Ordering::Relaxed);
            self.mat_depth[i] = self.state.mat_depth[i].value();
        }

        // Key tracking: find highest sounding voice.
        let key_track = self.state.filter_key_track.value();
        let base_cutoff = self.state.cutoff.value().clamp(80.0, 18000.0);
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
        self.keyed_cutoff = base_cutoff * key_mult;

        // Glide: smooth voice frequencies.
        let glide_time = self.state.glide_time.value();
        for vi in 0..VOICE_COUNT {
            let target = self.state.voice_freq_targets[vi].value();
            if glide_time < 0.001 {
                self.smoothed_freqs[vi] = target;
            } else {
                let coeff = (-(frames as f32) / (glide_time * sr_f)).exp();
                self.smoothed_freqs[vi] = coeff * self.smoothed_freqs[vi] + (1.0 - coeff) * target;
            }
            self.state.voice_freqs[vi].set(self.smoothed_freqs[vi]);
        }
    }

    /// Called once per sample. Returns post-DC-blocker (L, R) with global vol
    /// and duck envelope applied, ready to be mixed.
    #[inline]
    fn get_stereo_frame(&mut self, dc_coeff: f32) -> (f32, f32) {
        // LFO 1.
        self.lfo_phase += self.lfo_dt;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= 1.0;
        }
        let lfo_raw = lfo_shape_sample(self.lfo_phase, self.lfo_shape);

        // LFO 2.
        self.lfo2_phase += self.lfo2_dt;
        if self.lfo2_phase >= 1.0 {
            self.lfo2_phase -= 1.0;
        }
        let lfo2_raw = lfo_shape_sample(self.lfo2_phase, self.lfo2_shape);

        // Apply mod wheel to LFO depth before the combining loop when dest==LFO Depth.
        let eff_lfo_depth = if self.mod_wheel_dest == 2 {
            (self.lfo_depth + self.state.mod_wheel.value() * self.mod_wheel_depth * self.lfo_depth)
                .min(1.0)
        } else {
            self.lfo_depth
        };
        let eff_lfo_depth = if self.aftertouch_dest == 2 {
            (eff_lfo_depth + self.state.aftertouch.value() * self.aftertouch_depth * eff_lfo_depth)
                .min(1.0)
        } else {
            eff_lfo_depth
        };

        // Combine modulation.
        let mut pitch_mod: f32 = 0.0;
        let mut filter_mod: f32 = 0.0;
        let mut amp_mod: f32 = 1.0;
        for (raw, depth, dest) in [
            (lfo_raw, eff_lfo_depth, self.lfo_dest),
            (lfo2_raw, self.lfo2_depth, self.lfo2_dest),
        ] {
            match dest {
                0 => pitch_mod += raw * depth,
                2 => amp_mod *= 1.0 - depth * (1.0 - raw) * 0.5,
                _ => filter_mod += raw * depth,
            }
        }
        // Mod wheel / aftertouch — DC signal routed to filter or amp.
        for (raw, depth, dest) in [
            (
                self.state.mod_wheel.value(),
                self.mod_wheel_depth,
                self.mod_wheel_dest,
            ),
            (
                self.state.aftertouch.value(),
                self.aftertouch_depth,
                self.aftertouch_dest,
            ),
        ] {
            match dest {
                1 => filter_mod += raw * depth,
                3 => amp_mod *= 1.0 - raw * depth * 0.5,
                _ => {}
            }
        }

        // Mod matrix — 4 free-routing slots.
        let mw_raw = self.state.mod_wheel.value();
        let at_raw = self.state.aftertouch.value();
        for i in 0..4 {
            let sig = match self.mat_src[i] {
                1 => lfo_raw,
                2 => lfo2_raw,
                3 => mw_raw,
                4 => at_raw,
                _ => continue,
            };
            let d = self.mat_depth[i];
            match self.mat_dst[i] {
                1 => filter_mod += sig * d,
                2 => amp_mod = (amp_mod - sig * d * 0.5).max(0.0),
                3 => pitch_mod += sig * d,
                _ => {}
            }
        }
        self.state
            .lfo_pitch_mult
            .set(2_f32.powf(pitch_mod * 2.0 / 12.0));
        self.state
            .effective_cutoff
            .set((self.keyed_cutoff + filter_mod * self.keyed_cutoff * 0.5).clamp(80.0, 18000.0));

        // Gate lane "Pulse" (amp ducker).
        if self.gate_aenv_enabled {
            self.gate_aenv_acc += self.gate_aenv_dt;
            if self.gate_aenv_acc >= 1.0 {
                self.gate_aenv_acc -= 1.0;
                let step_idx = (self.gate_aenv_step % self.gate_aenv_length) as u8;
                if (self.gate_aenv_pattern >> step_idx) & 1 != 0 {
                    self.duck_attacking = true;
                }
                self.gate_aenv_step = self.gate_aenv_step.wrapping_add(1);
            }
        }
        // Gate lanes for LFO retrigger.
        if self.gate_lfo1_enabled {
            self.gate_lfo1_acc += self.gate_lfo1_dt;
            if self.gate_lfo1_acc >= 1.0 {
                self.gate_lfo1_acc -= 1.0;
                let step_idx = (self.gate_lfo1_step % self.gate_lfo1_length) as u8;
                if (self.gate_lfo1_pattern >> step_idx) & 1 != 0 {
                    self.lfo_phase = 0.0;
                }
                self.gate_lfo1_step = self.gate_lfo1_step.wrapping_add(1);
            }
        }
        if self.gate_lfo2_enabled {
            self.gate_lfo2_acc += self.gate_lfo2_dt;
            if self.gate_lfo2_acc >= 1.0 {
                self.gate_lfo2_acc -= 1.0;
                let step_idx = (self.gate_lfo2_step % self.gate_lfo2_length) as u8;
                if (self.gate_lfo2_pattern >> step_idx) & 1 != 0 {
                    self.lfo2_phase = 0.0;
                }
                self.gate_lfo2_step = self.gate_lfo2_step.wrapping_add(1);
            }
        }
        // Duck envelope.
        if self.duck_attacking {
            self.duck_env = 1.0 + self.duck_attack_coeff * (self.duck_env - 1.0);
            if self.duck_env > 0.99 {
                self.duck_attacking = false;
            }
        } else {
            self.duck_env *= self.duck_decay_coeff;
        }

        self.voices.tick_sample(&self.state);

        let (raw_l, raw_r) = self.graph.get_stereo();

        // DC blocker.
        let dc_l = raw_l - self.dc_x_prev_l + dc_coeff * self.dc_y_prev_l;
        let dc_r = raw_r - self.dc_x_prev_r + dc_coeff * self.dc_y_prev_r;
        self.dc_x_prev_l = raw_l;
        self.dc_y_prev_l = dc_l;
        self.dc_x_prev_r = raw_r;
        self.dc_y_prev_r = dc_r;

        // Global volume + duck.
        let target_global = self.state.global_vol.value() as f32;
        self.global_vol_smooth =
            target_global + self.global_vol_coeff * (self.global_vol_smooth - target_global);
        let target_depth = self.state.gate_aenv_depth.value();
        self.depth_smooth =
            target_depth + self.depth_smooth_coeff * (self.depth_smooth - target_depth);
        let duck_mult = 1.0 - self.duck_env * self.depth_smooth;

        let l = if dc_l.is_finite() { dc_l.tanh() } else { 0.0 }
            * amp_mod
            * self.global_vol_smooth
            * duck_mult;
        let r = if dc_r.is_finite() { dc_r.tanh() } else { 0.0 }
            * amp_mod
            * self.global_vol_smooth
            * duck_mult;

        (l, r)
    }
}

#[inline]
fn lfo_shape_sample(phase: f32, shape: u8) -> f32 {
    match shape {
        1 => {
            if phase < 0.5 {
                4.0 * phase - 1.0
            } else {
                3.0 - 4.0 * phase
            }
        }
        2 => 2.0 * phase - 1.0,
        _ => (phase * std::f32::consts::TAU).sin(),
    }
}

// ── Drum synthesis ───────────────────────────────────────────────────────────
//
// Channel map (matches CHANNEL_NAMES in drum_machine_ui):
//   0=KICK  1=SNARE  2=HAT  3=CLAP  4=TOM1  5=TOM2  6=PERC  7=NOISE

const DRUM_AMP_DECAY_S: [f32; DRUM_CHANNELS] = [0.25, 0.12, 0.045, 0.09, 0.20, 0.26, 0.06, 0.35];
const DRUM_PITCH_DECAY_S: [f32; DRUM_CHANNELS] = [0.08, 0.0, 0.0, 0.0, 0.12, 0.14, 0.03, 0.0];
const DRUM_BASE_FREQ: [f32; DRUM_CHANNELS] = [55.0, 180.0, 0.0, 0.0, 120.0, 75.0, 350.0, 0.0];
const DRUM_PITCH_RANGE: [f32; DRUM_CHANNELS] = [150.0, 0.0, 0.0, 0.0, 80.0, 60.0, 200.0, 0.0];
// Noise blend per channel: 0.0 = pure sine tone, 1.0 = pure noise
pub const DRUM_DEFAULT_NOISE_MIX: [f32; DRUM_CHANNELS] = [0.0, 0.6, 1.0, 1.0, 0.0, 0.0, 0.5, 1.0];

struct DrumVoice {
    phase: f32,
    env: f32,
    pitch_env: f32,
    hp_x1: f32,
    hp_y1: f32,
    noise: u32,
}

impl DrumVoice {
    fn new(seed: u32) -> Self {
        Self {
            phase: 0.0,
            env: 0.0,
            pitch_env: 0.0,
            hp_x1: 0.0,
            hp_y1: 0.0,
            noise: seed,
        }
    }

    fn trigger(&mut self) {
        self.env = 1.0;
        self.pitch_env = 1.0;
    }

    fn trigger_vel(&mut self, vel: f32) {
        self.env = vel.clamp(0.0, 1.0);
        self.pitch_env = 1.0;
    }

    #[inline]
    fn next_noise(&mut self) -> f32 {
        self.noise = self
            .noise
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        (self.noise as i32 as f32) * (1.0 / i32::MAX as f32)
    }
}

struct DrumProcessor {
    atomics: Arc<DrumEngineAtomics>,
    voices: [DrumVoice; DRUM_CHANNELS],
    amp_coeff: [f32; DRUM_CHANNELS],
    pitch_coeff: [f32; DRUM_CHANNELS],
    hp_coeff: f32,
    sr: f32,
    // Step clock
    step_acc: f32,
    step_dt: f32,
    step_idx: usize,
    // Per-buffer cached params
    patterns: [u16; DRUM_CHANNELS],
    ch_muted: [bool; DRUM_CHANNELS],
    ch_vol: [f32; DRUM_CHANNELS],
    ch_base_freq: [f32; DRUM_CHANNELS],
    ch_pitch_range: [f32; DRUM_CHANNELS],
    ch_noise_mix: [f32; DRUM_CHANNELS],
    enabled: bool,
}

impl DrumProcessor {
    fn new(sr: f64, atomics: Arc<DrumEngineAtomics>) -> Self {
        let sr_f = sr as f32;
        let amp_coeff = std::array::from_fn(|ch| {
            let d = DRUM_AMP_DECAY_S[ch];
            if d > 0.0 {
                (-1.0 / (d * sr_f)).exp()
            } else {
                0.0
            }
        });
        let pitch_coeff = std::array::from_fn(|ch| {
            let d = DRUM_PITCH_DECAY_S[ch];
            if d > 0.0 {
                (-1.0 / (d * sr_f)).exp()
            } else {
                0.0
            }
        });
        // 1-pole HP coefficient for the hat (6 kHz cutoff)
        let hp_coeff = (-(std::f32::consts::TAU * 6000.0) / sr_f).exp();
        Self {
            atomics,
            voices: std::array::from_fn(|ch| {
                DrumVoice::new(0x5EED_1234u32.wrapping_mul(ch as u32 + 1))
            }),
            amp_coeff,
            pitch_coeff,
            hp_coeff,
            sr: sr_f,
            step_acc: 1.0,
            step_dt: 0.0,
            step_idx: 15,
            patterns: [0u16; DRUM_CHANNELS],
            ch_muted: [false; DRUM_CHANNELS],
            ch_vol: [0.8; DRUM_CHANNELS],
            ch_base_freq: DRUM_BASE_FREQ,
            ch_pitch_range: DRUM_PITCH_RANGE,
            ch_noise_mix: DRUM_DEFAULT_NOISE_MIX,
            enabled: false,
        }
    }

    fn begin_buffer(&mut self, _frames: usize, _sr: f64) {
        let bpm = self.atomics.bpm();
        self.step_dt = bpm * 4.0 / 60.0 / self.sr; // 16th-note steps
        self.enabled = self.atomics.enabled.load(Ordering::Relaxed);
        if self.atomics.phase_reset.swap(false, Ordering::Relaxed) {
            // Snap to step 0 at the next step boundary.
            self.step_acc = 1.0;
            self.step_idx = 15; // advances to 0 on next trigger
        }
        for ch in 0..DRUM_CHANNELS {
            self.patterns[ch] = self.atomics.step_patterns[ch].load(Ordering::Relaxed);
            self.ch_muted[ch] = self.atomics.channel_muted[ch].load(Ordering::Relaxed);
            self.ch_vol[ch] = self.atomics.channel_volume(ch);
            self.ch_base_freq[ch] = self.atomics.base_freq(ch);
            self.ch_pitch_range[ch] = self.atomics.pitch_range(ch);
            self.ch_noise_mix[ch] = self.atomics.noise_mix(ch);
            // Recompute amplitude decay coefficient from the UI-controlled decay time.
            let amp_d = self.atomics.amp_decay_s(ch).max(0.001);
            self.amp_coeff[ch] = (-1.0 / (amp_d * self.sr)).exp();
        }
    }

    #[inline]
    fn get_stereo_frame(&mut self) -> (f32, f32) {
        if !self.enabled {
            return (0.0, 0.0);
        }

        // Step clock
        self.step_acc += self.step_dt;
        if self.step_acc >= 1.0 {
            self.step_acc -= 1.0;
            self.step_idx = (self.step_idx + 1) % 16;
            self.atomics
                .current_step
                .store(self.step_idx, Ordering::Relaxed);
            for ch in 0..DRUM_CHANNELS {
                if !self.ch_muted[ch] && (self.patterns[ch] >> self.step_idx) & 1 != 0 {
                    let vel = self.atomics.step_vel[ch][self.step_idx].load(Ordering::Relaxed)
                        as f32
                        / 127.0;
                    self.voices[ch].trigger_vel(vel);
                }
            }
        }

        let mut out: f32 = 0.0;
        for ch in 0..DRUM_CHANNELS {
            let v = &mut self.voices[ch];
            if v.env < 0.001 {
                continue;
            }
            v.env *= self.amp_coeff[ch];
            if self.pitch_coeff[ch] > 0.0 {
                v.pitch_env *= self.pitch_coeff[ch];
            }
            let freq = self.ch_base_freq[ch] + self.ch_pitch_range[ch] * v.pitch_env;
            if freq > 1.0 {
                v.phase += freq / self.sr;
                if v.phase >= 1.0 {
                    v.phase -= 1.0;
                }
            }
            let noise_raw = v.next_noise();
            // HAT channel runs noise through a 1-pole HP filter for a crisp tone.
            let noise = if ch == 2 {
                let hp = noise_raw - v.hp_x1 + self.hp_coeff * v.hp_y1;
                v.hp_x1 = noise_raw;
                v.hp_y1 = hp;
                hp
            } else {
                noise_raw
            };
            let sine = (v.phase * std::f32::consts::TAU).sin();
            let nm = self.ch_noise_mix[ch];
            let sample = (sine * (1.0 - nm) + noise * nm) * v.env;
            out += sample * self.ch_vol[ch];
        }
        // Soft-clip the summed drum bus
        let out = out.tanh() * 0.85;
        (out, out) // mono → stereo; pan applied in mix bus
    }
}

// ── AudioEngine ──────────────────────────────────────────────────────────────

pub struct AudioEngine {
    /// One handle per synth track. Track 0 = the existing forma synth UI.
    pub handles: [SynthEngineHandle; TRACK_COUNT],
    /// Per-track mixer atomics — shared with the audio callback.
    pub mixers: [Arc<TrackMixerAtomics>; TRACK_COUNT],
    /// Drum engine atomics — shared with the audio callback.
    pub drum: Arc<DrumEngineAtomics>,
    /// Mix-bus parametric EQ params — UI writes, audio thread reads.
    pub eq: Arc<Mutex<EqParams>>,
    _stream: Stream,
}

impl AudioEngine {
    pub fn new(recorder_sink: RecorderSink) -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("No output device"))?;
        let config = device.default_output_config()?;
        let sr = config.sample_rate().0 as f64;

        // Create 4 independent engine instances.
        let mut states: Vec<Arc<AudioState>> = Vec::with_capacity(TRACK_COUNT);
        let mut rxs: Vec<ControlReceiver> = Vec::with_capacity(TRACK_COUNT);
        let mut handle_vec: Vec<SynthEngineHandle> = Vec::with_capacity(TRACK_COUNT);
        let mut mixer_vec: Vec<Arc<TrackMixerAtomics>> = Vec::with_capacity(TRACK_COUNT);

        for i in 0..TRACK_COUNT {
            let state = Arc::new(AudioState::new());
            state.sample_rate.store(sr as u32, Ordering::Relaxed);
            let (tx, rx) = make_control_channel(1024);
            let handle = SynthEngineHandle::new(Arc::clone(&state), tx);
            // Track 0 is live at full volume; tracks 1–3 start muted.
            let mixer = TrackMixerAtomics::new(0.8, i > 0);
            states.push(state);
            rxs.push(rx);
            handle_vec.push(handle);
            mixer_vec.push(mixer);
        }

        let drum = DrumEngineAtomics::new();
        let eq: Arc<Mutex<EqParams>> = Arc::new(Mutex::new(EqParams::default()));

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build_stream::<f32>(
                &device,
                &config.into(),
                states,
                rxs,
                Arc::clone(&mixer_vec[0]),
                Arc::clone(&mixer_vec[1]),
                Arc::clone(&mixer_vec[2]),
                Arc::clone(&mixer_vec[3]),
                Arc::clone(&drum),
                Arc::clone(&eq),
                sr,
                recorder_sink,
            )?,
            cpal::SampleFormat::I16 => build_stream::<i16>(
                &device,
                &config.into(),
                states,
                rxs,
                Arc::clone(&mixer_vec[0]),
                Arc::clone(&mixer_vec[1]),
                Arc::clone(&mixer_vec[2]),
                Arc::clone(&mixer_vec[3]),
                Arc::clone(&drum),
                Arc::clone(&eq),
                sr,
                recorder_sink,
            )?,
            cpal::SampleFormat::U16 => build_stream::<u16>(
                &device,
                &config.into(),
                states,
                rxs,
                Arc::clone(&mixer_vec[0]),
                Arc::clone(&mixer_vec[1]),
                Arc::clone(&mixer_vec[2]),
                Arc::clone(&mixer_vec[3]),
                Arc::clone(&drum),
                Arc::clone(&eq),
                sr,
                recorder_sink,
            )?,
            _ => anyhow::bail!("Unsupported sample format"),
        };
        stream.play()?;

        Ok(Self {
            handles: [
                handle_vec.remove(0),
                handle_vec.remove(0),
                handle_vec.remove(0),
                handle_vec.remove(0),
            ],
            mixers: [
                mixer_vec.remove(0),
                mixer_vec.remove(0),
                mixer_vec.remove(0),
                mixer_vec.remove(0),
            ],
            drum,
            eq,
            _stream: stream,
        })
    }
}

// ── Stream builder ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    states: Vec<Arc<AudioState>>,
    rxs: Vec<ControlReceiver>,
    mixer0: Arc<TrackMixerAtomics>,
    mixer1: Arc<TrackMixerAtomics>,
    mixer2: Arc<TrackMixerAtomics>,
    mixer3: Arc<TrackMixerAtomics>,
    drum_atomics: Arc<DrumEngineAtomics>,
    eq_params: Arc<Mutex<EqParams>>,
    sr: f64,
    recorder_sink: RecorderSink,
) -> anyhow::Result<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;

    let mut states = states;
    let mut rxs = rxs;

    // Build 4 track processors.
    let mut tracks: Vec<TrackProcessor> = (0..TRACK_COUNT)
        .map(|_| TrackProcessor::new(states.remove(0), rxs.remove(0), sr))
        .collect();

    // Drum processor.
    let mut drum_proc = DrumProcessor::new(sr, Arc::clone(&drum_atomics));

    // Per-track scope ring-buffer write indices.
    let mut osc_idx: [usize; TRACK_COUNT] = [0; TRACK_COUNT];
    let mut buffer_size_captured = false;

    // Lookahead limiter on the mix bus.
    let mut lookahead_lim = LookaheadLimiter::new(sr as f32, 1.5, 80.0);

    // Mix-bus parametric EQ.
    let mut mix_eq = ParametricEq::new(sr as f32);

    // DC blocker coefficient (shared constant, same formula for all tracks).
    let dc_coeff = 1.0_f32 - (std::f32::consts::TAU * 20.0 / sr as f32);

    // Mixer references for solo logic.
    let mixers = [mixer0, mixer1, mixer2, mixer3];

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let frames = data.len() / channels;

            // First-callback setup: flush-to-zero / denormals-are-zero.
            if !buffer_size_captured {
                forma_engine::enable_ftz_on_current_thread();
                let frames_u32 = (data.len() / channels) as u32;
                tracks[0]
                    .state
                    .buffer_frames
                    .store(frames_u32, Ordering::Relaxed);
                buffer_size_captured = true;
            }

            // Latency measurement on track 0.
            if let Ok(mut guard) = tracks[0].state.note_on_time.try_lock() {
                if let Some(t) = guard.take() {
                    let us = t.elapsed().as_micros() as u32;
                    tracks[0].state.last_latency_us.store(us, Ordering::Relaxed);
                }
            }

            // Sync EQ params from UI (try_lock — never block).
            if let Ok(p) = eq_params.try_lock() {
                mix_eq.update(&p);
            }

            // Per-buffer setup for all tracks.
            for track in tracks.iter_mut() {
                track.begin_buffer(frames, sr);
            }
            drum_proc.begin_buffer(frames, sr);

            // Solo logic: if any track is soloed, mute all others.
            let any_solo = mixers.iter().any(|m| m.solo());

            // Extract limiter settings from track 0 before mutably borrowing `tracks`.
            let limiter_on = tracks[0].state.limiter_enabled.load(Ordering::Relaxed);
            let threshold = tracks[0].state.limiter_threshold.value();

            // Clone each track's scope buffer Arc so we can lock them independently
            // of the mutable borrow of `tracks` inside the frame loop.
            // try_lock — never block the audio thread.
            let scope_arcs: Vec<Arc<std::sync::Mutex<Vec<f32>>>> = tracks
                .iter()
                .map(|t| Arc::clone(&t.state.osc_buffer))
                .collect();
            let mut scope_guards: Vec<Option<std::sync::MutexGuard<Vec<f32>>>> =
                scope_arcs.iter().map(|a| a.try_lock().ok()).collect();
            let mut peak_l_local: f32 = 0.0;
            let mut peak_r_local: f32 = 0.0;
            // Per-track peak accumulators (flushed to TrackMixerAtomics after the frame loop).
            let mut track_peak_l = [0.0f32; TRACK_COUNT];
            let mut track_peak_r = [0.0f32; TRACK_COUNT];
            let mut drum_peak_l: f32 = 0.0;
            let mut drum_peak_r: f32 = 0.0;

            for (frame_i, frame) in data.chunks_mut(channels).enumerate() {
                let mut mix_l: f32 = 0.0;
                let mut mix_r: f32 = 0.0;

                for (t_idx, track) in tracks.iter_mut().enumerate() {
                    let (tl, tr) = track.get_stereo_frame(dc_coeff);

                    // Track peak before mute/solo so VU shows signal even when muted.
                    if tl.abs() > track_peak_l[t_idx] {
                        track_peak_l[t_idx] = tl.abs();
                    }
                    if tr.abs() > track_peak_r[t_idx] {
                        track_peak_r[t_idx] = tr.abs();
                    }

                    // Write pre-mix signal to this track's scope buffer (every 4th sample).
                    if frame_i & 3 == 0 {
                        if let Some(buf) = scope_guards[t_idx].as_mut() {
                            let len = buf.len();
                            buf[osc_idx[t_idx] % len] = tl;
                            osc_idx[t_idx] = osc_idx[t_idx].wrapping_add(1);
                        }
                    }

                    let muted = mixers[t_idx].muted();
                    let soloed = mixers[t_idx].solo();
                    let silenced = muted || (any_solo && !soloed);
                    if silenced {
                        continue;
                    }

                    let vol = mixers[t_idx].volume();
                    let pan = mixers[t_idx].pan(); // -1..+1
                                                   // Constant-power pan: equal-loudness at center.
                    let pan_r = (std::f32::consts::FRAC_PI_4 * (pan + 1.0)).sin();
                    let pan_l = (std::f32::consts::FRAC_PI_4 * (1.0 - pan)).sin();
                    mix_l += tl * vol * pan_l;
                    mix_r += tr * vol * pan_r;
                }

                // Drum bus
                let (dl, dr) = drum_proc.get_stereo_frame();
                if dl.abs() > drum_peak_l {
                    drum_peak_l = dl.abs();
                }
                if dr.abs() > drum_peak_r {
                    drum_peak_r = dr.abs();
                }
                if !drum_atomics.muted.load(Ordering::Relaxed) {
                    let dvol = drum_atomics.volume();
                    let dpan = drum_atomics.pan();
                    let pan_r = (std::f32::consts::FRAC_PI_4 * (dpan + 1.0)).sin();
                    let pan_l = (std::f32::consts::FRAC_PI_4 * (1.0 - dpan)).sin();
                    mix_l += dl * dvol * pan_l;
                    mix_r += dr * dvol * pan_r;
                }

                // Mix-bus parametric EQ.
                let (mix_l, mix_r) = mix_eq.process(mix_l, mix_r);

                // Lookahead limiter on the mix bus.
                let (lim_l, lim_r) = if limiter_on {
                    lookahead_lim.process_stereo(mix_l, mix_r, threshold)
                } else {
                    (mix_l, mix_r)
                };

                // Peak metering.
                if lim_l.abs() > peak_l_local {
                    peak_l_local = lim_l.abs();
                }
                if lim_r.abs() > peak_r_local {
                    peak_r_local = lim_r.abs();
                }

                // Recorder writes to the final mix output.
                if let Ok(rec) = recorder_sink.try_lock() {
                    if let Some(rec) = rec.as_ref() {
                        rec.push(lim_l, lim_r);
                    }
                }

                let left = T::from_sample(lim_l);
                let right = T::from_sample(lim_r);
                for (i, smp) in frame.iter_mut().enumerate() {
                    *smp = if i & 1 == 0 { left } else { right };
                }
            }

            // Flush per-track peaks to mixer atomics for VU meters.
            for t_idx in 0..TRACK_COUNT {
                mixers[t_idx]
                    .peak_l
                    .store(track_peak_l[t_idx].to_bits(), Ordering::Relaxed);
                mixers[t_idx]
                    .peak_r
                    .store(track_peak_r[t_idx].to_bits(), Ordering::Relaxed);
            }
            drum_atomics
                .peak_l
                .store(drum_peak_l.to_bits(), Ordering::Relaxed);
            drum_atomics
                .peak_r
                .store(drum_peak_r.to_bits(), Ordering::Relaxed);

            // Write mix-bus peak to track 0's state for the legacy latency/peak display.
            tracks[0]
                .state
                .peak_l
                .store(peak_l_local.to_bits(), Ordering::Relaxed);
            tracks[0]
                .state
                .peak_r
                .store(peak_r_local.to_bits(), Ordering::Relaxed);
        },
        |err| eprintln!("audio error: {err}"),
        None,
    )?;

    Ok(stream)
}
