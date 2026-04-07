//! ControlEvent — the universal language between input sources and the engine.

use crossbeam_channel::{bounded, Receiver, Sender};

/// All discrete events that any input source can produce for the audio engine.
#[derive(Debug, Clone)]
pub enum ControlEvent {
    /// Play a note (MIDI pitch 0–127, velocity 0–127).
    /// `track` selects the destination track (0 = default/only track).
    NoteOn  { pitch: u8, velocity: u8, track: u8 },
    /// Stop a note.
    /// `track` selects the destination track (0 = default/only track).
    NoteOff { pitch: u8, track: u8 },
    /// Write a named parameter directly.
    SetParam { param: ParamId, value: f32 },
}

/// Addressable engine parameters reachable via `ControlEvent::SetParam`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamId {
    FilterCutoff,
    FilterResonance,
    LfoDepth,
    MasterVolume,
    LfoPitchMult,
}

/// Push side of the control channel (clone-able, Send).
pub type ControlSender   = Sender<ControlEvent>;
/// Pull side of the control channel (single consumer — the audio callback).
pub type ControlReceiver = Receiver<ControlEvent>;

/// Create a bounded lock-free control event channel.
pub fn make_control_channel(capacity: usize) -> (ControlSender, ControlReceiver) {
    bounded(capacity)
}
