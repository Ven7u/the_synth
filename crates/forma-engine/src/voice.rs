//! `VoiceAllocator` — audio-thread-owned voice lifecycle + event dispatch.
//!
//! Every host that drives the engine (the egui synth, a Bevy plugin, a CLAP
//! shell, a Swift app, a headless test harness) needs the same voice-
//! allocation logic:
//!
//! - A `VOICE_COUNT`-slot pool, with oldest-first stealing.
//! - A 128-wide hold-count table so keyboard + sequencer can't cut each
//!   other off on shared pitches.
//! - Click-free retrigger when the same note is played while still audible.
//! - An arpeggiator and a scale walker ticked once per audio buffer.
//!
//! Before this module existed, every host reimplemented this dance inline
//! in its cpal callback. Now the callback just holds a `VoiceAllocator`
//! and calls three methods:
//!
//! ```text
//!   allocator.begin_buffer(&state, &rx, frames, sr);  // once per buffer
//!   for _ in 0..frames {
//!       allocator.tick_sample(&state);                // once per sample
//!       // ... run DSP graph + post-processing ...
//!   }
//! ```
//!
//! All state is stack-allocated (no heap). All writes are lock-free atomics.
//! Safe to run on the real-time audio thread.

use std::sync::atomic::Ordering;

use forma_control::{ControlEvent, ControlReceiver, ParamId};
use fundsp::prelude::midi_hz;

use crate::arp::{ArpState, ScaleWalker};
use crate::audio::{AudioState, VOICE_COUNT};

/// Voice allocation + event dispatch for one audio thread.
pub struct VoiceAllocator {
    /// Per-slot: `Some(midi_pitch)` while a voice is held or still ringing
    /// in its release tail, `None` once fully idle.
    voice_notes: [Option<u8>; VOICE_COUNT],
    /// Round-robin steal cursor when every slot is occupied.
    steal_idx: usize,
    /// Reference count per MIDI pitch across all sources (keyboard, MIDI,
    /// sequencer). Voice gate only drops when the count reaches zero.
    pitch_hold_count: [u8; 128],
    /// Per-slot countdown (in samples). While > 0 the gate is forced low;
    /// it flips back to 1.0 when the countdown expires. Guarantees the ADSR
    /// sees a real 0→1 edge on same-buffer NoteOff+NoteOn sequences and on
    /// polyphonic retriggers.
    retrigger_countdown: [u8; VOICE_COUNT],
    /// Mono mode note stack — held notes newest-first. When the top note is
    /// released the previous note resumes. Max 32 simultaneous held notes.
    mono_stack: [u8; 32],
    mono_stack_len: usize,
    /// Arpeggiator internal state. Config is read each tick from
    /// `state.arp` (lock-free atomics).
    arp: ArpState,
    /// Scale-walker internal state.
    walker: ScaleWalker,
}

impl Default for VoiceAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceAllocator {
    pub fn new() -> Self {
        Self {
            voice_notes: [None; VOICE_COUNT],
            steal_idx: 0,
            pitch_hold_count: [0; 128],
            retrigger_countdown: [0; VOICE_COUNT],
            mono_stack: [0u8; 32],
            mono_stack_len: 0,
            arp: ArpState::new(),
            walker: ScaleWalker::new(),
        }
    }

    // =======================================================================
    // Buffer-level entry points
    // =======================================================================

    /// Call once at the top of each audio buffer, before sample generation.
    ///
    /// Steps, in order:
    ///
    /// 1. **Release cleanup** — free voice slots whose amp envelope has
    ///    decayed below the idle threshold. Skips slots still in a
    ///    retrigger countdown (their gate is briefly 0 by design).
    /// 2. **Event drain** — pull every `ControlEvent` from `rx` via
    ///    `try_recv` (never blocks), dispatching to the voice pool or the
    ///    arpeggiator depending on `state.arp.enabled`.
    /// 3. **Arp / walker tick** — both run once per buffer; resulting note
    ///    on/off events feed back into the voice pool.
    pub fn begin_buffer(
        &mut self,
        state: &AudioState,
        rx: &ControlReceiver,
        frames: usize,
        sr: f64,
    ) {
        self.release_cleanup(state);
        self.drain_events(state, rx);
        self.tick_arp_walker(state, frames, sr);
        // After all event processing for this buffer is final, publish the
        // per-voice audibility flag for the DSP graph's `GatedVoice` wrappers
        // to read. Voices whose flag is `false` will skip their entire
        // sub-graph (oscillators + filter + envelopes) for every sample of
        // this buffer — the main CPU win at idle.
        self.update_audibility(state);
    }

    /// Call once per sample, inside the buffer loop, before running the DSP
    /// graph. Drives the retrigger countdown and flips voice gates back to
    /// 1.0 when the countdown hits zero.
    #[inline]
    pub fn tick_sample(&mut self, state: &AudioState) {
        for vi in 0..VOICE_COUNT {
            if self.retrigger_countdown[vi] > 0 {
                self.retrigger_countdown[vi] -= 1;
                if self.retrigger_countdown[vi] == 0 {
                    state.voice_gates[vi].set(1.0);
                }
            }
        }
    }

    // =======================================================================
    // Private steps
    // =======================================================================

    fn release_cleanup(&mut self, state: &AudioState) {
        for (slot, note) in self.voice_notes.iter_mut().enumerate() {
            if note.is_some()
                && self.retrigger_countdown[slot] == 0
                && state.voice_gates[slot].value() < 0.5
                && state.amp_cursors[slot].value() < 0.5
            {
                if let Some(p) = *note {
                    self.pitch_hold_count[p as usize] = 0;
                }
                *note = None;
            }
        }
    }

    fn drain_events(&mut self, state: &AudioState, rx: &ControlReceiver) {
        while let Ok(ev) = rx.try_recv() {
            match ev {
                ControlEvent::NoteOn {
                    pitch, velocity, ..
                } => {
                    if state.arp.enabled.load(Ordering::Relaxed) {
                        self.arp.note_on(pitch);
                    } else {
                        let mono = state.mono_mode.load(Ordering::Relaxed);
                        if mono > 0 {
                            self.mono_note_on(state, pitch, velocity, mono == 2);
                        } else {
                            self.trigger_note(state, pitch, velocity);
                        }
                    }
                }
                ControlEvent::NoteOff { pitch, .. } => {
                    if state.arp.enabled.load(Ordering::Relaxed) {
                        let hold = state.arp.hold.load(Ordering::Relaxed);
                        self.arp.note_off(pitch, hold);
                    } else {
                        let mono = state.mono_mode.load(Ordering::Relaxed);
                        if mono > 0 {
                            self.mono_note_off(state, pitch, mono == 2);
                        } else {
                            self.release_note(state, pitch);
                        }
                    }
                }
                ControlEvent::SetParam { param, value } => {
                    // Only the legacy five params are routed inline. Everything
                    // else is written via `SynthEngineHandle`'s typed setters
                    // (see crates/forma-engine/src/handle.rs). Remaining
                    // ParamId variants reach the engine through direct atomic
                    // writes, not through this channel path.
                    match param {
                        ParamId::FilterCutoff => state.cutoff.set(value),
                        ParamId::FilterResonance => state.resonance.set(value),
                        ParamId::LfoDepth => state.lfo_depth.set(value),
                        ParamId::MasterVolume => state.master_vol.set(value),
                        ParamId::LfoPitchMult => state.lfo_pitch_mult.set(value),
                        _ => {}
                    }
                }
                ControlEvent::ChordHold { notes, .. } => {
                    self.arp.set_chord(&notes);
                }
                ControlEvent::ArpRestart { .. } => {
                    if let Some(pitch) = self.arp.restart() {
                        self.release_note(state, pitch);
                    }
                }
                ControlEvent::WalkerRestart { .. } => {
                    if let Some(pitch) = self.walker.restart() {
                        self.release_note(state, pitch);
                    }
                }
            }
        }
    }

    fn tick_arp_walker(&mut self, state: &AudioState, frames: usize, sr: f64) {
        let arp_ev = self.arp.tick(&state.arp, frames, sr);
        let walk_ev = self.walker.tick(&state.walker, frames, sr);
        for ev in [arp_ev, walk_ev] {
            if let Some(pitch) = ev.note_off {
                self.release_note(state, pitch);
            }
            if let Some(pitch) = ev.note_on {
                self.trigger_note(state, pitch, 127);
            }
        }
    }

    /// Recompute per-voice audibility and publish to `state.voice_audible`.
    ///
    /// A voice is audible (its DSP sub-graph must run) when any of:
    /// * gate is currently held high (note still pressed), or
    /// * amp envelope is not idle — cursor > 0.5 means the envelope is in
    ///   attack/decay/sustain/release (any non-zero stage), or
    /// * the per-slot retrigger countdown is still running (we deliberately
    ///   forced gate=0 for a few samples; the voice *is* active).
    ///
    /// Voices that fail all three checks will have their sub-graph skipped
    /// by `GatedVoice` for every sample of this buffer.
    fn update_audibility(&self, state: &AudioState) {
        use std::sync::atomic::Ordering;
        for vi in 0..VOICE_COUNT {
            let audible = state.voice_gates[vi].value() > 0.5
                || state.amp_cursors[vi].value() > 0.5
                || self.retrigger_countdown[vi] > 0;
            state.voice_audible[vi].store(audible, Ordering::Relaxed);
        }
    }

    // =======================================================================
    // Voice-slot primitives
    // =======================================================================

    /// Trigger or retrigger a voice for `pitch`.
    ///
    /// - Increments the pitch's hold count.
    /// - Allocates (or reuses) a slot (existing-pitch → empty → round-robin steal).
    /// - If the target slot is audibly playing (gate still high OR amp
    ///   envelope not yet idle): forces gate=0 and starts a 4-sample
    ///   countdown. The next `tick_sample` calls flip the gate back to 1.0,
    ///   giving the ADSR a real 0→1 edge.
    /// - Otherwise (fresh / silent slot): sets gate=1.0 immediately.
    fn trigger_note(&mut self, state: &AudioState, pitch: u8, velocity: u8) {
        let count = &mut self.pitch_hold_count[pitch as usize];
        *count = count.saturating_add(1);

        let n = self.voice_notes.len();
        let slot = self
            .voice_notes
            .iter()
            .position(|&v| v == Some(pitch))
            .or_else(|| self.voice_notes.iter().position(|v| v.is_none()))
            .unwrap_or_else(|| {
                let s = self.steal_idx % n;
                self.steal_idx += 1;
                s
            });

        // Audible if the gate is still held, or the amp envelope hasn't gone
        // idle yet (cursor > 0.5 means attack/decay/sustain/release — i.e.
        // still producing sound). Amp_cursor encoding: 0=idle, 1.x=A, 2.x=D,
        // 3=S, 4.x=R.
        let audible =
            state.voice_gates[slot].value() > 0.5 || state.amp_cursors[slot].value() > 0.5;

        self.voice_notes[slot] = Some(pitch);
        state.voice_freq_targets[slot].set(midi_hz(pitch as f64) as f32);
        state.voice_velocities[slot].set(velocity as f32 / 127.0);

        if audible {
            // Force a brief gap so the ADSR sees a real 0→1 edge.
            state.voice_gates[slot].set(0.0);
            self.retrigger_countdown[slot] = 4; // ~90 µs at 44.1 kHz
        } else {
            state.voice_gates[slot].set(1.0);
            self.retrigger_countdown[slot] = 0;
        }
    }

    /// Decrement the hold count for `pitch` and kill its gate only when the
    /// count reaches 0. `voice_notes[slot]` stays `Some(pitch)` until the
    /// envelope decays — the release-cleanup pass handles final slot freeing.
    fn release_note(&mut self, state: &AudioState, pitch: u8) {
        let count = &mut self.pitch_hold_count[pitch as usize];
        if *count > 0 {
            *count -= 1;
        }
        if *count == 0 {
            for (slot, note) in self.voice_notes.iter_mut().enumerate() {
                if *note == Some(pitch) {
                    state.voice_gates[slot].set(0.0);
                    break;
                }
            }
        }
    }

    // =======================================================================
    // Mono / legato primitives
    // =======================================================================

    /// Mono NoteOn — always uses voice slot 0.
    ///
    /// Pushes `pitch` onto the held-note stack.
    /// In legato mode, if the gate is already high the frequency is updated
    /// silently (no retrigger) so the ADSR keeps running and glide slides in.
    /// In mono mode the voice is always retriggered.
    fn mono_note_on(&mut self, state: &AudioState, pitch: u8, velocity: u8, legato: bool) {
        // Push onto stack (cap at 32; if full, drop the oldest entry).
        if self.mono_stack_len < self.mono_stack.len() {
            self.mono_stack[self.mono_stack_len] = pitch;
            self.mono_stack_len += 1;
        } else {
            // Shift left, drop oldest
            self.mono_stack.copy_within(1.., 0);
            self.mono_stack[self.mono_stack.len() - 1] = pitch;
        }

        let gate_high = state.voice_gates[0].value() > 0.5;

        state.voice_freq_targets[0].set(midi_hz(pitch as f64) as f32);
        state.voice_velocities[0].set(velocity as f32 / 127.0);
        self.voice_notes[0] = Some(pitch);

        if legato && gate_high {
            // Legato: slide to new pitch without envelope retrigger.
        } else {
            // Mono: always retrigger.
            let audible = gate_high || state.amp_cursors[0].value() > 0.5;
            if audible {
                state.voice_gates[0].set(0.0);
                self.retrigger_countdown[0] = 4;
            } else {
                state.voice_gates[0].set(1.0);
                self.retrigger_countdown[0] = 0;
            }
        }
    }

    /// Mono NoteOff — removes `pitch` from the stack.
    ///
    /// If other notes are still held the previous note resumes (last-note
    /// priority).  If the stack is empty the voice is released normally.
    fn mono_note_off(&mut self, state: &AudioState, pitch: u8, legato: bool) {
        // Remove pitch from stack.
        if let Some(pos) = self.mono_stack[..self.mono_stack_len]
            .iter()
            .rposition(|&p| p == pitch)
        {
            self.mono_stack
                .copy_within(pos + 1..self.mono_stack_len, pos);
            self.mono_stack_len -= 1;
        } else {
            return; // note wasn't in stack (spurious NoteOff)
        }

        if self.mono_stack_len > 0 {
            // Resume the most recently held note.
            let resume = self.mono_stack[self.mono_stack_len - 1];
            state.voice_freq_targets[0].set(midi_hz(resume as f64) as f32);
            self.voice_notes[0] = Some(resume);
            // In legato the gate stays high (smooth glide back).
            if !legato {
                state.voice_gates[0].set(0.0);
                self.retrigger_countdown[0] = 4;
            }
        } else {
            // No more held notes — release.
            state.voice_gates[0].set(0.0);
            self.voice_notes[0] = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use forma_control::{make_control_channel, ControlEvent};
    use std::sync::Arc;

    fn setup() -> (
        VoiceAllocator,
        Arc<AudioState>,
        forma_control::ControlSender,
        forma_control::ControlReceiver,
    ) {
        let state = Arc::new(AudioState::new());
        let (tx, rx) = make_control_channel(64);
        (VoiceAllocator::new(), state, tx, rx)
    }

    #[test]
    fn note_on_sets_gate_and_freq() {
        let (mut va, state, tx, rx) = setup();
        tx.try_send(ControlEvent::NoteOn {
            pitch: 69,
            velocity: 100,
            track: 0,
        })
        .unwrap();
        va.begin_buffer(&state, &rx, 64, 48_000.0);

        // First free slot is 0.
        assert_eq!(state.voice_gates[0].value(), 1.0);
        let a4 = midi_hz(69.0) as f32;
        assert!((state.voice_freq_targets[0].value() - a4).abs() < 1e-3);
    }

    #[test]
    fn note_off_kills_gate_when_count_reaches_zero() {
        let (mut va, state, tx, rx) = setup();
        tx.try_send(ControlEvent::NoteOn {
            pitch: 60,
            velocity: 100,
            track: 0,
        })
        .unwrap();
        tx.try_send(ControlEvent::NoteOff {
            pitch: 60,
            track: 0,
        })
        .unwrap();
        va.begin_buffer(&state, &rx, 64, 48_000.0);

        assert_eq!(state.voice_gates[0].value(), 0.0);
    }

    #[test]
    fn double_hold_keeps_gate_until_matching_release() {
        // Two sources holding the same pitch: one NoteOff must not kill it.
        // Send the second NoteOn in a *separate* buffer to avoid the
        // same-buffer retrigger countdown from zeroing the gate mid-test.
        let (mut va, state, tx, rx) = setup();

        // Source A: NoteOn(60).
        tx.try_send(ControlEvent::NoteOn {
            pitch: 60,
            velocity: 100,
            track: 0,
        })
        .unwrap();
        va.begin_buffer(&state, &rx, 64, 48_000.0);
        assert_eq!(state.voice_gates[0].value(), 1.0);

        // Source B: NoteOn(60) — count goes 1→2. Retrigger countdown fires
        // because the slot is still audible; advance 4 samples to clear it.
        tx.try_send(ControlEvent::NoteOn {
            pitch: 60,
            velocity: 100,
            track: 0,
        })
        .unwrap();
        va.begin_buffer(&state, &rx, 64, 48_000.0);
        for _ in 0..4 {
            va.tick_sample(&state);
        }
        assert_eq!(state.voice_gates[0].value(), 1.0);

        // Source A: NoteOff(60) — count 2→1, must NOT kill gate.
        tx.try_send(ControlEvent::NoteOff {
            pitch: 60,
            track: 0,
        })
        .unwrap();
        va.begin_buffer(&state, &rx, 64, 48_000.0);
        assert_eq!(state.voice_gates[0].value(), 1.0);

        // Source B: NoteOff(60) — count 1→0, gate drops.
        tx.try_send(ControlEvent::NoteOff {
            pitch: 60,
            track: 0,
        })
        .unwrap();
        va.begin_buffer(&state, &rx, 64, 48_000.0);
        assert_eq!(state.voice_gates[0].value(), 0.0);
    }

    #[test]
    fn retrigger_countdown_flips_gate_back_up() {
        // Simulate: note playing → NoteOff + NoteOn land in the same buffer.
        // With the retrigger countdown, gate must end at 0 right after
        // begin_buffer, then flip back to 1.0 after 4 tick_sample calls.
        let (mut va, state, tx, rx) = setup();
        // Drive the first attack.
        tx.try_send(ControlEvent::NoteOn {
            pitch: 62,
            velocity: 100,
            track: 0,
        })
        .unwrap();
        va.begin_buffer(&state, &rx, 64, 48_000.0);
        // Pretend the ADSR advanced — set amp_cursor so "audible" is true.
        state.amp_cursors[0].set(3.0);
        assert_eq!(state.voice_gates[0].value(), 1.0);

        // Same-buffer NoteOff + NoteOn for the same pitch.
        tx.try_send(ControlEvent::NoteOff {
            pitch: 62,
            track: 0,
        })
        .unwrap();
        tx.try_send(ControlEvent::NoteOn {
            pitch: 62,
            velocity: 100,
            track: 0,
        })
        .unwrap();
        va.begin_buffer(&state, &rx, 64, 48_000.0);

        // Gate must be 0 on entry to sample loop.
        assert_eq!(state.voice_gates[0].value(), 0.0);
        // Ticking 3 times: still in countdown.
        va.tick_sample(&state);
        va.tick_sample(&state);
        va.tick_sample(&state);
        assert_eq!(state.voice_gates[0].value(), 0.0);
        // Fourth tick expires the countdown and flips the gate high.
        va.tick_sample(&state);
        assert_eq!(state.voice_gates[0].value(), 1.0);
    }

    #[test]
    fn set_param_legacy_five_still_routed() {
        let (mut va, state, tx, rx) = setup();
        tx.try_send(ControlEvent::SetParam {
            param: ParamId::FilterCutoff,
            value: 2345.0,
        })
        .unwrap();
        tx.try_send(ControlEvent::SetParam {
            param: ParamId::MasterVolume,
            value: 0.42,
        })
        .unwrap();
        va.begin_buffer(&state, &rx, 64, 48_000.0);
        assert!((state.cutoff.value() - 2345.0).abs() < 1e-3);
        assert!((state.master_vol.value() - 0.42).abs() < 1e-5);
    }

    #[test]
    fn steal_rotates_round_robin_when_pool_full() {
        let (mut va, state, tx, rx) = setup();
        // Fill every slot with distinct pitches.
        for p in 0..VOICE_COUNT as u8 {
            tx.try_send(ControlEvent::NoteOn {
                pitch: 60 + p,
                velocity: 100,
                track: 0,
            })
            .unwrap();
        }
        va.begin_buffer(&state, &rx, 64, 48_000.0);
        for s in 0..VOICE_COUNT {
            assert_eq!(
                state.voice_gates[s].value(),
                1.0,
                "slot {s} should be gated"
            );
        }

        // One more NoteOn forces a steal. Slot 0 is the first victim.
        tx.try_send(ControlEvent::NoteOn {
            pitch: 80,
            velocity: 100,
            track: 0,
        })
        .unwrap();
        va.begin_buffer(&state, &rx, 64, 48_000.0);
        let f = midi_hz(80.0) as f32;
        assert!((state.voice_freq_targets[0].value() - f).abs() < 1e-3);
    }
}
