pub mod crystallizer;
pub mod dynamics;
pub mod envelope;
pub mod osc;
pub mod shimmer;
pub use crystallizer::{Crystallizer, CrystallizerShared};
pub use dynamics::{LookaheadLimiter, PeakLimiter};
pub use shimmer::{ShimmerReverb, ShimmerShared};

/// Shared stability assertion for DSP unit tests.
///
/// Drives `process` with an impulse train (1.0 every `impulse_period` samples,
/// 0.0 otherwise) for `n_samples` and asserts every output is finite and
/// within `max_amp`. Call from any `#[cfg(test)]` block in this crate.
#[cfg(test)]
pub fn assert_dsp_stable<F: FnMut(f32) -> f32>(
    mut process: F,
    n_samples: usize,
    impulse_period: usize,
    max_amp: f32,
    label: &str,
) {
    for i in 0..n_samples {
        let inp = if i % impulse_period == 0 { 1.0 } else { 0.0 };
        let y = process(inp);
        assert!(y.is_finite(), "{label}: sample {i} not finite (NaN/Inf)");
        assert!(y.abs() < max_amp, "{label}: sample {i} too loud: {y:.3}");
    }
}
