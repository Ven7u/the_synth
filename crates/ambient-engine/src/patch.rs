use serde::{Deserialize, Serialize};

use crate::TrackState;
use std::sync::atomic::Ordering;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmbientPatch {
    pub name: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub synth_model: String,

    pub osc_wave: [usize; 3],
    pub osc_octave: [i32; 3],
    pub osc_detune: [f32; 3],
    pub osc_vol: [f32; 3],
    pub osc_enabled: [bool; 3],
    pub osc_pulse_width: [f32; 3],
    pub osc_pw_enabled: [bool; 3],
    pub osc_unison_enabled: [bool; 3],
    pub osc_unison_count: [usize; 3],
    pub osc_unison_spread: [f32; 3],
    pub hard_sync: bool,
    pub fm_enabled: bool,
    pub fm_depth: f32,
    pub ring_enabled: bool,
    pub ring_depth: f32,

    pub noise_vol: f32,

    pub lfo_enabled: bool,
    pub lfo_rate: f32,
    pub lfo_depth: f32,
    pub lfo_shape: usize,
    pub lfo_dest: usize,
    #[serde(default)]
    pub lfo_sync: bool,
    #[serde(default = "default_lfo_division")]
    pub lfo_division: u8, // ClockDivision::to_u8()

    pub filter_enabled: bool,
    pub filter_cutoff: f32,
    pub filter_q: f32,
    pub filter_env_amount: f32,
    pub fenv_adsr: [f32; 4],

    pub amp_adsr: [f32; 4],

    #[serde(default)]
    pub glide_time: f32,
}

fn default_category() -> String {
    "User".to_string()
}

fn default_lfo_division() -> u8 {
    // ClockDivision::Quarter = 2
    2
}

impl Default for AmbientPatch {
    fn default() -> Self {
        Self {
            name: "Init".to_string(),
            category: default_category(),
            synth_model: String::new(),
            osc_wave: [1, 0, 0],
            osc_octave: [0, 0, 0],
            osc_detune: [0.0, 0.0, 0.0],
            osc_vol: [0.4, 0.3, 0.0],
            osc_enabled: [true, true, false],
            osc_pulse_width: [0.5, 0.5, 0.5],
            osc_pw_enabled: [false, false, false],
            osc_unison_enabled: [false, false, false],
            osc_unison_count: [2, 2, 2],
            osc_unison_spread: [20.0, 20.0, 20.0],
            hard_sync: false,
            fm_enabled: false,
            fm_depth: 1.0,
            ring_enabled: false,
            ring_depth: 1.0,
            noise_vol: 0.0,
            lfo_enabled: false,
            lfo_rate: 2.0,
            lfo_depth: 0.0,
            lfo_shape: 0,
            lfo_dest: 1,
            lfo_sync: false,
            lfo_division: default_lfo_division(),
            filter_enabled: true,
            filter_cutoff: 3000.0,
            filter_q: 0.3,
            filter_env_amount: 0.3,
            fenv_adsr: [0.01, 0.3, 0.0, 0.2],
            amp_adsr: [0.01, 0.15, 0.7, 0.4],
            glide_time: 0.0,
        }
    }
}

impl AmbientPatch {
    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let patch = serde_json::from_str::<Self>(&json)?;
        Ok(patch)
    }

    pub fn apply_to_track(&self, track: &TrackState) {
        for i in 0..3 {
            track.osc_wave[i].store(self.osc_wave[i] as u8, Ordering::Relaxed);
            track.osc_vol[i].set(if self.osc_enabled[i] {
                self.osc_vol[i]
            } else {
                0.0
            });
            track.osc_pulse_width[i].set(self.osc_pulse_width[i].clamp(0.01, 0.99));
            Self::set_freq_mult(self, track, i);
            Self::set_unison(self, track, i);
        }

        track
            .hard_sync_enabled
            .store(self.hard_sync, Ordering::Relaxed);
        track.fm_depth.set(if self.fm_enabled {
            self.fm_depth.max(0.0)
        } else {
            0.0
        });
        track.ring_depth.set(if self.ring_enabled {
            self.ring_depth.max(0.0)
        } else {
            0.0
        });

        track.noise_vol.set(self.noise_vol.clamp(0.0, 1.0));
        track.lfo_rate.set(self.lfo_rate.clamp(0.1, 20.0));
        track.lfo_depth.set(if self.lfo_enabled {
            self.lfo_depth.clamp(0.0, 1.0)
        } else {
            0.0
        });
        track
            .lfo_shape
            .store(self.lfo_shape as u8, Ordering::Relaxed);
        track.lfo_dest.store(self.lfo_dest as u8, Ordering::Relaxed);
        track.lfo_sync.store(self.lfo_sync as u8, Ordering::Relaxed);
        track
            .lfo_division
            .store(self.lfo_division, Ordering::Relaxed);

        track.cutoff.set(if self.filter_enabled {
            self.filter_cutoff.clamp(80.0, 18_000.0)
        } else {
            18_000.0
        });
        track.resonance.set(if self.filter_enabled {
            self.filter_q.clamp(0.0, 10.0)
        } else {
            0.0
        });
        track
            .filter_env_amount
            .set(self.filter_env_amount.clamp(0.0, 1.0));
        track.fenv_attack.set(self.fenv_adsr[0].max(0.0));
        track.fenv_decay.set(self.fenv_adsr[1].max(0.0));
        track.fenv_sustain.set(self.fenv_adsr[2].clamp(0.0, 1.0));
        track.fenv_release.set(self.fenv_adsr[3].max(0.0));

        track.adsr_attack.set(self.amp_adsr[0].max(0.0));
        track.adsr_decay.set(self.amp_adsr[1].max(0.0));
        track.adsr_sustain.set(self.amp_adsr[2].clamp(0.0, 1.0));
        track.adsr_release.set(self.amp_adsr[3].max(0.0));
        track.glide_time.set(self.glide_time.clamp(0.0, 0.5));
    }

    fn set_freq_mult(&self, track: &TrackState, i: usize) {
        let oct = self.osc_octave[i] as f32;
        let cents = self.osc_detune[i];
        let mult = 2_f32.powf(oct + cents / 1200.0);
        track.osc_freq_mult[i].set(mult);
    }

    fn set_unison(&self, track: &TrackState, i: usize) {
        let mut count = self.osc_unison_count[i].clamp(1, 5);
        let spread = self.osc_unison_spread[i];
        if !self.osc_unison_enabled[i] || count <= 1 {
            count = 1;
        }

        let vol = 1.0 / count as f32;
        for c in 0..5 {
            if c < count {
                let t = if count > 1 {
                    c as f32 / (count - 1) as f32
                } else {
                    0.5
                };
                let cents = -spread * 0.5 + t * spread;
                let detune = 2_f32.powf(cents / 1200.0);
                track.osc_unison_detune[i][c].set(detune);
                track.osc_unison_vol[i][c].set(vol);
            } else {
                track.osc_unison_detune[i][c].set(1.0);
                track.osc_unison_vol[i][c].set(0.0);
            }
        }
    }
}
