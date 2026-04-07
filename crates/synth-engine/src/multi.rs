//! Generic multi-track engine.
//!
//! Four independent tracks, each a full 6-voice subtractive synth voice bank.
//! All tracks feed into two global effect buses (send_a and send_b) via
//! per-track send levels. The buses are dry pass-through until Phase 5.
//!
//! Design invariants:
//! - Graph built once at init; runtime changes via Shared atomics only.
//! - No heap allocation, no mutex, no blocking on the audio thread.
//! - Inactive voices have vol Shared = 0.0; inactive buses have send Shared = 0.0.

#![allow(clippy::precedence)]

use fundsp::prelude32::*;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::Arc;

use synth_dsp::envelope::LiveAdsr;
use synth_dsp::osc::{MultiWaveOsc, SyncRole};
use synth_dsp::shimmer::{ShimmerShared, ShimmerReverb};
use synth_dsp::crystallizer::{CrystallizerShared, Crystallizer};
use crate::arp::{ArpShared, ScaleWalkerShared};

pub const TRACK_COUNT: usize = 4;
pub const VOICE_COUNT: usize = 6;

// ---------------------------------------------------------------------------
// TrackState — parameter store for one track
// ---------------------------------------------------------------------------

/// All runtime-adjustable parameters for one track.
pub struct TrackState {
    // OSC bank — 3 oscillators per voice
    pub osc_wave: [Arc<AtomicU8>; 3],
    pub osc_freq_mult: [Shared; 3],
    pub osc_vol: [Shared; 3],
    pub osc_pulse_width: [Shared; 3],
    pub osc_unison_detune: [[Shared; 5]; 3],
    pub osc_unison_vol: [[Shared; 5]; 3],
    pub hard_sync_enabled: Arc<AtomicBool>,
    pub hard_sync_gen: Vec<Arc<std::sync::atomic::AtomicU8>>,
    pub fm_depth: Shared,
    pub fm_tap: Vec<Shared>,
    pub ring_depth: Shared,
    pub ring_tap: Vec<Shared>,
    pub noise_vol: Shared,

    // Filter
    pub cutoff: Shared,
    pub resonance: Shared,
    pub filter_env_amount: Shared,
    pub fenv_attack: Shared,
    pub fenv_decay: Shared,
    pub fenv_sustain: Shared,
    pub fenv_release: Shared,

    // LFO
    pub lfo_rate: Shared,
    pub lfo_depth: Shared,
    pub lfo_shape: Arc<AtomicU8>,
    pub lfo_dest: Arc<AtomicU8>,
    pub lfo_pitch_mult: Shared,

    // Voice freq/gate
    pub voice_freq_targets: Vec<Shared>,
    pub voice_freqs: Vec<Shared>,
    pub voice_gates: Vec<Shared>,
    pub effective_cutoff: Shared,

    // Amp ADSR
    pub adsr_attack: Shared,
    pub adsr_decay: Shared,
    pub adsr_sustain: Shared,
    pub adsr_release: Shared,
    pub amp_cursors: Vec<Shared>,
    pub fenv_cursors: Vec<Shared>,

    // Glide + volume
    pub glide_time: Shared,
    pub track_vol: Shared,

    // Effect send levels (0.0 = dry, 1.0 = fully sent)
    pub shimmer_send: Shared,
    pub crystal_send: Shared,
}

impl TrackState {
    pub fn new() -> Self {
        Self {
            osc_wave: [
                Arc::new(AtomicU8::new(1)),
                Arc::new(AtomicU8::new(0)),
                Arc::new(AtomicU8::new(0)),
            ],
            osc_freq_mult: [shared(1.0), shared(1.0), shared(1.0)],
            osc_vol: [shared(0.4), shared(0.3), shared(0.0)],
            osc_pulse_width: [shared(0.5), shared(0.5), shared(0.5)],
            osc_unison_detune: [
                [shared(1.0), shared(1.0), shared(1.0), shared(1.0), shared(1.0)],
                [shared(1.0), shared(1.0), shared(1.0), shared(1.0), shared(1.0)],
                [shared(1.0), shared(1.0), shared(1.0), shared(1.0), shared(1.0)],
            ],
            osc_unison_vol: [
                [shared(1.0), shared(0.0), shared(0.0), shared(0.0), shared(0.0)],
                [shared(1.0), shared(0.0), shared(0.0), shared(0.0), shared(0.0)],
                [shared(1.0), shared(0.0), shared(0.0), shared(0.0), shared(0.0)],
            ],
            hard_sync_enabled: Arc::new(AtomicBool::new(false)),
            hard_sync_gen: (0..VOICE_COUNT).map(|_| Arc::new(std::sync::atomic::AtomicU8::new(0))).collect(),
            fm_depth: shared(0.0),
            fm_tap: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            ring_depth: shared(0.0),
            ring_tap: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            noise_vol: shared(0.0),
            cutoff: shared(3000.0),
            resonance: shared(0.3),
            filter_env_amount: shared(0.3),
            fenv_attack: shared(0.01),
            fenv_decay: shared(0.3),
            fenv_sustain: shared(0.0),
            fenv_release: shared(0.2),
            lfo_rate: shared(2.0),
            lfo_depth: shared(0.0),
            lfo_shape: Arc::new(AtomicU8::new(0)),
            lfo_dest: Arc::new(AtomicU8::new(1)),
            lfo_pitch_mult: shared(1.0),
            voice_freq_targets: (0..VOICE_COUNT).map(|_| shared(440.0)).collect(),
            voice_freqs: (0..VOICE_COUNT).map(|_| shared(440.0)).collect(),
            voice_gates: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            effective_cutoff: shared(3000.0),
            adsr_attack: shared(0.01),
            adsr_decay: shared(0.15),
            adsr_sustain: shared(0.7),
            adsr_release: shared(0.4),
            amp_cursors: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            fenv_cursors: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            glide_time: shared(0.0),
            track_vol: shared(1.0),
            shimmer_send: shared(0.0),
            crystal_send: shared(0.0),
        }
    }
}

impl Default for TrackState {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// DSP graph builder for one track
// ---------------------------------------------------------------------------

fn build_track_graph(state: &TrackState, sr: f64) -> Box<dyn AudioUnit + Send> {

    let make_voice = |vi: usize| {
        let vf = &state.voice_freqs[vi];
        let vg = &state.voice_gates[vi];

        let sync_enabled = Arc::clone(&state.hard_sync_enabled);
        let sync_gen = Arc::clone(&state.hard_sync_gen[vi]);

        let vf_lfo = var(vf) * var(&state.lfo_pitch_mult);

        let osc0 = {
            let fm = var(&state.fm_tap[vi]) * var(&state.fm_depth)
                * vf_lfo.clone() * var(&state.osc_freq_mult[0]);
            let c0 = (vf_lfo.clone() * var(&state.osc_freq_mult[0]) * var(&state.osc_unison_detune[0][0]) + fm.clone()
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[0]), state.osc_pulse_width[0].clone(), sr as f32, 0.0 / 5.0,
                    SyncRole::Master { sync_enabled: Arc::clone(&sync_enabled), gen: Arc::clone(&sync_gen) },
                    Some(state.ring_tap[vi].clone()))))
                * var(&state.osc_unison_vol[0][0]);
            let c1 = (vf_lfo.clone() * var(&state.osc_freq_mult[0]) * var(&state.osc_unison_detune[0][1]) + fm.clone()
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[0]), state.osc_pulse_width[0].clone(), sr as f32, 1.0 / 5.0, SyncRole::None, None)))
                * var(&state.osc_unison_vol[0][1]);
            let c2 = (vf_lfo.clone() * var(&state.osc_freq_mult[0]) * var(&state.osc_unison_detune[0][2]) + fm.clone()
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[0]), state.osc_pulse_width[0].clone(), sr as f32, 2.0 / 5.0, SyncRole::None, None)))
                * var(&state.osc_unison_vol[0][2]);
            let c3 = (vf_lfo.clone() * var(&state.osc_freq_mult[0]) * var(&state.osc_unison_detune[0][3]) + fm.clone()
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[0]), state.osc_pulse_width[0].clone(), sr as f32, 3.0 / 5.0, SyncRole::None, None)))
                * var(&state.osc_unison_vol[0][3]);
            let c4 = (vf_lfo.clone() * var(&state.osc_freq_mult[0]) * var(&state.osc_unison_detune[0][4]) + fm
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[0]), state.osc_pulse_width[0].clone(), sr as f32, 4.0 / 5.0, SyncRole::None, None)))
                * var(&state.osc_unison_vol[0][4]);
            (c0 + c1 + c2 + c3 + c4) * var(&state.osc_vol[0])
        };

        let osc1 = {
            let c0 = (vf_lfo.clone() * var(&state.osc_freq_mult[1]) * var(&state.osc_unison_detune[1][0])
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[1]), state.osc_pulse_width[1].clone(), sr as f32, 0.0 / 5.0,
                    SyncRole::Slave { sync_enabled: Arc::clone(&sync_enabled), gen: Arc::clone(&sync_gen), last_gen: 0 },
                    Some(state.fm_tap[vi].clone()))))
                * var(&state.osc_unison_vol[1][0]);
            let c1 = (vf_lfo.clone() * var(&state.osc_freq_mult[1]) * var(&state.osc_unison_detune[1][1])
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[1]), state.osc_pulse_width[1].clone(), sr as f32, 1.0 / 5.0,
                    SyncRole::Slave { sync_enabled: Arc::clone(&sync_enabled), gen: Arc::clone(&sync_gen), last_gen: 0 }, None)))
                * var(&state.osc_unison_vol[1][1]);
            let c2 = (vf_lfo.clone() * var(&state.osc_freq_mult[1]) * var(&state.osc_unison_detune[1][2])
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[1]), state.osc_pulse_width[1].clone(), sr as f32, 2.0 / 5.0,
                    SyncRole::Slave { sync_enabled: Arc::clone(&sync_enabled), gen: Arc::clone(&sync_gen), last_gen: 0 }, None)))
                * var(&state.osc_unison_vol[1][2]);
            let c3 = (vf_lfo.clone() * var(&state.osc_freq_mult[1]) * var(&state.osc_unison_detune[1][3])
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[1]), state.osc_pulse_width[1].clone(), sr as f32, 3.0 / 5.0,
                    SyncRole::Slave { sync_enabled: Arc::clone(&sync_enabled), gen: Arc::clone(&sync_gen), last_gen: 0 }, None)))
                * var(&state.osc_unison_vol[1][3]);
            let c4 = (vf_lfo.clone() * var(&state.osc_freq_mult[1]) * var(&state.osc_unison_detune[1][4])
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[1]), state.osc_pulse_width[1].clone(), sr as f32, 4.0 / 5.0,
                    SyncRole::Slave { sync_enabled: Arc::clone(&sync_enabled), gen: Arc::clone(&sync_gen), last_gen: 0 }, None)))
                * var(&state.osc_unison_vol[1][4]);
            (c0 + c1 + c2 + c3 + c4) * var(&state.osc_vol[1])
        };

        let osc2 = {
            let c0 = (vf_lfo.clone() * var(&state.osc_freq_mult[2]) * var(&state.osc_unison_detune[2][0])
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[2]), state.osc_pulse_width[2].clone(), sr as f32, 0.0 / 5.0, SyncRole::None, None)))
                * var(&state.osc_unison_vol[2][0]);
            let c1 = (vf_lfo.clone() * var(&state.osc_freq_mult[2]) * var(&state.osc_unison_detune[2][1])
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[2]), state.osc_pulse_width[2].clone(), sr as f32, 1.0 / 5.0, SyncRole::None, None)))
                * var(&state.osc_unison_vol[2][1]);
            let c2 = (vf_lfo.clone() * var(&state.osc_freq_mult[2]) * var(&state.osc_unison_detune[2][2])
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[2]), state.osc_pulse_width[2].clone(), sr as f32, 2.0 / 5.0, SyncRole::None, None)))
                * var(&state.osc_unison_vol[2][2]);
            let c3 = (vf_lfo.clone() * var(&state.osc_freq_mult[2]) * var(&state.osc_unison_detune[2][3])
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[2]), state.osc_pulse_width[2].clone(), sr as f32, 3.0 / 5.0, SyncRole::None, None)))
                * var(&state.osc_unison_vol[2][3]);
            let c4 = (vf_lfo.clone() * var(&state.osc_freq_mult[2]) * var(&state.osc_unison_detune[2][4])
                >> An(MultiWaveOsc::with_sync(Arc::clone(&state.osc_wave[2]), state.osc_pulse_width[2].clone(), sr as f32, 4.0 / 5.0, SyncRole::None, None)))
                * var(&state.osc_unison_vol[2][4]);
            (c0 + c1 + c2 + c3 + c4) * var(&state.osc_vol[2])
        };

        let ring = var(&state.ring_tap[vi]) * var(&state.fm_tap[vi]) * var(&state.ring_depth);
        let noise = noise() * var(&state.noise_vol);
        let osc = osc0 + osc1 + osc2 + ring + noise;

        let fenv = var(vg) >> An(LiveAdsr::new(
            state.fenv_attack.clone(), state.fenv_decay.clone(),
            state.fenv_sustain.clone(), state.fenv_release.clone(),
            Some(state.fenv_cursors[vi].clone()), sr as f32,
        ));
        let dyn_cutoff = var(&state.effective_cutoff)
            + fenv * var(&state.filter_env_amount) * dc(12000.0_f32);
        let filtered = (osc | dyn_cutoff | var(&state.resonance)) >> moog();

        let env = var(vg) >> An(LiveAdsr::new(
            state.adsr_attack.clone(), state.adsr_decay.clone(),
            state.adsr_sustain.clone(), state.adsr_release.clone(),
            Some(state.amp_cursors[vi].clone()), sr as f32,
        ));
        filtered * env
    };

    let v0 = make_voice(0);
    let v1 = make_voice(1);
    let v2 = make_voice(2);
    let v3 = make_voice(3);
    let v4 = make_voice(4);
    let v5 = make_voice(5);

    let voice_mix = v0 + v1 + v2 + v3 + v4 + v5;
    let track_out = voice_mix * var(&state.track_vol);
    let mut g: Box<dyn AudioUnit + Send> = Box::new(track_out);
    g.set_sample_rate(sr);
    g.allocate();
    g
}

// ---------------------------------------------------------------------------
// MultiTrackEngine — N tracks + global buses
// ---------------------------------------------------------------------------

/// Generic multi-track engine.
///
/// Holds `TRACK_COUNT` tracks and two placeholder global send buses.
/// The audio callback calls `get_stereo` each sample to get stereo output.
pub struct MultiTrackEngine {
    pub tracks: [TrackState; TRACK_COUNT],
    /// Global shimmer reverb parameters (UI-accessible).
    pub shimmer: ShimmerShared,
    /// Global crystal bus parameters (UI-accessible).
    pub crystal: CrystallizerShared,
    /// Master output volume.
    pub master_vol: Shared,

    /// Per-track arpeggiator config (UI-accessible).
    pub arp_configs:    [ArpShared; TRACK_COUNT],
    /// Per-track scale walker config (UI-accessible).
    pub walker_configs: [ScaleWalkerShared; TRACK_COUNT],

    track_graphs:  Vec<BlockRateAdapter>,
    shimmer_state: ShimmerReverb,
    crystal_state: Crystallizer,
    sr: f64,
    smoothed_freqs: Vec<Vec<f32>>,
}

impl MultiTrackEngine {
    pub fn new(sr: f64) -> Self {
        let tracks: [TrackState; TRACK_COUNT] = std::array::from_fn(|_| TrackState::new());
        let track_graphs: Vec<_> = tracks
            .iter()
            .map(|t| BlockRateAdapter::new(build_track_graph(t, sr)))
            .collect();
        let smoothed_freqs = vec![vec![440.0f32; VOICE_COUNT]; TRACK_COUNT];
        Self {
            tracks,
            shimmer: ShimmerShared::new(),
            crystal: CrystallizerShared::new(),
            master_vol: shared(0.7),
            arp_configs:    std::array::from_fn(|_| ArpShared::new()),
            walker_configs: std::array::from_fn(|_| ScaleWalkerShared::new()),
            track_graphs,
            shimmer_state: ShimmerReverb::new(sr as f32),
            crystal_state: Crystallizer::new(sr as f32),
            sr,
            smoothed_freqs,
        }
    }

    /// Advance glide smoothing for all tracks.
    /// Call once per audio buffer, before calling `get_stereo`.
    pub fn tick_glide(&mut self, frames: usize) {
        let sr_f = self.sr as f32;
        for (ti, track) in self.tracks.iter().enumerate() {
            let glide_time = track.glide_time.value();
            for vi in 0..VOICE_COUNT {
                let target = track.voice_freq_targets[vi].value();
                if glide_time < 0.001 {
                    self.smoothed_freqs[ti][vi] = target;
                } else {
                    let coeff = (-(frames as f32) / (glide_time * sr_f)).exp();
                    self.smoothed_freqs[ti][vi] =
                        coeff * self.smoothed_freqs[ti][vi] + (1.0 - coeff) * target;
                }
                track.voice_freqs[vi].set(self.smoothed_freqs[ti][vi]);
            }
        }
    }

    /// Update LFO state for a single track.
    /// Call once per sample from the audio callback.
    pub fn tick_lfo_sample(&self, ti: usize, lfo_phase: f32) {
        let track = &self.tracks[ti];
        let lfo_depth = track.lfo_depth.value();
        let lfo_shape = track.lfo_shape.load(std::sync::atomic::Ordering::Relaxed);
        let lfo_dest  = track.lfo_dest.load(std::sync::atomic::Ordering::Relaxed);
        let base_cutoff = track.cutoff.value().clamp(80.0, 18000.0);

        let lfo_raw = match lfo_shape {
            1 => if lfo_phase < 0.5 { 4.0*lfo_phase-1.0 } else { 3.0-4.0*lfo_phase },
            2 => 2.0 * lfo_phase - 1.0,
            _ => (lfo_phase * std::f32::consts::TAU).sin(),
        };
        let lfo_out = lfo_raw * lfo_depth;

        match lfo_dest {
            0 => {
                track.lfo_pitch_mult.set(2_f32.powf(lfo_out * 2.0 / 12.0));
                track.effective_cutoff.set(base_cutoff);
            }
            _ => {
                track.lfo_pitch_mult.set(1.0);
                track.effective_cutoff.set(
                    (base_cutoff + lfo_out * base_cutoff * 0.5).clamp(80.0, 18000.0));
            }
        }
    }

    /// Get one stereo sample pair summed from all tracks.
    /// Call `tick_glide` once per buffer and `tick_lfo_sample` once per sample before this.
    #[inline]
    pub fn get_stereo(&mut self) -> (f32, f32) {
        let mut dry_sum = 0.0f32;
        let mut shim_bus = 0.0f32;
        let mut crys_bus = 0.0f32;
        for (ti, graph) in self.track_graphs.iter_mut().enumerate() {
            let (l, _r) = graph.get_stereo();
            dry_sum += l;
            shim_bus += l * self.tracks[ti].shimmer_send.value();
            crys_bus += l * self.tracks[ti].crystal_send.value();
        }

        let shim_mix = self.shimmer.mix.value();
        let shim_wet = if shim_mix > 0.0001 {
            self.shimmer_state.tick(
                shim_bus / TRACK_COUNT as f32,
                self.shimmer.size.value(),
                self.shimmer.damp.value(),
                self.shimmer.shimmer.value(),
                self.shimmer.pitch.load(std::sync::atomic::Ordering::Relaxed),
            )
        } else {
            0.0
        };

        let crys_mix = self.crystal.mix.value();
        let crys_wet = if crys_mix > 0.0001 {
            self.crystal_state.tick(
                crys_bus / TRACK_COUNT as f32,
                self.crystal.grain_ms.value(),
                self.crystal.scatter.value(),
                self.crystal.feedback.value(),
                self.crystal.delay_ms.value(),
                self.crystal.pitch.load(std::sync::atomic::Ordering::Relaxed),
            )
        } else {
            0.0
        };

        let dry = dry_sum / TRACK_COUNT as f32;
        let wet_total = (shim_mix + crys_mix).clamp(0.0, 1.0);
        let mix = dry * (1.0 - wet_total) + shim_mix * shim_wet + crys_mix * crys_wet;
        let out = (mix * self.master_vol.value()).tanh();
        (out, out)
    }
}
