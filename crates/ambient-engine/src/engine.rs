//! Ambient-engine — thin wrapper over synth-engine's generic multi-track layer.
//!
//! All core multi-track infrastructure lives in `synth_engine::multi`.
//! This crate re-exports it and provides a type alias for backwards compatibility.

pub use synth_engine::{TrackState, MultiTrackEngine, TRACK_COUNT, VOICE_COUNT};

/// Backwards-compatible alias. `AmbientEngine` is `MultiTrackEngine`.
pub type AmbientEngine = MultiTrackEngine;
