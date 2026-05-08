pub mod arp;
pub use arp::{
    ArpEvents, ArpMode, ArpShared, ArpState, ClockDiv, Scale, ScaleWalker, ScaleWalkerShared,
};

pub mod audio;
pub use audio::{build_synth_graph, AudioState, VOICE_COUNT};

pub mod denormals;
pub use denormals::enable_ftz_on_current_thread;

pub mod gated_voice;
pub use gated_voice::GatedVoice;

pub mod handle;
pub use handle::SynthEngineHandle;

pub mod patch;
pub use patch::Patch;

pub mod voice;
pub use voice::VoiceAllocator;

pub mod drum;
pub use drum::{
    DrumDspState, DrumTrackState, DRUM_LANES, DRUM_LANE_NAMES, DRUM_PATTERNS, DRUM_STEPS,
};

pub mod multi;
pub use forma_dsp::crystallizer::{Crystallizer, CrystallizerShared};
pub use forma_dsp::shimmer::{ShimmerReverb, ShimmerShared};
pub use multi::{MultiTrackEngine, TrackState, TRACK_COUNT};

/// Generative music layer (Markov, scenes, macros, timeline).
/// Lived in a separate `ambient-engine` crate pre-consolidation.
pub mod generative;

// Re-export the wire-ready protocol so downstream crates depending on
// forma-engine can pick up the types without a separate forma-control import.
pub use forma_control::{all_params, Command, ParamDescriptor, ParamId, ParamKind};
