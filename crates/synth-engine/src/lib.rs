pub mod arp;
pub use arp::{
    ArpMode, ArpShared, ArpState, ArpEvents,
    ClockDiv, Scale,
    ScaleWalker, ScaleWalkerShared,
};

pub mod audio;
pub use audio::{AudioState, VOICE_COUNT, build_synth_graph};

pub mod handle;
pub use handle::SynthEngineHandle;

pub mod patch;
pub use patch::Patch;

pub mod multi;
pub use multi::{TrackState, MultiTrackEngine, TRACK_COUNT};
pub use synth_dsp::shimmer::{ShimmerShared, ShimmerReverb};
pub use synth_dsp::crystallizer::{CrystallizerShared, Crystallizer};

// Re-export the wire-ready protocol so downstream crates depending on
// synth-engine can pick up the types without a separate synth-control import.
pub use synth_control::{Command, ParamId, ParamKind, ParamDescriptor, all_params};
