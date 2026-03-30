//! Audio engine: cpal stream + fundsp synthesis.
//!
//! Single unified poly graph: 3 OSCs per voice → filter → amp ADSR.
//! LFO is computed in the callback and modulates effective_cutoff via a Shared.

#![allow(clippy::precedence)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use fundsp::prelude32::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU8};

use crate::osc::MultiWaveOsc;

pub const VOICE_COUNT: usize = 6;

pub struct AudioState {
    // OSC bank — 3 oscillators per voice
    pub osc_wave: [Arc<AtomicU8>; 3],  // 0=sine 1=saw 2=square 3=triangle
    pub osc_freq_mult: [Shared; 3],       // octave+detune combined multiplier (1.0 = no change)
    pub osc_vol:    [Shared; 3],          // 0.0..1.0 mix level

    // Noise
    pub noise_vol:  Shared,               // 0.0..1.0

    // Filter
    pub cutoff:     Shared,               // base cutoff Hz (80..18000)
    pub resonance:  Shared,               // Q (0.5..20)
    pub filter_env_amount: Shared,        // 0.0..1.0
    // Filter ADSR
    pub fenv_attack:  Shared,
    pub fenv_decay:   Shared,
    pub fenv_sustain: Shared,
    pub fenv_release: Shared,

    // LFO
    pub lfo_rate:   Shared,               // 0.1..20 Hz
    pub lfo_depth:  Shared,               // 0.0..1.0
    pub lfo_dest:   Arc<AtomicU8>,        // 0=pitch 1=filter 2=amp

    // Amp ADSR
    pub adsr_attack:  Shared,
    pub adsr_decay:   Shared,
    pub adsr_sustain: Shared,
    pub adsr_release: Shared,

    // Glide
    pub glide_time: Shared,               // 0.0..0.5 s

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
            osc_vol:    [shared(0.65), shared(0.5), shared(0.0)],
            noise_vol:  shared(0.0),
            cutoff:     shared(3000.0),
            resonance:  shared(1.0),
            filter_env_amount: shared(0.3),
            fenv_attack:  shared(0.01),
            fenv_decay:   shared(0.3),
            fenv_sustain: shared(0.6),
            fenv_release: shared(0.2),
            lfo_rate:   shared(2.0),
            lfo_depth:  shared(0.0),
            lfo_dest:   Arc::new(AtomicU8::new(1)), // filter
            adsr_attack:  shared(0.01),
            adsr_decay:   shared(0.15),
            adsr_sustain: shared(0.7),
            adsr_release: shared(0.4),
            glide_time: shared(0.0),
            master_vol: shared(0.4),
            voice_freqs: (0..VOICE_COUNT).map(|_| shared(440.0)).collect(),
            voice_gates: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            effective_cutoff: shared(3000.0),
            osc_buffer:      Arc::new(std::sync::Mutex::new(vec![0.0f32; 1024])),
            buffer_frames:   Arc::new(AtomicU32::new(0)),
            sample_rate:     Arc::new(AtomicU32::new(0)),
            note_on_time:    Arc::new(std::sync::Mutex::new(None)),
            last_latency_us: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl Default for AudioState {
    fn default() -> Self { Self::new() }
}

pub struct AudioEngine {
    pub state: Arc<AudioState>,
    _stream:   Stream,
}

impl AudioEngine {
    pub fn new() -> anyhow::Result<Self> {
        let state = Arc::new(AudioState::new());
        let stream = build_stream(Arc::clone(&state))?;
        stream.play()?;
        Ok(Self { state, _stream: stream })
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// DSP graph builder
// ---------------------------------------------------------------------------

/// Build the unified 6-voice poly graph.
/// Each voice: 3 OSCs + noise → lowpass(effective_cutoff) → amp ADSR
fn build_synth_graph(state: &AudioState, sr: f64) -> Box<dyn AudioUnit + Send> {
    let a = state.adsr_attack.value();
    let d = state.adsr_decay.value();
    let s = state.adsr_sustain.value();
    let r = state.adsr_release.value();
    let scale = 1.0 / VOICE_COUNT as f32;

    let make_voice = |vi: usize| {
        let vf = &state.voice_freqs[vi];
        let vg = &state.voice_gates[vi];

        let osc0 = (var(vf) * var(&state.osc_freq_mult[0]) >> follow(0.002)
                 >> An(MultiWaveOsc::new(Arc::clone(&state.osc_wave[0]), sr as f32)))
                 * var(&state.osc_vol[0]);
        let osc1 = (var(vf) * var(&state.osc_freq_mult[1]) >> follow(0.002)
                 >> An(MultiWaveOsc::new(Arc::clone(&state.osc_wave[1]), sr as f32)))
                 * var(&state.osc_vol[1]);
        let osc2 = (var(vf) * var(&state.osc_freq_mult[2]) >> follow(0.002)
                 >> An(MultiWaveOsc::new(Arc::clone(&state.osc_wave[2]), sr as f32)))
                 * var(&state.osc_vol[2]);

        let osc = osc0 + osc1 + osc2;
        let env = var(vg) >> adsr_live(a, d, s, r);
        osc * env
    };

    let v0 = make_voice(0);
    let v1 = make_voice(1);
    let v2 = make_voice(2);
    let v3 = make_voice(3);
    let v4 = make_voice(4);
    let v5 = make_voice(5);

    let mut g: Box<dyn AudioUnit + Send> = Box::new(
        (v0 + v1 + v2 + v3 + v4 + v5) * var(&state.master_vol) * scale >> pan(0.0)
    );
    g.set_sample_rate(sr);
    g.allocate();
    g
}

// ---------------------------------------------------------------------------
// cpal stream
// ---------------------------------------------------------------------------

fn build_stream(state: Arc<AudioState>) -> anyhow::Result<Stream> {
    let host   = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No output device"))?;
    let config = device.default_output_config()?;
    let sr     = config.sample_rate().0 as f64;

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

    state.sample_rate.store(sr as u32, std::sync::atomic::Ordering::Relaxed);

    // Fundsp best practice for callback efficiency: run the graph through a
    // block-rate adapter instead of raw sample-by-sample graph traversal.
    let mut graph = BlockRateAdapter::new(build_synth_graph(&state, sr));

    let mut osc_idx: usize = 0;
    let mut buffer_size_captured = false;

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            // Capture actual buffer size on first callback (cpal may use Default buffer size
            // which is only known at runtime).
            if !buffer_size_captured {
                let frames = (data.len() / channels) as u32;
                state.buffer_frames.store(frames, std::sync::atomic::Ordering::Relaxed);
                buffer_size_captured = true;
            }

            state.effective_cutoff.set(state.cutoff.value().clamp(80.0, 18000.0));

            // Real-time latency measurement: if a note_on timestamp is pending,
            // consume it now and record how long it took to reach this callback.
            // try_lock ensures we never block the audio thread.
            if let Ok(mut guard) = state.note_on_time.try_lock() {
                if let Some(t) = guard.take() {
                    let us = t.elapsed().as_micros() as u32;
                    state.last_latency_us.store(us, std::sync::atomic::Ordering::Relaxed);
                }
            }

            // Try to lock oscilloscope buffer once per callback (instead of once per sample).
            let mut scope_buf = state.osc_buffer.try_lock().ok();

            for (frame_i, frame) in data.chunks_mut(channels).enumerate() {
                let (raw_l, raw_r) = graph.get_stereo();
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

                let left  = T::from_sample(l);
                let right = T::from_sample(r_out);
                for (i, smp) in frame.iter_mut().enumerate() {
                    *smp = if i & 1 == 0 { left } else { right };
                }
            }
        },
        |err| eprintln!("audio error: {err}"),
        None,
    )?;

    Ok(stream)
}
