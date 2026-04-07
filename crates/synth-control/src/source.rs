//! ControlSource trait — any type that produces ControlEvents.

use crate::ControlEvent;

/// A source that can be polled for control events.
/// Implementations include MIDI devices, keyboard adapters, and generative engines.
pub trait ControlSource {
    /// Return the next available event, or `None` if the source is empty.
    fn poll(&mut self) -> Option<ControlEvent>;
}
