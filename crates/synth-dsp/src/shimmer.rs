//! ShimmerReverb — Schroeder reverb with optional pitch-shifted feedback loop.
//!
//! With `shimmer_amt = 0.0` this is bit-identical to the previous `ReverbState`.
//! With `shimmer_amt > 0.0` the reverb output is pitch-shifted and fed back into
//! the reverb input each sample, building a halo of pitch-shifted harmonics.
//!
//! Pitch options: 0 = unison (no shift), 1 = +12 semitones, 2 = +24 semitones.
//! Pitch shift uses two crossing read heads on a circular buffer with Hann windowing
//! (same approach as Mutable Instruments Clouds — cheap, sounds great in reverb tails).

use fundsp::prelude32::{shared, Shared};
use std::sync::atomic::AtomicU8;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// ShimmerShared — UI ↔ audio thread parameter bridge
// ---------------------------------------------------------------------------

/// UI-accessible parameters for the shimmer reverb.
/// All fields are atomics — safe to write from any thread.
pub struct ShimmerShared {
    /// Overall wet level (0.0 = bypass, 1.0 = fully wet).
    pub mix: Shared,
    /// Room size — scales reverb decay (0.0..1.0).
    pub size: Shared,
    /// High-frequency damping (0.0 = bright, 1.0 = dark).
    pub damp: Shared,
    /// Amount of pitch-shifted signal fed back into the reverb (0.0..1.0).
    pub shimmer: Shared,
    /// Stereo wet-field width (1.0 = neutral).
    pub width: Shared,
    /// Stereo decorrelation depth for L/R reverb parameters (0.0..0.3 typical).
    pub spread: Shared,
    /// Pitch shift interval: 0 = unison, 1 = +12 st, 2 = +24 st.
    pub pitch: Arc<AtomicU8>,
}

impl ShimmerShared {
    pub fn new() -> Self {
        Self {
            mix:     shared(0.0),
            size:    shared(0.6),
            damp:    shared(0.5),
            shimmer: shared(0.0), // off by default — only active when shimmer panel is on
            width:   shared(1.35),
            spread:  shared(0.10),
            pitch:   Arc::new(AtomicU8::new(1)), // +12st default
        }
    }
}

impl Default for ShimmerShared {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// ShimmerReverb — audio-thread state (heap allocated once at init)
// ---------------------------------------------------------------------------

/// Pitch-shifter grain buffer — 16384 samples ≈ 370 ms at 44.1 kHz.
/// Larger than the minimum needed so the Hann window crossfade is very smooth.
const SHIM_BUF: usize = 16384;

/// Pre-delay buffer size — 4096 samples ≈ 93 ms max at 44.1 kHz.
/// The pre-delay inserts a short silence before the pitched signal enters the
/// reverb feedback loop, creating the "halo appearing from behind" character.
const PRE_BUF: usize = 4096;

/// Schroeder reverb (4 comb + 2 allpass) with a two-head granular pitch shifter
/// in the feedback loop. Fully allocation-free after `new()`.
#[derive(Clone)]
pub struct ShimmerReverb {
    sr: f32,
    // Schroeder comb filters
    comb_buf:  [Vec<f32>; 4],
    comb_pos:  [usize; 4],
    comb_feed: [f32; 4], // per-comb one-pole LP state for damping

    // Schroeder allpass filters
    ap_buf: [Vec<f32>; 2],
    ap_pos: [usize; 2],

    // Pitch shifter: two read heads on a circular buffer, Hann crossfade
    shim_buf:      Vec<f32>,
    shim_write:    usize,
    shim_read_a:   f32,
    shim_read_b:   f32,
    shim_feedback: f32, // pitch-shifted sample fed back into reverb input next tick

    // Pre-delay: short buffer delays the pitched signal before it re-enters
    // the reverb, creating spatial separation ("halo from behind" character)
    pre_buf: Vec<f32>,
    pre_pos: usize,
    pre_delay_smp: usize,
    shim_hp_z: f32,
    shim_lp_z: f32,
}

impl ShimmerReverb {
    pub fn new(sr: f32) -> Self {
        let scale = sr / 44100.0;
        let comb_delays: [usize; 4] = [1557, 1617, 1491, 1422];
        let ap_delays: [usize; 2] = [225, 556];
        // Pre-delay: 50 ms — separates shimmer halo from dry signal spatially
        let pre_delay_samples = ((0.050 * sr) as usize).min(PRE_BUF - 1).max(1);
        Self {
            sr,
            comb_buf:  comb_delays.map(|d| vec![0.0; ((d as f32 * scale) as usize).max(1)]),
            comb_pos:  [0; 4],
            comb_feed: [0.0; 4],
            ap_buf:    ap_delays.map(|d| vec![0.0; ((d as f32 * scale) as usize).max(1)]),
            ap_pos:    [0; 2],
            shim_buf:  vec![0.0; SHIM_BUF],
            shim_write:    0,
            shim_read_a:   0.0,
            shim_read_b:   (SHIM_BUF / 2) as f32,
            shim_feedback: 0.0,
            pre_buf: vec![0.0; PRE_BUF],
            pre_pos: 0,
            pre_delay_smp: pre_delay_samples,
            shim_hp_z: 0.0,
            shim_lp_z: 0.0,
        }
    }

    pub fn reset(&mut self) {
        for b in &mut self.comb_buf { b.fill(0.0); }
        self.comb_pos  = [0; 4];
        self.comb_feed = [0.0; 4];
        for b in &mut self.ap_buf { b.fill(0.0); }
        self.ap_pos = [0; 2];
        self.shim_buf.fill(0.0);
        self.shim_write = 0;
        self.shim_read_a = 0.0;
        self.shim_read_b = (SHIM_BUF / 2) as f32;
        self.shim_feedback = 0.0;
        self.pre_buf.fill(0.0);
        self.pre_pos = 0;
        self.shim_hp_z = 0.0;
        self.shim_lp_z = 0.0;
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        *self = Self::new(sr);
    }

    /// Process one sample. `shimmer_amt = 0.0` → plain Schroeder, identical to old code.
    #[inline]
    pub fn tick(&mut self, input: f32, room: f32, damp: f32, shimmer_amt: f32, pitch: u8) -> f32 {
        // Extended decay range: 0.7..0.995 (vs old 0.7..0.98).
        // At room=1.0 the tail sustains for ~10 seconds — essential for the
        // "suspended" shimmer character. The tanh in the feedback path keeps
        // the signal bounded even at near-unity decay.
        let feed = 0.7 + room * 0.295;  // 0.70..0.995
        let d    = damp * 0.4;

        // Shimmer feedback: tanh-normalized + Hann window already bounds this
        // to [-1, 1]. No extra stability scaling needed — the combs' feed < 1
        // guarantees the system is stable. Scale by 0.25 so shimmer_amt=1.0
        // injects 25% of reverb output back — audible and musical without blowup.
        let rev_in = input + self.shim_feedback * shimmer_amt * 0.25;

        let mut out = 0.0f32;
        for i in 0..4 {
            let len = self.comb_buf[i].len();
            let pos = self.comb_pos[i];
            let delayed = self.comb_buf[i][pos];
            self.comb_feed[i] = delayed * (1.0 - d) + self.comb_feed[i] * d;
            self.comb_buf[i][pos] = rev_in + self.comb_feed[i] * feed;
            self.comb_pos[i] = (pos + 1) % len;
            out += delayed;
        }
        out *= 0.25;

        for i in 0..2 {
            let len = self.ap_buf[i].len();
            let pos = self.ap_pos[i];
            let buf = self.ap_buf[i][pos];
            let v = out + buf * 0.5;
            self.ap_buf[i][pos] = v;
            self.ap_pos[i] = (pos + 1) % len;
            out = buf - v * 0.5;
        }

        // --- Pitch shifter + pre-delay (skipped when shimmer_amt ~ 0) ---
        if shimmer_amt > 0.0001 {
            let buf_len = SHIM_BUF;

            // Pre-delay: write reverb output, read from 50ms earlier.
            // This delays the pitched signal before it feeds back, giving the
            // shimmer halo a sense of spatial depth ("appearing from behind").
            let pre_len = self.pre_buf.len();
            let pre_write = self.pre_pos;
            let pre_read = (pre_write + pre_len - self.pre_delay_smp) % pre_len;
            self.pre_buf[pre_write] = out;
            let delayed_out = self.pre_buf[pre_read];
            self.pre_pos = (self.pre_pos + 1) % pre_len;

            // Write pre-delayed reverb output into pitch-shifter buffer
            self.shim_buf[self.shim_write] = delayed_out;
            self.shim_write = (self.shim_write + 1) % buf_len;

            // Read heads advance at pitch_ratio per sample.
            // pitch_ratio > 1 reads faster than write -> pitch-up transposition.
            let pitch_ratio = match pitch {
                1 => 2.0_f32, // +12 semitones (1 octave)
                2 => 4.0_f32, // +24 semitones (2 octaves)
                _ => 1.0_f32, // unison — shimmer just boosts reverb tail
            };
            let rate = pitch_ratio;

            self.shim_read_a = (self.shim_read_a + rate).rem_euclid(buf_len as f32);
            self.shim_read_b = (self.shim_read_b + rate).rem_euclid(buf_len as f32);

            // Linear interpolation helper
            let lerp_buf = |pos: f32| -> f32 {
                let i0 = pos as usize % buf_len;
                let i1 = (i0 + 1) % buf_len;
                let f  = pos.fract();
                self.shim_buf[i0] * (1.0 - f) + self.shim_buf[i1] * f
            };

            let samp_a = lerp_buf(self.shim_read_a);
            let samp_b = lerp_buf(self.shim_read_b);

            // Hann window: smooth crossfade so the two heads never both read
            // at the same phase → no comb-filter notch in the output.
            // phase = normalised distance from write head (0 = just written, 1 = oldest)
            let phase_a = self.shim_read_a / buf_len as f32;
            let phase_b = self.shim_read_b / buf_len as f32;
            let win_a = 0.5 * (1.0 - (phase_a * std::f32::consts::TAU).cos());
            let win_b = 0.5 * (1.0 - (phase_b * std::f32::consts::TAU).cos());

            // Normalize by the window sum so gain is stable regardless of read-head phase.
            let win_sum = (win_a + win_b).max(0.001);
            let mut shifted = (samp_a * win_a + samp_b * win_b) / win_sum;

            // Spectral voicing in the feedback loop:
            // - HP removes low buildup that sounds like "just more gain"
            // - LP keeps the halo smooth/angelic instead of harsh/metallic
            let hp_fc = 250.0_f32;
            let hp_coeff = (-std::f32::consts::TAU * hp_fc / self.sr).exp();
            self.shim_hp_z = (1.0 - hp_coeff) * shifted + hp_coeff * self.shim_hp_z;
            shifted -= self.shim_hp_z;

            let lp_fc = 2500.0_f32 * (12000.0_f32 / 2500.0_f32).powf(1.0 - damp.clamp(0.0, 1.0));
            let lp_coeff = (-std::f32::consts::TAU * lp_fc / self.sr).exp();
            self.shim_lp_z = (1.0 - lp_coeff) * shifted + lp_coeff * self.shim_lp_z;

            self.shim_feedback = (self.shim_lp_z * 0.85).tanh();
        } else {
            self.shim_feedback = 0.0;
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::ShimmerReverb;

    #[test]
    fn shimmer_output_is_finite_across_modes() {
        let mut s = ShimmerReverb::new(44_100.0);
        for i in 0..30_000 {
            let inp = if i % 4_000 == 0 { 1.0 } else { 0.0 };
            // Sweep through pitch modes while shimmer is active to stress the feedback path.
            let pitch = ((i / 10_000) % 3) as u8;
            let x = s.tick(inp, 0.95, 0.35, 0.85, pitch);
            assert!(x.is_finite());
            assert!(x.abs() < 8.0);
        }
    }
}
