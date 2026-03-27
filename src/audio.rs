//! Audio engine: cpal stream + fundsp synthesis.
//!
//! Continuous parameters use `Shared` (thread-safe atomic f32).
//! The piano tab uses a pool of VOICE_COUNT independent voices, each with its own
//! `freq` and `gate` Shared — enabling true polyphony without any locking.

#![allow(clippy::precedence)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use fundsp::prelude32::*;
use std::sync::Arc;

/// Number of simultaneous voices for the piano tab.
pub const VOICE_COUNT: usize = 6;

pub struct AudioState {
    // Waveform / sequencer (monophonic)
    pub frequency:     Shared,
    pub gate:          Shared,
    pub wave_type:     Arc<std::sync::atomic::AtomicU8>,
    pub volume:        Shared,

    // Filter tab
    pub cutoff:        Shared,
    pub resonance:     Shared,
    pub filter_type:   Arc<std::sync::atomic::AtomicU8>,
    pub filter_source: Arc<std::sync::atomic::AtomicU8>,

    // ADSR (shared by piano and sequencer)
    pub adsr_attack:   Shared,
    pub adsr_decay:    Shared,
    pub adsr_sustain:  Shared,
    pub adsr_release:  Shared,

    // Piano polyphonic voice pool — VOICE_COUNT independent freq/gate pairs.
    // The UI thread writes here on key press/release; the audio callback reads
    // them every sample via fundsp's Var nodes (zero allocation, zero locking).
    pub voice_freqs: Vec<Shared>,
    pub voice_gates: Vec<Shared>,

    pub active_tab:    Arc<std::sync::atomic::AtomicU8>,
    pub osc_buffer:    Arc<std::sync::Mutex<Vec<f32>>>,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            frequency:     shared(440.0),
            gate:          shared(0.0),
            wave_type:     Arc::new(std::sync::atomic::AtomicU8::new(0)),
            volume:        shared(0.4),
            cutoff:        shared(1000.0),
            resonance:     shared(1.0),
            filter_type:   Arc::new(std::sync::atomic::AtomicU8::new(0)),
            filter_source: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            adsr_attack:   shared(0.01),
            adsr_decay:    shared(0.1),
            adsr_sustain:  shared(0.7),
            adsr_release:  shared(0.4),
            voice_freqs:   (0..VOICE_COUNT).map(|_| shared(440.0)).collect(),
            voice_gates:   (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            active_tab:    Arc::new(std::sync::atomic::AtomicU8::new(0)),
            osc_buffer:    Arc::new(std::sync::Mutex::new(vec![0.0f32; 1024])),
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
// Graph builders
// ---------------------------------------------------------------------------

fn build_waveform_graph(
    wave_type: u8, freq: &Shared, vol: &Shared, sr: f64,
) -> Box<dyn AudioUnit + Send> {
    let mut g: Box<dyn AudioUnit + Send> = match wave_type {
        1 => Box::new((var(freq) >> saw())      * var(vol) >> pan(0.0)),
        2 => Box::new((var(freq) >> square())   * var(vol) >> pan(0.0)),
        3 => Box::new((var(freq) >> triangle()) * var(vol) >> pan(0.0)),
        _ => Box::new((var(freq) >> sine())     * var(vol) >> pan(0.0)),
    };
    g.set_sample_rate(sr);
    g.allocate();
    g
}

fn build_filter_graph(
    filter_type: u8, source: u8, cutoff: f32, q: f32, sr: f64,
) -> Box<dyn AudioUnit + Send> {
    let mut g: Box<dyn AudioUnit + Send> = match (source, filter_type) {
        (1, 1) => Box::new(pink() * 0.4 >> highpass_hz(cutoff, q) >> pan(0.0)),
        (1, 2) => Box::new(pink() * 0.4 >> bandpass_hz(cutoff, q) >> pan(0.0)),
        (1, _) => Box::new(pink() * 0.4 >> lowpass_hz(cutoff, q)  >> pan(0.0)),
        (_, 1) => Box::new(saw_hz(110.0) * 0.4 >> highpass_hz(cutoff, q) >> pan(0.0)),
        (_, 2) => Box::new(saw_hz(110.0) * 0.4 >> bandpass_hz(cutoff, q) >> pan(0.0)),
        (_, _) => Box::new(saw_hz(110.0) * 0.4 >> lowpass_hz(cutoff, q)  >> pan(0.0)),
    };
    g.set_sample_rate(sr);
    g.allocate();
    g
}

/// Monophonic piano graph (used by the sequencer tab).
fn build_mono_graph(
    wave_type: u8, freq: &Shared, gate: &Shared, vol: &Shared,
    a: f32, d: f32, s: f32, r: f32, sr: f64,
) -> Box<dyn AudioUnit + Send> {
    let mut g: Box<dyn AudioUnit + Send> = match wave_type {
        1 => Box::new((var(freq) >> saw())      * (var(gate) >> adsr_live(a,d,s,r)) * var(vol) >> pan(0.0)),
        2 => Box::new((var(freq) >> square())   * (var(gate) >> adsr_live(a,d,s,r)) * var(vol) >> pan(0.0)),
        3 => Box::new((var(freq) >> triangle()) * (var(gate) >> adsr_live(a,d,s,r)) * var(vol) >> pan(0.0)),
        _ => Box::new((var(freq) >> sine())     * (var(gate) >> adsr_live(a,d,s,r)) * var(vol) >> pan(0.0)),
    };
    g.set_sample_rate(sr);
    g.allocate();
    g
}

/// Polyphonic piano graph: VOICE_COUNT independent voices summed together.
///
/// Each voice is:  `(var(freq[i]) >> osc) * (var(gate[i]) >> adsr_live(...))`
///
/// The `&` (Bus) operator sums multiple AudioNodes with identical I/O counts.
/// All voices run in parallel; the audio callback needs no additional logic —
/// it just calls `get_stereo()` and fundsp evaluates every voice automatically.
fn build_poly_graph(
    vf: &[Shared], vg: &[Shared],
    wave_type: u8, vol: &Shared,
    a: f32, d: f32, s: f32, r: f32, sr: f64,
) -> Box<dyn AudioUnit + Send> {
    let scale = 1.0 / VOICE_COUNT as f32;

    let mut g: Box<dyn AudioUnit + Send> = match wave_type {
        1 => Box::new({
            let mk = |i: usize| (var(&vf[i]) >> saw()) * (var(&vg[i]) >> adsr_live(a,d,s,r));
            let (v0,v1,v2,v3,v4,v5) = (mk(0),mk(1),mk(2),mk(3),mk(4),mk(5));
            (v0 & v1 & v2 & v3 & v4 & v5) * var(vol) * scale >> pan(0.0)
        }),
        2 => Box::new({
            let mk = |i: usize| (var(&vf[i]) >> square()) * (var(&vg[i]) >> adsr_live(a,d,s,r));
            let (v0,v1,v2,v3,v4,v5) = (mk(0),mk(1),mk(2),mk(3),mk(4),mk(5));
            (v0 & v1 & v2 & v3 & v4 & v5) * var(vol) * scale >> pan(0.0)
        }),
        3 => Box::new({
            let mk = |i: usize| (var(&vf[i]) >> triangle()) * (var(&vg[i]) >> adsr_live(a,d,s,r));
            let (v0,v1,v2,v3,v4,v5) = (mk(0),mk(1),mk(2),mk(3),mk(4),mk(5));
            (v0 & v1 & v2 & v3 & v4 & v5) * var(vol) * scale >> pan(0.0)
        }),
        _ => Box::new({
            let mk = |i: usize| (var(&vf[i]) >> sine()) * (var(&vg[i]) >> adsr_live(a,d,s,r));
            let (v0,v1,v2,v3,v4,v5) = (mk(0),mk(1),mk(2),mk(3),mk(4),mk(5));
            (v0 & v1 & v2 & v3 & v4 & v5) * var(vol) * scale >> pan(0.0)
        }),
    };
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

    let mut wave_graph  = build_waveform_graph(0, &state.frequency, &state.volume, sr);
    let mut filter_graph = build_filter_graph(0, 0, 1000.0, 1.0, sr);
    let mut poly_graph  = build_poly_graph(
        &state.voice_freqs, &state.voice_gates, 0, &state.volume,
        0.01, 0.1, 0.7, 0.4, sr,
    );
    let mut seq_graph   = build_mono_graph(
        0, &state.frequency, &state.gate, &state.volume,
        0.01, 0.1, 0.7, 0.4, sr,
    );

    let mut prev_wave_type:   u8  = 0;
    let mut prev_filter_type: u8  = 0;
    let mut prev_filter_src:  u8  = 0;
    let mut prev_cutoff:      f32 = 1000.0;
    let mut prev_resonance:   f32 = 1.0;
    let mut prev_piano_wave:  u8  = 0;
    let mut prev_adsr:        (f32, f32, f32, f32) = (0.01, 0.1, 0.7, 0.4);

    let mut osc_idx: usize = 0;

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            use std::sync::atomic::Ordering::Relaxed;

            let tab       = state.active_tab.load(Relaxed);
            let wave_type = state.wave_type.load(Relaxed);
            let flt_type  = state.filter_type.load(Relaxed);
            let flt_src   = state.filter_source.load(Relaxed);
            let cutoff    = state.cutoff.value();
            let resonance = state.resonance.value();
            let a = state.adsr_attack.value();
            let d = state.adsr_decay.value();
            let s = state.adsr_sustain.value();
            let r = state.adsr_release.value();

            if tab == 0 && wave_type != prev_wave_type {
                prev_wave_type = wave_type;
                wave_graph = build_waveform_graph(wave_type, &state.frequency, &state.volume, sr);
            }
            if tab == 1
                && (flt_type != prev_filter_type
                    || flt_src != prev_filter_src
                    || (cutoff - prev_cutoff).abs() > 0.5
                    || (resonance - prev_resonance).abs() > 0.01)
            {
                prev_filter_type = flt_type;
                prev_filter_src  = flt_src;
                prev_cutoff      = cutoff;
                prev_resonance   = resonance;
                filter_graph = build_filter_graph(flt_type, flt_src, cutoff, resonance, sr);
            }
            if (tab == 2 || tab == 3)
                && (wave_type != prev_piano_wave || (a, d, s, r) != prev_adsr)
            {
                prev_piano_wave = wave_type;
                prev_adsr       = (a, d, s, r);
                poly_graph = build_poly_graph(
                    &state.voice_freqs, &state.voice_gates, wave_type, &state.volume,
                    a, d, s, r, sr,
                );
                seq_graph = build_mono_graph(
                    wave_type, &state.frequency, &state.gate, &state.volume,
                    a, d, s, r, sr,
                );
            }

            let active: &mut Box<dyn AudioUnit + Send> = match tab {
                1 => &mut filter_graph,
                2 => &mut poly_graph,
                3 => &mut seq_graph,
                _ => &mut wave_graph,
            };

            for frame in data.chunks_mut(channels) {
                let (l, r_out) = active.get_stereo();

                if let Ok(mut buf) = state.osc_buffer.try_lock() {
                    let len = buf.len();
                    buf[osc_idx % len] = l;
                    osc_idx = osc_idx.wrapping_add(1);
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
