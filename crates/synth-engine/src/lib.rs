pub mod arp;
pub use arp::{
    ArpEvents, ArpMode, ArpShared, ArpState, ClockDiv, Scale, ScaleWalker, ScaleWalkerShared,
};

pub mod audio;
pub use audio::{build_synth_graph, AudioState, VOICE_COUNT};

pub mod denormals;
pub use denormals::enable_ftz_on_current_thread;

pub mod handle;
pub use handle::SynthEngineHandle;

pub mod patch;
pub use patch::Patch;

pub mod voice;
pub use voice::VoiceAllocator;

pub mod multi;
pub use multi::{MultiTrackEngine, TrackState, TRACK_COUNT};
pub use synth_dsp::crystallizer::{Crystallizer, CrystallizerShared};
pub use synth_dsp::shimmer::{ShimmerReverb, ShimmerShared};

// Re-export the wire-ready protocol so downstream crates depending on
// synth-engine can pick up the types without a separate synth-control import.
pub use synth_control::{all_params, Command, ParamDescriptor, ParamId, ParamKind};
