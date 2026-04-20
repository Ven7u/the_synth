//! Offline DSP quality metrics harness.
//!
//! Runs each DSP component through standardised test signals and prints
//! pass/fail metric reports. Use in CI or locally to catch regressions.
//!
//! Usage:
//!   cargo run -p synth-bench              # run all suites
//!   cargo run -p synth-bench -- reverb    # run matching suites only

use synth_dsp::{ShimmerReverb, PeakLimiter};

fn main() {
    let filter = std::env::args().nth(1);
    let mut passed = 0usize;
    let mut failed = 0usize;

    let run_suite = |name: &str, body: &mut dyn FnMut(), passed: &mut usize, failed: &mut usize| {
        if filter.as_deref().map(|f| name.contains(f)).unwrap_or(true) {
            print!("  {:<40} ", name);
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body())) {
                Ok(()) => { println!("PASS"); *passed += 1; }
                Err(e) => {
                    let msg = e.downcast_ref::<String>()
                        .map(|s| s.as_str())
                        .or_else(|| e.downcast_ref::<&str>().copied())
                        .unwrap_or("panic");
                    println!("FAIL  {msg}");
                    *failed += 1;
                }
            }
        }
    };
    macro_rules! suite {
        ($name:expr, $body:expr) => {
            run_suite($name, &mut || { $body }, &mut passed, &mut failed);
        };
    }

    println!("\n=== synth-bench ===\n");

    // ── Reverb suites ──────────────────────────────────────────────────────
    println!("--- Reverb (ShimmerReverb) ---");

    for &(rev_type, type_name) in &[(0u8,"freeverb"), (1u8,"plate"), (2u8,"fdn")] {
        for &(room, damp, shimmer, pitch, label) in &[
            (0.5_f32, 0.5_f32, 0.0_f32, 0u8, "dry mid"),
            (1.0,     0.0,     0.0,      0,   "max room bright"),
            (1.0,     1.0,     0.0,      0,   "max room dark"),
            (0.95,    0.35,    0.85,     1,   "shimmer +12st"),
            (0.95,    0.35,    1.0,      2,   "shimmer +24st"),
        ] {
            let suite_name = format!("{type_name}/{label}");
            suite!(&suite_name, {
                check_reverb(rev_type, room, damp, shimmer, pitch);
            });
        }
    }

    // ── Limiter suites ─────────────────────────────────────────────────────
    println!("\n--- Limiter (PeakLimiter) ---");

    suite!("limiter/hot signal (4×) → ceiling", {
        check_limiter(4.0, 0.9, 1.0, "hot 4×");
    });
    suite!("limiter/silence passthrough", {
        check_limiter_silence();
    });
    suite!("limiter/below threshold passthrough", {
        check_limiter(0.5, 0.9, 0.55, "quiet signal");
    });

    // ── Oscillator suites ──────────────────────────────────────────────────
    println!("\n--- Oscillator (WaveShape) ---");
    use synth_dsp::osc::WaveShape;

    for &(shape, name) in &[
        (WaveShape::Sine,     "sine"),
        (WaveShape::Saw,      "saw"),
        (WaveShape::Square,   "square"),
        (WaveShape::Triangle, "triangle"),
    ] {
        let sr = 44100.0_f32;
        for &(freq, flabel) in &[(220.0_f32,"220Hz"), (1000.0,"1kHz"), (8000.0,"8kHz")] {
            let suite_name = format!("{name}/{flabel}");
            suite!(&suite_name, {
                let dt = freq / sr;
                let metrics = measure_waveform(|ph| shape.sample(ph, dt, 0.5), 60_000, dt);
                assert!(metrics.peak <= 1.05, "peak {:.3} > 1.05", metrics.peak);
                assert!(metrics.dc.abs() < 0.01, "|DC| {:.5} >= 0.01", metrics.dc);
                assert!(metrics.all_finite, "non-finite samples");
            });
        }
    }

    // ── Summary ───────────────────────────────────────────────────────────
    println!("\n{passed} passed, {failed} failed\n");
    if failed > 0 {
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Check helpers
// ---------------------------------------------------------------------------

fn check_reverb(rev_type: u8, room: f32, damp: f32, shimmer_amt: f32, pitch: u8) {
    let mut rev = ShimmerReverb::new(44_100.0);
    let metrics = measure_unit(|inp| rev.tick(inp, room, damp, shimmer_amt, pitch, rev_type), 90_000, 5_000);
    assert!(metrics.all_finite, "non-finite output");
    assert!(metrics.peak < 10.0, "peak {:.3} >= 10.0", metrics.peak);
    assert!(metrics.dc.abs() < 0.5, "|DC| {:.4} too large", metrics.dc);
}

fn check_limiter(drive: f32, threshold: f32, expected_ceil: f32, label: &str) {
    let mut lim = PeakLimiter::new(44_100.0, 0.1, 50.0);
    // Warm up: let the attack converge before measuring
    for _ in 0..500 { lim.process(drive, threshold); }
    let mut max_out = 0.0_f32;
    let mut all_finite = true;
    for i in 0..20_000 {
        let inp = if i % 100 == 0 { drive } else { drive * 0.5 };
        let y = lim.process(inp, threshold);
        if !y.is_finite() { all_finite = false; }
        max_out = max_out.max(y.abs());
    }
    assert!(all_finite, "{label}: non-finite output");
    assert!(max_out <= expected_ceil, "{label}: peak {max_out:.3} > {expected_ceil:.3}");
}

fn check_limiter_silence() {
    let mut lim = PeakLimiter::new(44_100.0, 0.1, 50.0);
    for _ in 0..10_000 {
        let y = lim.process(0.0, 0.9);
        assert!(y == 0.0, "silence not passed through: {y}");
    }
}

// ---------------------------------------------------------------------------
// Measurement primitives
// ---------------------------------------------------------------------------

struct Metrics {
    peak: f32,
    rms: f32,
    dc: f32,
    all_finite: bool,
}

/// Drive a mono DSP function with an impulse train and measure the output.
fn measure_unit<F: FnMut(f32) -> f32>(mut process: F, n: usize, period: usize) -> Metrics {
    let mut peak = 0.0_f32;
    let mut sum = 0.0_f32;
    let mut sum_sq = 0.0_f32;
    let mut all_finite = true;
    for i in 0..n {
        let inp = if i % period == 0 { 1.0_f32 } else { 0.0 };
        let y = process(inp);
        if !y.is_finite() { all_finite = false; continue; }
        peak = peak.max(y.abs());
        sum += y;
        sum_sq += y * y;
    }
    Metrics {
        peak,
        rms: (sum_sq / n as f32).sqrt(),
        dc: sum / n as f32,
        all_finite,
    }
}

/// Measure a stateless waveform sample function over N samples.
fn measure_waveform<F: FnMut(f32) -> f32>(mut sample_fn: F, n: usize, dt: f32) -> Metrics {
    let mut peak = 0.0_f32;
    let mut sum = 0.0_f32;
    let mut sum_sq = 0.0_f32;
    let mut all_finite = true;
    let mut phase = 0.0_f32;
    for _ in 0..n {
        let y = sample_fn(phase);
        if !y.is_finite() { all_finite = false; }
        peak = peak.max(y.abs());
        sum += y;
        sum_sq += y * y;
        phase = (phase + dt).fract();
    }
    Metrics {
        peak,
        rms: (sum_sq / n as f32).sqrt(),
        dc: sum / n as f32,
        all_finite,
    }
}
