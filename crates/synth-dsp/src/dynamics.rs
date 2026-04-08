//! Dynamics DSP utilities shared by synth engines.

/// Simple envelope-follower peak limiter.
///
/// Behavior mirrors the limiter logic used in the_synth callback:
/// - fast attack, slower release envelope
/// - gain reduction only when envelope exceeds threshold
#[derive(Clone, Debug)]
pub struct PeakLimiter {
    env: f32,
    attack_coeff: f32,
    release_coeff: f32,
}

impl PeakLimiter {
    /// Build limiter coefficients from sample rate and time constants.
    pub fn new(sr: f32, attack_ms: f32, release_ms: f32) -> Self {
        let atk_s = (attack_ms * 0.001).max(0.000_01);
        let rel_s = (release_ms * 0.001).max(0.000_01);
        let attack_coeff = (-1.0_f32 / (atk_s * sr)).exp();
        let release_coeff = (-1.0_f32 / (rel_s * sr)).exp();
        Self {
            env: 0.0,
            attack_coeff,
            release_coeff,
        }
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
    }

    /// Apply limiting to one sample.
    ///
    /// `threshold` should typically be in `0.5..=1.0`.
    #[inline]
    pub fn process(&mut self, sample: f32, threshold: f32) -> f32 {
        let abs = sample.abs();
        self.env = if abs > self.env {
            self.attack_coeff * self.env + (1.0 - self.attack_coeff) * abs
        } else {
            self.release_coeff * self.env + (1.0 - self.release_coeff) * abs
        };

        if self.env > threshold && self.env > 0.000_001 {
            sample * (threshold / self.env)
        } else {
            sample
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PeakLimiter;

    #[test]
    fn limiter_bounds_hot_signal() {
        let mut lim = PeakLimiter::new(44_100.0, 2.0, 80.0);
        let thr = 0.9;
        let mut max = 0.0_f32;
        for _ in 0..20_000 {
            let y = lim.process(4.0, thr);
            max = max.max(y.abs());
        }
        assert!(max <= 1.1);
    }
}
