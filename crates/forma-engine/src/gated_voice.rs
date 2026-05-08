//! `GatedVoice` — an `AudioNode` wrapper that short-circuits when a voice
//! is confirmed silent.
//!
//! Context: the per-voice DSP chain (3 oscillators × up to 5 unison copies,
//! Moog filter, two ADSR envelopes) runs on every sample regardless of
//! whether the voice is making sound. Zeroing the output amplitude via the
//! amp envelope still pays the full cost of evaluating the chain. With six
//! voices, that's ~90 always-ticking oscillators even at idle.
//!
//! `GatedVoice` wraps the voice's sub-graph and reads a per-voice
//! `AtomicBool` flag at the start of every `tick`. When the flag is `false`
//! the wrapper returns a zeroed output immediately and skips the inner
//! chain entirely. When the flag transitions back to `true` (new note
//! allocated to this slot) we call `inner.reset()` so the filter and
//! oscillator state start clean — avoids pops and phase drift after long
//! idle periods.
//!
//! The flag is maintained by `VoiceAllocator::update_audibility`, run once
//! per audio buffer after event drain and arp/walker tick. "Audible" means:
//! the gate is currently held, OR the amp envelope hasn't gone idle, OR a
//! retrigger countdown is pending. This matches the signal used by the
//! existing release-cleanup pass.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use fundsp::prelude32::*;

/// Voice-level bypass wrapper. Generic over the wrapped sub-graph so the
/// whole voice chain's concrete type can flow through fundsp's DSL without
/// boxing. `Inputs = U0` because voices generate their own signal.
#[derive(Clone)]
pub struct GatedVoice<X>
where
    X: AudioNode<Inputs = U0>,
{
    inner: X,
    audible: Arc<AtomicBool>,
    /// Tracks whether we ticked the inner graph on the previous sample.
    /// Used to trigger `inner.reset()` on false→true transitions so the
    /// filter/envelope/oscillator state starts clean after long silences.
    was_active: bool,
}

impl<X> GatedVoice<X>
where
    X: AudioNode<Inputs = U0>,
{
    pub fn new(inner: X, audible: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            audible,
            was_active: false,
        }
    }
}

impl<X> AudioNode for GatedVoice<X>
where
    X: AudioNode<Inputs = U0>,
{
    // Unique ID: "gated_vo" in ASCII.
    const ID: u64 = 0x6761_7465_645f_766f;
    type Inputs = U0;
    type Outputs = X::Outputs;

    fn reset(&mut self) {
        self.inner.reset();
        self.was_active = false;
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.inner.set_sample_rate(sample_rate);
    }

    #[inline]
    fn tick(&mut self, input: &Frame<f32, Self::Inputs>) -> Frame<f32, Self::Outputs> {
        // Relaxed load: the flag is written once per audio buffer by
        // VoiceAllocator; within a buffer it's constant. Relaxed is fine
        // because we don't depend on any other cross-thread ordering.
        let active = self.audible.load(Ordering::Relaxed);

        if !active {
            self.was_active = false;
            return Frame::default(); // Zeroed output.
        }

        if !self.was_active {
            // Wake-up transition. Reset the inner graph so the filter's
            // 4-pole memory, the oscillators' phases, and the envelopes'
            // stage machine all start from a known state. Prevents pops
            // from stale filter memory and phase drift from unison copies
            // that were frozen for seconds.
            self.inner.reset();
            self.was_active = true;
        }

        self.inner.tick(input)
    }
}
