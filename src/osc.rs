//! Oscillator primitives: WaveShape and MultiWaveOsc.
//!
//! `WaveShape` defines the waveform math (including PolyBLEP band-limiting).
//! `MultiWaveOsc` is a fundsp `AudioNode` (1 input: Hz → 1 output: audio)
//! that reads the active shape from an `Arc<AtomicU8>` with no graph rebuild.

use fundsp::prelude32::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

// ---------------------------------------------------------------------------
// WaveShape
// ---------------------------------------------------------------------------

/// Waveform selector. Stored as u8 in AtomicU8 for lock-free thread sharing.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
pub enum WaveShape {
    Sine     = 0,
    Saw      = 1,
    Square   = 2,
    Triangle = 3,
}

impl WaveShape {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Saw,
            2 => Self::Square,
            3 => Self::Triangle,
            _ => Self::Sine,
        }
    }

    /// Compute one sample given phase `p` ∈ [0, 1) and phase increment `dt` = freq / sr.
    /// `dt` is used by band-limited shapes for PolyBLEP correction; ignored by others.
    #[inline]
    pub fn sample(self, p: f32, dt: f32) -> f32 {
        match self {
            Self::Sine     => (p * f32::TAU).sin(),
            Self::Saw      => (2.0 * p - 1.0) - poly_blep(p, dt),
            Self::Square   => {
                let naive = if p < 0.5 { 1.0_f32 } else { -1.0 };
                naive + poly_blep(p, dt) - poly_blep((p + 0.5) % 1.0, dt)
            }
            Self::Triangle => if p < 0.5 { 4.0 * p - 1.0 } else { 3.0 - 4.0 * p },
        }
    }
}

// ---------------------------------------------------------------------------
// PolyBLEP
// ---------------------------------------------------------------------------

/// Polynomial Band-Limited Step correction.
/// Smooths the discontinuity at phase = 0 over ±1 sample.
/// `t`: current phase [0, 1) — `dt`: phase increment per sample (freq / sr).
#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// MultiWaveOsc
// ---------------------------------------------------------------------------

/// Single oscillator fundsp node: 1 input (freq Hz) → 1 output (audio).
/// Waveform is selected at runtime via an `Arc<AtomicU8>` — no graph rebuild needed.
/// Saw and square use PolyBLEP band-limiting; triangle and sine are alias-free.
#[derive(Clone)]
pub struct MultiWaveOsc {
    wave:  Arc<AtomicU8>,
    phase: f32,
    sr:    f32,
}

impl MultiWaveOsc {
    pub fn new(wave: Arc<AtomicU8>, sr: f32) -> Self {
        Self { wave, phase: 0.0, sr }
    }
}

impl AudioNode for MultiWaveOsc {
    const ID: u64 = 0x4d756c74_69576176; // "MultiWav"
    type Inputs  = U1;
    type Outputs = U1;

    #[inline]
    fn tick(&mut self, input: &Frame<f32, U1>) -> Frame<f32, U1> {
        let freq = input[0].max(0.0);
        let dt   = freq / self.sr;
        self.phase += dt;
        self.phase -= self.phase.floor();
        let shape = WaveShape::from_u8(self.wave.load(Ordering::Relaxed));
        [shape.sample(self.phase, dt)].into()
    }

    fn reset(&mut self) { self.phase = 0.0; }

    fn set_sample_rate(&mut self, sr: f64) { self.sr = sr as f32; }
}
