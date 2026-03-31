//! Oscillator primitives: WaveShape and MultiWaveOsc.
//!
//! `WaveShape` defines the waveform math (including PolyBLEP band-limiting).
//! `MultiWaveOsc` is a fundsp `AudioNode` (1 input: Hz → 1 output: audio)
//! that reads the active shape from an `Arc<AtomicU8>` with no graph rebuild.

use fundsp::prelude32::*;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// WaveShape
// ---------------------------------------------------------------------------

/// Waveform selector. Stored as u8 in AtomicU8 for lock-free thread sharing.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
pub enum WaveShape {
    Sine = 0,
    Saw = 1,
    Square = 2,
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

    /// Compute one sample given phase `p` ∈ [0, 1), phase increment `dt` = freq / sr,
    /// and pulse width `pw` ∈ (0, 1) — only used by Square, ignored by other shapes.
    #[inline]
    pub fn sample(self, p: f32, dt: f32, pw: f32) -> f32 {
        match self {
            Self::Sine => (p * f32::TAU).sin(),
            Self::Saw => (2.0 * p - 1.0) - poly_blep(p, dt),
            Self::Square => {
                let pw = pw.clamp(0.01, 0.99);
                let naive = if p < pw { 1.0_f32 } else { -1.0 };
                // PolyBLEP at the rising edge (phase=0) and falling edge (phase=pw)
                naive + poly_blep(p, dt) - poly_blep((p + (1.0 - pw)) % 1.0, dt)
            }
            Self::Triangle => {
                if p < 0.5 {
                    4.0 * p - 1.0
                } else {
                    3.0 - 4.0 * p
                }
            }
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

/// Hard sync role for an oscillator instance.
#[derive(Clone)]
pub enum SyncRole {
    /// Not participating in hard sync.
    None,
    /// Master: increments the generation counter on every phase wrap.
    Master {
        sync_enabled: Arc<AtomicBool>,
        gen: Arc<AtomicU8>,
    },
    /// Slave: resets phase when it sees a new generation from the master.
    Slave {
        sync_enabled: Arc<AtomicBool>,
        gen: Arc<AtomicU8>,
        last_gen: u8,
    },
}

/// Single oscillator fundsp node: 1 input (freq Hz) → 1 output (audio).
/// Waveform is selected at runtime via an `Arc<AtomicU8>` — no graph rebuild needed.
/// Saw and square use PolyBLEP band-limiting; triangle and sine are alias-free.
/// Supports hard sync via a generation counter shared between master and slave instances.
#[derive(Clone)]
pub struct MultiWaveOsc {
    wave: Arc<AtomicU8>,
    pulse_width: Shared,
    phase: f32,
    sr: f32,
    sync: SyncRole,
}

impl MultiWaveOsc {
    #[allow(dead_code)]
    pub fn new(wave: Arc<AtomicU8>, pulse_width: Shared, sr: f32) -> Self {
        Self::with_sync(wave, pulse_width, sr, 0.0, SyncRole::None)
    }

    pub fn with_sync(
        wave: Arc<AtomicU8>,
        pulse_width: Shared,
        sr: f32,
        initial_phase: f32,
        sync: SyncRole,
    ) -> Self {
        Self {
            wave,
            pulse_width,
            phase: initial_phase % 1.0,
            sr,
            sync,
        }
    }
}

impl AudioNode for MultiWaveOsc {
    const ID: u64 = 0x4d756c74_69576176; // "MultiWav"
    type Inputs = U1;
    type Outputs = U1;

    #[inline]
    fn tick(&mut self, input: &Frame<f32, U1>) -> Frame<f32, U1> {
        let freq = input[0].max(0.0);
        let dt = freq / self.sr;
        let pw = self.pulse_width.value();

        // Slave: reset phase if master has wrapped since our last tick.
        if let SyncRole::Slave { sync_enabled, gen, last_gen } = &mut self.sync {
            if sync_enabled.load(Ordering::Relaxed) {
                let current_gen = gen.load(Ordering::Relaxed);
                if current_gen != *last_gen {
                    self.phase = 0.0;
                    *last_gen = current_gen;
                }
            }
        }

        let prev_phase = self.phase;
        self.phase += dt;
        let wrapped = self.phase >= 1.0;
        self.phase -= self.phase.floor();

        // Master: signal slaves when phase wraps.
        if wrapped {
            if let SyncRole::Master { sync_enabled, gen } = &self.sync {
                if sync_enabled.load(Ordering::Relaxed) {
                    gen.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        let _ = prev_phase;
        let shape = WaveShape::from_u8(self.wave.load(Ordering::Relaxed));
        [shape.sample(self.phase, dt, pw)].into()
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        if let SyncRole::Slave { last_gen, gen, .. } = &mut self.sync {
            *last_gen = gen.load(Ordering::Relaxed);
        }
    }

    fn set_sample_rate(&mut self, sr: f64) {
        self.sr = sr as f32;
    }
}
