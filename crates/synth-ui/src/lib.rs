//! Shared egui panels, widgets, and types for synth applications.
//!
//! Used by both `the-synth` (standalone) and `synth-plugin` (CLAP/VST3).
//! Zero audio dependencies — pure egui.

pub mod frame;
pub mod panels;
pub mod param_writer;
pub mod state;
pub mod theme;
pub mod widgets;

pub use frame::SynthFrame;
pub use param_writer::ParamWriter;
pub use state::SynthUiState;
pub use theme::{builtin_themes, midnight, phosphor, winamp_classic, SynthTheme};
pub use widgets::knob;
