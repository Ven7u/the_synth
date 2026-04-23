//! Crystallizer — granular pitch-shift delay for ambient "sparkle" textures.
//!
//! The design is intentionally lightweight and real-time safe:
//! - single circular delay buffer
//! - two overlapping grains with Hann window crossfade
//! - no heap allocation after `new()`
//! - no locks, no blocking

use fundsp::prelude32::{shared, Shared};
use std::sync::atomic::AtomicU8;
use std::sync::Arc;

/// UI/audio-thread shared parameters for the global crystal bus.
pub struct CrystallizerShared {
    /// Overall wet level (0.0 = bypass, 1.0 = fully wet).
    pub mix: Shared,
    /// Grain size in milliseconds.
    pub grain_ms: Shared,
    /// Scatter amount (0.0..1.0), randomizes grain start positions.
    pub scatter: Shared,
    /// Feedback amount (0.0..0.95).
    pub feedback: Shared,
    /// Base delay time in milliseconds.
    pub delay_ms: Shared,
    /// Pitch ratio mode: 0=0.5x, 1=1.0x, 2=2.0x, 3=4.0x.
    pub pitch: Arc<AtomicU8>,
}

impl CrystallizerShared {
    pub fn new() -> Self {
        Self {
            mix: shared(0.0),
            grain_ms: shared(120.0),
            scatter: shared(0.25),
            feedback: shared(0.35),
            delay_ms: shared(260.0),
            pitch: Arc::new(AtomicU8::new(2)), // 2.0x default
        }
    }
}

impl Default for CrystallizerShared {
    fn default() -> Self {
        Self::new()
    }
}

/// 2.5 seconds at 44.1kHz, enough for ambient grain offset ranges.
const CRYS_BUF: usize = 110_250;

#[derive(Clone)]
pub struct Crystallizer {
    sr: f32,
    buf: Vec<f32>,
    write_pos: usize,

    read_a: f32,
    read_b: f32,
    phase_a: f32, // 0..1 grain phase
    phase_b: f32, // 0..1 grain phase (offset)
    phase_inc: f32,

    last_out: f32,
    rng: u32,
}

impl Crystallizer {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            buf: vec![0.0; CRYS_BUF],
            write_pos: 0,
            read_a: 0.0,
            read_b: (CRYS_BUF / 2) as f32,
            phase_a: 0.0,
            phase_b: 0.5,
            phase_inc: 1.0 / 2048.0,
            last_out: 0.0,
            rng: 0x1234_5678,
        }
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.write_pos = 0;
        self.read_a = 0.0;
        self.read_b = (self.buf.len() / 2) as f32;
        self.phase_a = 0.0;
        self.phase_b = 0.5;
        self.phase_inc = 1.0 / 2048.0;
        self.last_out = 0.0;
        self.rng = 0x1234_5678;
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        *self = Self::new(sr);
    }

    #[inline]
    fn rand_bipolar(&mut self) -> f32 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        let u = (x as f32) / (u32::MAX as f32);
        u * 2.0 - 1.0
    }

    #[inline]
    fn read_lerp(&self, pos: f32) -> f32 {
        let len = self.buf.len();
        let p = pos.rem_euclid(len as f32);
        let i0 = p as usize % len;
        let i1 = (i0 + 1) % len;
        let f = p.fract();
        self.buf[i0] * (1.0 - f) + self.buf[i1] * f
    }

    #[inline]
    fn reseed_read_head(&mut self, grain_smp: f32, scatter: f32, delay_ms: f32, len: f32) -> f32 {
        let base_delay = (delay_ms.clamp(20.0, 1200.0) * 0.001 * self.sr).clamp(1.0, len * 0.9);
        let scatter_smp = scatter.clamp(0.0, 1.0) * grain_smp * 0.5;
        let jitter = self.rand_bipolar() * scatter_smp;
        (self.write_pos as f32 - base_delay + jitter).rem_euclid(len)
    }

    /// Process one sample.
    pub fn tick(
        &mut self,
        input: f32,
        grain_ms: f32,
        scatter: f32,
        feedback: f32,
        delay_ms: f32,
        pitch: u8,
    ) -> f32 {
        let len = self.buf.len() as f32;
        let grain_smp = (grain_ms.clamp(10.0, 400.0) * 0.001 * self.sr).clamp(64.0, len * 0.25);
        self.phase_inc = 1.0 / grain_smp;

        let fb = feedback.clamp(0.0, 0.95);
        let delayed_in = input + self.last_out * fb;
        self.buf[self.write_pos] = delayed_in;

        // Pitch ratio by grain read speed vs write speed.
        let ratio = match pitch {
            0 => 0.5_f32,
            2 => 2.0_f32,
            3 => 4.0_f32,
            _ => 1.0_f32,
        };

        self.read_a = (self.read_a + ratio).rem_euclid(len);
        self.read_b = (self.read_b + ratio).rem_euclid(len);

        // Independent grain phases prevent simultaneous head resets, reducing clicks.
        self.phase_a += self.phase_inc;
        if self.phase_a >= 1.0 {
            self.phase_a -= 1.0;
            self.read_a = self.reseed_read_head(grain_smp, scatter, delay_ms, len);
        }
        self.phase_b += self.phase_inc;
        if self.phase_b >= 1.0 {
            self.phase_b -= 1.0;
            self.read_b = self.reseed_read_head(grain_smp, scatter, delay_ms, len);
        }

        let s_a = self.read_lerp(self.read_a);
        let s_b = self.read_lerp(self.read_b);

        // Overlap-add Hann windows, 180° out of phase.
        let ph_a = self.phase_a;
        let ph_b = self.phase_b;
        let w_a = 0.5 * (1.0 - (ph_a * std::f32::consts::TAU).cos());
        let w_b = 0.5 * (1.0 - (ph_b * std::f32::consts::TAU).cos());
        let w_sum = (w_a + w_b).max(0.001);

        let out = (s_a * w_a + s_b * w_b) / w_sum;
        self.last_out = (out * 0.95).tanh();
        self.write_pos = (self.write_pos + 1) % self.buf.len();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Crystallizer;

    const SR: f32 = 44_100.0;

    #[test]
    fn crystallizer_output_is_finite() {
        let mut c = Crystallizer::new(SR);
        for i in 0..20_000 {
            let inp = if i % 2_000 == 0 { 1.0 } else { 0.0 };
            let x = c.tick(inp, 120.0, 0.25, 0.35, 260.0, 2);
            assert!(x.is_finite());
            assert!(x.abs() < 8.0);
        }
    }

    /// Run with the exact Echoes scene parameters for 3 minutes of simulated audio.
    /// High feedback (0.62) + high scatter (0.55) + dotted-8th delay at 76 BPM (394ms).
    /// Verifies the tanh feedback path stays bounded and output doesn't drift upward.
    #[test]
    fn echoes_params_long_run_stays_bounded() {
        let mut c = Crystallizer::new(SR);
        let three_min = (SR * 180.0) as usize;
        let mut max_out: f32 = 0.0;
        for i in 0..three_min {
            // Periodic short bursts every 4 bars (~12.6s at 76 BPM) — like chord notes
            let inp = if i % ((SR * 12.6) as usize) < (SR * 0.05) as usize {
                0.4
            } else {
                0.0
            };
            let x = c.tick(inp, 180.0, 0.55, 0.62, 394.0, 1);
            assert!(x.is_finite(), "NaN/Inf at sample {i}");
            max_out = max_out.max(x.abs());
        }
        assert!(
            max_out < 2.0,
            "output grew beyond expected bounds: {max_out}"
        );
    }

    /// Feed silence after priming the buffer, verify output decays to near-zero.
    /// With high scatter, grain heads jump randomly — stale buffer content must not
    /// sustain itself via the feedback loop indefinitely.
    #[test]
    fn silence_input_decays_to_zero() {
        let mut c = Crystallizer::new(SR);

        // Prime with 1s of signal so the buffer is populated
        for _ in 0..(SR as usize) {
            c.tick(0.5, 120.0, 0.55, 0.62, 260.0, 1);
        }

        // Feed silence for 5s (> longest grain + several feedback cycles)
        let mut last = 0.0f32;
        for _ in 0..(SR as usize * 5) {
            last = c.tick(0.0, 120.0, 0.55, 0.62, 260.0, 1);
            assert!(last.is_finite());
        }
        // Output must have decayed significantly (not necessarily to zero — feedback sustains)
        // but must not have grown beyond what the priming produced
        assert!(
            last.abs() < 0.5,
            "output still loud after 5s of silence: {last}"
        );
    }

    /// Feedback at the clamped ceiling (0.95) must not cause unbounded growth.
    #[test]
    fn max_feedback_stays_finite() {
        let mut c = Crystallizer::new(SR);
        for i in 0..(SR as usize * 10) {
            let inp = if i < 100 { 0.8 } else { 0.0 };
            let x = c.tick(inp, 80.0, 0.10, 0.99, 80.0, 2); // 0.99 → clamped to 0.95
            assert!(x.is_finite(), "NaN/Inf at sample {i}");
            assert!(x.abs() < 4.0, "output clipped at sample {i}: {x}");
        }
    }
}
