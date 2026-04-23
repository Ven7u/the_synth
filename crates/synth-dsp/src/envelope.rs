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
enum AdsrStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone)]
pub struct LiveAdsr {
    pub attack: Shared,
    pub decay: Shared,
    pub sustain: Shared,
    pub release: Shared,
    pub cursor: Option<Shared>, // written each sample for UI

    stage: AdsrStage,
    level: f32,
    progress: f32,    // 0..1 within current timed stage
    start_level: f32, // level snapshot at stage entry (for click-free transitions)
    sr: f32,
    prev_gate: f32,
}

impl LiveAdsr {
    pub fn new(
        attack: Shared,
        decay: Shared,
        sustain: Shared,
        release: Shared,
        cursor: Option<Shared>,
        sr: f32,
    ) -> Self {
        Self {
            attack,
            decay,
            sustain,
            release,
            cursor,
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
    type Inputs = U1;
    type Outputs = U1;

    fn reset(&mut self) {
        self.stage = AdsrStage::Idle;
        self.level = 0.0;
        self.start_level = 0.0;
        self.progress = 0.0;
        self.prev_gate = 0.0;
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
            self.stage = AdsrStage::Attack;
            self.start_level = self.level;
            self.progress = 0.0;
        }
        // Gate falling edge → release from current level (avoids click when releasing early)
        if gate <= 0.5 && self.prev_gate > 0.5 {
            self.stage = AdsrStage::Release;
            self.start_level = self.level;
            self.progress = 0.0;
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
                    self.stage = AdsrStage::Decay;
                    self.progress = 0.0;
                }
            }
            AdsrStage::Decay => {
                self.progress += dt / d;
                self.level = 1.0 - (1.0 - s) * self.progress.min(1.0);
                if self.progress >= 1.0 {
                    self.stage = AdsrStage::Sustain;
                    self.progress = 0.0;
                    self.level = s;
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
                    self.stage = AdsrStage::Idle;
                    self.level = 0.0;
                    self.progress = 0.0;
                }
            }
        }

        // Write cursor for UI
        if let Some(cur) = &self.cursor {
            let v = match self.stage {
                AdsrStage::Idle => 0.0,
                AdsrStage::Attack => 1.0 + self.progress.min(0.99),
                AdsrStage::Decay => 2.0 + self.progress.min(0.99),
                AdsrStage::Sustain => 3.0,
                AdsrStage::Release => 4.0 + self.progress.min(0.99),
            };
            cur.set(v);
        }

        [self.level].into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fundsp::prelude32::shared;

    fn make_adsr(a: f32, d: f32, s: f32, r: f32) -> LiveAdsr {
        LiveAdsr::new(shared(a), shared(d), shared(s), shared(r), None, 44100.0)
    }

    fn tick_gate(adsr: &mut LiveAdsr, gate: f32) -> f32 {
        adsr.tick(&[gate].into())[0]
    }

    fn run_gate(adsr: &mut LiveAdsr, gate: f32, samples: usize) -> f32 {
        let mut last = 0.0;
        for _ in 0..samples {
            last = tick_gate(adsr, gate);
        }
        last
    }

    /// Full cycle: gate on → sustain → gate off → full release → level must be ~0.
    #[test]
    fn full_cycle_returns_to_zero() {
        let mut adsr = make_adsr(0.01, 0.1, 0.7, 0.5);
        // Attack + decay
        run_gate(&mut adsr, 1.0, (44100.0 * 0.5) as usize);
        // Sustain
        let sus = run_gate(&mut adsr, 1.0, (44100.0 * 0.2) as usize);
        assert!((sus - 0.7).abs() < 0.02, "expected sustain ~0.7, got {sus}");
        // Release
        run_gate(&mut adsr, 0.0, (44100.0 * 1.0) as usize);
        let level = tick_gate(&mut adsr, 0.0);
        assert!(
            level < 0.01,
            "expected near zero after release, got {level}"
        );
    }

    /// Retrigger mid-sustain: attack must restart from current level (no click jump to 0).
    #[test]
    fn retrigger_from_sustain_starts_from_current_level() {
        let mut adsr = make_adsr(0.01, 0.1, 0.75, 0.5);
        // Reach sustain
        run_gate(&mut adsr, 1.0, (44100.0 * 0.5) as usize);
        let level_before = adsr.level;
        assert!((level_before - 0.75).abs() < 0.02);

        // Gate off then on in same "buffer" boundary (simulates steal-retrigger)
        tick_gate(&mut adsr, 0.0); // falling edge
        let after_fall = tick_gate(&mut adsr, 1.0); // rising edge: attack starts from start_level
                                                    // start_level was captured at the falling edge, so attack begins from ~sustain level
        assert!(
            after_fall > 0.5,
            "retrigger should start from current level, not zero, got {after_fall}"
        );
    }

    /// Retrigger mid-release: must start attack from release level, not from 0.
    #[test]
    fn retrigger_mid_release_starts_from_release_level() {
        let mut adsr = make_adsr(0.01, 0.05, 0.8, 2.0);
        // Reach sustain
        run_gate(&mut adsr, 1.0, (44100.0 * 0.3) as usize);
        // Gate off, run 300ms into 2s release
        run_gate(&mut adsr, 0.0, (44100.0 * 0.3) as usize);
        let release_level = adsr.level;
        assert!(
            release_level > 0.2,
            "should still be audible mid-release, got {release_level}"
        );

        // Retrigger
        let after_retrigger = tick_gate(&mut adsr, 1.0);
        // First sample of attack should be close to the release level (not a hard jump to 0)
        assert!(after_retrigger > release_level * 0.9,
            "retrigger should start smoothly from release level {release_level}, got {after_retrigger}");
    }

    /// Envelope output must always be in [0.0, 1.0].
    #[test]
    fn level_stays_in_unit_range() {
        let mut adsr = make_adsr(0.005, 0.05, 0.6, 0.3);
        // Rapid gate toggling
        for i in 0..88200 {
            let gate = if (i / 4410) % 2 == 0 { 1.0 } else { 0.0 };
            let level = tick_gate(&mut adsr, gate);
            assert!(
                level >= -0.001 && level <= 1.001,
                "level {level} out of [0,1] at sample {i}"
            );
        }
    }
}
