//! cpal stream setup. AudioState and DSP graph live in `synth-engine`.

#![allow(clippy::precedence)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use fundsp::prelude32::*;
use fundsp::prelude::midi_hz;
use std::sync::Arc;

use synth_engine::audio::build_synth_graph;
use synth_control::{make_control_channel, ControlSender, ControlReceiver, ControlEvent, ParamId};

// Re-export so main.rs can keep its existing import unchanged.
pub use synth_engine::audio::{AudioState, VOICE_COUNT};

pub struct AudioEngine {
    pub state: Arc<AudioState>,
    pub control_tx: ControlSender,
    _stream: Stream,
}

impl AudioEngine {
    pub fn new() -> anyhow::Result<Self> {
        let state = Arc::new(AudioState::new());
        let (tx, rx) = make_control_channel(1024);
        let stream = build_stream(Arc::clone(&state), rx)?;
        stream.play()?;
        Ok(Self {
            state,
            control_tx: tx,
            _stream: stream,
        })
    }
}

// ---------------------------------------------------------------------------
// cpal stream
// ---------------------------------------------------------------------------

fn build_stream(state: Arc<AudioState>, rx: ControlReceiver) -> anyhow::Result<Stream> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No output device"))?;
    let config = device.default_output_config()?;
    let sr = config.sample_rate().0 as f64;

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => make_stream::<f32>(&device, &config.into(), state, sr, rx)?,
        cpal::SampleFormat::I16 => make_stream::<i16>(&device, &config.into(), state, sr, rx)?,
        cpal::SampleFormat::U16 => make_stream::<u16>(&device, &config.into(), state, sr, rx)?,
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

    // Voice allocation state — moved from UI thread to audio callback.
    // slot → Option<MIDI pitch> for each of VOICE_COUNT voices.
    let mut voice_notes: [Option<u8>; VOICE_COUNT] = [None; VOICE_COUNT];
    let mut steal_idx: usize = 0;

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            // --- Release cleanup: free slots whose envelopes have finished ---
            for (slot, note) in voice_notes.iter_mut().enumerate() {
                if note.is_some() && state.voice_gates[slot].value() < 0.5
                    && state.amp_cursors[slot].value() < 0.5
                {
                    *note = None;
                }
            }

            // --- Drain control events ---
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    ControlEvent::NoteOn { pitch, track: _, .. } => {
                        // Ignore if this pitch is already playing at full gate (key repeat).
                        if voice_notes.iter().enumerate().any(|(s, &n)| {
                            n == Some(pitch) && state.voice_gates[s].value() > 0.5
                        }) {
                            continue;
                        }
                        let slot = voice_notes.iter().position(|&n| n == Some(pitch))
                            .or_else(|| voice_notes.iter().position(|n| n.is_none()))
                            .unwrap_or_else(|| {
                                let s = steal_idx % VOICE_COUNT;
                                steal_idx += 1;
                                s
                            });
                        voice_notes[slot] = Some(pitch);
                        state.voice_freq_targets[slot].set(midi_hz(pitch as f64) as f32);
                        state.voice_gates[slot].set(1.0);
                    }
                    ControlEvent::NoteOff { pitch, track: _ } => {
                        for (slot, note) in voice_notes.iter_mut().enumerate() {
                            if *note == Some(pitch) {
                                state.voice_gates[slot].set(0.0);
                                break;
                            }
                        }
                    }
                    ControlEvent::SetParam { param, value } => {
                        match param {
                            ParamId::FilterCutoff    => state.cutoff.set(value),
                            ParamId::FilterResonance => state.resonance.set(value),
                            ParamId::LfoDepth        => state.lfo_depth.set(value),
                            ParamId::MasterVolume    => state.master_vol.set(value),
                            ParamId::LfoPitchMult    => state.lfo_pitch_mult.set(value),
                        }
                    }
                }
            }

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
