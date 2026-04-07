//! Audio engine: cpal stream + fundsp synthesis.
//!
//! Single unified poly graph: 3 OSCs per voice → filter → amp ADSR.
//! LFO is computed in the callback and modulates effective_cutoff via a Shared.

#![allow(clippy::precedence)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use fundsp::prelude32::*;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8};
use std::sync::Arc;

use crate::envelope::LiveAdsr;
use crate::osc::{MultiWaveOsc, SyncRole};

pub const VOICE_COUNT: usize = 6;

pub struct AudioState {
    // OSC bank — 3 oscillators per voice
    pub osc_wave: [Arc<AtomicU8>; 3], // 0=sine 1=saw 2=square 3=triangle
    pub osc_freq_mult: [Shared; 3],   // octave+detune combined multiplier (1.0 = no change)
    pub osc_vol: [Shared; 3],         // 0.0..1.0 mix level
    pub osc_pulse_width: [Shared; 3], // 0.01..0.99, only used by Square
    // Unison: 5 copies max per OSC slot; inactive copies have vol=0.0
    pub osc_unison_detune: [[Shared; 5]; 3], // freq multiplier per copy (1.0 = no detune)
    pub osc_unison_vol: [[Shared; 5]; 3],    // mix weight per copy (sums to 1.0 when active)
    // Hard sync: OSC 1 → OSC 2. One generation counter per voice.
    // OSC 1 copy 0 increments on phase wrap; OSC 2 copies reset when they see a new generation.
    pub hard_sync_enabled: Arc<AtomicBool>,
    pub hard_sync_gen: Vec<Arc<AtomicU8>>,   // one per voice

    // FM: OSC 2 audio output → OSC 1 frequency input (audio-rate FM).
    // fm_tap[vi] is written by OSC 2 copy 0 each sample; fm_depth scales the deviation.
    // deviation (Hz) = fm_tap × fm_depth × voice_freq × osc1_freq_mult
    pub fm_depth: Shared,          // 0.0 = off, ~1.0 = strong FM
    pub fm_tap: Vec<Shared>,       // one per voice — written by OSC 2 copy 0

    // Ring modulation: OSC 1 × OSC 2 → added to voice mix.
    // ring_tap[vi] is written by OSC 1 copy 0; ring signal = ring_tap × fm_tap × ring_depth.
    // User mutes OSC 1/2 in mixer for pure ring mod sound.
    pub ring_depth: Shared,        // 0.0 = off
    pub ring_tap: Vec<Shared>,     // one per voice — written by OSC 1 copy 0

    // Noise
    pub noise_vol: Shared, // 0.0..1.0

    // Filter
    pub cutoff: Shared,            // base cutoff Hz (80..18000)
    pub resonance: Shared,         // Q (0.5..20)
    pub filter_env_amount: Shared, // 0.0..1.0
    // Filter ADSR
    pub fenv_attack: Shared,
    pub fenv_decay: Shared,
    pub fenv_sustain: Shared,
    pub fenv_release: Shared,

    // LFO
    pub lfo_rate: Shared,         // 0.1..20 Hz
    pub lfo_depth: Shared,        // 0.0..1.0
    pub lfo_shape: Arc<AtomicU8>, // 0=sin 1=tri 2=saw
    pub lfo_dest: Arc<AtomicU8>,  // 0=pitch 1=filter 2=amp
    // Written by callback each buffer; read by graph
    pub lfo_pitch_mult: Shared,   // frequency multiplier (1.0 = no pitch mod)

    // Voice target frequencies — UI writes here; callback smooths to voice_freqs for glide
    pub voice_freq_targets: Vec<Shared>,

    // Amp ADSR
    pub adsr_attack: Shared,
    pub adsr_decay: Shared,
    pub adsr_sustain: Shared,
    pub adsr_release: Shared,

    // ADSR cursors — written by LiveAdsr each sample, read by UI for visualizer
    // Encoding: 0=idle, 1.x=attack, 2.x=decay, 3.0=sustain, 4.x=release (frac=progress)
    pub amp_cursors:  Vec<Shared>, // one per voice
    pub fenv_cursors: Vec<Shared>, // one per voice

    // Glide
    pub glide_time: Shared, // 0.0..0.5 s

    // Master
    pub master_vol: Shared,

    // Polyphonic voice pool
    pub voice_freqs: Vec<Shared>,
    pub voice_gates: Vec<Shared>,

    // Internal: effective cutoff written by callback, read by graph
    pub effective_cutoff: Shared,

    // Oscilloscope
    pub osc_buffer: Arc<std::sync::Mutex<Vec<f32>>>,

    // Latency measurement
    // Buffer size in frames, written by audio callback on first call.
    pub buffer_frames: Arc<AtomicU32>,
    // Sample rate in Hz, written once during stream creation.
    pub sample_rate: Arc<AtomicU32>,
    // Timestamp of the last voice_on call — written by UI, cleared by audio callback.
    // Stored as a Mutex<Option<Instant>> so both sides can access without blocking
    // (callback uses try_lock so it never stalls the audio thread).
    pub note_on_time: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    // Last measured round-trip latency in microseconds, written by audio callback.
    pub last_latency_us: Arc<AtomicU32>,

    // Peak metering (pre-clip level, written by audio callback each buffer)
    pub peak_l: Arc<AtomicU32>, // f32 bits stored as u32
    pub peak_r: Arc<AtomicU32>,

    // Master limiter (envelope-follower, runs in callback before tanh)
    pub limiter_enabled: Arc<AtomicBool>,
    pub limiter_threshold: Shared, // 0.5..1.0

    // FX chain (post-mix, pre-output) — all wet/dry 0.0 = bypass
    pub fx_overdrive_drive: Shared, // 1.0..10.0
    pub fx_overdrive_mix:   Shared, // 0.0..1.0
    pub fx_overdrive_tone:  Shared, // 0.0..1.0 — post-clipper LP (0=dark, 1=bright)
    pub fx_overdrive_asym:  Shared, // 0.0..1.0 — asymmetric bias (0=sym, 1=full asym)
    pub fx_distortion_drive: Shared, // 1.0..20.0
    pub fx_distortion_mix:   Shared,
    pub fx_distortion_tone:  Shared, // 0.0..1.0 — post-clipper LP
    pub fx_distortion_pre:   Shared, // 0.0..1.0 — pre-clipper HP (controls bass going in)
    pub fx_chorus_rate:  Shared, // 0.1..5.0 Hz
    pub fx_chorus_depth: Shared, // 0.0..0.02 (seconds of modulation)
    pub fx_chorus_mix:   Shared,
    pub fx_delay_time:     Shared, // 0.0..1.0 s
    pub fx_delay_feedback: Shared, // 0.0..0.95
    pub fx_delay_mix:      Shared,
    pub fx_reverb_size:    Shared, // 0.0..1.0 (room size)
    pub fx_reverb_damp:    Shared, // 0.0..1.0 (high-freq damping)
    pub fx_reverb_mix:     Shared,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            osc_wave: [
                Arc::new(AtomicU8::new(1)), // OSC1: saw — needed for filter to have audible effect
                Arc::new(AtomicU8::new(0)),
                Arc::new(AtomicU8::new(0)),
            ],
            osc_freq_mult: [shared(1.0), shared(1.0), shared(1.0)],
            osc_vol: [shared(0.4), shared(0.3), shared(0.0)],
            osc_pulse_width: [shared(0.5), shared(0.5), shared(0.5)],
            // Unison off by default: copy 0 at full weight, copies 1-4 silent
            osc_unison_detune: [
                [
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                ],
                [
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                ],
                [
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                ],
            ],
            osc_unison_vol: [
                [
                    shared(1.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                ],
                [
                    shared(1.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                ],
                [
                    shared(1.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                ],
            ],
            noise_vol: shared(0.0),
            cutoff: shared(3000.0),
            resonance: shared(0.3),
            filter_env_amount: shared(0.3),
            fenv_attack: shared(0.01),
            fenv_decay: shared(0.3),
            fenv_sustain: shared(0.0), // matches UI default fenv_adsr[2]
            fenv_release: shared(0.2),
            lfo_rate: shared(2.0),
            lfo_depth: shared(0.0),
            lfo_shape: Arc::new(AtomicU8::new(0)), // sine
            lfo_dest: Arc::new(AtomicU8::new(1)),  // filter
            lfo_pitch_mult: shared(1.0),
            voice_freq_targets: (0..VOICE_COUNT).map(|_| shared(440.0)).collect(),
            adsr_attack: shared(0.01),
            adsr_decay: shared(0.15),
            adsr_sustain: shared(0.7),
            adsr_release: shared(0.4),
            amp_cursors:  (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            fenv_cursors: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            glide_time: shared(0.0),
            master_vol: shared(0.4),
            hard_sync_enabled: Arc::new(AtomicBool::new(false)),
            hard_sync_gen: (0..VOICE_COUNT).map(|_| Arc::new(AtomicU8::new(0))).collect(),
            fm_depth: shared(0.0),
            fm_tap: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            ring_depth: shared(0.0),
            ring_tap: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            voice_freqs: (0..VOICE_COUNT).map(|_| shared(440.0)).collect(),
            voice_gates: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            effective_cutoff: shared(3000.0),
            osc_buffer: Arc::new(std::sync::Mutex::new(vec![0.0f32; 1024])),
            buffer_frames: Arc::new(AtomicU32::new(0)),
            sample_rate: Arc::new(AtomicU32::new(0)),
            note_on_time: Arc::new(std::sync::Mutex::new(None)),
            last_latency_us: Arc::new(AtomicU32::new(0)),
            peak_l: Arc::new(AtomicU32::new(0)),
            peak_r: Arc::new(AtomicU32::new(0)),
            limiter_enabled: Arc::new(AtomicBool::new(true)),
            limiter_threshold: shared(0.95),
            fx_overdrive_drive: shared(3.0),
            fx_overdrive_mix:   shared(0.0),
            fx_overdrive_tone:  shared(0.8),
            fx_overdrive_asym:  shared(0.0),
            fx_distortion_drive: shared(8.0),
            fx_distortion_mix:   shared(0.0),
            fx_distortion_tone:  shared(0.8),
            fx_distortion_pre:   shared(0.0),
            fx_chorus_rate:  shared(0.8),
            fx_chorus_depth: shared(0.008),
            fx_chorus_mix:   shared(0.0),
            fx_delay_time:     shared(0.35),
            fx_delay_feedback: shared(0.4),
            fx_delay_mix:      shared(0.0),
            fx_reverb_size:    shared(0.6),
            fx_reverb_damp:    shared(0.5),
            fx_reverb_mix:     shared(0.0),
        }
    }
}

impl Default for AudioState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AudioEngine {
    pub state: Arc<AudioState>,
    _stream: Stream,
}

impl AudioEngine {
    pub fn new() -> anyhow::Result<Self> {
        let state = Arc::new(AudioState::new());
        let stream = build_stream(Arc::clone(&state))?;
        stream.play()?;
        Ok(Self {
            state,
            _stream: stream,
        })
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// DSP graph builder
// ---------------------------------------------------------------------------

/// Build the unified 6-voice poly graph.
/// Each voice: 3 OSCs + noise → lowpass(effective_cutoff) → amp ADSR
fn build_synth_graph(state: &AudioState, sr: f64) -> Box<dyn AudioUnit + Send> {
    let scale = 1.0 / VOICE_COUNT as f32;

    let make_voice = |vi: usize| {
        let vf = &state.voice_freqs[vi];
        let vg = &state.voice_gates[vi];

        // Each OSC slot: 5 unison copies, inactive ones have vol=0.0.
        // Phases spread evenly across [0, 1) to avoid phase coherence and beating artifacts.
        // Hard sync: OSC 0 copy 0 is master, all OSC 1 copies are slaves.
        let sync_enabled = Arc::clone(&state.hard_sync_enabled);
        let sync_gen     = Arc::clone(&state.hard_sync_gen[vi]);

        // LFO pitch modulation: applied at the voice frequency level so all OSCs track together.
        // lfo_pitch_mult is 1.0 when LFO dest != pitch.
        let vf_lfo = var(vf) * var(&state.lfo_pitch_mult);

        // FM: frequency deviation added to OSC 1's input (pitch-tracking).
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

        // Ring mod: OSC1 × OSC2 added to the mix.
        let ring = var(&state.ring_tap[vi]) * var(&state.fm_tap[vi]) * var(&state.ring_depth);
        let noise = noise() * var(&state.noise_vol);
        let osc = osc0 + osc1 + osc2 + ring + noise;

        // Moog lowpass filter with per-voice filter ADSR (fully live-parametric).
        let fenv = var(vg) >> An(LiveAdsr::new(
            state.fenv_attack.clone(), state.fenv_decay.clone(),
            state.fenv_sustain.clone(), state.fenv_release.clone(),
            Some(state.fenv_cursors[vi].clone()), sr as f32,
        ));
        // Filter env sweep: additive in Hz with a fixed max range so the sweep covers
        // musically useful territory regardless of base cutoff.
        // env_amount=1.0 adds up to 12 kHz above base (≈2–3 octaves); at 0.3 it adds ~3.6 kHz.
        let dyn_cutoff = var(&state.effective_cutoff)
            + fenv * var(&state.filter_env_amount) * dc(12000.0_f32);
        let filtered = (osc | dyn_cutoff | var(&state.resonance)) >> moog();

        // Amp ADSR envelope (fully live-parametric).
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

    let voice_mix = (v0 + v1 + v2 + v3 + v4 + v5) * scale;

    let chain = voice_mix >> An(FxChain::new(state, sr as f32));

    let mut g: Box<dyn AudioUnit + Send> =
        Box::new(chain * var(&state.master_vol) >> pan(0.0));
    g.set_sample_rate(sr);
    g.allocate();
    g
}

// ---------------------------------------------------------------------------
// FX chain — custom AudioNode (tick-based, plain f32)
// ---------------------------------------------------------------------------

/// Schroeder reverb: 4 parallel comb filters → 2 serial allpass filters.
#[derive(Clone)]
struct ReverbState {
    comb_buf:  [Vec<f32>; 4],
    comb_pos:  [usize;    4],
    comb_feed: [f32;      4], // one-pole LP state per comb (damping)
    ap_buf:    [Vec<f32>; 2],
    ap_pos:    [usize;    2],
}

impl ReverbState {
    fn new(sr: f32) -> Self {
        let scale = sr / 44100.0;
        let comb_delays: [usize; 4] = [1557, 1617, 1491, 1422];
        let ap_delays:   [usize; 2] = [225, 556];
        Self {
            comb_buf:  comb_delays.map(|d| vec![0.0; std::cmp::max((d as f32 * scale) as usize, 1)]),
            comb_pos:  [0; 4],
            comb_feed: [0.0; 4],
            ap_buf:    ap_delays.map(|d| vec![0.0; std::cmp::max((d as f32 * scale) as usize, 1)]),
            ap_pos:    [0; 2],
        }
    }

    fn tick(&mut self, input: f32, room: f32, damp: f32) -> f32 {
        let feed = 0.7 + room * 0.28; // 0.7..0.98 decay
        let d    = damp * 0.4;        // HF rolloff coefficient

        let mut out = 0.0f32;
        for i in 0..4 {
            let len = self.comb_buf[i].len();
            let pos = self.comb_pos[i];
            let delayed = self.comb_buf[i][pos];
            self.comb_feed[i] = delayed * (1.0 - d) + self.comb_feed[i] * d;
            self.comb_buf[i][pos] = input + self.comb_feed[i] * feed;
            self.comb_pos[i] = (pos + 1) % len;
            out += delayed;
        }
        out *= 0.25;

        for i in 0..2 {
            let len = self.ap_buf[i].len();
            let pos = self.ap_pos[i];
            let buf = self.ap_buf[i][pos];
            let v = out + buf * 0.5;
            self.ap_buf[i][pos] = v;
            self.ap_pos[i] = (pos + 1) % len;
            out = buf - v * 0.5;
        }
        out
    }
}

/// One-pole exponential smoother wrapping a `Shared` parameter.
/// Prevents audio artifacts (clicks/pops) when a parameter is changed live.
/// `tau_s` is the smoothing time constant in seconds (63% convergence time).
#[derive(Clone)]
struct SmoothedParam {
    shared:  Shared,
    current: f32,
    coeff:   f32, // recomputed on sample-rate change
    tau_s:   f32,
}

impl SmoothedParam {
    fn new(shared: Shared, tau_s: f32, sr: f32) -> Self {
        let current = shared.value() as f32;
        let coeff   = (-1.0_f32 / (tau_s * sr)).exp();
        Self { shared, current, coeff, tau_s }
    }

    fn set_sample_rate(&mut self, sr: f32) {
        self.coeff = (-1.0_f32 / (self.tau_s * sr)).exp();
    }

    fn reset(&mut self) {
        self.current = self.shared.value() as f32;
    }

    #[inline]
    fn next(&mut self) -> f32 {
        let target   = self.shared.value() as f32;
        self.current = target + self.coeff * (self.current - target);
        self.current
    }
}

/// All five effects in a single sample-accurate node.
/// Each effect blends dry/wet: out = dry + mix*(wet−dry). mix=0 → bypass.
#[derive(Clone)]
struct FxChain {
    // Plain Shared — no smoothing needed (tone/asym affect filter coefficients gradually)
    od_tone:      Shared,
    od_asym:      Shared,
    dist_tone:    Shared,
    dist_pre:     Shared,
    // Smoothed — prevents zipper noise when moving sliders live
    od_drive:     SmoothedParam,
    dist_drive:   SmoothedParam,
    cho_rate:     Shared,
    cho_depth:    Shared,
    del_feedback: Shared,
    // Smoothed params — use SmoothedParam to prevent clicks/artifacts
    od_mix:   SmoothedParam, // 5 ms — pop-free toggle
    dist_mix: SmoothedParam, // 5 ms
    cho_mix:  SmoothedParam, // 5 ms
    del_time: SmoothedParam, // 20 ms — prevents pitch-jump noise on slider move
    del_mix:  SmoothedParam, // 5 ms
    rev_size: SmoothedParam, // 50 ms — smooth reverb tail transitions
    rev_damp: SmoothedParam, // 50 ms
    rev_mix:  SmoothedParam, // 5 ms
    // Internal state
    cho_phase:   f32,
    del_buf:     Vec<f32>,
    del_pos:     usize,
    od_tone_z:   f32, // one-pole LP state — OD post-filter
    dist_tone_z: f32, // one-pole LP state — DIST post-filter
    dist_pre_z:  f32, // one-pole LP state for HP = input - LP
    rev:         ReverbState,
    sr:          f32,
}

impl FxChain {
    fn new(state: &AudioState, sr: f32) -> Self {
        const MIX_TAU:  f32 = 0.005; // 5 ms — pop-free mix/toggle transitions
        const DEL_TAU:  f32 = 0.020; // 20 ms — delay time, prevents pitch-jump noise
        const REV_TAU:  f32 = 0.050; // 50 ms — reverb room/damp, smooth tail changes
        let buf_len = (sr * 1.1) as usize;
        Self {
            od_tone:    state.fx_overdrive_tone.clone(),
            od_asym:    state.fx_overdrive_asym.clone(),
            dist_tone:  state.fx_distortion_tone.clone(),
            dist_pre:   state.fx_distortion_pre.clone(),
            od_drive:   SmoothedParam::new(state.fx_overdrive_drive.clone(),  DEL_TAU, sr),
            dist_drive: SmoothedParam::new(state.fx_distortion_drive.clone(), DEL_TAU, sr),
            cho_rate:     state.fx_chorus_rate.clone(),
            cho_depth:    state.fx_chorus_depth.clone(),
            del_feedback: state.fx_delay_feedback.clone(),
            od_mix:   SmoothedParam::new(state.fx_overdrive_mix.clone(),  MIX_TAU, sr),
            dist_mix: SmoothedParam::new(state.fx_distortion_mix.clone(), MIX_TAU, sr),
            cho_mix:  SmoothedParam::new(state.fx_chorus_mix.clone(),     MIX_TAU, sr),
            del_time: SmoothedParam::new(state.fx_delay_time.clone(),     DEL_TAU, sr),
            del_mix:  SmoothedParam::new(state.fx_delay_mix.clone(),      MIX_TAU, sr),
            rev_size: SmoothedParam::new(state.fx_reverb_size.clone(),    REV_TAU, sr),
            rev_damp: SmoothedParam::new(state.fx_reverb_damp.clone(),    REV_TAU, sr),
            rev_mix:  SmoothedParam::new(state.fx_reverb_mix.clone(),     MIX_TAU, sr),
            cho_phase:   0.0,
            del_buf:     vec![0.0f32; buf_len],
            del_pos:     0,
            od_tone_z:   0.0,
            dist_tone_z: 0.0,
            dist_pre_z:  0.0,
            rev:         ReverbState::new(sr),
            sr,
        }
    }
}

impl AudioNode for FxChain {
    const ID: u64 = 0x7468655F_78636861; // "the_xcha"
    type Inputs  = U1;
    type Outputs = U1;

    fn reset(&mut self) {
        self.cho_phase = 0.0;
        self.del_buf.fill(0.0);
        self.del_pos = 0;
        self.od_drive.reset(); self.od_mix.reset();
        self.dist_drive.reset(); self.dist_mix.reset();
        self.cho_mix.reset();  self.del_time.reset(); self.del_mix.reset();
        self.rev_size.reset(); self.rev_damp.reset(); self.rev_mix.reset();
        self.rev = ReverbState::new(self.sr);
    }

    fn set_sample_rate(&mut self, sr: f64) {
        self.sr = sr as f32;
        let buf_len = (self.sr * 1.1) as usize;
        self.del_buf = vec![0.0f32; buf_len];
        self.del_pos = 0;
        self.od_drive.set_sample_rate(self.sr); self.od_mix.set_sample_rate(self.sr);
        self.dist_drive.set_sample_rate(self.sr); self.dist_mix.set_sample_rate(self.sr);
        self.cho_mix.set_sample_rate(self.sr);  self.del_time.set_sample_rate(self.sr);
        self.del_mix.set_sample_rate(self.sr);  self.rev_size.set_sample_rate(self.sr);
        self.rev_damp.set_sample_rate(self.sr); self.rev_mix.set_sample_rate(self.sr);
        self.rev = ReverbState::new(self.sr);
    }

    #[inline]
    fn tick(&mut self, input: &Frame<f32, U1>) -> Frame<f32, U1> {
        let dry = input[0];

        // ── Overdrive (tanh soft clip) ──────────────────────────────────────
        let od_drive = self.od_drive.next().max(1.0);
        let od_mix   = self.od_mix.next();
        let od_tone  = self.od_tone.value() as f32;
        let od_asym  = self.od_asym.value() as f32;
        let od_wet = if od_mix > 0.0001 {
            // Asymmetric bias: scaled to match the driven signal level so it
            // actually shifts the clipping point. bias up to ±2.0 in tanh space.
            let driven_signal = dry * od_drive * 5.0;
            let bias    = od_asym * 2.0;
            let clipped = (driven_signal + bias).tanh() - bias.tanh();
            // Post-clipper tone LP: 0 = 400 Hz (dark/muffled), 1 = 8 kHz (bright).
            // Narrower range than before so movement is always audible.
            let fc       = 400.0_f32 * (8000.0_f32 / 400.0).powf(od_tone);
            let lp_coeff = (-std::f32::consts::TAU * fc / self.sr).exp();
            self.od_tone_z = (1.0 - lp_coeff) * clipped + lp_coeff * self.od_tone_z;
            self.od_tone_z
        } else { dry };
        let s1 = dry + od_mix * (od_wet - dry);

        // ── Distortion (hard clip) ──────────────────────────────────────────
        let dist_drive = self.dist_drive.next().max(1.0);
        let dist_mix   = self.dist_mix.next();
        let dist_tone  = self.dist_tone.value() as f32;
        let dist_pre   = self.dist_pre.value() as f32;
        let dist_wet = if dist_mix > 0.0001 {
            // Pre-clipper HP: removes bass before clipping to avoid mud.
            // Maps 0→1 to 20 Hz → 800 Hz. HP = input - LP(input).
            let hp_fc     = 20.0_f32 + dist_pre * 780.0;
            let hp_coeff  = (-std::f32::consts::TAU * hp_fc / self.sr).exp();
            self.dist_pre_z = (1.0 - hp_coeff) * s1 + hp_coeff * self.dist_pre_z;
            let hp_out    = s1 - self.dist_pre_z;
            // Hard clip
            let clipped   = (hp_out * dist_drive * 10.0).clamp(-1.0, 1.0);
            // Post-clipper tone LP: rolls off harsh high harmonics
            let fc        = 400.0_f32 * (18000.0_f32 / 400.0).powf(dist_tone);
            let lp_coeff  = (-std::f32::consts::TAU * fc / self.sr).exp();
            self.dist_tone_z = (1.0 - lp_coeff) * clipped + lp_coeff * self.dist_tone_z;
            self.dist_tone_z
        } else { s1 };
        let s2 = s1 + dist_mix * (dist_wet - s1);

        // ── Chorus (LFO-modulated short delay) ─────────────────────────────
        let cho_mix = self.cho_mix.next();
        let buf_len = self.del_buf.len();
        self.del_buf[self.del_pos] = s2;

        let cho_wet = if cho_mix > 0.0001 {
            let rate  = self.cho_rate.value() as f32;
            let depth = self.cho_depth.value() as f32;
            self.cho_phase = (self.cho_phase + rate / self.sr).fract();
            let lfo = (self.cho_phase * std::f32::consts::TAU).sin();
            let delay_smp = ((0.01 + depth * lfo) * self.sr).max(0.0);
            let read = (self.del_pos as f32 - delay_smp).rem_euclid(buf_len as f32);
            let i0 = read as usize % buf_len;
            let i1 = (i0 + 1) % buf_len;
            self.del_buf[i0] * (1.0 - read.fract()) + self.del_buf[i1] * read.fract()
        } else { s2 };
        let s3 = s2 + cho_mix * (cho_wet - s2);

        // ── Delay ──────────────────────────────────────────────────────────
        let del_mix      = self.del_mix.next();
        let del_time     = self.del_time.next(); // smoothed — prevents pitch-jump noise
        let del_feedback = self.del_feedback.value() as f32;
        let del_feedback = del_feedback.clamp(0.0, 0.95);
        let del_wet = if del_mix > 0.0001 {
            let delay_smp = (del_time * self.sr).clamp(1.0, (buf_len - 2) as f32);
            let read_f    = (self.del_pos as f32 - delay_smp).rem_euclid(buf_len as f32);
            let i0 = read_f as usize % buf_len;
            let i1 = (i0 + 1) % buf_len;
            let delayed = self.del_buf[i0] * (1.0 - read_f.fract()) + self.del_buf[i1] * read_f.fract();
            self.del_buf[self.del_pos] = s3 + delayed * del_feedback;
            delayed
        } else { s3 };
        let s4 = s3 + del_mix * (del_wet - s3);

        self.del_pos = (self.del_pos + 1) % buf_len;

        // ── Reverb ─────────────────────────────────────────────────────────
        let rev_mix  = self.rev_mix.next();
        let rev_size = self.rev_size.next(); // smoothed — prevents tail surge on size change
        let rev_damp = self.rev_damp.next(); // smoothed — prevents tail surge on damp change
        let rev_wet = if rev_mix > 0.0001 {
            self.rev.tick(s4, rev_size, rev_damp)
        } else { s4 };
        let s5 = s4 + rev_mix * (rev_wet - s4);

        Frame::from([s5])
    }
}

// ---------------------------------------------------------------------------
// cpal stream
// ---------------------------------------------------------------------------

fn build_stream(state: Arc<AudioState>) -> anyhow::Result<Stream> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No output device"))?;
    let config = device.default_output_config()?;
    let sr = config.sample_rate().0 as f64;

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => make_stream::<f32>(&device, &config.into(), state, sr)?,
        cpal::SampleFormat::I16 => make_stream::<i16>(&device, &config.into(), state, sr)?,
        cpal::SampleFormat::U16 => make_stream::<u16>(&device, &config.into(), state, sr)?,
        _ => anyhow::bail!("Unsupported sample format"),
    };
    Ok(stream)
}

fn make_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: Arc<AudioState>,
    sr: f64,
) -> anyhow::Result<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;

    state
        .sample_rate
        .store(sr as u32, std::sync::atomic::Ordering::Relaxed);

    // Fundsp best practice for callback efficiency: run the graph through a
    // block-rate adapter instead of raw sample-by-sample graph traversal.
    let mut graph = BlockRateAdapter::new(build_synth_graph(&state, sr));

    let mut osc_idx: usize = 0;
    let mut buffer_size_captured = false;

    // Limiter envelope state (lives across callbacks)
    let mut env_l: f32 = 0.0;
    let mut env_r: f32 = 0.0;

    // LFO phase accumulator (0..1, advances per buffer)
    let mut lfo_phase: f32 = 0.0;

    // Per-voice smoothed frequencies for glide (callback writes to voice_freqs from these)
    let mut smoothed_freqs: Vec<f32> = vec![440.0; VOICE_COUNT];
    let attack_coeff = (-1.0_f64 / (0.0001 * sr)).exp() as f32; // ~0.1ms attack
    let release_coeff = (-1.0_f64 / (0.05 * sr)).exp() as f32; // ~50ms release

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            // Capture actual buffer size on first callback (cpal may use Default buffer size
            // which is only known at runtime).
            if !buffer_size_captured {
                let frames = (data.len() / channels) as u32;
                state
                    .buffer_frames
                    .store(frames, std::sync::atomic::Ordering::Relaxed);
                buffer_size_captured = true;
            }

            // Read per-buffer params once (cheap; avoids repeated atomic loads per sample)
            let frames = data.len() / channels;
            let sr_f = sr as f32;
            let lfo_rate  = state.lfo_rate.value();
            let lfo_depth = state.lfo_depth.value();
            let lfo_shape = state.lfo_shape.load(std::sync::atomic::Ordering::Relaxed);
            let lfo_dest  = state.lfo_dest.load(std::sync::atomic::Ordering::Relaxed);
            let lfo_dt    = lfo_rate / sr_f; // phase increment per sample
            let base_cutoff = state.cutoff.value().clamp(80.0, 18000.0);

            // --- Glide: smooth voice_freq_targets → voice_freqs once per buffer ---
            let glide_time = state.glide_time.value();
            for vi in 0..VOICE_COUNT {
                let target = state.voice_freq_targets[vi].value();
                if glide_time < 0.001 {
                    smoothed_freqs[vi] = target;
                } else {
                    let coeff = (-(frames as f32) / (glide_time * sr_f)).exp();
                    smoothed_freqs[vi] = coeff * smoothed_freqs[vi] + (1.0 - coeff) * target;
                }
                state.voice_freqs[vi].set(smoothed_freqs[vi]);
            }

            // Real-time latency measurement: if a note_on timestamp is pending,
            // consume it now and record how long it took to reach this callback.
            // try_lock ensures we never block the audio thread.
            if let Ok(mut guard) = state.note_on_time.try_lock() {
                if let Some(t) = guard.take() {
                    let us = t.elapsed().as_micros() as u32;
                    state
                        .last_latency_us
                        .store(us, std::sync::atomic::Ordering::Relaxed);
                }
            }

            // Try to lock oscilloscope buffer once per callback (instead of once per sample).
            let mut scope_buf = state.osc_buffer.try_lock().ok();

            let limiter_on = state
                .limiter_enabled
                .load(std::sync::atomic::Ordering::Relaxed);
            let threshold = state.limiter_threshold.value();
            let mut peak_l_local: f32 = 0.0;
            let mut peak_r_local: f32 = 0.0;

            for (frame_i, frame) in data.chunks_mut(channels).enumerate() {
                // --- LFO: advance phase and write Shared params every sample ---
                lfo_phase += lfo_dt;
                if lfo_phase >= 1.0 { lfo_phase -= 1.0; }
                let lfo_raw = match lfo_shape {
                    1 => if lfo_phase < 0.5 { 4.0*lfo_phase-1.0 } else { 3.0-4.0*lfo_phase },
                    2 => 2.0 * lfo_phase - 1.0,
                    _ => (lfo_phase * std::f32::consts::TAU).sin(),
                };
                let lfo_out = lfo_raw * lfo_depth;
                // lfo_amp is applied directly to output samples below (not via graph)
                // so it is truly sample-accurate and bypasses BlockRateAdapter quantisation.
                let lfo_amp = match lfo_dest {
                    2 => 1.0 - lfo_depth * (1.0 - lfo_raw) * 0.5, // tremolo: 1-depth … 1.0
                    _ => 1.0,
                };
                match lfo_dest {
                    0 => {
                        state.lfo_pitch_mult.set(2_f32.powf(lfo_out * 2.0 / 12.0));
                        state.effective_cutoff.set(base_cutoff);
                    }
                    2 => {
                        state.lfo_pitch_mult.set(1.0);
                        state.effective_cutoff.set(base_cutoff);
                    }
                    _ => {
                        state.lfo_pitch_mult.set(1.0);
                        state.effective_cutoff.set(
                            (base_cutoff + lfo_out * base_cutoff * 0.5).clamp(80.0, 18000.0));
                    }
                }

                let (mut raw_l, mut raw_r) = graph.get_stereo();

                // Peak metering: track pre-clip level
                let abs_l = raw_l.abs();
                let abs_r = raw_r.abs();
                if abs_l > peak_l_local {
                    peak_l_local = abs_l;
                }
                if abs_r > peak_r_local {
                    peak_r_local = abs_r;
                }

                // Optional envelope-follower limiter (before soft clip)
                if limiter_on {
                    // Fast attack, slow release envelope follower
                    env_l = if abs_l > env_l {
                        attack_coeff * env_l + (1.0 - attack_coeff) * abs_l
                    } else {
                        release_coeff * env_l + (1.0 - release_coeff) * abs_l
                    };
                    env_r = if abs_r > env_r {
                        attack_coeff * env_r + (1.0 - attack_coeff) * abs_r
                    } else {
                        release_coeff * env_r + (1.0 - release_coeff) * abs_r
                    };

                    if env_l > threshold {
                        raw_l *= threshold / env_l;
                    }
                    if env_r > threshold {
                        raw_r *= threshold / env_r;
                    }
                }

                // Gentle soft clip for occasional overshoots.
                // Apply tremolo after limiter so the limiter doesn't fight the modulation.
                let l = raw_l.tanh() * lfo_amp;
                let r_out = raw_r.tanh() * lfo_amp;

                if let Some(buf) = scope_buf.as_mut() {
                    // Downsample scope writes to reduce callback pressure.
                    if frame_i & 3 == 0 {
                        let len = buf.len();
                        buf[osc_idx % len] = l;
                        osc_idx = osc_idx.wrapping_add(1);
                    }
                }

                let left = T::from_sample(l);
                let right = T::from_sample(r_out);
                for (i, smp) in frame.iter_mut().enumerate() {
                    *smp = if i & 1 == 0 { left } else { right };
                }
            }

            // Write peak levels for UI metering
            state
                .peak_l
                .store(peak_l_local.to_bits(), std::sync::atomic::Ordering::Relaxed);
            state
                .peak_r
                .store(peak_r_local.to_bits(), std::sync::atomic::Ordering::Relaxed);
        },
        |err| eprintln!("audio error: {err}"),
        None,
    )?;

    Ok(stream)
}
