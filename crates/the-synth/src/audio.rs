//! cpal stream setup. AudioState and DSP graph live in `synth-engine`.

#![allow(clippy::precedence)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use fundsp::prelude32::*;
use fundsp::prelude::midi_hz;
use std::sync::{Arc, Mutex};

use synth_engine::audio::build_synth_graph;
use synth_engine::arp::{ArpState, ScaleWalker};
use synth_control::{make_control_channel, ControlSender, ControlReceiver, ControlEvent, ParamId};
use crate::recorder::Recorder;

type RecorderSink = Arc<Mutex<Option<Recorder>>>;

// Re-export so main.rs can keep its existing import unchanged.
pub use synth_engine::audio::{AudioState, VOICE_COUNT};

// ---------------------------------------------------------------------------
// Voice allocation helpers
// ---------------------------------------------------------------------------

/// Trigger or retrigger a voice for `pitch`.
///
/// - Increments the pitch's hold count.
/// - If the count was 0 (fresh note): allocates a slot and sets gate=1.0 immediately.
/// - If the count was >0 (retrigger): sets gate=0.0 and starts a 4-sample countdown;
///   the sample loop will flip the gate back to 1.0 once it expires, giving the ADSR
///   a real 0→1 transition without a same-buffer NoteOff+NoteOn collision.
fn trigger_note(
    pitch: u8,
    voice_notes: &mut [Option<u8>],
    steal_idx: &mut usize,
    pitch_hold_count: &mut [u8; 128],
    retrigger_countdown: &mut [u8],
    voice_freq_targets: &[fundsp::prelude32::Shared],
    voice_gates: &[fundsp::prelude32::Shared],
) {
    let count = &mut pitch_hold_count[pitch as usize];
    let was_zero = *count == 0;
    *count = count.saturating_add(1);

    let n = voice_notes.len();
    let slot = voice_notes.iter().position(|&v| v == Some(pitch))
        .or_else(|| voice_notes.iter().position(|v| v.is_none()))
        .unwrap_or_else(|| {
            let s = *steal_idx % n;
            *steal_idx += 1;
            s
        });

    voice_notes[slot] = Some(pitch);
    voice_freq_targets[slot].set(midi_hz(pitch as f64) as f32);

    if was_zero {
        voice_gates[slot].set(1.0);
        retrigger_countdown[slot] = 0;
    } else {
        // Retrigger: brief silence so the ADSR sees a real gate edge.
        voice_gates[slot].set(0.0);
        retrigger_countdown[slot] = 4; // ~90 µs at 44.1 kHz — inaudible gap
    }
}

/// Decrement the hold count for `pitch` and kill its gate only when count reaches 0.
fn release_note(
    pitch: u8,
    voice_notes: &mut [Option<u8>],
    pitch_hold_count: &mut [u8; 128],
    voice_gates: &[fundsp::prelude32::Shared],
) {
    let count = &mut pitch_hold_count[pitch as usize];
    if *count > 0 { *count -= 1; }
    if *count == 0 {
        for (slot, note) in voice_notes.iter_mut().enumerate() {
            if *note == Some(pitch) {
                voice_gates[slot].set(0.0);
                break;
            }
        }
    }
}

pub struct AudioEngine {
    pub state: Arc<AudioState>,
    pub control_tx: ControlSender,
    _stream: Stream,
}

impl AudioEngine {
    pub fn new(recorder_sink: RecorderSink) -> anyhow::Result<Self> {
        let state = Arc::new(AudioState::new());
        let (tx, rx) = make_control_channel(1024);
        let stream = build_stream(Arc::clone(&state), rx, recorder_sink)?;
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

fn build_stream(state: Arc<AudioState>, rx: ControlReceiver, recorder_sink: RecorderSink) -> anyhow::Result<Stream> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No output device"))?;
    let config = device.default_output_config()?;
    let sr = config.sample_rate().0 as f64;

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => make_stream::<f32>(&device, &config.into(), state, sr, rx, recorder_sink)?,
        cpal::SampleFormat::I16 => make_stream::<i16>(&device, &config.into(), state, sr, rx, recorder_sink)?,
        cpal::SampleFormat::U16 => make_stream::<u16>(&device, &config.into(), state, sr, rx, recorder_sink)?,
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

    // Limiter envelope state (lives across callbacks)
    let mut env_l: f32 = 0.0;
    let mut env_r: f32 = 0.0;

    // LFO phase accumulators (0..1, advance per sample)
    let mut lfo_phase: f32 = 0.0;
    let mut lfo2_phase: f32 = 0.25; // offset by 90° so LFO1 and LFO2 don't start in sync

    // Per-voice smoothed frequencies for glide (callback writes to voice_freqs from these)
    let mut smoothed_freqs: Vec<f32> = vec![440.0; VOICE_COUNT];
    let attack_coeff = (-1.0_f64 / (0.0001 * sr)).exp() as f32; // ~0.1ms attack
    let release_coeff = (-1.0_f64 / (0.05 * sr)).exp() as f32; // ~50ms release

    // Voice allocation state — moved from UI thread to audio callback.
    // slot → Option<MIDI pitch> for each of VOICE_COUNT voices.
    let mut voice_notes: [Option<u8>; VOICE_COUNT] = [None; VOICE_COUNT];
    let mut steal_idx: usize = 0;

    // Reference count per MIDI pitch: how many sources are currently holding it.
    // A gate is only killed when the count reaches 0, so keyboard and sequencer
    // can't accidentally cut each other off on shared pitches.
    let mut pitch_hold_count: [u8; 128] = [0; 128];

    // Per-voice retrigger countdown (in samples). When > 0 the gate is forced to
    // 0.0; when it hits 0 the gate is set to 1.0. This gives the ADSR a real
    // 0→1 transition even when a NoteOn arrives while the note is already playing.
    let mut retrigger_countdown: [u8; VOICE_COUNT] = [0; VOICE_COUNT];

    // Arpeggiator and scale walker internal state (audio thread only).
    // Config is read each tick from state.arp / state.walker (Shared atomics).
    let mut arp    = ArpState::new();
    let mut walker = ScaleWalker::new();

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            // --- Release cleanup: free slots whose envelopes have finished ---
            // Skip voices that are mid-retrigger — their gate is briefly 0 by design.
            for (slot, note) in voice_notes.iter_mut().enumerate() {
                if note.is_some()
                    && retrigger_countdown[slot] == 0
                    && state.voice_gates[slot].value() < 0.5
                    && state.amp_cursors[slot].value() < 0.5
                {
                    pitch_hold_count[note.unwrap() as usize] = 0;
                    *note = None;
                }
            }

            // --- Drain control events ---
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    ControlEvent::NoteOn { pitch, track: _, .. } => {
                        if state.arp.enabled.load(std::sync::atomic::Ordering::Relaxed) {
                            arp.note_on(pitch);
                        } else {
                            trigger_note(pitch, &mut voice_notes, &mut steal_idx,
                                &mut pitch_hold_count, &mut retrigger_countdown,
                                &state.voice_freq_targets, &state.voice_gates);
                        }
                    }
                    ControlEvent::NoteOff { pitch, track: _ } => {
                        if state.arp.enabled.load(std::sync::atomic::Ordering::Relaxed) {
                            let hold = state.arp.hold.load(std::sync::atomic::Ordering::Relaxed);
                            arp.note_off(pitch, hold);
                        } else {
                            release_note(pitch, &mut voice_notes,
                                &mut pitch_hold_count, &state.voice_gates);
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
                    ControlEvent::ChordHold { notes, .. } => {
                        arp.set_chord(&notes);
                    }
                    ControlEvent::ArpRestart { .. } => {
                        if let Some(pitch) = arp.restart() {
                            release_note(pitch, &mut voice_notes,
                                &mut pitch_hold_count, &state.voice_gates);
                        }
                    }
                    ControlEvent::WalkerRestart { .. } => {
                        if let Some(pitch) = walker.restart() {
                            release_note(pitch, &mut voice_notes,
                                &mut pitch_hold_count, &state.voice_gates);
                        }
                    }
                }
            }

            // --- Arp + scale walker tick (once per buffer) ---
            let frames = data.len() / channels;
            let arp_ev = arp.tick(&state.arp, frames, sr);
            let walk_ev = walker.tick(&state.walker, frames, sr);

            for ev in [arp_ev, walk_ev] {
                if let Some(pitch) = ev.note_off {
                    release_note(pitch, &mut voice_notes,
                        &mut pitch_hold_count, &state.voice_gates);
                }
                if let Some(pitch) = ev.note_on {
                    trigger_note(pitch, &mut voice_notes, &mut steal_idx,
                        &mut pitch_hold_count, &mut retrigger_countdown,
                        &state.voice_freq_targets, &state.voice_gates);
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
            let sr_f = sr as f32;
            let lfo_rate  = state.lfo_rate.value();
            let lfo_depth = state.lfo_depth.value();
            let lfo_shape = state.lfo_shape.load(std::sync::atomic::Ordering::Relaxed);
            let lfo_dest  = state.lfo_dest.load(std::sync::atomic::Ordering::Relaxed);
            let lfo_dt    = lfo_rate / sr_f;
            let lfo2_rate  = state.lfo2_rate.value();
            let lfo2_depth = state.lfo2_depth.value();
            let lfo2_shape = state.lfo2_shape.load(std::sync::atomic::Ordering::Relaxed);
            let lfo2_dest  = state.lfo2_dest.load(std::sync::atomic::Ordering::Relaxed);
            let lfo2_dt    = lfo2_rate / sr_f;
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
                // --- LFO 1 & 2: advance phases and combine modulation ---
                lfo_phase += lfo_dt;
                if lfo_phase >= 1.0 { lfo_phase -= 1.0; }
                let lfo_raw = match lfo_shape {
                    1 => if lfo_phase < 0.5 { 4.0*lfo_phase-1.0 } else { 3.0-4.0*lfo_phase },
                    2 => 2.0 * lfo_phase - 1.0,
                    _ => (lfo_phase * std::f32::consts::TAU).sin(),
                };
                lfo2_phase += lfo2_dt;
                if lfo2_phase >= 1.0 { lfo2_phase -= 1.0; }
                let lfo2_raw = match lfo2_shape {
                    1 => if lfo2_phase < 0.5 { 4.0*lfo2_phase-1.0 } else { 3.0-4.0*lfo2_phase },
                    2 => 2.0 * lfo2_phase - 1.0,
                    _ => (lfo2_phase * std::f32::consts::TAU).sin(),
                };

                // Accumulate pitch, filter, and amp contributions from both LFOs
                let mut pitch_mod: f32 = 0.0;  // additive semitones * 2
                let mut filter_mod: f32 = 0.0; // additive cutoff multiplier
                let mut amp_mod: f32 = 1.0;    // multiplicative

                for (raw, depth, dest) in [
                    (lfo_raw, lfo_depth, lfo_dest),
                    (lfo2_raw, lfo2_depth, lfo2_dest),
                ] {
                    match dest {
                        0 => pitch_mod  += raw * depth,
                        2 => amp_mod    *= 1.0 - depth * (1.0 - raw) * 0.5,
                        _ => filter_mod += raw * depth,
                    }
                }

                state.lfo_pitch_mult.set(2_f32.powf(pitch_mod * 2.0 / 12.0));
                state.effective_cutoff.set(
                    (base_cutoff + filter_mod * base_cutoff * 0.5).clamp(80.0, 18000.0));
                let lfo_amp = amp_mod;

                // Retrigger countdown: hold gate=0 for N samples then flip to 1,
                // giving the ADSR a real 0→1 transition for a clean attack.
                for vi in 0..VOICE_COUNT {
                    if retrigger_countdown[vi] > 0 {
                        retrigger_countdown[vi] -= 1;
                        if retrigger_countdown[vi] == 0 {
                            state.voice_gates[vi].set(1.0);
                        }
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
