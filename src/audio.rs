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
    pub lfo_amp_mult: Shared,     // amplitude multiplier (1.0 = no amp mod)

    // Voice target frequencies — UI writes here; callback smooths to voice_freqs for glide
    pub voice_freq_targets: Vec<Shared>,

    // Amp ADSR
    pub adsr_attack: Shared,
    pub adsr_decay: Shared,
    pub adsr_sustain: Shared,
    pub adsr_release: Shared,

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
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            osc_wave: [
                Arc::new(AtomicU8::new(0)),
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
            fenv_sustain: shared(0.6),
            fenv_release: shared(0.2),
            lfo_rate: shared(2.0),
            lfo_depth: shared(0.0),
            lfo_shape: Arc::new(AtomicU8::new(0)), // sine
            lfo_dest: Arc::new(AtomicU8::new(1)),  // filter
            lfo_pitch_mult: shared(1.0),
            lfo_amp_mult: shared(1.0),
            voice_freq_targets: (0..VOICE_COUNT).map(|_| shared(440.0)).collect(),
            adsr_attack: shared(0.01),
            adsr_decay: shared(0.15),
            adsr_sustain: shared(0.7),
            adsr_release: shared(0.4),
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
    let a  = state.adsr_attack.value();
    let d  = state.adsr_decay.value();
    let s  = state.adsr_sustain.value();
    let r  = state.adsr_release.value();
    let fa = state.fenv_attack.value();
    let fd = state.fenv_decay.value();
    let fs = state.fenv_sustain.value();
    let fr = state.fenv_release.value();
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
        let osc = osc0 + osc1 + osc2 + ring;

        // Moog lowpass filter with per-voice filter ADSR.
        // dyn_cutoff = effective_cutoff (base + LFO) + fenv × env_amount × base_cutoff.
        // Note: fa/fd/fs/fr are baked in at graph build time (same as amp ADSR).
        let fenv = var(vg) >> adsr_live(fa, fd, fs, fr);
        let dyn_cutoff = var(&state.effective_cutoff)
            + fenv * var(&state.filter_env_amount) * var(&state.cutoff);
        let filtered = (osc | dyn_cutoff | var(&state.resonance)) >> moog();

        // Amp ADSR × LFO amplitude modulation (lfo_amp_mult = 1.0 when LFO dest != amp).
        let env = var(vg) >> adsr_live(a, d, s, r);
        filtered * env * var(&state.lfo_amp_mult)
    };

    let v0 = make_voice(0);
    let v1 = make_voice(1);
    let v2 = make_voice(2);
    let v3 = make_voice(3);
    let v4 = make_voice(4);
    let v5 = make_voice(5);

    let mut g: Box<dyn AudioUnit + Send> =
        Box::new((v0 + v1 + v2 + v3 + v4 + v5) * var(&state.master_vol) * scale >> pan(0.0));
    g.set_sample_rate(sr);
    g.allocate();
    g
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

            // --- LFO ---
            let frames = data.len() / channels;
            let sr_f = sr as f32;
            let lfo_rate  = state.lfo_rate.value();
            let lfo_depth = state.lfo_depth.value();
            let lfo_shape = state.lfo_shape.load(std::sync::atomic::Ordering::Relaxed);
            let lfo_dest  = state.lfo_dest.load(std::sync::atomic::Ordering::Relaxed);
            lfo_phase += lfo_rate * frames as f32 / sr_f;
            while lfo_phase >= 1.0 { lfo_phase -= 1.0; }
            let lfo_raw = match lfo_shape {
                1 => if lfo_phase < 0.5 { 4.0*lfo_phase-1.0 } else { 3.0-4.0*lfo_phase }, // tri
                2 => 2.0 * lfo_phase - 1.0,                                                  // saw
                _ => (lfo_phase * std::f32::consts::TAU).sin(),                              // sin
            };
            let lfo_out = lfo_raw * lfo_depth;
            let base_cutoff = state.cutoff.value().clamp(80.0, 18000.0);
            match lfo_dest {
                0 => { // pitch: ±2 semitones at depth=1
                    state.lfo_pitch_mult.set(2_f32.powf(lfo_out * 2.0 / 12.0));
                    state.lfo_amp_mult.set(1.0);
                    state.effective_cutoff.set(base_cutoff);
                }
                2 => { // amp: tremolo
                    state.lfo_pitch_mult.set(1.0);
                    state.lfo_amp_mult.set((1.0 + lfo_out).max(0.0));
                    state.effective_cutoff.set(base_cutoff);
                }
                _ => { // filter (default=1)
                    state.lfo_pitch_mult.set(1.0);
                    state.lfo_amp_mult.set(1.0);
                    state.effective_cutoff.set(
                        (base_cutoff + lfo_out * base_cutoff * 0.5).clamp(80.0, 18000.0));
                }
            }

            // --- Glide: smooth voice_freq_targets → voice_freqs ---
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
                let l = raw_l.tanh();
                let r_out = raw_r.tanh();

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
