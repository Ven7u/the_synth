//! LiveAdsr — a fully live-parametric ADSR envelope AudioNode.
//!
//! Unlike fundsp's `adsr_live`, all four time/level parameters are read from
//! `Shared` values every sample, so slider changes take effect immediately.
//!
//! Input 0 : gate (0.0 = off, 1.0 = on)
//! Output 0: envelope level [0.0, 1.0]
//!
//! The node also writes a cursor value to an optional `Shared` each sample,
//! encoding phase + progress for the UI visualizer:
//!
//!   0.0        = idle
//!   1.0–1.99   = attack  (frac = progress 0→1)
//!   2.0–2.99   = decay   (frac = progress 0→1)
//!   3.0        = sustain (held)
//!   4.0–4.99   = release (frac = progress 0→1)

use fundsp::prelude32::*;

#[derive(Clone, Copy, PartialEq)]
enum AdsrStage { Idle, Attack, Decay, Sustain, Release }

#[derive(Clone)]
pub struct LiveAdsr {
    pub attack:  Shared,
    pub decay:   Shared,
    pub sustain: Shared,
    pub release: Shared,
    pub cursor:  Option<Shared>, // written each sample for UI

    stage:       AdsrStage,
    level:       f32,
    progress:    f32, // 0..1 within current timed stage
    start_level: f32, // level snapshot at stage entry (for click-free transitions)
    sr:          f32,
    prev_gate:   f32,
}

impl LiveAdsr {
    pub fn new(
        attack:  Shared,
        decay:   Shared,
        sustain: Shared,
        release: Shared,
        cursor:  Option<Shared>,
        sr:      f32,
    ) -> Self {
        Self {
            attack, decay, sustain, release, cursor,
            stage: AdsrStage::Idle,
            level: 0.0,
            start_level: 0.0,
            progress: 0.0,
            sr,
            prev_gate: 0.0,
        }
    }
}

impl AudioNode for LiveAdsr {
    const ID: u64 = 0x4c697665_41647372; // "LiveAdsr"
    type Inputs  = U1;
    type Outputs = U1;

    fn reset(&mut self) {
        self.stage       = AdsrStage::Idle;
        self.level       = 0.0;
        self.start_level = 0.0;
        self.progress    = 0.0;
        self.prev_gate   = 0.0;
    }

    fn set_sample_rate(&mut self, sr: f64) {
        self.sr = sr as f32;
    }

    #[inline]
    fn tick(&mut self, input: &Frame<f32, U1>) -> Frame<f32, U1> {
        let gate = input[0];
        let a = self.attack.value().max(0.0001);
        let d = self.decay.value().max(0.0001);
        let s = self.sustain.value().clamp(0.0, 1.0);
        let r = self.release.value().max(0.0001);

        // Gate rising edge → restart attack from current level (avoids click on retrigger)
        if gate > 0.5 && self.prev_gate <= 0.5 {
            self.stage       = AdsrStage::Attack;
            self.start_level = self.level;
            self.progress    = 0.0;
        }
        // Gate falling edge → release from current level (avoids click when releasing early)
        if gate <= 0.5 && self.prev_gate > 0.5 {
            self.stage       = AdsrStage::Release;
            self.start_level = self.level;
            self.progress    = 0.0;
        }
        self.prev_gate = gate;

        let dt = 1.0 / self.sr;

        match self.stage {
            AdsrStage::Idle => {
                self.level = 0.0;
            }
            AdsrStage::Attack => {
                self.progress += dt / a;
                // Ramp from start_level → 1.0 so retriggers don't click
                self.level = self.start_level + (1.0 - self.start_level) * self.progress.min(1.0);
                if self.progress >= 1.0 {
                    self.stage    = AdsrStage::Decay;
                    self.progress = 0.0;
                }
            }
            AdsrStage::Decay => {
                self.progress += dt / d;
                self.level = 1.0 - (1.0 - s) * self.progress.min(1.0);
                if self.progress >= 1.0 {
                    self.stage    = AdsrStage::Sustain;
                    self.progress = 0.0;
                    self.level    = s;
                }
            }
            AdsrStage::Sustain => {
                self.level = s;
            }
            AdsrStage::Release => {
                self.progress += dt / r;
                // Ramp from start_level → 0 so early releases don't click
                self.level = self.start_level * (1.0 - self.progress.min(1.0));
                if self.progress >= 1.0 {
                    self.stage    = AdsrStage::Idle;
                    self.level    = 0.0;
                    self.progress = 0.0;
                }
            }
        }

        // Write cursor for UI
        if let Some(cur) = &self.cursor {
            let v = match self.stage {
                AdsrStage::Idle    => 0.0,
                AdsrStage::Attack  => 1.0 + self.progress.min(0.99),
                AdsrStage::Decay   => 2.0 + self.progress.min(0.99),
                AdsrStage::Sustain => 3.0,
                AdsrStage::Release => 4.0 + self.progress.min(0.99),
            };
            cur.set(v);
        }

        [self.level].into()
    }
}
