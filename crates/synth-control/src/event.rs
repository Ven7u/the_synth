//! ControlEvent — the universal language between input sources and the engine.
//!
//! `ParamId` used inside `ControlEvent::SetParam` is the one from
//! [`crate::protocol`] — a single canonical identifier enum shared with the
//! wire-ready `Command` layer.

use crate::protocol::ParamId;
use crossbeam_channel::{bounded, Receiver, Sender};

/// All discrete events that any input source can produce for the audio engine.
#[derive(Debug, Clone)]
pub enum ControlEvent {
    /// Play a note (MIDI pitch 0–127, velocity 0–127).
    /// `track` selects the destination track (0 = default/only track).
    NoteOn { pitch: u8, velocity: u8, track: u8 },
    /// Stop a note.
    /// `track` selects the destination track (0 = default/only track).
    NoteOff { pitch: u8, track: u8 },
    /// Write a named parameter directly.
    SetParam { param: ParamId, value: f32 },
    /// Latch a chord into the track's arpeggiator.
    /// The arp iterates these pitches until a new ChordHold arrives.
    /// Sent by the UI/MIDI layer; heap allocation is fine on the sender thread.
    ChordHold { track: u8, notes: Vec<u8> },
    /// Restart arpeggiator timing/step state for a track.
    ArpRestart { track: u8 },
    /// Restart scale-walker timing/index state for a track.
    WalkerRestart { track: u8 },
}

/// Push side of the control channel (clone-able, Send).
pub type ControlSender = Sender<ControlEvent>;
/// Pull side of the control channel (single consumer — the audio callback).
pub type ControlReceiver = Receiver<ControlEvent>;

/// Create a bounded lock-free control event channel.
pub fn make_control_channel(capacity: usize) -> (ControlSender, ControlReceiver) {
    bounded(capacity)
}
