//! Realtime CPU benchmark harness for the-synth.
//!
//! Renders N seconds of audio for a set of representative scenarios and
//! reports the **realtime factor** (RTF = audio_seconds / wall_seconds).
//! Higher is better; RTF ≥ 100× is a comfortable margin for DAW / game-engine
//! use, RTF < 10× is stressful, RTF < 1× means the engine can't keep up.
//!
//! Scenarios exercise realistic synth states so fixes can be compared
//! apples-to-apples. The harness mirrors the work the cpal callback does:
//! `VoiceAllocator::begin_buffer` + per-sample `tick_sample` + graph render +
//! lookahead limiter + `tanh` soft-clip + peak metering. It does NOT include
//! the scope-buffer mutex, recorder, latency measurement — those are UI
//! bookkeeping, not audio work.
//!
//! Run:
//!
//! ```text
//!   cargo run -p synth-bench --bin synth-perf --release
//! ```
//!
//! Build with `--release`. Measuring debug builds is meaningless for DSP.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use fundsp::prelude32::*;
use synth_control::{make_control_channel, ControlReceiver};
use synth_dsp::LookaheadLimiter;
use synth_engine::audio::{build_synth_graph, AudioState, VOICE_COUNT};
use synth_engine::{enable_ftz_on_current_thread, SynthEngineHandle, VoiceAllocator};

const SR: f64 = 48_000.0;
const SR_F: f32 = 48_000.0;
const BLOCK_SIZE: usize = 256;
/// Samples of warmup before timing starts — lets caches populate, voice
/// envelopes settle, FX tails wake up.
const WARMUP_SECS: f32 = 0.5;
/// Measured audio duration per scenario.
const MEASURE_SECS: f32 = 10.0;

// ---------------------------------------------------------------------------
// Scenario runner
// ---------------------------------------------------------------------------

struct ScenarioResult {
    name:       &'static str,
    rtf:        f32,
    wall_ms:    f32,
    us_per_buf: f32,
    voices_on:  usize,
}

fn bench(name: &'static str, setup: impl FnOnce(&SynthEngineHandle)) -> ScenarioResult {
    let state = Arc::new(AudioState::new());
    let (tx, rx) = make_control_channel(4096);
    let handle = SynthEngineHandle::new(Arc::clone(&state), tx);

    setup(&handle);

    // Let the VoiceAllocator drain any event queue from setup.
    let mut voices = VoiceAllocator::new();
    voices.begin_buffer(&state, &rx, 0, SR);

    let mut graph = BlockRateAdapter::new(build_synth_graph(&state, SR));
    let mut lim   = LookaheadLimiter::new(SR_F, 1.5, 80.0);

    // Warm up.
    let warmup_samples = (SR_F * WARMUP_SECS) as usize;
    render_chunk(&state, &rx, &mut voices, &mut graph, &mut lim, warmup_samples);

    // Measure.
    let samples = (SR_F * MEASURE_SECS) as usize;
    let start = Instant::now();
    render_chunk(&state, &rx, &mut voices, &mut graph, &mut lim, samples);
    let wall = start.elapsed();

    let wall_ms = wall.as_secs_f32() * 1000.0;
    let audio_ms = MEASURE_SECS * 1000.0;
    let rtf = audio_ms / wall_ms;
    let buffers = (samples + BLOCK_SIZE - 1) / BLOCK_SIZE;
    let us_per_buf = wall.as_secs_f64() as f32 * 1_000_000.0 / buffers as f32;

    let voices_on = (0..VOICE_COUNT)
        .filter(|&i| state.voice_gates[i].value() > 0.5 || state.amp_cursors[i].value() > 0.5)
        .count();

    ScenarioResult { name, rtf, wall_ms, us_per_buf, voices_on }
}

fn render_chunk(
    state:  &AudioState,
    rx:     &ControlReceiver,
    voices: &mut VoiceAllocator,
    graph:  &mut BlockRateAdapter,
    lim:    &mut LookaheadLimiter,
    total_samples: usize,
) {
    // Callback-shaped state.
    let mut lfo_phase:  f32 = 0.0;
    let mut lfo2_phase: f32 = 0.25;
    let mut peak_l:     f32 = 0.0;
    let mut peak_r:     f32 = 0.0;
    let mut smoothed_freqs: [f32; VOICE_COUNT] = [440.0; VOICE_COUNT];

    let mut done = 0;
    while done < total_samples {
        let frames = std::cmp::min(BLOCK_SIZE, total_samples - done);

        voices.begin_buffer(state, rx, frames, SR);

        // Per-buffer atomic reads (mirror of the real callback).
        let lfo_rate  = state.lfo_rate.value();
        let lfo_depth = state.lfo_depth.value();
        let lfo_shape = state.lfo_shape.load(std::sync::atomic::Ordering::Relaxed);
        let lfo_dest  = state.lfo_dest.load(std::sync::atomic::Ordering::Relaxed);
        let lfo_dt    = lfo_rate / SR_F;
        let lfo2_rate  = state.lfo2_rate.value();
        let lfo2_depth = state.lfo2_depth.value();
        let lfo2_shape = state.lfo2_shape.load(std::sync::atomic::Ordering::Relaxed);
        let lfo2_dest  = state.lfo2_dest.load(std::sync::atomic::Ordering::Relaxed);
        let lfo2_dt    = lfo2_rate / SR_F;
        let base_cutoff = state.cutoff.value().clamp(80.0, 18_000.0);
        let threshold = state.limiter_threshold.value();
        let limiter_on = state.limiter_enabled.load(std::sync::atomic::Ordering::Relaxed);

        // Glide smoothing (once per buffer).
        let glide_time = state.glide_time.value();
        for vi in 0..VOICE_COUNT {
            let target = state.voice_freq_targets[vi].value();
            if glide_time < 0.001 {
                smoothed_freqs[vi] = target;
            } else {
                let coeff = (-(frames as f32) / (glide_time * SR_F)).exp();
                smoothed_freqs[vi] = coeff * smoothed_freqs[vi] + (1.0 - coeff) * target;
            }
            state.voice_freqs[vi].set(smoothed_freqs[vi]);
        }

        for _ in 0..frames {
            // LFO 1 & 2 phase advance + waveform + mod routing.
            lfo_phase += lfo_dt;
            if lfo_phase >= 1.0 { lfo_phase -= 1.0; }
            let lfo_raw = match lfo_shape {
                1 => if lfo_phase < 0.5 { 4.0 * lfo_phase - 1.0 } else { 3.0 - 4.0 * lfo_phase },
                2 => 2.0 * lfo_phase - 1.0,
                _ => (lfo_phase * std::f32::consts::TAU).sin(),
            };
            lfo2_phase += lfo2_dt;
            if lfo2_phase >= 1.0 { lfo2_phase -= 1.0; }
            let lfo2_raw = match lfo2_shape {
                1 => if lfo2_phase < 0.5 { 4.0 * lfo2_phase - 1.0 } else { 3.0 - 4.0 * lfo2_phase },
                2 => 2.0 * lfo2_phase - 1.0,
                _ => (lfo2_phase * std::f32::consts::TAU).sin(),
            };

            let mut pitch_mod: f32 = 0.0;
            let mut filter_mod: f32 = 0.0;
            let mut amp_mod: f32 = 1.0;
            for (raw, depth, dest) in [
                (lfo_raw,  lfo_depth,  lfo_dest),
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
                (base_cutoff + filter_mod * base_cutoff * 0.5).clamp(80.0, 18_000.0));
            let lfo_amp = amp_mod;

            voices.tick_sample(state);

            let (raw_l, raw_r) = graph.get_stereo();

            let (mut out_l, mut out_r) = (raw_l, raw_r);
            if limiter_on {
                let (l, r) = lim.process_stereo(out_l, out_r, threshold);
                out_l = l;
                out_r = r;
            }
            let out_l = if out_l.is_finite() { out_l.tanh() } else { 0.0 } * lfo_amp;
            let out_r = if out_r.is_finite() { out_r.tanh() } else { 0.0 } * lfo_amp;

            if out_l.abs() > peak_l { peak_l = out_l.abs(); }
            if out_r.abs() > peak_r { peak_r = out_r.abs(); }

            // Prevent LLVM from optimising the whole loop away.
            black_box((out_l, out_r));
        }

        done += frames;
    }

    // Consume peaks so nothing is dead-stripped.
    state.peak_l.store(peak_l.to_bits(), std::sync::atomic::Ordering::Relaxed);
    state.peak_r.store(peak_r.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

fn scenarios() -> Vec<ScenarioResult> {
    let mut out = Vec::new();

    out.push(bench("idle (no notes, default FX)", |_| {}));

    out.push(bench("1 note held (A4, clean)", |h| {
        h.note_on(69, 100);
    }));

    out.push(bench("6 notes held (chord, clean)", |h| {
        for p in [48, 52, 55, 60, 64, 67] { h.note_on(p, 100); }
    }));

    out.push(bench("6 notes + mod FX (chorus + delay + reverb)", |h| {
        for p in [48, 52, 55, 60, 64, 67] { h.note_on(p, 100); }
        h.set_fx_chorus_mix(0.35);
        h.set_fx_delay_mix(0.35);
        h.set_fx_reverb_mix(0.4);
    }));

    out.push(bench("6 notes + all FX heavy (+ shimmer + crystal)", |h| {
        for p in [48, 52, 55, 60, 64, 67] { h.note_on(p, 100); }
        h.set_fx_overdrive_mix(0.5);
        h.set_fx_distortion_mix(0.3);
        h.set_fx_chorus_mix(0.5);
        h.set_fx_delay_mix(0.4);
        h.set_fx_reverb_mix(0.5);
        h.set_shimmer_mix(0.5);
        h.set_shimmer_amount(0.6);
        h.set_crystal_mix(0.4);
    }));

    out.push(bench("6 notes + unison 5× on all 3 osc", |h| {
        for p in [48, 52, 55, 60, 64, 67] { h.note_on(p, 100); }
        // Activate all 5 unison copies per oscillator.
        for osc in 0..3 {
            for c in 0..5 {
                h.set_osc_unison_detune(osc, c, 1.0 + (c as f32 - 2.0) * 0.003);
                h.set_osc_unison_vol(osc, c, 0.2);
            }
        }
    }));

    out
}

// ---------------------------------------------------------------------------
// Pretty-printer
// ---------------------------------------------------------------------------

fn print_header() {
    let arch = std::env::consts::ARCH;
    let os   = std::env::consts::OS;
    let profile = if cfg!(debug_assertions) { "DEBUG" } else { "release" };
    println!();
    println!("=== synth-perf — realtime CPU benchmark ===");
    println!("sample rate {} Hz, block size {}, measure {:.1}s/scenario, warmup {:.1}s",
        SR as u32, BLOCK_SIZE, MEASURE_SECS, WARMUP_SECS);
    println!("target {os}/{arch}, profile {profile}");
    println!();

    if cfg!(debug_assertions) {
        println!("!!!  WARNING: built without --release. Numbers are meaningless.  !!!");
        println!();
    }

    println!("{:<52} {:>8}  {:>10}  {:>12}  {:>6}",
        "scenario", "RTF", "wall (ms)", "µs/buffer", "voices");
    println!("{}", "-".repeat(100));
}

fn print_row(r: &ScenarioResult) {
    let flag = if r.rtf >= 100.0 { "✓" }
        else if r.rtf >= 10.0 { "·" }
        else if r.rtf >= 1.0 { "!" }
        else { "✗" };
    println!("{flag} {:<50} {:>6.1}×  {:>10.2}  {:>12.2}  {:>6}",
        r.name, r.rtf, r.wall_ms, r.us_per_buf, r.voices_on);
}

fn print_legend() {
    println!();
    println!("RTF = audio-seconds / wall-seconds. Higher is better.");
    println!("  ✓ ≥ 100×   comfortable (≥ 1% CPU budget used)");
    println!("  ·  ≥  10×   acceptable (≤ 10% CPU budget used)");
    println!("  !  ≥   1×   realtime — no headroom");
    println!("  ✗  <   1×   cannot keep up — dropouts");
    println!();
    println!("µs/buffer is the wall-clock cost of rendering one {BLOCK_SIZE}-sample buffer.");
    println!("At 48 kHz that buffer is {:.2} ms of audio.",
        BLOCK_SIZE as f32 / SR_F * 1000.0);
}

fn main() {
    // Match the audio-thread CPU mode. Must run before any DSP so reverb
    // tails / feedback loops don't slip into subnormal range mid-bench.
    enable_ftz_on_current_thread();

    print_header();
    for r in scenarios() {
        print_row(&r);
    }
    print_legend();
}
