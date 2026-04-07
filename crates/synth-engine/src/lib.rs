pub mod arp;
pub use arp::{
    ArpMode, ArpShared, ArpState, ArpEvents,
    ClockDiv, Scale,
    ScaleWalker, ScaleWalkerShared,
};

pub mod audio;
pub use audio::{AudioState, VOICE_COUNT, build_synth_graph};

pub mod multi;
pub use multi::{TrackState, MultiTrackEngine, TRACK_COUNT};
pub use synth_dsp::shimmer::{ShimmerShared, ShimmerReverb};
pub use synth_dsp::crystallizer::{CrystallizerShared, Crystallizer};
