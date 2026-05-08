//! cpal stream setup. AudioState, voice allocation, and DSP graph live in
//! `forma-engine`. This file wires cpal's output callback to an engine
//! `VoiceAllocator` plus the per-sample modulation / metering passes that
//! the callback owns (LFO phase accumulators, DC blocker, lookahead limiter,
//! scope, peak metering).

#![allow(clippy::precedence)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use fundsp::prelude32::*;
use std::sync::{Arc, Mutex};

use crate::recorder::Recorder;
use forma_control::{make_control_channel, ControlReceiver};
use forma_dsp::LookaheadLimiter;
use forma_engine::audio::build_synth_graph;
use forma_engine::{SynthEngineHandle, VoiceAllocator};

type RecorderSink = Arc<Mutex<Option<Recorder>>>;

// Re-export so main.rs can keep its existing import unchanged.
pub use forma_engine::audio::{AudioState, VOICE_COUNT};

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

    // Gate-lane "Pulse" (master ducker) state.
    //   acc starts at 1.0 so the first sample after enabling fires step 0.
    //   step counter wraps modulo `length` from the engine state.
    //   duck_env follows a one-pole asymmetric envelope: fast attack toward 1.0,
    //   slow exponential decay back to 0. Bandlimits the modulator → no clicks.
    let mut gate_aenv_acc: f32 = 1.0;
    let mut gate_aenv_step: u32 = 0;
    let mut gate_aenv_was_enabled: bool = false;
    let mut duck_env: f32 = 0.0;
    let mut duck_attacking: bool = false;
    let duck_attack_coeff: f32 = (-1.0_f32 / (0.0015 * sr as f32)).exp(); // 1.5 ms attack
    let duck_decay_coeff: f32 = (-1.0_f32 / (0.150 * sr as f32)).exp(); // 150 ms decay
                                                                        // Smoothed depth — prevents zipper noise / micro-clicks when the user moves the slider.
    let depth_smooth_coeff: f32 = (-1.0_f32 / (0.010 * sr as f32)).exp(); // 10 ms
    let mut depth_smooth: f32 = 0.0;

    // Gate-lane retrigger state for LFO1 and LFO2 — each lane resets its LFO's
    // phase to 0 on every fired step. Same accumulator pattern as the duck lane.
    let mut gate_lfo1_acc: f32 = 1.0;
    let mut gate_lfo1_step: u32 = 0;
    let mut gate_lfo1_was_enabled: bool = false;
    let mut gate_lfo2_acc: f32 = 1.0;
    let mut gate_lfo2_step: u32 = 0;
    let mut gate_lfo2_was_enabled: bool = false;

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
                forma_engine::enable_ftz_on_current_thread();
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

            // Gate lanes: read each lane's state once per buffer. Each lane stores
            // (enabled, pattern, length, rate); per-callback locals carry the phase
            // accumulator and step counter so all lanes stay phase-coherent within a buffer.
            // Rising-edge resets ensure step 0 fires the moment a lane is enabled.
            macro_rules! read_gate_lane {
                ($lane:ident, $acc:ident, $step:ident, $was:ident) => {{
                    let enabled = state
                        .$lane
                        .enabled
                        .load(std::sync::atomic::Ordering::Relaxed);
                    let pattern = state
                        .$lane
                        .pattern
                        .load(std::sync::atomic::Ordering::Relaxed);
                    let length = {
                        let raw = state
                            .$lane
                            .length
                            .load(std::sync::atomic::Ordering::Relaxed);
                        if raw < 1 { 1u32 } else { raw as u32 }
                    };
                    let dt = state.$lane.rate.value() / sr_f;
                    if enabled && !$was {
                        $acc = 1.0;
                        $step = 0;
                    }
                    $was = enabled;
                    (enabled, pattern, length, dt)
                }};
            }
            let (gate_aenv_enabled, gate_aenv_pattern, gate_aenv_length, gate_aenv_dt) =
                read_gate_lane!(gate_aenv, gate_aenv_acc, gate_aenv_step, gate_aenv_was_enabled);
            let (gate_lfo1_enabled, gate_lfo1_pattern, gate_lfo1_length, gate_lfo1_dt) =
                read_gate_lane!(gate_lfo1, gate_lfo1_acc, gate_lfo1_step, gate_lfo1_was_enabled);
            let (gate_lfo2_enabled, gate_lfo2_pattern, gate_lfo2_length, gate_lfo2_dt) =
                read_gate_lane!(gate_lfo2, gate_lfo2_acc, gate_lfo2_step, gate_lfo2_was_enabled);
            let base_cutoff =
                (state.cutoff.value() + state.mod_wheel_cutoff_add.value()).clamp(80.0, 18000.0);

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

                // Gate-lane "Pulse": advance step accumulator, fire on each step boundary.
                // acc wraps from 1.0 — when it crosses 1.0 we've reached the next step.
                if gate_aenv_enabled {
                    gate_aenv_acc += gate_aenv_dt;
                    if gate_aenv_acc >= 1.0 {
                        gate_aenv_acc -= 1.0;
                        let step_idx = (gate_aenv_step % gate_aenv_length) as u8;
                        if (gate_aenv_pattern >> step_idx) & 1 != 0 {
                            duck_attacking = true;
                        }
                        gate_aenv_step = gate_aenv_step.wrapping_add(1);
                    }
                }
                // Gate lanes for LFO1/LFO2: each "on" step resets the LFO's phase to 0.
                // Phase reset is a discontinuity in the modulator — a small click at high LFO
                // depth. Documented in click-prevention plan; click-free retrigger is a follow-up.
                if gate_lfo1_enabled {
                    gate_lfo1_acc += gate_lfo1_dt;
                    if gate_lfo1_acc >= 1.0 {
                        gate_lfo1_acc -= 1.0;
                        let step_idx = (gate_lfo1_step % gate_lfo1_length) as u8;
                        if (gate_lfo1_pattern >> step_idx) & 1 != 0 {
                            lfo_phase = 0.0;
                        }
                        gate_lfo1_step = gate_lfo1_step.wrapping_add(1);
                    }
                }
                if gate_lfo2_enabled {
                    gate_lfo2_acc += gate_lfo2_dt;
                    if gate_lfo2_acc >= 1.0 {
                        gate_lfo2_acc -= 1.0;
                        let step_idx = (gate_lfo2_step % gate_lfo2_length) as u8;
                        if (gate_lfo2_pattern >> step_idx) & 1 != 0 {
                            lfo2_phase = 0.0;
                        }
                        gate_lfo2_step = gate_lfo2_step.wrapping_add(1);
                    }
                }
                // Asymmetric envelope: ramp up to 1.0 with fast attack, then decay slowly.
                // Bandlimits the modulator and kills the trigger click.
                if duck_attacking {
                    duck_env = 1.0 + duck_attack_coeff * (duck_env - 1.0);
                    if duck_env > 0.99 {
                        duck_attacking = false;
                    }
                } else {
                    duck_env *= duck_decay_coeff;
                }

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
                let target_depth = state.gate_aenv_depth.value();
                depth_smooth =
                    target_depth + depth_smooth_coeff * (depth_smooth - target_depth);
                let duck_mult = 1.0 - duck_env * depth_smooth;
                let l = if raw_l.is_finite() { raw_l.tanh() } else { 0.0 }
                    * lfo_amp
                    * global_vol_smooth
                    * duck_mult;
                let r_out = if raw_r.is_finite() { raw_r.tanh() } else { 0.0 }
                    * lfo_amp
                    * global_vol_smooth
                    * duck_mult;

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
