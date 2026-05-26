//! 8-band parametric EQ — biquad DSP + shared config.
//!
//! Band layout: Low Shelf, 6× Peak (Bell), High Shelf.
//! Audio EQ Cookbook formulas (R. Bristow-Johnson).

pub const BAND_COUNT: usize = 8;

// ---------------------------------------------------------------------------
// Config (UI ↔ audio thread via Arc<Mutex<EqParams>>)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BandType {
    LowShelf,
    Peak,
    HighShelf,
}

#[derive(Clone, Copy, Debug)]
pub struct BandParams {
    pub enabled: bool,
    pub band_type: BandType,
    pub freq: f32,    // Hz, 20–20 000
    pub gain_db: f32, // −18..+18
    pub q: f32,       // 0.1..10 (bandwidth for peak; slope-factor for shelves)
}

impl BandParams {
    pub fn default_for_band(i: usize) -> Self {
        let (band_type, freq) = match i {
            0 => (BandType::LowShelf, 80.0),
            1 => (BandType::Peak, 200.0),
            2 => (BandType::Peak, 500.0),
            3 => (BandType::Peak, 1000.0),
            4 => (BandType::Peak, 2500.0),
            5 => (BandType::Peak, 5000.0),
            6 => (BandType::Peak, 10000.0),
            _ => (BandType::HighShelf, 16000.0),
        };
        Self {
            enabled: true,
            band_type,
            freq,
            gain_db: 0.0,
            q: 0.707,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EqParams {
    pub enabled: bool,
    pub bands: [BandParams; BAND_COUNT],
}

impl Default for EqParams {
    fn default() -> Self {
        Self {
            enabled: false,
            bands: std::array::from_fn(BandParams::default_for_band),
        }
    }
}

// ---------------------------------------------------------------------------
// Biquad internals
// ---------------------------------------------------------------------------

/// Normalised biquad coefficients (a0 divided out).
#[derive(Clone, Copy, Default)]
pub struct BiquadCoeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl BiquadCoeffs {
    /// Compute coefficients from band params and sample rate.
    pub fn compute(p: &BandParams, sr: f32) -> Self {
        if !p.enabled || p.gain_db.abs() < 1e-3 {
            return Self {
                b0: 1.0,
                b1: 0.0,
                b2: 0.0,
                a1: 0.0,
                a2: 0.0,
            };
        }
        let freq = p.freq.clamp(20.0, sr * 0.499);
        let w0 = 2.0 * std::f32::consts::PI * freq / sr;
        let cos_w = w0.cos();
        let sin_w = w0.sin();
        let q = p.q.clamp(0.1, 10.0);
        let alpha = sin_w / (2.0 * q);
        let a = 10.0_f32.powf(p.gain_db / 40.0);

        let (b0, b1, b2, a0, a1, a2) = match p.band_type {
            BandType::Peak => {
                let b0 = 1.0 + alpha * a;
                let b1 = -2.0 * cos_w;
                let b2 = 1.0 - alpha * a;
                let a0 = 1.0 + alpha / a;
                let a1 = -2.0 * cos_w;
                let a2 = 1.0 - alpha / a;
                (b0, b1, b2, a0, a1, a2)
            }
            BandType::LowShelf => {
                let sa = a.sqrt();
                let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w + 2.0 * sa * alpha);
                let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w);
                let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w - 2.0 * sa * alpha);
                let a0 = (a + 1.0) + (a - 1.0) * cos_w + 2.0 * sa * alpha;
                let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w);
                let a2 = (a + 1.0) + (a - 1.0) * cos_w - 2.0 * sa * alpha;
                (b0, b1, b2, a0, a1, a2)
            }
            BandType::HighShelf => {
                let sa = a.sqrt();
                let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w + 2.0 * sa * alpha);
                let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w);
                let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w - 2.0 * sa * alpha);
                let a0 = (a + 1.0) - (a - 1.0) * cos_w + 2.0 * sa * alpha;
                let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w);
                let a2 = (a + 1.0) - (a - 1.0) * cos_w - 2.0 * sa * alpha;
                (b0, b1, b2, a0, a1, a2)
            }
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    #[inline]
    fn tick(&self, x: f32, x1: &mut f32, x2: &mut f32, y1: &mut f32, y2: &mut f32) -> f32 {
        let y = self.b0 * x + self.b1 * *x1 + self.b2 * *x2 - self.a1 * *y1 - self.a2 * *y2;
        *x2 = *x1;
        *x1 = x;
        *y2 = *y1;
        *y1 = y;
        y
    }

    /// Magnitude response (linear) at angular frequency w = 2π·f/sr.
    pub fn magnitude_at_w(&self, w: f32) -> f32 {
        let cos_w = w.cos();
        let cos_2w = (2.0 * w).cos();
        let sin_w = w.sin();
        let sin_2w = (2.0 * w).sin();
        let nr = self.b0 + self.b1 * cos_w + self.b2 * cos_2w;
        let ni = -(self.b1 * sin_w + self.b2 * sin_2w);
        let dr = 1.0 + self.a1 * cos_w + self.a2 * cos_2w;
        let di = -(self.a1 * sin_w + self.a2 * sin_2w);
        let num = nr * nr + ni * ni;
        let den = dr * dr + di * di;
        if den < 1e-30 {
            1.0
        } else {
            (num / den).sqrt()
        }
    }
}

// ---------------------------------------------------------------------------
// Real-time EQ processor (lives on the audio thread)
// ---------------------------------------------------------------------------

// Individual fields avoid split-borrow issues with array indexing.
#[derive(Default, Clone, Copy)]
struct BandState {
    xl1: f32,
    xl2: f32,
    yl1: f32,
    yl2: f32,
    xr1: f32,
    xr2: f32,
    yr1: f32,
    yr2: f32,
}

pub struct ParametricEq {
    coeffs: [BiquadCoeffs; BAND_COUNT],
    state: [BandState; BAND_COUNT],
    pub last_params: EqParams,
    sr: f32,
}

impl ParametricEq {
    pub fn new(sr: f32) -> Self {
        let defaults = EqParams::default();
        let coeffs = std::array::from_fn(|i| BiquadCoeffs::compute(&defaults.bands[i], sr));
        Self {
            coeffs,
            state: [BandState::default(); BAND_COUNT],
            last_params: defaults,
            sr,
        }
    }

    pub fn update(&mut self, p: &EqParams) {
        for i in 0..BAND_COUNT {
            self.coeffs[i] = BiquadCoeffs::compute(&p.bands[i], self.sr);
        }
        self.last_params = p.clone();
    }

    #[inline]
    pub fn process(&mut self, mut l: f32, mut r: f32) -> (f32, f32) {
        if !self.last_params.enabled {
            return (l, r);
        }
        for i in 0..BAND_COUNT {
            let s = &mut self.state[i];
            let c = &self.coeffs[i];
            l = c.tick(l, &mut s.xl1, &mut s.xl2, &mut s.yl1, &mut s.yl2);
            r = c.tick(r, &mut s.xr1, &mut s.xr2, &mut s.yr1, &mut s.yr2);
        }
        (l, r)
    }
}

// ---------------------------------------------------------------------------
// Frequency response helpers (for the UI response curve)
// ---------------------------------------------------------------------------

/// Compute total EQ magnitude response in dB at each of `n` log-spaced
/// frequencies from 20 Hz to 20 kHz given `sr`.
pub fn response_curve_db(params: &EqParams, sr: f32, n: usize) -> Vec<f32> {
    let log_lo = 20.0_f32.ln();
    let log_hi = 20000.0_f32.ln();
    (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1).max(1) as f32;
            let freq = (log_lo + t * (log_hi - log_lo)).exp();
            let w = 2.0 * std::f32::consts::PI * freq / sr;
            let mut total_mag = 1.0_f32;
            for b in &params.bands {
                if b.enabled {
                    let c = BiquadCoeffs::compute(b, sr);
                    total_mag *= c.magnitude_at_w(w);
                }
            }
            20.0 * total_mag.max(1e-10).log10()
        })
        .collect()
}
