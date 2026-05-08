//! Generative music layer on top of `forma-engine`'s multi-track core.
//!
//! Originally lived in a separate `ambient-engine` crate; merged in so
//! there is a single engine crate (hosting both the single-voice DSP graph
//! and the ambient / scene-driven / Markov-generative orchestration) that
//! any host — `forma`, `forma-ambient`, `forma-bevy`, a future plugin
//! shell — can depend on with one import.
//!
//! # Modules
//!
//! * [`engine`] — [`AmbientEngine`], macros, scene capture/load (JSON I/O).
//! * [`generators`] — event-driven pattern generators (Euclidean, probability tables).
//! * [`markov`] — Markov music system: harmonic/rhythmic/melodic chains,
//!   moods, voice roles, phrase counter, timeline.
//! * [`patch`] — [`AmbientPatch`] struct (scene-side per-track patch snapshot).
//!
//! # Public surface
//!
//! This module re-exports every public item from the four submodules so
//! downstream code can `use forma_engine::generative::*;` without needing
//! to know which submodule a type lives in.

pub mod engine;
pub mod generators;
pub mod markov;
pub mod patch;

pub use engine::{
    load_scene_json, migrate_patch_json_to_scene_json, save_scene_json, scene_from_single_patch,
    AmbientEngine, HarmonicSlot, MacroParam, MacroSetKind, MacroTarget, MarkovScene, Scene,
    SceneGlobal, SceneMacro, SceneTrack, ACTIVE_MACRO_KNOBS, MACRO_COUNT,
};
// Convenience re-exports: the engine crate's multi-track constants so that
// consumers of `forma_engine::generative::*` don't need a separate import
// from the parent crate for `TRACK_COUNT` / `VOICE_COUNT` / `TrackState`.
pub use crate::{TrackState, TRACK_COUNT, VOICE_COUNT};
pub use forma_common::{BeatClock, BeatClockShared, BeatEvents, BeatPosition};
pub use generators::{
    EuclideanGen, EuclideanShared, GenEvent, GenerativeMode, ProbTableGen, ProbTableShared,
    EUCLIDEAN_MAX_STEPS, PROB_TABLE_MAX_STEPS,
};
pub use markov::{
    EffectsTargets,
    // Chains
    HarmonicChain,
    // State enums
    HarmonicFunction,
    // Matrix types
    HarmonicMatrix,
    // Engine
    MarkovEngine,
    MarkovEngineShared,
    MarkovVoice,
    MelodicChain,
    MelodicMatrix,
    // Mood
    MoodBlend,
    MoodSet,
    // Phrase
    PhraseCounter,
    PhraseEvents,
    ResolvedState,
    RhythmicChain,
    RhythmicMatrix,
    RhythmicState,
    Scale,
    // Timeline
    Timeline,
    TimelineSection,
    TimelineStatus,
    VoiceEvent,
    VoiceRole,
    ALL_MOODS,
    HARMONIC_STATES,
    // Launchpad
    LAUNCHPAD_COLS,
    MELODIC_STATES,
    MOOD_CALM,
    MOOD_COSMIC,
    MOOD_DARK,
    MOOD_EUPHORIC,
    MOOD_GRAVITY,
    MOOD_TENSE,
    N_MOODS,
    PHRASE_BOUNDARY_HARMONIC,
    RHYTHMIC_STATES,
};
pub use patch::AmbientPatch;
