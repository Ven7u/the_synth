//! cpal stream setup. AudioState, voice allocation, and DSP graph live in
//! `synth-engine`. This file wires cpal's output callback to an engine
//! `VoiceAllocator` plus the per-sample modulation / metering passes that
//! the callback owns (LFO phase accumulators, DC blocker, lookahead limiter,
//! scope, peak metering).

#![allow(clippy::precedence)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use fundsp::prelude32::*;
use std::sync::{Arc, Mutex};

use crate::recorder::Recorder;
use synth_control::{make_control_channel, ControlReceiver};
use synth_dsp::LookaheadLimiter;
use synth_engine::audio::build_synth_graph;
use synth_engine::{SynthEngineHandle, VoiceAllocator};

type RecorderSink = Arc<Mutex<Option<Recorder>>>;

// Re-export so main.rs can keep its existing import unchanged.
pub use synth_engine::audio::{AudioState, VOICE_COUNT};

pub struct AudioEngine {
    /// Typed facade over the engine's internal state and control channel.
    /// Clone this into any thread (UI, MIDI, sequencer, future OSC/WS/FFI
    /// bridges) that needs to talk to the engine.
    pub handle: SynthEngineHandle,
    _stream: Stream,
}

impl AudioEngine {
    pub fn new(recorder_sink: RecorderSink) -> anyhow::Result<Self> {
        let state = Arc::new(AudioState::new());
        let (tx, rx) = make_control_channel(1024);
        let stream = build_stream(Arc::clone(&state), rx, recorder_sink)?;
        stream.play()?;
        let handle = SynthEngineHandle::new(state, tx);
        Ok(Self {
            handle,
            _stream: stream,
        })
    }
}

// ---------------------------------------------------------------------------
// cpal stream
// ---------------------------------------------------------------------------

fn build_stream(
    state: Arc<AudioState>,
    rx: ControlReceiver,
    recorder_sink: RecorderSink,
) -> anyhow::Result<Stream> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No output device"))?;
    let config = device.default_output_config()?;
    let sr = config.sample_rate().0 as f64;

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            make_stream::<f32>(&device, &config.into(), state, sr, rx, recorder_sink)?
        }
        cpal::SampleFormat::I16 => {
            make_stream::<i16>(&device, &config.into(), state, sr, rx, recorder_sink)?
        }
        cpal::SampleFormat::U16 => {
            make_stream::<u16>(&device, &config.into(), state, sr, rx, recorder_sink)?
        }
        _ => anyhow::bail!("Unsupported sample format"),
    };
    Ok(stream)
}

fn make_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: Arc<AudioState>,
    sr: f64,
    rx: ControlReceiver,
    recorder_sink: RecorderSink,
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

    // Lookahead true-peak limiter: 1.5ms lookahead, 80ms release
    let mut lookahead_lim = LookaheadLimiter::new(sr as f32, 1.5, 80.0);

    // Smoothed global volume — 10ms one-pole to prevent clicks on slider moves
    let global_vol_coeff = (-1.0_f64 / (0.010 * sr)).exp() as f32;
    let mut global_vol_smooth: f32 = state.global_vol.value() as f32;

    // DC blocker: 1-pole high-pass at ~20 Hz. Removes low-frequency bias that
    // builds up from FX chains (reverb, chorus, delay) and eats headroom.
    // y[n] = x[n] - x[n-1] + coeff * y[n-1]  (coeff ≈ 1 - 2π·20/sr)
    let dc_coeff = 1.0_f32 - (std::f32::consts::TAU * 20.0 / sr as f32);
    let mut dc_x_prev_l: f32 = 0.0;
    let mut dc_x_prev_r: f32 = 0.0;
    let mut dc_y_prev_l: f32 = 0.0;
    let mut dc_y_prev_r: f32 = 0.0;

    // Voice gain staging: smooth 1/sqrt(active_voices) to prevent polyphonic
    // passages from sounding louder than monophonic notes.
    let vgs_coeff = (-1.0_f64 / (0.020 * sr)).exp() as f32; // 20ms smoothing
    let mut voice_gain_smooth: f32 = 1.0;

    // LFO phase accumulators (0..1, advance per sample)
    let mut lfo_phase: f32 = 0.0;
    let mut lfo2_phase: f32 = 0.25; // offset by 90° so LFO1 and LFO2 don't start in sync

    // Per-voice smoothed frequencies for glide (callback writes to voice_freqs from these)
    let mut smoothed_freqs: Vec<f32> = vec![440.0; VOICE_COUNT];

    // Voice allocation + event dispatch + arp/walker state. Audio-thread-owned.
    let mut voices = VoiceAllocator::new();

    // Last frequency used for key tracking — persists across buffers so the
    // filter doesn't snap back to C4 during note release or retrigger gaps.
    let mut last_keyed_freq: f32 = 261.63;

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let frames = data.len() / channels;

            // First-callback setup: flush-to-zero / denormals-are-zero.
            // Prevents reverb / crystallizer tails from triggering a 10–100×
            // CPU cliff when values slip into subnormal range. Thread-local
            // CPU state, safe to re-set each callback but only needs once.
            if !buffer_size_captured {
                synth_engine::enable_ftz_on_current_thread();
            }

            // --- Per-buffer: release cleanup + event drain + arp/walker tick ---
            voices.begin_buffer(&state, &rx, frames, sr);

            // Capture actual buffer size on first callback (cpal may use Default buffer size
            // which is only known at runtime).
            if !buffer_size_captured {
                let frames = (data.len() / channels) as u32;
                state
                    .buffer_frames
                    .store(frames, std::sync::atomic::Ordering::Relaxed);
                buffer_size_captured = true;
            }

            // Voice gain staging: count sounding voices, smooth 1/sqrt(n) gain
            {
                let n_active = state
                    .amp_cursors
                    .iter()
                    .filter(|c| c.value() > 0.01)
                    .count();
                let n_active = if n_active < 1 { 1 } else { n_active };
                let target_scale = 1.0_f32 / (n_active as f32).sqrt();
                voice_gain_smooth = target_scale + vgs_coeff * (voice_gain_smooth - target_scale);
                state.voice_gain_scale.set(voice_gain_smooth);
            }

            // Read per-buffer params once (cheap; avoids repeated atomic loads per sample)
            let sr_f = sr as f32;
            let lfo_rate = state.lfo_rate.value();
            let lfo_depth = state.lfo_depth.value();
            let lfo_shape = state.lfo_shape.load(std::sync::atomic::Ordering::Relaxed);
            let lfo_dest = state.lfo_dest.load(std::sync::atomic::Ordering::Relaxed);
            let lfo_dt = lfo_rate / sr_f;
            let lfo2_rate = state.lfo2_rate.value();
            let lfo2_depth = state.lfo2_depth.value();
            let lfo2_shape = state.lfo2_shape.load(std::sync::atomic::Ordering::Relaxed);
            let lfo2_dest = state.lfo2_dest.load(std::sync::atomic::Ordering::Relaxed);
            let lfo2_dt = lfo2_rate / sr_f;
            let base_cutoff = state.cutoff.value().clamp(80.0, 18000.0);

            // Key tracking: scale cutoff by the pitch of the highest sounding voice.
            // Uses amp_cursors (non-zero from attack through release) so a fresh
            // note's pitch is picked up even during the 4-sample retrigger gap.
            // last_keyed_freq persists so release phase keeps the note's tracking.
            let key_track = state.filter_key_track.value();
            if key_track > 0.001 {
                let mut top_freq: f32 = 0.0;
                for vi in 0..VOICE_COUNT {
                    if state.amp_cursors[vi].value() > 0.5 {
                        let f = state.voice_freq_targets[vi].value();
                        if f > top_freq {
                            top_freq = f;
                        }
                    }
                }
                if top_freq > 0.0 {
                    last_keyed_freq = top_freq;
                }
            }
            // Exponent × 2 so KEY=0.5 = standard 1:1 tracking (one octave → 2× cutoff)
            // and KEY=1.0 = hyper tracking (one octave → 4× cutoff).
            let key_mult = if key_track > 0.001 {
                (last_keyed_freq / 261.63_f32).powf(key_track * 2.0)
            } else {
                1.0
            };
            let keyed_cutoff = base_cutoff * key_mult;

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
                // --- LFO 1 & 2: advance phases and combine modulation ---
                lfo_phase += lfo_dt;
                if lfo_phase >= 1.0 {
                    lfo_phase -= 1.0;
                }
                let lfo_raw = match lfo_shape {
                    1 => {
                        if lfo_phase < 0.5 {
                            4.0 * lfo_phase - 1.0
                        } else {
                            3.0 - 4.0 * lfo_phase
                        }
                    }
                    2 => 2.0 * lfo_phase - 1.0,
                    _ => (lfo_phase * std::f32::consts::TAU).sin(),
                };
                lfo2_phase += lfo2_dt;
                if lfo2_phase >= 1.0 {
                    lfo2_phase -= 1.0;
                }
                let lfo2_raw = match lfo2_shape {
                    1 => {
                        if lfo2_phase < 0.5 {
                            4.0 * lfo2_phase - 1.0
                        } else {
                            3.0 - 4.0 * lfo2_phase
                        }
                    }
                    2 => 2.0 * lfo2_phase - 1.0,
                    _ => (lfo2_phase * std::f32::consts::TAU).sin(),
                };

                // Accumulate pitch, filter, and amp contributions from both LFOs
                let mut pitch_mod: f32 = 0.0; // additive semitones * 2
                let mut filter_mod: f32 = 0.0; // additive cutoff multiplier
                let mut amp_mod: f32 = 1.0; // multiplicative

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

                state.lfo_pitch_mult.set(2_f32.powf(pitch_mod * 2.0 / 12.0));
                state
                    .effective_cutoff
                    .set((keyed_cutoff + filter_mod * keyed_cutoff * 0.5).clamp(80.0, 18000.0));
                let lfo_amp = amp_mod;

                // Drive the voice allocator's per-sample retrigger countdown.
                // Flips any voice's gate back to 1.0 when its countdown
                // expires, giving the ADSR a real 0→1 transition for a clean
                // attack after same-buffer NoteOff+NoteOn sequences.
                voices.tick_sample(&state);

                let (raw_l_pre, raw_r_pre) = graph.get_stereo();

                // DC blocker applied before limiter so limiter sees clean signal
                let dc_l = raw_l_pre - dc_x_prev_l + dc_coeff * dc_y_prev_l;
                let dc_r = raw_r_pre - dc_x_prev_r + dc_coeff * dc_y_prev_r;
                dc_x_prev_l = raw_l_pre;
                dc_y_prev_l = dc_l;
                dc_x_prev_r = raw_r_pre;
                dc_y_prev_r = dc_r;
                let (mut raw_l, mut raw_r) = (dc_l, dc_r);

                // Lookahead true-peak limiter: applies gain before the peak arrives
                if limiter_on {
                    let (lim_l, lim_r) = lookahead_lim.process_stereo(raw_l, raw_r, threshold);
                    raw_l = lim_l;
                    raw_r = lim_r;
                }

                // Gentle soft clip for occasional overshoots.
                // Apply tremolo after limiter so the limiter doesn't fight the modulation.
                let target_global = state.global_vol.value() as f32;
                global_vol_smooth =
                    target_global + global_vol_coeff * (global_vol_smooth - target_global);
                let l = if raw_l.is_finite() { raw_l.tanh() } else { 0.0 }
                    * lfo_amp
                    * global_vol_smooth;
                let r_out = if raw_r.is_finite() { raw_r.tanh() } else { 0.0 }
                    * lfo_amp
                    * global_vol_smooth;

                // Peak metering: track true output level (post-limiter, post-tanh)
                if l.abs() > peak_l_local {
                    peak_l_local = l.abs();
                }
                if r_out.abs() > peak_r_local {
                    peak_r_local = r_out.abs();
                }

                if let Some(buf) = scope_buf.as_mut() {
                    // Downsample scope writes to reduce callback pressure.
                    if frame_i & 3 == 0 {
                        let len = buf.len();
                        buf[osc_idx % len] = l;
                        osc_idx = osc_idx.wrapping_add(1);
                    }
                }

                if let Ok(rec) = recorder_sink.try_lock() {
                    if let Some(rec) = rec.as_ref() {
                        rec.push(l, r_out);
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
