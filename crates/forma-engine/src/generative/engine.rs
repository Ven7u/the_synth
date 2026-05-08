//! `AmbientEngine` — multi-track engine plus macro/scene layer.
//!
//! The `MultiTrackEngine` from the parent `forma-engine` crate remains the
//! DSP core. This module adds orchestration features used by ambient hosts:
//! - Macro runtime (many params moved by few controls)
//! - Scene capture/load (JSON-serializable)

use anyhow::Context;
use fundsp::prelude32::{shared, Shared};
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};
use std::path::Path;

use super::generators::{EuclideanShared, GenerativeMode, ProbTableShared};
use super::markov::MarkovEngineShared;
use super::patch::AmbientPatch;

pub use crate::{MultiTrackEngine, TrackState, TRACK_COUNT, VOICE_COUNT};

pub const MACRO_COUNT: usize = 8;
pub const ACTIVE_MACRO_KNOBS: usize = 6;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MacroSetKind {
    AmbientCore,
    PulseSequencer,
    CinematicScore,
}

impl MacroSetKind {
    pub const ALL: [MacroSetKind; 3] = [
        MacroSetKind::AmbientCore,
        MacroSetKind::PulseSequencer,
        MacroSetKind::CinematicScore,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MacroSetKind::AmbientCore => "Ambient Core",
            MacroSetKind::PulseSequencer => "Pulse / Sequencer",
            MacroSetKind::CinematicScore => "Cinematic Score",
        }
    }
}

fn default_macro_set() -> MacroSetKind {
    MacroSetKind::AmbientCore
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MacroParam {
    TrackVolume,
    TrackCutoff,
    TrackResonance,
    TrackShimmerSend,
    TrackCrystalSend,
    MasterVolume,
    ShimmerMix,
    ShimmerAmount,
    ShimmerSize,
    ShimmerDamp,
    CrystalMix,
    CrystalGrainMs,
    CrystalScatter,
    CrystalFeedback,
    CrystalDelayMs,
    ArpGate,
    WalkerGate,
    WalkerBpm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroTarget {
    pub track: Option<usize>,
    pub param: MacroParam,
    pub min: f32,
    pub max: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneMacro {
    pub name: String,
    pub value: f32,
    pub targets: Vec<MacroTarget>,
}

/// One slot in a harmonic sequence: key, scale, and how many phrases it lasts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarmonicSlot {
    pub root: u8,
    pub scale: u8,
    pub phrases: u8,
}

fn default_clock_div() -> u8 {
    4
}

/// Serializable snapshot of the Markov engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkovScene {
    pub root: u8,
    pub scale: u8,
    pub density: f32,
    pub bars_per_phrase: u32,
    pub mood: Vec<f32>,
    pub voice_roles: Vec<u8>,
    pub voice_densities: Vec<f32>,
    pub voice_enabled: Vec<bool>,
    pub chord_attraction: f32,
    pub bass_lock: bool,
    pub dissonance_resolve: bool,
    pub dissonance_threshold: u8,
    pub register_drift: f32,
    pub generative_mode: u8,
    /// Clock division: how many BeatClock subdivisions per Markov step.
    /// 1=16th, 2=8th, 4=quarter (default), 8=half note.
    #[serde(default = "default_clock_div")]
    pub clock_div: u8,
    /// Harmonic sequence. If empty or length 1, root/scale above is used statically.
    /// Ignored when `timeline` is present.
    #[serde(default)]
    pub harmonic_seq: Vec<HarmonicSlot>,

    /// Timeline: ordered sections that modulate mood, density, tonality, etc. over time.
    /// When present, replaces `harmonic_seq` for temporal modulation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline: Option<Vec<super::markov::TimelineSection>>,

    /// Whether the timeline loops back to section 0 after the last section.
    #[serde(default)]
    pub timeline_loop: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTrack {
    pub patch_path: String,
    pub patch: AmbientPatch,
    pub volume: f32,
    pub cutoff: f32,
    pub resonance: f32,
    pub shimmer_send: f32,
    pub crystal_send: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneGlobal {
    pub master_vol: f32,
    pub shimmer_mix: f32,
    pub shimmer_amount: f32,
    pub shimmer_size: f32,
    pub shimmer_damp: f32,
    pub shimmer_pitch: u8,
    pub crystal_mix: f32,
    pub crystal_grain_ms: f32,
    pub crystal_scatter: f32,
    pub crystal_feedback: f32,
    pub crystal_delay_ms: f32,
    pub crystal_pitch: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub name: String,
    pub bpm: u32,
    pub key: u8,
    pub scale: String,
    #[serde(default = "default_macro_set")]
    pub macro_set: MacroSetKind,
    pub tracks: [SceneTrack; TRACK_COUNT],
    pub macros: Vec<SceneMacro>,
    pub global: SceneGlobal,
    /// Markov engine state. Present when mode was Markov at save time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markov: Option<MarkovScene>,
}

pub struct AmbientEngine {
    core: MultiTrackEngine,
    pub macro_values: [Shared; MACRO_COUNT],
    pub macro_names: [String; MACRO_COUNT],
    pub macro_targets: [Vec<MacroTarget>; MACRO_COUNT],
    pub track_patch_paths: [String; TRACK_COUNT],
    pub track_patch_names: [String; TRACK_COUNT],
    pub track_patches: [AmbientPatch; TRACK_COUNT],
    pub macro_set_kind: MacroSetKind,
    // Generative pattern configs (Phase 8.2)
    pub euclidean_configs: [EuclideanShared; TRACK_COUNT],
    pub prob_table_configs: [ProbTableShared; TRACK_COUNT],
    pub generative_modes: [std::sync::Arc<std::sync::atomic::AtomicU8>; TRACK_COUNT],
    // Markov music engine shared config (Phase 8.3). Thread-safe half; audio-thread
    // `MarkovEngine` lives in the cpal callback as local state.
    pub markov_shared: MarkovEngineShared,
}

impl AmbientEngine {
    pub fn new(sr: f64) -> Self {
        let mut this = Self {
            core: MultiTrackEngine::new(sr),
            macro_values: std::array::from_fn(|_| shared(0.0)),
            macro_names: std::array::from_fn(|i| format!("Macro {}", i + 1)),
            macro_targets: std::array::from_fn(|_| Vec::new()),
            track_patch_paths: std::array::from_fn(|_| String::new()),
            track_patch_names: std::array::from_fn(|_| "Init".to_string()),
            track_patches: std::array::from_fn(|_| AmbientPatch::default()),
            macro_set_kind: MacroSetKind::AmbientCore,
            euclidean_configs: std::array::from_fn(|_| EuclideanShared::default()),
            prob_table_configs: std::array::from_fn(|_| ProbTableShared::default()),
            generative_modes: std::array::from_fn(|_| {
                std::sync::Arc::new(std::sync::atomic::AtomicU8::new(GenerativeMode::Off as u8))
            }),
            markov_shared: MarkovEngineShared::new(TRACK_COUNT),
        };
        this.apply_macro_set(MacroSetKind::AmbientCore);
        this
    }

    fn clear_macro_set(&mut self) {
        for i in 0..MACRO_COUNT {
            self.macro_names[i] = format!("Macro {}", i + 1);
            self.macro_targets[i].clear();
        }
    }

    fn apply_macro_set(&mut self, kind: MacroSetKind) {
        self.macro_set_kind = kind;
        self.clear_macro_set();
        match kind {
            MacroSetKind::AmbientCore => self.build_set_ambient_core(),
            MacroSetKind::PulseSequencer => self.build_set_pulse(),
            MacroSetKind::CinematicScore => self.build_set_cinematic(),
        }
    }

    fn build_set_ambient_core(&mut self) {
        self.macro_names[0] = "Space".to_string();
        self.macro_names[1] = "Tone".to_string();
        self.macro_names[2] = "Motion".to_string();
        self.macro_names[3] = "Density".to_string();
        self.macro_names[4] = "Tension".to_string();
        self.macro_names[5] = "Texture".to_string();

        for ti in 0..TRACK_COUNT {
            self.macro_targets[0].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::TrackShimmerSend,
                min: 0.05,
                max: 0.85,
            });
            self.macro_targets[0].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::TrackCrystalSend,
                min: 0.0,
                max: 0.45,
            });
            self.macro_targets[1].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::TrackCutoff,
                min: 900.0,
                max: 9000.0,
            });
            self.macro_targets[1].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::TrackResonance,
                min: 0.2,
                max: 1.2,
            });
            self.macro_targets[2].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::ArpGate,
                min: 0.35,
                max: 0.85,
            });
            self.macro_targets[2].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::WalkerGate,
                min: 0.30,
                max: 0.85,
            });
            self.macro_targets[2].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::WalkerBpm,
                min: 90.0,
                max: 162.0,
            });
            self.macro_targets[4].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::TrackResonance,
                min: 0.25,
                max: 1.60,
            });
        }

        self.macro_targets[0].push(MacroTarget {
            track: None,
            param: MacroParam::ShimmerMix,
            min: 0.12,
            max: 0.75,
        });
        self.macro_targets[0].push(MacroTarget {
            track: None,
            param: MacroParam::ShimmerSize,
            min: 0.45,
            max: 1.00,
        });
        self.macro_targets[1].push(MacroTarget {
            track: None,
            param: MacroParam::ShimmerDamp,
            min: 0.75,
            max: 0.25,
        });
        self.macro_targets[2].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalScatter,
            min: 0.10,
            max: 0.70,
        });
        self.macro_targets[3].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalMix,
            min: 0.00,
            max: 0.35,
        });
        self.macro_targets[3].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalFeedback,
            min: 0.20,
            max: 0.60,
        });
        self.macro_targets[4].push(MacroTarget {
            track: None,
            param: MacroParam::ShimmerAmount,
            min: 0.25,
            max: 0.90,
        });
        self.macro_targets[5].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalGrainMs,
            min: 220.0,
            max: 70.0,
        });
        self.macro_targets[5].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalDelayMs,
            min: 420.0,
            max: 140.0,
        });
        self.macro_targets[5].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalScatter,
            min: 0.05,
            max: 0.85,
        });
        self.macro_targets[5].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalMix,
            min: 0.05,
            max: 0.45,
        });
    }

    fn build_set_pulse(&mut self) {
        self.macro_names[0] = "Groove".to_string();
        self.macro_names[1] = "Swing".to_string();
        self.macro_names[2] = "Accent".to_string();
        self.macro_names[3] = "Drive".to_string();
        self.macro_names[4] = "Delay Rh".to_string();
        self.macro_names[5] = "Width".to_string();

        for ti in 0..TRACK_COUNT {
            self.macro_targets[0].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::ArpGate,
                min: 0.30,
                max: 0.95,
            });
            self.macro_targets[0].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::WalkerGate,
                min: 0.25,
                max: 0.90,
            });
            self.macro_targets[0].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::WalkerBpm,
                min: 80.0,
                max: 180.0,
            });
            self.macro_targets[2].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::TrackResonance,
                min: 0.2,
                max: 2.0,
            });
            self.macro_targets[3].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::TrackCutoff,
                min: 700.0,
                max: 14000.0,
            });
        }
        self.macro_targets[3].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalMix,
            min: 0.0,
            max: 0.25,
        });
        self.macro_targets[4].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalDelayMs,
            min: 1200.0,
            max: 120.0,
        });
        self.macro_targets[4].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalFeedback,
            min: 0.10,
            max: 0.70,
        });
        self.macro_targets[5].push(MacroTarget {
            track: None,
            param: MacroParam::ShimmerMix,
            min: 0.05,
            max: 0.45,
        });
    }

    fn build_set_cinematic(&mut self) {
        self.macro_names[0] = "Scale".to_string();
        self.macro_names[1] = "Lift".to_string();
        self.macro_names[2] = "Suspense".to_string();
        self.macro_names[3] = "Air".to_string();
        self.macro_names[4] = "Weight".to_string();
        self.macro_names[5] = "Release".to_string();

        for ti in 0..TRACK_COUNT {
            self.macro_targets[0].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::TrackVolume,
                min: 0.45,
                max: 0.95,
            });
            self.macro_targets[1].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::TrackCutoff,
                min: 600.0,
                max: 10000.0,
            });
            self.macro_targets[2].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::TrackResonance,
                min: 0.20,
                max: 1.40,
            });
            self.macro_targets[4].push(MacroTarget {
                track: Some(ti),
                param: MacroParam::TrackVolume,
                min: 0.30,
                max: 1.00,
            });
        }

        self.macro_targets[0].push(MacroTarget {
            track: None,
            param: MacroParam::ShimmerSize,
            min: 0.40,
            max: 1.00,
        });
        self.macro_targets[2].push(MacroTarget {
            track: None,
            param: MacroParam::ShimmerAmount,
            min: 0.10,
            max: 0.75,
        });
        self.macro_targets[3].push(MacroTarget {
            track: None,
            param: MacroParam::ShimmerMix,
            min: 0.10,
            max: 0.80,
        });
        self.macro_targets[3].push(MacroTarget {
            track: None,
            param: MacroParam::ShimmerDamp,
            min: 0.65,
            max: 0.20,
        });
        self.macro_targets[5].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalMix,
            min: 0.0,
            max: 0.35,
        });
        self.macro_targets[5].push(MacroTarget {
            track: None,
            param: MacroParam::CrystalFeedback,
            min: 0.10,
            max: 0.55,
        });
    }

    pub fn macro_set_kind(&self) -> MacroSetKind {
        self.macro_set_kind
    }

    pub fn set_macro_set(&mut self, kind: MacroSetKind) {
        self.apply_macro_set(kind);
        self.evaluate_macros();
    }

    pub fn macro_set_catalog() -> &'static [MacroSetKind] {
        &MacroSetKind::ALL
    }

    pub fn set_macro_value(&self, idx: usize, value: f32) {
        if idx < MACRO_COUNT {
            self.macro_values[idx].set(value.clamp(0.0, 1.0));
        }
    }

    pub fn apply_patch_to_track(
        &mut self,
        track: usize,
        patch_path: impl Into<String>,
        patch: &AmbientPatch,
    ) {
        if track >= TRACK_COUNT {
            return;
        }
        patch.apply_to_track(&self.core.tracks[track]);
        self.track_patch_paths[track] = patch_path.into();
        self.track_patch_names[track] = patch.name.clone();
        self.track_patches[track] = patch.clone();
    }

    pub fn macro_value(&self, idx: usize) -> f32 {
        if idx < MACRO_COUNT {
            self.macro_values[idx].value()
        } else {
            0.0
        }
    }

    pub fn evaluate_macros(&mut self) {
        for mi in 0..MACRO_COUNT {
            let v = self.macro_values[mi].value().clamp(0.0, 1.0);
            let targets = self.macro_targets[mi].clone();
            for t in &targets {
                let mapped = t.min + (t.max - t.min) * v;
                self.apply_macro_target(t, mapped);
            }
        }
        self.enforce_macro_safety();
    }

    fn enforce_macro_safety(&mut self) {
        let shim = self.core.shimmer.mix.value().clamp(0.0, 0.80);
        let crys = self.core.crystal.mix.value().clamp(0.0, 0.45);
        let wet_sum = shim + crys;
        if wet_sum > 0.95 {
            let scale = 0.95 / wet_sum;
            self.core.shimmer.mix.set(shim * scale);
            self.core.crystal.mix.set(crys * scale);
        } else {
            self.core.shimmer.mix.set(shim);
            self.core.crystal.mix.set(crys);
        }
        self.core
            .crystal
            .feedback
            .set(self.core.crystal.feedback.value().clamp(0.0, 0.65));
    }

    fn apply_macro_target(&mut self, target: &MacroTarget, value: f32) {
        match target.param {
            MacroParam::TrackVolume => {
                if let Some(ti) = target.track.filter(|&ti| ti < TRACK_COUNT) {
                    self.core.tracks[ti].track_vol.set(value.clamp(0.0, 1.0));
                }
            }
            MacroParam::TrackCutoff => {
                if let Some(ti) = target.track.filter(|&ti| ti < TRACK_COUNT) {
                    self.core.tracks[ti].cutoff.set(value.clamp(80.0, 18_000.0));
                }
            }
            MacroParam::TrackResonance => {
                if let Some(ti) = target.track.filter(|&ti| ti < TRACK_COUNT) {
                    self.core.tracks[ti].resonance.set(value.clamp(0.1, 10.0));
                }
            }
            MacroParam::TrackShimmerSend => {
                if let Some(ti) = target.track.filter(|&ti| ti < TRACK_COUNT) {
                    self.core.tracks[ti].shimmer_send.set(value.clamp(0.0, 1.0));
                }
            }
            MacroParam::TrackCrystalSend => {
                if let Some(ti) = target.track.filter(|&ti| ti < TRACK_COUNT) {
                    self.core.tracks[ti].crystal_send.set(value.clamp(0.0, 1.0));
                }
            }
            MacroParam::MasterVolume => self.core.master_vol.set(value.clamp(0.0, 1.0)),
            MacroParam::ShimmerMix => self.core.shimmer.mix.set(value.clamp(0.0, 1.0)),
            MacroParam::ShimmerAmount => self.core.shimmer.shimmer.set(value.clamp(0.0, 1.0)),
            MacroParam::ShimmerSize => self.core.shimmer.size.set(value.clamp(0.0, 1.0)),
            MacroParam::ShimmerDamp => self.core.shimmer.damp.set(value.clamp(0.0, 1.0)),
            MacroParam::CrystalMix => self.core.crystal.mix.set(value.clamp(0.0, 1.0)),
            MacroParam::CrystalGrainMs => self.core.crystal.grain_ms.set(value.clamp(10.0, 400.0)),
            MacroParam::CrystalScatter => self.core.crystal.scatter.set(value.clamp(0.0, 1.0)),
            MacroParam::CrystalFeedback => self.core.crystal.feedback.set(value.clamp(0.0, 0.95)),
            MacroParam::CrystalDelayMs => self.core.crystal.delay_ms.set(value.clamp(20.0, 1200.0)),
            MacroParam::ArpGate => {
                if let Some(ti) = target.track.filter(|&ti| ti < TRACK_COUNT) {
                    self.core.arp_configs[ti].gate.set(value.clamp(0.05, 1.0));
                }
            }
            MacroParam::WalkerGate => {
                if let Some(ti) = target.track.filter(|&ti| ti < TRACK_COUNT) {
                    self.core.walker_configs[ti]
                        .gate
                        .set(value.clamp(0.05, 1.0));
                }
            }
            MacroParam::WalkerBpm => {
                if let Some(ti) = target.track.filter(|&ti| ti < TRACK_COUNT) {
                    self.core.walker_configs[ti]
                        .bpm
                        .set(value.clamp(20.0, 300.0));
                }
            }
        }
    }

    pub fn tick_glide(&mut self, frames: usize) {
        self.evaluate_macros();
        self.core.tick_glide(frames);
    }

    pub fn tick_lfo_sample(&self, ti: usize, lfo_phase: f32) {
        self.core.tick_lfo_sample(ti, lfo_phase);
    }

    pub fn get_stereo(&mut self) -> (f32, f32) {
        self.core.get_stereo()
    }

    /// Snapshot the Markov shared state into a serializable struct.
    pub fn capture_markov_scene(&self, generative_mode: u8) -> MarkovScene {
        let ms = &self.markov_shared;
        use std::sync::atomic::Ordering;
        let seq_len = ms.seq_len();
        let harmonic_seq = (0..seq_len)
            .map(|i| HarmonicSlot {
                root: ms.seq_root(i),
                scale: ms.seq_scales[i].load(Ordering::Relaxed),
                phrases: ms.seq_phrases(i),
            })
            .collect();
        MarkovScene {
            root: ms.root.load(Ordering::Relaxed),
            scale: ms.scale.load(Ordering::Relaxed),
            density: ms.density.value(),
            bars_per_phrase: ms.bars_per_phrase.load(Ordering::Relaxed),
            mood: (0..super::markov::N_MOODS)
                .map(|i| ms.mood.weight(i))
                .collect(),
            voice_roles: (0..TRACK_COUNT)
                .map(|i| ms.roles[i].load(Ordering::Relaxed))
                .collect(),
            voice_densities: (0..TRACK_COUNT)
                .map(|i| ms.voice_density[i].value())
                .collect(),
            voice_enabled: (0..TRACK_COUNT)
                .map(|i| ms.voice_enabled[i].load(Ordering::Relaxed))
                .collect(),
            chord_attraction: ms.chord_attraction.value(),
            bass_lock: ms.bass_lock.load(Ordering::Relaxed),
            dissonance_resolve: ms.dissonance_resolve.load(Ordering::Relaxed),
            dissonance_threshold: ms.dissonance_threshold.load(Ordering::Relaxed),
            register_drift: ms.register_drift.value(),
            generative_mode,
            clock_div: ms.clock_div(),
            harmonic_seq,
            // Timeline is populated by the caller (it lives on the control thread, not in the engine).
            timeline: None,
            timeline_loop: false,
        }
    }

    pub fn capture_scene(
        &self,
        name: impl Into<String>,
        bpm: u32,
        key: u8,
        scale: impl Into<String>,
    ) -> Scene {
        let tracks: [SceneTrack; TRACK_COUNT] = std::array::from_fn(|ti| {
            let t = &self.core.tracks[ti];
            SceneTrack {
                patch_path: self.track_patch_paths[ti].clone(),
                patch: self.track_patches[ti].clone(),
                volume: t.track_vol.value(),
                cutoff: t.cutoff.value(),
                resonance: t.resonance.value(),
                shimmer_send: t.shimmer_send.value(),
                crystal_send: t.crystal_send.value(),
            }
        });

        let mut macros = Vec::with_capacity(MACRO_COUNT);
        for i in 0..MACRO_COUNT {
            macros.push(SceneMacro {
                name: self.macro_names[i].clone(),
                value: self.macro_values[i].value(),
                targets: self.macro_targets[i].clone(),
            });
        }

        Scene {
            name: name.into(),
            bpm,
            key,
            scale: scale.into(),
            macro_set: self.macro_set_kind,
            tracks,
            macros,
            markov: None, // caller sets this via capture_markov_scene() if in Markov mode
            global: SceneGlobal {
                master_vol: self.core.master_vol.value(),
                shimmer_mix: self.core.shimmer.mix.value(),
                shimmer_amount: self.core.shimmer.shimmer.value(),
                shimmer_size: self.core.shimmer.size.value(),
                shimmer_damp: self.core.shimmer.damp.value(),
                shimmer_pitch: self
                    .core
                    .shimmer
                    .pitch
                    .load(std::sync::atomic::Ordering::Relaxed),
                crystal_mix: self.core.crystal.mix.value(),
                crystal_grain_ms: self.core.crystal.grain_ms.value(),
                crystal_scatter: self.core.crystal.scatter.value(),
                crystal_feedback: self.core.crystal.feedback.value(),
                crystal_delay_ms: self.core.crystal.delay_ms.value(),
                crystal_pitch: self
                    .core
                    .crystal
                    .pitch
                    .load(std::sync::atomic::Ordering::Relaxed),
            },
        }
    }

    pub fn apply_scene(&mut self, scene: &Scene) {
        self.apply_macro_set(scene.macro_set);
        for ti in 0..TRACK_COUNT {
            let src = &scene.tracks[ti];
            self.apply_patch_to_track(ti, src.patch_path.clone(), &src.patch);
            let t = &self.core.tracks[ti];
            t.track_vol.set(src.volume.clamp(0.0, 1.0));
            t.cutoff.set(src.cutoff.clamp(80.0, 18_000.0));
            t.resonance.set(src.resonance.clamp(0.1, 10.0));
            t.shimmer_send.set(src.shimmer_send.clamp(0.0, 1.0));
            t.crystal_send.set(src.crystal_send.clamp(0.0, 1.0));
        }

        self.core
            .master_vol
            .set(scene.global.master_vol.clamp(0.0, 1.0));
        self.core
            .shimmer
            .mix
            .set(scene.global.shimmer_mix.clamp(0.0, 1.0));
        self.core
            .shimmer
            .shimmer
            .set(scene.global.shimmer_amount.clamp(0.0, 1.0));
        self.core
            .shimmer
            .size
            .set(scene.global.shimmer_size.clamp(0.0, 1.0));
        self.core
            .shimmer
            .damp
            .set(scene.global.shimmer_damp.clamp(0.0, 1.0));
        self.core.shimmer.pitch.store(
            scene.global.shimmer_pitch,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.core
            .crystal
            .mix
            .set(scene.global.crystal_mix.clamp(0.0, 1.0));
        self.core
            .crystal
            .grain_ms
            .set(scene.global.crystal_grain_ms.clamp(10.0, 400.0));
        self.core
            .crystal
            .scatter
            .set(scene.global.crystal_scatter.clamp(0.0, 1.0));
        self.core
            .crystal
            .feedback
            .set(scene.global.crystal_feedback.clamp(0.0, 0.95));
        self.core
            .crystal
            .delay_ms
            .set(scene.global.crystal_delay_ms.clamp(20.0, 1200.0));
        self.core.crystal.pitch.store(
            scene.global.crystal_pitch,
            std::sync::atomic::Ordering::Relaxed,
        );

        for i in 0..MACRO_COUNT {
            if let Some(sm) = scene.macros.get(i) {
                self.macro_names[i] = sm.name.clone();
                self.macro_values[i].set(sm.value.clamp(0.0, 1.0));
                self.macro_targets[i] = sm.targets.clone();
            }
        }

        self.evaluate_macros();

        // Restore Markov state if present
        if let Some(ms_data) = &scene.markov {
            use std::sync::atomic::Ordering;
            let ms = &self.markov_shared;
            ms.root.store(ms_data.root, Ordering::Relaxed);
            ms.scale.store(ms_data.scale, Ordering::Relaxed);
            ms.density.set(ms_data.density);
            ms.bars_per_phrase
                .store(ms_data.bars_per_phrase, Ordering::Relaxed);
            if ms_data.mood.len() == super::markov::N_MOODS {
                let arr: [f32; super::markov::N_MOODS] = std::array::from_fn(|i| ms_data.mood[i]);
                ms.mood.set(&arr);
            }
            for i in 0..TRACK_COUNT {
                if let Some(&r) = ms_data.voice_roles.get(i) {
                    ms.roles[i].store(r, Ordering::Relaxed);
                }
                if let Some(&d) = ms_data.voice_densities.get(i) {
                    ms.voice_density[i].set(d);
                }
                if let Some(&e) = ms_data.voice_enabled.get(i) {
                    ms.voice_enabled[i].store(e, Ordering::Relaxed);
                }
            }
            ms.chord_attraction.set(ms_data.chord_attraction);
            ms.bass_lock.store(ms_data.bass_lock, Ordering::Relaxed);
            ms.dissonance_resolve
                .store(ms_data.dissonance_resolve, Ordering::Relaxed);
            ms.dissonance_threshold
                .store(ms_data.dissonance_threshold, Ordering::Relaxed);
            ms.register_drift.set(ms_data.register_drift);
            ms.clock_div
                .store(ms_data.clock_div.max(1), Ordering::Relaxed);
            // Restore harmonic sequence
            let seq = &ms_data.harmonic_seq;
            let seq_len = seq
                .len()
                .clamp(1, super::markov::MarkovEngineShared::SEQ_MAX);
            ms.seq_len.store(seq_len as u8, Ordering::Relaxed);
            for (i, slot) in seq.iter().take(seq_len).enumerate() {
                ms.seq_roots[i].store(slot.root, Ordering::Relaxed);
                ms.seq_scales[i].store(slot.scale, Ordering::Relaxed);
                ms.seq_phrases[i].store(slot.phrases.max(1), Ordering::Relaxed);
            }
            // If sequence non-empty, seed active root/scale from slot 0
            if !seq.is_empty() {
                ms.root.store(seq[0].root, Ordering::Relaxed);
                ms.scale.store(seq[0].scale, Ordering::Relaxed);
            }
            // Propagate generative mode to all tracks
            for ti in 0..TRACK_COUNT {
                self.generative_modes[ti].store(ms_data.generative_mode, Ordering::Relaxed);
            }
        }
    }
}

impl Deref for AmbientEngine {
    type Target = MultiTrackEngine;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl DerefMut for AmbientEngine {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

pub fn save_scene_json(path: impl AsRef<Path>, scene: &Scene) -> anyhow::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(scene).context("serialize scene")?;
    std::fs::write(path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn load_scene_json(path: impl AsRef<Path>) -> anyhow::Result<Scene> {
    let path = path.as_ref();
    let json = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let scene = serde_json::from_str::<Scene>(&json).context("parse scene json")?;
    Ok(scene)
}

/// Wrap a single legacy patch into a 4-track Scene:
/// - Track 1: original patch
/// - Track 2..4: silent init layers
pub fn scene_from_single_patch(
    patch_path: impl Into<String>,
    patch: &AmbientPatch,
    scene_name: impl Into<String>,
    bpm: u32,
    key: u8,
    scale: impl Into<String>,
) -> Scene {
    let patch_path = patch_path.into();
    let scale = scale.into();
    let tracks: [SceneTrack; TRACK_COUNT] = std::array::from_fn(|ti| {
        if ti == 0 {
            SceneTrack {
                patch_path: patch_path.clone(),
                patch: patch.clone(),
                volume: 1.0,
                cutoff: patch.filter_cutoff.clamp(80.0, 18_000.0),
                resonance: patch.filter_q.clamp(0.1, 10.0),
                shimmer_send: 0.0,
                crystal_send: 0.0,
            }
        } else {
            SceneTrack {
                patch_path: String::new(),
                patch: AmbientPatch::default(),
                volume: 0.0,
                cutoff: 3000.0,
                resonance: 0.3,
                shimmer_send: 0.0,
                crystal_send: 0.0,
            }
        }
    });

    Scene {
        name: scene_name.into(),
        bpm,
        key: key % 12,
        scale,
        macro_set: MacroSetKind::AmbientCore,
        tracks,
        macros: Vec::new(),
        markov: None,
        global: SceneGlobal {
            master_vol: 0.7,
            shimmer_mix: 0.0,
            shimmer_amount: 0.0,
            shimmer_size: 0.6,
            shimmer_damp: 0.5,
            shimmer_pitch: 1,
            crystal_mix: 0.0,
            crystal_grain_ms: 120.0,
            crystal_scatter: 0.25,
            crystal_feedback: 0.35,
            crystal_delay_ms: 260.0,
            crystal_pitch: 2,
        },
    }
}

/// Convenience migration utility:
/// read a legacy single-patch JSON and save an equivalent Scene JSON.
pub fn migrate_patch_json_to_scene_json(
    patch_json_path: impl AsRef<Path>,
    scene_json_path: impl AsRef<Path>,
    scene_name: impl Into<String>,
    bpm: u32,
    key: u8,
    scale: impl Into<String>,
) -> anyhow::Result<()> {
    let patch_path = patch_json_path.as_ref();
    let patch = AmbientPatch::from_file(patch_path)
        .with_context(|| format!("load patch {}", patch_path.display()))?;
    let scene = scene_from_single_patch(
        patch_path.to_string_lossy().to_string(),
        &patch,
        scene_name,
        bpm,
        key,
        scale,
    );
    save_scene_json(scene_json_path, &scene)
}
