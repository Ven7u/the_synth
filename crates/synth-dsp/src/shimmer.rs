//! ShimmerReverb — three reverb algorithms with LFO-modulated delay lines and
//! optional pitch-shifted feedback loop.
//!
//! rev_type = 0: Freeverb — 8 modulated comb filters + 4 allpass diffusers.
//! rev_type = 1: Plate (Dattorro-inspired) — 4 input diffusers, modulated allpass,
//!              two modulated tank delays with cross-damping.
//! rev_type = 2: FDN Hall — 8 modulated delay lines + 8×8 Hadamard feedback matrix.
//!
//! All delay lines in all algorithms are modulated by slow LFOs (0.3–1.3 Hz,
//! depth ≈ 0.25 ms). This smears out the static standing-wave patterns that
//! fixed delays create at high feedback, eliminating the "metallic ring" that
//! plagues simple Schroeder/Freeverb designs at cinematic decay settings.
//!
//! With `shimmer_amt > 0.0` any algorithm gets pitch-shifted feedback.

use fundsp::prelude32::{shared, Shared};
use std::sync::atomic::AtomicU8;
use std::sync::Arc;

/// Replace non-finite values (NaN, ±Inf) with 0.0 before writing to circular
/// buffers. A single NaN written to a delay line circulates forever; this
/// guard prevents permanent buffer poisoning at no cost on the normal path.
#[inline(always)]
fn sanitize(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// ShimmerShared — UI ↔ audio thread parameter bridge
// ---------------------------------------------------------------------------

pub struct ShimmerShared {
    pub mix: Shared,
    pub size: Shared,
    pub damp: Shared,
    pub shimmer: Shared,
    pub width: Shared,
    pub spread: Shared,
    pub pitch: Arc<AtomicU8>,
}

impl ShimmerShared {
    pub fn new() -> Self {
        Self {
            mix: shared(0.0),
            size: shared(0.6),
            damp: shared(0.5),
            shimmer: shared(0.0),
            width: shared(1.35),
            spread: shared(0.10),
            pitch: Arc::new(AtomicU8::new(1)),
        }
    }
}

impl Default for ShimmerShared {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Shared modulation constants
// ---------------------------------------------------------------------------

/// Modulation depth in seconds. ~0.25 ms keeps pitch modulation well under
/// 2 cents at these slow LFO rates — inaudible as chorus, but enough to smear
/// the comb-filter peaks that fixed delays produce.
const MOD_DEPTH_SEC: f32 = 0.00025;

/// Fractional read from a circular buffer. `delay_samples` is the distance
/// behind the write head (positive).
#[inline(always)]
fn frac_read(buf: &[f32], write_pos: usize, delay_samples: f32) -> f32 {
    let len = buf.len();
    let read_pos = write_pos as f32 - delay_samples;
    let ri = read_pos.floor() as isize;
    let rf = read_pos - ri as f32;
    let i0 = ri.rem_euclid(len as isize) as usize;
    let i1 = (ri + 1).rem_euclid(len as isize) as usize;
    buf[i0] * (1.0 - rf) + buf[i1] * rf
}

// ---------------------------------------------------------------------------
// PlateState — Dattorro 1997 plate reverb with modulated tank delays
// ---------------------------------------------------------------------------

/// LFO rates for plate tank delays (Hz). Prime-ish non-harmonic ratios.
const PLATE_LFO_RATES: [f32; 2] = [0.39, 0.63];

#[derive(Clone)]
struct PlateState {
    bw_z: f32,
    id_buf: [Vec<f32>; 4],
    id_pos: [usize; 4],
    ma_buf: Vec<f32>, // modulated allpass (tank entry)
    ma_pos: usize,
    ma_lfo: f32,
    td1_buf: Vec<f32>,
    td1_pos: usize,
    td1_lp: f32,
    td1_delay: f32, // nominal delay in samples
    td1_lfo: f32,
    ta2_buf: Vec<f32>, // second tank allpass
    ta2_pos: usize,
    td2_buf: Vec<f32>,
    td2_pos: usize,
    td2_delay: f32,
    td2_lfo: f32,
    feedback: f32,
    mod_depth: f32,
}

impl PlateState {
    fn new(sr: f32) -> Self {
        let s = sr / 29761.0;
        let id_d = [142usize, 107, 379, 277];
        let mod_depth = MOD_DEPTH_SEC * sr;
        let pad = (mod_depth.ceil() as usize) + 4;
        let td1_d = 4453.0 * s;
        let td2_d = 3720.0 * s;
        PlateState {
            bw_z: 0.0,
            id_buf: id_d.map(|d| vec![0.0; ((d as f32 * s) as usize).max(4)]),
            id_pos: [0; 4],
            ma_buf: vec![0.0; ((700.0 * s) as usize + 24).max(32)],
            ma_pos: 0,
            ma_lfo: 0.0,
            td1_buf: vec![0.0; (td1_d as usize + pad).max(8)],
            td1_pos: 0,
            td1_lp: 0.0,
            td1_delay: td1_d,
            td1_lfo: 0.0,
            ta2_buf: vec![0.0; ((1800.0 * s) as usize).max(1)],
            ta2_pos: 0,
            td2_buf: vec![0.0; (td2_d as usize + pad).max(8)],
            td2_pos: 0,
            td2_delay: td2_d,
            td2_lfo: std::f32::consts::PI, // 180° out of phase with td1
            feedback: 0.0,
            mod_depth,
        }
    }

    fn reset(&mut self) {
        self.bw_z = 0.0;
        for b in &mut self.id_buf {
            b.fill(0.0);
        }
        self.id_pos = [0; 4];
        self.ma_buf.fill(0.0);
        self.ma_pos = 0;
        self.ma_lfo = 0.0;
        self.td1_buf.fill(0.0);
        self.td1_pos = 0;
        self.td1_lp = 0.0;
        self.td1_lfo = 0.0;
        self.ta2_buf.fill(0.0);
        self.ta2_pos = 0;
        self.td2_buf.fill(0.0);
        self.td2_pos = 0;
        self.td2_lfo = std::f32::consts::PI;
        self.feedback = 0.0;
    }

    #[inline]
    fn tick(&mut self, input: f32, decay: f32, damp: f32, sr: f32) -> f32 {
        let s = sr / 29761.0;
        let tau = std::f32::consts::TAU;

        // Input bandwidth — damp rolls off high frequencies entering the tank
        let bw = damp * 0.70;
        self.bw_z = input * (1.0 - bw) + self.bw_z * bw;

        // 4-stage input diffusion: Schroeder allpass (fixed — not in feedback loop)
        let g = [0.75f32, 0.75, 0.625, 0.625];
        let mut diff = self.bw_z;
        for (i, &gi) in g.iter().enumerate() {
            let len = self.id_buf[i].len();
            let pos = self.id_pos[i];
            let buf = self.id_buf[i][pos];
            self.id_buf[i][pos] = sanitize(diff + buf * gi);
            self.id_pos[i] = (pos + 1) % len;
            diff = buf - diff * gi;
        }

        let tank_in = diff + self.feedback * decay;

        // Modulated allpass — flat-magnitude Schroeder form.
        self.ma_lfo = (self.ma_lfo + tau * 0.5 / sr).rem_euclid(tau);
        let ma_base = 672.0 * s;
        let ma_mod = 6.0 * s;
        let ma_delay = (ma_base + self.ma_lfo.sin() * ma_mod).max(1.0);
        let u_old = frac_read(&self.ma_buf, self.ma_pos, ma_delay);
        const MA_G: f32 = 0.6;
        self.ma_buf[self.ma_pos] = sanitize((1.0 - MA_G * MA_G) * tank_in + MA_G * u_old);
        self.ma_pos = (self.ma_pos + 1) % self.ma_buf.len();
        let ma_out = -MA_G * tank_in + u_old;

        // Tank delay 1 — LFO-modulated read
        self.td1_lfo = (self.td1_lfo + tau * PLATE_LFO_RATES[0] / sr).rem_euclid(tau);
        let td1_d_mod = self.td1_delay + self.td1_lfo.sin() * self.mod_depth;
        let td1_out = frac_read(&self.td1_buf, self.td1_pos, td1_d_mod);
        self.td1_buf[self.td1_pos] = sanitize(ma_out);
        self.td1_pos = (self.td1_pos + 1) % self.td1_buf.len();

        // LP damping — consistent scaling with Freeverb/FDN
        let d = damp * 0.85;
        self.td1_lp = sanitize(td1_out * (1.0 - d) + self.td1_lp * d);

        // Second allpass — flat-magnitude Schroeder (g=0.5)
        let len = self.ta2_buf.len();
        let pos = self.ta2_pos;
        let u_old2 = self.ta2_buf[pos];
        self.ta2_buf[pos] = sanitize(0.75 * self.td1_lp + 0.5 * u_old2);
        self.ta2_pos = (pos + 1) % len;
        let ta2_out = -0.5 * self.td1_lp + u_old2;

        // Tank delay 2 — LFO-modulated read (out of phase with td1)
        self.td2_lfo = (self.td2_lfo + tau * PLATE_LFO_RATES[1] / sr).rem_euclid(tau);
        let td2_d_mod = self.td2_delay + self.td2_lfo.sin() * self.mod_depth;
        let td2_out = frac_read(&self.td2_buf, self.td2_pos, td2_d_mod);
        self.td2_buf[self.td2_pos] = sanitize(ta2_out);
        self.td2_pos = (self.td2_pos + 1) % self.td2_buf.len();

        self.feedback = td2_out;

        // Mix tank output with diffused input as early reflections.
        // Tank delays are ~150ms so without this the output is silent for the
        // first note attack. 0.5 weight gives immediate onset while staying behind
        // the tank tail in level.
        (td1_out + td2_out) * 0.5 + diff * 0.5
    }
}

// ---------------------------------------------------------------------------
// FdnState — 8×8 Hadamard FDN with modulated delay lines
// ---------------------------------------------------------------------------

const FDN_DELAYS: [usize; 8] = [1481, 1867, 2383, 2791, 3209, 3643, 4127, 4519];
const FDN_LFO_RATES: [f32; 8] = [0.29, 0.41, 0.47, 0.59, 0.67, 0.79, 0.89, 1.09];

#[derive(Clone)]
struct FdnState {
    buf: [Vec<f32>; 8],
    pos: [usize; 8],
    delay: [f32; 8], // nominal delays in samples
    lfo: [f32; 8],
    lfo_inc: [f32; 8],
    lp: [f32; 8],
    mod_depth: f32,
}

impl FdnState {
    fn new(sr: f32) -> Self {
        let scale = sr / 44100.0;
        let mod_depth = MOD_DEPTH_SEC * sr;
        let pad = (mod_depth.ceil() as usize) + 4;
        let delay: [f32; 8] = std::array::from_fn(|i| FDN_DELAYS[i] as f32 * scale);
        let buf: [Vec<f32>; 8] =
            std::array::from_fn(|i| vec![0.0; (delay[i] as usize + pad).max(8)]);
        let lfo_inc: [f32; 8] =
            std::array::from_fn(|i| std::f32::consts::TAU * FDN_LFO_RATES[i] / sr);
        // Stagger initial phases to maximally decorrelate the modulation
        let lfo: [f32; 8] = std::array::from_fn(|i| std::f32::consts::TAU * i as f32 / 8.0);
        FdnState {
            buf,
            pos: [0; 8],
            delay,
            lfo,
            lfo_inc,
            lp: [0.0; 8],
            mod_depth,
        }
    }

    fn reset(&mut self) {
        for b in &mut self.buf {
            b.fill(0.0);
        }
        self.pos = [0; 8];
        self.lp = [0.0; 8];
        for i in 0..8 {
            self.lfo[i] = std::f32::consts::TAU * i as f32 / 8.0;
        }
    }

    #[inline]
    fn tick(&mut self, input: f32, decay: f32, damp: f32) -> f32 {
        let tau = std::f32::consts::TAU;

        // Read all delay lines with LFO-modulated fractional offsets
        let mut x = [0.0f32; 8];
        for (i, xi) in x.iter_mut().enumerate() {
            self.lfo[i] = (self.lfo[i] + self.lfo_inc[i]).rem_euclid(tau);
            let d = self.delay[i] + self.lfo[i].sin() * self.mod_depth;
            *xi = frac_read(&self.buf[i], self.pos[i], d);
        }

        let out = x.iter().sum::<f32>() * 0.125;

        hadamard8(&mut x);

        let d = damp * 0.85;
        for (i, &xi) in x.iter().enumerate() {
            self.lp[i] = sanitize(xi * (1.0 - d) + self.lp[i] * d);
            self.buf[i][self.pos[i]] = sanitize(self.lp[i] * decay + input * 0.125);
            self.pos[i] = (self.pos[i] + 1) % self.buf[i].len();
        }

        out
    }
}

#[inline]
fn hadamard8(x: &mut [f32; 8]) {
    for step in [4usize, 2, 1] {
        let mut i = 0;
        while i < 8 {
            for j in 0..step {
                let a = x[i + j];
                let b = x[i + j + step];
                x[i + j] = a + b;
                x[i + j + step] = a - b;
            }
            i += step * 2;
        }
    }
    const NORM: f32 = 0.353_553_4;
    for v in x.iter_mut() {
        *v *= NORM;
    }
}

// ---------------------------------------------------------------------------
// Freeverb comb-bank with modulated delays
// ---------------------------------------------------------------------------

const FV_COMB_DELAYS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const FV_AP_DELAYS: [usize; 4] = [225, 341, 441, 556];
const FV_LFO_RATES: [f32; 8] = [0.31, 0.37, 0.43, 0.53, 0.61, 0.71, 0.83, 0.97];

// ---------------------------------------------------------------------------
// ShimmerReverb — selectable algorithm + optional pitch-shifted feedback
// ---------------------------------------------------------------------------

const SHIM_BUF: usize = 16384;
const PRE_BUF: usize = 4096;

#[derive(Clone)]
pub struct ShimmerReverb {
    sr: f32,
    // ── Freeverb algorithm (rev_type = 0) ───────────────────────────────────
    comb_buf: [Vec<f32>; 8],
    comb_pos: [usize; 8],
    comb_feed: [f32; 8],
    comb_delay: [f32; 8],
    comb_lfo: [f32; 8],
    comb_lfo_inc: [f32; 8],
    comb_mod_depth: f32,
    ap_buf: [Vec<f32>; 4],
    ap_pos: [usize; 4],

    // ── Plate algorithm (rev_type = 1) ──────────────────────────────────────
    plate: PlateState,

    // ── FDN Hall algorithm (rev_type = 2) ───────────────────────────────────
    fdn: FdnState,

    // ── Pitch shifter (shared across all algorithms) ─────────────────────────
    shim_buf: Vec<f32>,
    shim_write: usize,
    shim_read_a: f32,
    shim_read_b: f32,
    shim_feedback: f32,
    pre_buf: Vec<f32>,
    pre_pos: usize,
    pre_delay_smp: usize,
    shim_hp_z: f32,
    shim_lp_z: f32,
}

impl ShimmerReverb {
    pub fn new(sr: f32) -> Self {
        let scale = sr / 44100.0;
        let pre_delay_samples = ((0.050 * sr) as usize).clamp(1, PRE_BUF - 1);

        let comb_mod_depth = MOD_DEPTH_SEC * sr;
        let pad = (comb_mod_depth.ceil() as usize) + 4;
        let comb_delay: [f32; 8] = std::array::from_fn(|i| FV_COMB_DELAYS[i] as f32 * scale);
        let comb_buf: [Vec<f32>; 8] =
            std::array::from_fn(|i| vec![0.0; (comb_delay[i] as usize + pad).max(8)]);
        let comb_lfo_inc: [f32; 8] =
            std::array::from_fn(|i| std::f32::consts::TAU * FV_LFO_RATES[i] / sr);
        let comb_lfo: [f32; 8] = std::array::from_fn(|i| std::f32::consts::TAU * i as f32 / 8.0);

        Self {
            sr,
            comb_buf,
            comb_pos: [0; 8],
            comb_feed: [0.0; 8],
            comb_delay,
            comb_lfo,
            comb_lfo_inc,
            comb_mod_depth,
            ap_buf: FV_AP_DELAYS.map(|d| vec![0.0; ((d as f32 * scale) as usize).max(1)]),
            ap_pos: [0; 4],
            plate: PlateState::new(sr),
            fdn: FdnState::new(sr),
            shim_buf: vec![0.0; SHIM_BUF],
            shim_write: 0,
            shim_read_a: 0.0,
            shim_read_b: (SHIM_BUF / 2) as f32,
            shim_feedback: 0.0,
            pre_buf: vec![0.0; PRE_BUF],
            pre_pos: 0,
            pre_delay_smp: pre_delay_samples,
            shim_hp_z: 0.0,
            shim_lp_z: 0.0,
        }
    }

    pub fn reset(&mut self) {
        for b in &mut self.comb_buf {
            b.fill(0.0);
        }
        self.comb_pos = [0; 8];
        self.comb_feed = [0.0; 8];
        for i in 0..8 {
            self.comb_lfo[i] = std::f32::consts::TAU * i as f32 / 8.0;
        }
        for b in &mut self.ap_buf {
            b.fill(0.0);
        }
        self.ap_pos = [0; 4];
        self.plate.reset();
        self.fdn.reset();
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

    #[inline]
    pub fn tick(
        &mut self,
        input: f32,
        room: f32,
        damp: f32,
        shimmer_amt: f32,
        pitch: u8,
        rev_type: u8,
    ) -> f32 {
        let feed = 0.3 + room * 0.695;
        let rev_in = input + self.shim_feedback * shimmer_amt * 0.25;

        let out = match rev_type {
            1 => self.plate.tick(rev_in, feed, damp, self.sr),
            2 => self.fdn.tick(rev_in, feed, damp),
            _ => self.tick_freeverb(rev_in, feed, damp),
        };

        if shimmer_amt > 0.0001 {
            let buf_len = SHIM_BUF;
            let pre_len = self.pre_buf.len();
            let pre_write = self.pre_pos;
            let pre_read = (pre_write + pre_len - self.pre_delay_smp) % pre_len;
            self.pre_buf[pre_write] = out;
            let delayed_out = self.pre_buf[pre_read];
            self.pre_pos = (self.pre_pos + 1) % pre_len;

            self.shim_buf[self.shim_write] = delayed_out;
            self.shim_write = (self.shim_write + 1) % buf_len;

            let pitch_ratio = match pitch {
                1 => 2.0_f32,
                2 => 4.0_f32,
                _ => 1.0_f32,
            };
            self.shim_read_a = (self.shim_read_a + pitch_ratio).rem_euclid(buf_len as f32);
            self.shim_read_b = (self.shim_read_b + pitch_ratio).rem_euclid(buf_len as f32);

            let lerp_buf = |pos: f32| -> f32 {
                let i0 = pos as usize % buf_len;
                let i1 = (i0 + 1) % buf_len;
                self.shim_buf[i0] * (1.0 - pos.fract()) + self.shim_buf[i1] * pos.fract()
            };

            let samp_a = lerp_buf(self.shim_read_a);
            let samp_b = lerp_buf(self.shim_read_b);

            let phase_a = self.shim_read_a / buf_len as f32;
            let phase_b = self.shim_read_b / buf_len as f32;
            let win_a = 0.5 * (1.0 - (phase_a * std::f32::consts::TAU).cos());
            let win_b = 0.5 * (1.0 - (phase_b * std::f32::consts::TAU).cos());
            let win_sum = (win_a + win_b).max(0.001);
            let mut shifted = (samp_a * win_a + samp_b * win_b) / win_sum;

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

    #[inline]
    fn tick_freeverb(&mut self, rev_in: f32, feed: f32, damp: f32) -> f32 {
        let tau = std::f32::consts::TAU;
        let d = damp * 0.85;
        let mut out = 0.0f32;

        // 8 comb filters with LFO-modulated fractional reads
        for i in 0..8 {
            self.comb_lfo[i] = (self.comb_lfo[i] + self.comb_lfo_inc[i]).rem_euclid(tau);
            let delay = self.comb_delay[i] + self.comb_lfo[i].sin() * self.comb_mod_depth;
            let delayed = frac_read(&self.comb_buf[i], self.comb_pos[i], delay);
            self.comb_feed[i] = sanitize(delayed * (1.0 - d) + self.comb_feed[i] * d);
            self.comb_buf[i][self.comb_pos[i]] = sanitize(rev_in + self.comb_feed[i] * feed);
            let len = self.comb_buf[i].len();
            self.comb_pos[i] = (self.comb_pos[i] + 1) % len;
            out += delayed;
        }
        out *= 0.125;

        // Allpass diffusers (unmodulated — not in feedback loop, gives tight onset)
        for i in 0..4 {
            let len = self.ap_buf[i].len();
            let pos = self.ap_pos[i];
            let buf = self.ap_buf[i][pos];
            let input_ap = out;
            self.ap_buf[i][pos] = sanitize(input_ap + buf * 0.5);
            self.ap_pos[i] = (pos + 1) % len;
            out = buf - input_ap;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::ShimmerReverb;

    fn assert_stable(rev_type: u8, room: f32, damp: f32, shimmer_amt: f32, pitch: u8, label: &str) {
        let mut s = ShimmerReverb::new(44_100.0);
        for i in 0..60_000usize {
            let inp = if i % 5_000 == 0 { 1.0 } else { 0.0 };
            let x = s.tick(inp, room, damp, shimmer_amt, pitch, rev_type);
            assert!(x.is_finite(), "{label}: sample {i} not finite (NaN/Inf)");
            assert!(x.abs() < 10.0, "{label}: sample {i} too loud: {x:.3}");
        }
    }

    #[test]
    fn freeverb_dry() {
        assert_stable(0, 0.5, 0.5, 0.0, 0, "freeverb dry");
    }
    #[test]
    fn freeverb_max_room() {
        assert_stable(0, 1.0, 0.0, 0.0, 0, "freeverb max room bright");
    }
    #[test]
    fn freeverb_max_room_damp() {
        assert_stable(0, 1.0, 1.0, 0.0, 0, "freeverb max room dark");
    }
    #[test]
    fn freeverb_shimmer() {
        assert_stable(0, 0.95, 0.35, 0.85, 1, "freeverb shimmer +12st");
    }
    #[test]
    fn freeverb_shimmer_2oct() {
        assert_stable(0, 0.95, 0.35, 1.0, 2, "freeverb shimmer +24st");
    }
    #[test]
    fn freeverb_near_unity() {
        assert_stable(0, 1.0, 0.0, 1.0, 1, "freeverb near-unity full shimmer");
    }

    #[test]
    fn plate_dry() {
        assert_stable(1, 0.5, 0.5, 0.0, 0, "plate dry");
    }
    #[test]
    fn plate_max_room() {
        assert_stable(1, 1.0, 0.0, 0.0, 0, "plate max room bright");
    }
    #[test]
    fn plate_max_room_damp() {
        assert_stable(1, 1.0, 1.0, 0.0, 0, "plate max room dark");
    }
    #[test]
    fn plate_shimmer() {
        assert_stable(1, 0.95, 0.35, 0.85, 1, "plate shimmer +12st");
    }
    #[test]
    fn plate_shimmer_2oct() {
        assert_stable(1, 0.95, 0.35, 1.0, 2, "plate shimmer +24st");
    }
    #[test]
    fn plate_near_unity() {
        assert_stable(1, 1.0, 0.0, 1.0, 1, "plate near-unity full shimmer");
    }

    #[test]
    fn fdn_dry() {
        assert_stable(2, 0.5, 0.5, 0.0, 0, "fdn dry");
    }
    #[test]
    fn fdn_max_room() {
        assert_stable(2, 1.0, 0.0, 0.0, 0, "fdn max room bright");
    }
    #[test]
    fn fdn_max_room_damp() {
        assert_stable(2, 1.0, 1.0, 0.0, 0, "fdn max room dark");
    }
    #[test]
    fn fdn_shimmer() {
        assert_stable(2, 0.95, 0.35, 0.85, 1, "fdn shimmer +12st");
    }
    #[test]
    fn fdn_shimmer_2oct() {
        assert_stable(2, 0.95, 0.35, 1.0, 2, "fdn shimmer +24st");
    }
    #[test]
    fn fdn_near_unity() {
        assert_stable(2, 1.0, 0.0, 1.0, 1, "fdn near-unity full shimmer");
    }

    #[test]
    fn reset_clears_state() {
        let mut s = ShimmerReverb::new(44_100.0);
        for i in 0..10_000usize {
            let inp = if i % 100 == 0 { 1.0 } else { 0.0 };
            s.tick(inp, 1.0, 0.0, 0.5, 1, 1);
        }
        s.reset();
        let x = s.tick(0.0, 1.0, 0.0, 0.5, 1, 1);
        assert!(x.abs() < 1e-6, "reset didn't clear state: {x}");
    }
}
