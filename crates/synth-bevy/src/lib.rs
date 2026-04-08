//! `synth-bevy` — Bevy integration shell for `ambient-engine`.
//!
//! Enable the `bevy` feature to get the plugin implementation:
//! `synth-bevy = { path = "../synth-bevy", features = ["bevy"] }`

#[cfg(feature = "bevy")]
mod plugin;

#[cfg(feature = "bevy")]
pub use plugin::*;

#[cfg(not(feature = "bevy"))]
pub const FEATURE_HINT: &str =
    "Enable the `bevy` feature on synth-bevy to access SynthPlugin and SynthEvent.";
