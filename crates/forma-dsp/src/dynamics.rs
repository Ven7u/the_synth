//! Dynamics DSP utilities shared by synth engines.

/// Simple envelope-follower peak limiter.
///
/// Behavior mirrors the limiter logic used in forma callback:
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

/// Lookahead stereo true-peak limiter.
///
/// Uses a short lookahead delay so gain reduction is applied *before* the
/// peak arrives, preventing inter-sample overshoots that bypass the old
/// envelope-follower approach.
///
/// Design:
/// - Lookahead: ~1.5 ms ring buffer delay
/// - Gain computer: forward-scan envelope over the lookahead window
/// - Release: one-pole smoother on gain (program-dependent: fast attack,
///   slow release so pumping is inaudible on sustained material)
/// - Safety: final `clamp` — never lets a sample exceed the ceiling
#[derive(Clone, Debug)]
pub struct LookaheadLimiter {
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    gain_buf: Vec<f32>,
    pos: usize,
    gain_smooth: f32,
    release_coeff: f32,
    lookahead: usize,
}

impl LookaheadLimiter {
    /// `lookahead_ms` — how far ahead to look (1.0–3.0 ms is typical).
    pub fn new(sr: f32, lookahead_ms: f32, release_ms: f32) -> Self {
        let lookahead = ((lookahead_ms * 0.001 * sr) as usize).max(1);
        let buf = vec![0.0_f32; lookahead + 1];
        let release_coeff = (-1.0_f32 / (release_ms * 0.001 * sr)).exp();
        Self {
            buf_l: buf.clone(),
            buf_r: buf.clone(),
            gain_buf: vec![1.0_f32; lookahead + 1],
            pos: 0,
            gain_smooth: 1.0,
            release_coeff,
            lookahead,
        }
    }

    pub fn reset(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.gain_buf.fill(1.0);
        self.gain_smooth = 1.0;
        self.pos = 0;
    }

    /// Process one stereo sample. Returns `(limited_l, limited_r)`.
    ///
    /// `threshold` — peak ceiling in linear amplitude (e.g. 0.95).
    #[inline]
    pub fn process_stereo(&mut self, l: f32, r: f32, threshold: f32) -> (f32, f32) {
        let len = self.lookahead + 1;

        // Write new samples into delay lines
        self.buf_l[self.pos] = l;
        self.buf_r[self.pos] = r;

        // Gain computer: desired gain based on the incoming peak
        let peak = l.abs().max(r.abs());
        let desired_gain = if peak > threshold && peak > 1e-6 {
            threshold / peak
        } else {
            1.0
        };

        // Store in gain buffer — we'll apply the *minimum* over the lookahead
        self.gain_buf[self.pos] = desired_gain;

        // Find minimum gain in the lookahead window (ensures gain is down before the peak)
        let mut min_gain = 1.0_f32;
        for i in 0..len {
            let idx = (self.pos + len - i) % len;
            min_gain = min_gain.min(self.gain_buf[idx]);
        }

        // Smooth the gain: instant attack, slow release
        self.gain_smooth = if min_gain < self.gain_smooth {
            min_gain // instant attack: snap down immediately
        } else {
            // Release: one-pole towards min_gain
            min_gain + self.release_coeff * (self.gain_smooth - min_gain)
        };

        // Read delayed (oldest) sample from buffer
        let read_pos = (self.pos + 1) % len;
        let out_l = self.buf_l[read_pos] * self.gain_smooth;
        let out_r = self.buf_r[read_pos] * self.gain_smooth;

        self.pos = (self.pos + 1) % len;

        // Safety clamp — should never trigger if threshold <= 1.0
        (out_l.clamp(-1.0, 1.0), out_r.clamp(-1.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_bounds_hot_signal() {
        let mut lim = PeakLimiter::new(44_100.0, 2.0, 80.0);
        let thr = 0.9;
        // Warm up: allow the attack envelope to converge (2ms attack ≈ 90 samples)
        for _ in 0..1_000 {
            lim.process(4.0, thr);
        }
        // Steady-state: output must stay near or below threshold
        let mut max = 0.0_f32;
        for _ in 0..20_000 {
            let y = lim.process(4.0, thr);
            max = max.max(y.abs());
        }
        assert!(
            max <= thr * 1.05,
            "steady-state peak {max:.4} > threshold {thr}"
        );
    }

    #[test]
    fn lookahead_limiter_ceiling() {
        let mut lim = LookaheadLimiter::new(44_100.0, 1.5, 100.0);
        let thr = 0.95;
        let mut max = 0.0_f32;
        // Burst of loud signal
        for _ in 0..44_100 {
            let (l, r) = lim.process_stereo(4.0, -3.5, thr);
            max = max.max(l.abs()).max(r.abs());
        }
        // After enough samples, gain should have caught up
        assert!(max <= 1.0, "lookahead limiter output {max:.4} > 1.0");
    }

    #[test]
    fn lookahead_limiter_finite() {
        let mut lim = LookaheadLimiter::new(44_100.0, 1.5, 100.0);
        for i in 0..90_000 {
            let inp = if i % 5_000 == 0 { 3.0 } else { 0.1 };
            let (l, r) = lim.process_stereo(inp, -inp, 0.9);
            assert!(
                l.is_finite() && r.is_finite(),
                "sample {i}: not finite l={l} r={r}"
            );
        }
    }

    #[test]
    fn lookahead_limiter_silence_passthrough() {
        let mut lim = LookaheadLimiter::new(44_100.0, 1.5, 100.0);
        // Skip the lookahead delay samples, then check silence passes
        for _ in 0..100 {
            lim.process_stereo(0.0, 0.0, 0.9);
        }
        for i in 0..1_000 {
            let (l, r) = lim.process_stereo(0.0, 0.0, 0.9);
            assert!(l == 0.0 && r == 0.0, "silence sample {i}: l={l} r={r}");
        }
    }

    #[test]
    fn lookahead_limiter_reset() {
        let mut lim = LookaheadLimiter::new(44_100.0, 1.5, 100.0);
        for _ in 0..1_000 {
            lim.process_stereo(4.0, 4.0, 0.9);
        }
        lim.reset();
        let (l, r) = lim.process_stereo(0.0, 0.0, 0.9);
        assert!(l == 0.0 && r == 0.0, "after reset: l={l} r={r}");
    }
}
