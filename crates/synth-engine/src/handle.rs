//! SynthEngineHandle — the typed, clonable facade over the synth engine.
//!
//! Two equivalent projections live on the handle:
//!
//! 1. **Typed sugar** — one set/get method per parameter (`set_filter_cutoff`,
//!    `filter_cutoff`, …). Fast path for same-process Rust callers. Writes
//!    land directly on the backing atomic.
//! 2. **Generic dispatch** — `apply(Command)` decodes a serialisable
//!    [`Command`] into the appropriate typed setter or event send. This is
//!    the bridging point for any transport (OSC, WebSocket, CLAP shell, FFI).
//!
//! Invariant: for every parameter,
//!
//! ```text
//! handle.set_foo(v) ≡ handle.apply(Command::SetParam { id: ParamId::Foo, value: v })
//! ```
//!
//! Both bottom out on the same atomic write.
//!
//! Events that must interact with the audio thread's voice allocator (notes,
//! chord hold, arp/walker restart) are channel-routed via `ControlSender`
//! instead of direct writes. Track is hardcoded to 0 for Stage 1.

#![allow(clippy::too_many_lines)]
#![allow(clippy::cognitive_complexity)]

use std::sync::Arc;
use std::sync::atomic::Ordering;

use synth_control::{Command, ControlEvent, ControlSender, ParamId};

use crate::audio::AudioState;

/// Clonable, `Send + Sync` facade over the audio engine. Hand one of these
/// to the UI, the MIDI thread, the sequencer — each thread `.clone()`s its
/// own copy. All clones point to the same underlying atomics + channel.
#[derive(Clone)]
pub struct SynthEngineHandle {
    state:   Arc<AudioState>,
    control: ControlSender,
}

impl SynthEngineHandle {
    /// Construct a handle from an engine's state + control sender. Called by
    /// the audio engine at startup; not expected to be called from UI code.
    pub fn new(state: Arc<AudioState>, control: ControlSender) -> Self {
        Self { state, control }
    }

    // =======================================================================
    // Param setters + getters — typed sugar
    // =======================================================================

    // -- Oscillator bank --

    pub fn set_osc_wave(&self, osc: u8, wave: u8) {
        if let Some(s) = self.state.osc_wave.get(osc as usize) {
            s.store(wave.min(3), Ordering::Relaxed);
        }
    }
    pub fn osc_wave(&self, osc: u8) -> u8 {
        self.state.osc_wave.get(osc as usize)
            .map(|s| s.load(Ordering::Relaxed)).unwrap_or(0)
    }

    pub fn set_osc_freq_mult(&self, osc: u8, v: f32) {
        if let Some(s) = self.state.osc_freq_mult.get(osc as usize) { s.set(v); }
    }
    pub fn osc_freq_mult(&self, osc: u8) -> f32 {
        self.state.osc_freq_mult.get(osc as usize).map(|s| s.value()).unwrap_or(1.0)
    }

    pub fn set_osc_vol(&self, osc: u8, v: f32) {
        if let Some(s) = self.state.osc_vol.get(osc as usize) { s.set(v); }
    }
    pub fn osc_vol(&self, osc: u8) -> f32 {
        self.state.osc_vol.get(osc as usize).map(|s| s.value()).unwrap_or(0.0)
    }

    pub fn set_osc_pulse_width(&self, osc: u8, v: f32) {
        if let Some(s) = self.state.osc_pulse_width.get(osc as usize) { s.set(v); }
    }
    pub fn osc_pulse_width(&self, osc: u8) -> f32 {
        self.state.osc_pulse_width.get(osc as usize).map(|s| s.value()).unwrap_or(0.5)
    }

    pub fn set_osc_unison_detune(&self, osc: u8, copy: u8, v: f32) {
        if let Some(row) = self.state.osc_unison_detune.get(osc as usize) {
            if let Some(s) = row.get(copy as usize) { s.set(v); }
        }
    }
    pub fn osc_unison_detune(&self, osc: u8, copy: u8) -> f32 {
        self.state.osc_unison_detune.get(osc as usize)
            .and_then(|row| row.get(copy as usize)).map(|s| s.value()).unwrap_or(1.0)
    }

    pub fn set_osc_unison_vol(&self, osc: u8, copy: u8, v: f32) {
        if let Some(row) = self.state.osc_unison_vol.get(osc as usize) {
            if let Some(s) = row.get(copy as usize) { s.set(v); }
        }
    }
    pub fn osc_unison_vol(&self, osc: u8, copy: u8) -> f32 {
        self.state.osc_unison_vol.get(osc as usize)
            .and_then(|row| row.get(copy as usize)).map(|s| s.value()).unwrap_or(0.0)
    }

    pub fn set_hard_sync_enabled(&self, on: bool)  { self.state.hard_sync_enabled.store(on, Ordering::Relaxed); }
    pub fn hard_sync_enabled(&self) -> bool         { self.state.hard_sync_enabled.load(Ordering::Relaxed) }

    pub fn set_fm_depth(&self, v: f32)   { self.state.fm_depth.set(v); }
    pub fn fm_depth(&self) -> f32        { self.state.fm_depth.value() }

    pub fn set_ring_depth(&self, v: f32) { self.state.ring_depth.set(v); }
    pub fn ring_depth(&self) -> f32      { self.state.ring_depth.value() }

    pub fn set_noise_vol(&self, v: f32)  { self.state.noise_vol.set(v); }
    pub fn noise_vol(&self) -> f32       { self.state.noise_vol.value() }

    // -- Filter --

    pub fn set_filter_cutoff(&self, hz: f32)       { self.state.cutoff.set(hz); }
    pub fn filter_cutoff(&self) -> f32              { self.state.cutoff.value() }

    pub fn set_filter_resonance(&self, v: f32)     { self.state.resonance.set(v); }
    pub fn filter_resonance(&self) -> f32          { self.state.resonance.value() }

    pub fn set_filter_env_amount(&self, v: f32)    { self.state.filter_env_amount.set(v); }
    pub fn filter_env_amount(&self) -> f32         { self.state.filter_env_amount.value() }

    pub fn set_fenv_attack(&self, s: f32)  { self.state.fenv_attack.set(s); }
    pub fn fenv_attack(&self) -> f32       { self.state.fenv_attack.value() }

    pub fn set_fenv_decay(&self, s: f32)   { self.state.fenv_decay.set(s); }
    pub fn fenv_decay(&self) -> f32        { self.state.fenv_decay.value() }

    pub fn set_fenv_sustain(&self, v: f32) { self.state.fenv_sustain.set(v); }
    pub fn fenv_sustain(&self) -> f32      { self.state.fenv_sustain.value() }

    pub fn set_fenv_release(&self, s: f32) { self.state.fenv_release.set(s); }
    pub fn fenv_release(&self) -> f32      { self.state.fenv_release.value() }

    // -- LFO 1 --

    pub fn set_lfo_rate(&self, hz: f32)   { self.state.lfo_rate.set(hz); }
    pub fn lfo_rate(&self) -> f32          { self.state.lfo_rate.value() }

    pub fn set_lfo_depth(&self, v: f32)   { self.state.lfo_depth.set(v); }
    pub fn lfo_depth(&self) -> f32         { self.state.lfo_depth.value() }

    pub fn set_lfo_shape(&self, s: u8)    { self.state.lfo_shape.store(s.min(2), Ordering::Relaxed); }
    pub fn lfo_shape(&self) -> u8          { self.state.lfo_shape.load(Ordering::Relaxed) }

    pub fn set_lfo_dest(&self, d: u8)     { self.state.lfo_dest.store(d.min(2), Ordering::Relaxed); }
    pub fn lfo_dest(&self) -> u8           { self.state.lfo_dest.load(Ordering::Relaxed) }

    pub fn set_lfo_sync(&self, s: u8)     { self.state.lfo_sync.store(s.min(1), Ordering::Relaxed); }
    pub fn lfo_sync(&self) -> u8           { self.state.lfo_sync.load(Ordering::Relaxed) }

    pub fn set_lfo_division(&self, d: u8) { self.state.lfo_division.store(d, Ordering::Relaxed); }
    pub fn lfo_division(&self) -> u8       { self.state.lfo_division.load(Ordering::Relaxed) }

    pub fn set_lfo_pitch_mult(&self, v: f32) { self.state.lfo_pitch_mult.set(v); }
    pub fn lfo_pitch_mult(&self) -> f32       { self.state.lfo_pitch_mult.value() }

    // -- LFO 2 --

    pub fn set_lfo2_rate(&self, hz: f32)  { self.state.lfo2_rate.set(hz); }
    pub fn lfo2_rate(&self) -> f32         { self.state.lfo2_rate.value() }

    pub fn set_lfo2_depth(&self, v: f32)  { self.state.lfo2_depth.set(v); }
    pub fn lfo2_depth(&self) -> f32        { self.state.lfo2_depth.value() }

    pub fn set_lfo2_shape(&self, s: u8)   { self.state.lfo2_shape.store(s.min(2), Ordering::Relaxed); }
    pub fn lfo2_shape(&self) -> u8         { self.state.lfo2_shape.load(Ordering::Relaxed) }

    pub fn set_lfo2_dest(&self, d: u8)    { self.state.lfo2_dest.store(d.min(2), Ordering::Relaxed); }
    pub fn lfo2_dest(&self) -> u8          { self.state.lfo2_dest.load(Ordering::Relaxed) }

    // -- Amp envelope + glide + master --

    pub fn set_amp_attack(&self, s: f32)  { self.state.adsr_attack.set(s); }
    pub fn amp_attack(&self) -> f32        { self.state.adsr_attack.value() }

    pub fn set_amp_decay(&self, s: f32)   { self.state.adsr_decay.set(s); }
    pub fn amp_decay(&self) -> f32         { self.state.adsr_decay.value() }

    pub fn set_amp_sustain(&self, v: f32) { self.state.adsr_sustain.set(v); }
    pub fn amp_sustain(&self) -> f32       { self.state.adsr_sustain.value() }

    pub fn set_amp_release(&self, s: f32) { self.state.adsr_release.set(s); }
    pub fn amp_release(&self) -> f32       { self.state.adsr_release.value() }

    pub fn set_glide_time(&self, s: f32)  { self.state.glide_time.set(s); }
    pub fn glide_time(&self) -> f32        { self.state.glide_time.value() }

    pub fn set_master_volume(&self, v: f32) { self.state.master_vol.set(v); }
    pub fn master_volume(&self) -> f32       { self.state.master_vol.value() }

    pub fn set_global_volume(&self, v: f32) { self.state.global_vol.set(v); }
    pub fn global_volume(&self) -> f32       { self.state.global_vol.value() }

    pub fn set_limiter_enabled(&self, on: bool)  { self.state.limiter_enabled.store(on, Ordering::Relaxed); }
    pub fn limiter_enabled(&self) -> bool         { self.state.limiter_enabled.load(Ordering::Relaxed) }

    pub fn set_limiter_threshold(&self, v: f32)  { self.state.limiter_threshold.set(v); }
    pub fn limiter_threshold(&self) -> f32        { self.state.limiter_threshold.value() }

    // -- FX: Overdrive --

    pub fn set_fx_overdrive_drive(&self, v: f32) { self.state.fx_overdrive_drive.set(v); }
    pub fn fx_overdrive_drive(&self) -> f32       { self.state.fx_overdrive_drive.value() }
    pub fn set_fx_overdrive_mix(&self, v: f32)   { self.state.fx_overdrive_mix.set(v); }
    pub fn fx_overdrive_mix(&self) -> f32         { self.state.fx_overdrive_mix.value() }
    pub fn set_fx_overdrive_tone(&self, v: f32)  { self.state.fx_overdrive_tone.set(v); }
    pub fn fx_overdrive_tone(&self) -> f32        { self.state.fx_overdrive_tone.value() }
    pub fn set_fx_overdrive_asym(&self, v: f32)  { self.state.fx_overdrive_asym.set(v); }
    pub fn fx_overdrive_asym(&self) -> f32        { self.state.fx_overdrive_asym.value() }

    // -- FX: Distortion --

    pub fn set_fx_distortion_drive(&self, v: f32) { self.state.fx_distortion_drive.set(v); }
    pub fn fx_distortion_drive(&self) -> f32       { self.state.fx_distortion_drive.value() }
    pub fn set_fx_distortion_mix(&self, v: f32)   { self.state.fx_distortion_mix.set(v); }
    pub fn fx_distortion_mix(&self) -> f32         { self.state.fx_distortion_mix.value() }
    pub fn set_fx_distortion_tone(&self, v: f32)  { self.state.fx_distortion_tone.set(v); }
    pub fn fx_distortion_tone(&self) -> f32        { self.state.fx_distortion_tone.value() }
    pub fn set_fx_distortion_pre(&self, v: f32)   { self.state.fx_distortion_pre.set(v); }
    pub fn fx_distortion_pre(&self) -> f32         { self.state.fx_distortion_pre.value() }

    // -- FX: Chorus --

    pub fn set_fx_chorus_rate(&self, hz: f32) { self.state.fx_chorus_rate.set(hz); }
    pub fn fx_chorus_rate(&self) -> f32        { self.state.fx_chorus_rate.value() }
    pub fn set_fx_chorus_depth(&self, s: f32) { self.state.fx_chorus_depth.set(s); }
    pub fn fx_chorus_depth(&self) -> f32       { self.state.fx_chorus_depth.value() }
    pub fn set_fx_chorus_mix(&self, v: f32)   { self.state.fx_chorus_mix.set(v); }
    pub fn fx_chorus_mix(&self) -> f32         { self.state.fx_chorus_mix.value() }

    // -- FX: Delay --

    pub fn set_fx_delay_time(&self, s: f32)     { self.state.fx_delay_time.set(s); }
    pub fn fx_delay_time(&self) -> f32           { self.state.fx_delay_time.value() }
    pub fn set_fx_delay_feedback(&self, v: f32) { self.state.fx_delay_feedback.set(v); }
    pub fn fx_delay_feedback(&self) -> f32       { self.state.fx_delay_feedback.value() }
    pub fn set_fx_delay_mix(&self, v: f32)      { self.state.fx_delay_mix.set(v); }
    pub fn fx_delay_mix(&self) -> f32            { self.state.fx_delay_mix.value() }
    pub fn set_fx_delay_sync(&self, s: u8)      { self.state.fx_delay_sync.store(s.min(1), Ordering::Relaxed); }
    pub fn fx_delay_sync(&self) -> u8            { self.state.fx_delay_sync.load(Ordering::Relaxed) }
    pub fn set_fx_delay_division(&self, d: u8)  { self.state.fx_delay_division.store(d, Ordering::Relaxed); }
    pub fn fx_delay_division(&self) -> u8        { self.state.fx_delay_division.load(Ordering::Relaxed) }

    // -- FX: Reverb --

    pub fn set_fx_reverb_size(&self, v: f32)     { self.state.fx_reverb_size.set(v); }
    pub fn fx_reverb_size(&self) -> f32           { self.state.fx_reverb_size.value() }
    pub fn set_fx_reverb_damp(&self, v: f32)     { self.state.fx_reverb_damp.set(v); }
    pub fn fx_reverb_damp(&self) -> f32           { self.state.fx_reverb_damp.value() }
    pub fn set_fx_reverb_mix(&self, v: f32)      { self.state.fx_reverb_mix.set(v); }
    pub fn fx_reverb_mix(&self) -> f32            { self.state.fx_reverb_mix.value() }
    pub fn set_fx_reverb_predelay(&self, s: f32) { self.state.fx_reverb_predelay.set(s); }
    pub fn fx_reverb_predelay(&self) -> f32       { self.state.fx_reverb_predelay.value() }
    pub fn set_fx_reverb_type(&self, t: u8)      { self.state.fx_reverb_type.store(t.min(2), Ordering::Relaxed); }
    pub fn fx_reverb_type(&self) -> u8            { self.state.fx_reverb_type.load(Ordering::Relaxed) }

    // -- Stereo --

    pub fn set_stereo_spread(&self, s: f32) { self.state.stereo_spread.set(s); }
    pub fn stereo_spread(&self) -> f32       { self.state.stereo_spread.value() }
    pub fn set_stereo_width(&self, v: f32)  { self.state.stereo_width.set(v); }
    pub fn stereo_width(&self) -> f32        { self.state.stereo_width.value() }

    // -- Shimmer --

    pub fn set_shimmer_size(&self, v: f32)   { self.state.fx_shimmer.size.set(v); }
    pub fn shimmer_size(&self) -> f32         { self.state.fx_shimmer.size.value() }
    pub fn set_shimmer_damp(&self, v: f32)   { self.state.fx_shimmer.damp.set(v); }
    pub fn shimmer_damp(&self) -> f32         { self.state.fx_shimmer.damp.value() }
    pub fn set_shimmer_mix(&self, v: f32)    { self.state.fx_shimmer.mix.set(v); }
    pub fn shimmer_mix(&self) -> f32          { self.state.fx_shimmer.mix.value() }
    pub fn set_shimmer_amount(&self, v: f32) { self.state.fx_shimmer.shimmer.set(v); }
    pub fn shimmer_amount(&self) -> f32       { self.state.fx_shimmer.shimmer.value() }
    pub fn set_shimmer_width(&self, v: f32)  { self.state.fx_shimmer.width.set(v); }
    pub fn shimmer_width(&self) -> f32        { self.state.fx_shimmer.width.value() }
    pub fn set_shimmer_spread(&self, v: f32) { self.state.fx_shimmer.spread.set(v); }
    pub fn shimmer_spread(&self) -> f32       { self.state.fx_shimmer.spread.value() }
    pub fn set_shimmer_pitch(&self, p: u8)   { self.state.fx_shimmer.pitch.store(p.min(2), Ordering::Relaxed); }
    pub fn shimmer_pitch(&self) -> u8         { self.state.fx_shimmer.pitch.load(Ordering::Relaxed) }

    // -- Crystallizer --

    pub fn set_crystal_grain(&self, ms: f32)    { self.state.fx_crystal.grain_ms.set(ms); }
    pub fn crystal_grain(&self) -> f32           { self.state.fx_crystal.grain_ms.value() }
    pub fn set_crystal_scatter(&self, v: f32)   { self.state.fx_crystal.scatter.set(v); }
    pub fn crystal_scatter(&self) -> f32         { self.state.fx_crystal.scatter.value() }
    pub fn set_crystal_feedback(&self, v: f32)  { self.state.fx_crystal.feedback.set(v); }
    pub fn crystal_feedback(&self) -> f32        { self.state.fx_crystal.feedback.value() }
    pub fn set_crystal_delay(&self, ms: f32)    { self.state.fx_crystal.delay_ms.set(ms); }
    pub fn crystal_delay(&self) -> f32           { self.state.fx_crystal.delay_ms.value() }
    pub fn set_crystal_mix(&self, v: f32)       { self.state.fx_crystal.mix.set(v); }
    pub fn crystal_mix(&self) -> f32             { self.state.fx_crystal.mix.value() }
    pub fn set_crystal_pitch(&self, p: u8)      { self.state.fx_crystal.pitch.store(p.min(4), Ordering::Relaxed); }
    pub fn crystal_pitch(&self) -> u8            { self.state.fx_crystal.pitch.load(Ordering::Relaxed) }

    // -- Arpeggiator --

    pub fn set_arp_enabled(&self, on: bool)    { self.state.arp.enabled.store(on, Ordering::Relaxed); }
    pub fn arp_enabled(&self) -> bool           { self.state.arp.enabled.load(Ordering::Relaxed) }
    pub fn set_arp_mode(&self, m: u8)          { self.state.arp.mode.store(m, Ordering::Relaxed); }
    pub fn arp_mode(&self) -> u8                { self.state.arp.mode.load(Ordering::Relaxed) }
    pub fn set_arp_division(&self, d: u8)      { self.state.arp.division.store(d, Ordering::Relaxed); }
    pub fn arp_division(&self) -> u8            { self.state.arp.division.load(Ordering::Relaxed) }
    pub fn set_arp_octave_range(&self, o: u8)  { self.state.arp.octave_range.store(o.max(1), Ordering::Relaxed); }
    pub fn arp_octave_range(&self) -> u8        { self.state.arp.octave_range.load(Ordering::Relaxed) }
    pub fn set_arp_gate(&self, v: f32)         { self.state.arp.gate.set(v); }
    pub fn arp_gate(&self) -> f32               { self.state.arp.gate.value() }
    pub fn set_arp_hold(&self, on: bool)       { self.state.arp.hold.store(on, Ordering::Relaxed); }
    pub fn arp_hold(&self) -> bool              { self.state.arp.hold.load(Ordering::Relaxed) }
    pub fn set_arp_bpm(&self, bpm: f32)        { self.state.arp.bpm.set(bpm); }
    pub fn arp_bpm(&self) -> f32                { self.state.arp.bpm.value() }

    // -- Scale walker --

    pub fn set_walker_enabled(&self, on: bool)   { self.state.walker.enabled.store(on, Ordering::Relaxed); }
    pub fn walker_enabled(&self) -> bool          { self.state.walker.enabled.load(Ordering::Relaxed) }
    pub fn set_walker_scale(&self, s: u8)        { self.state.walker.scale.store(s, Ordering::Relaxed); }
    pub fn walker_scale(&self) -> u8              { self.state.walker.scale.load(Ordering::Relaxed) }
    pub fn set_walker_root(&self, r: u8)         { self.state.walker.root.store(r, Ordering::Relaxed); }
    pub fn walker_root(&self) -> u8               { self.state.walker.root.load(Ordering::Relaxed) }
    pub fn set_walker_octave_range(&self, o: u8) { self.state.walker.octave_range.store(o.max(1), Ordering::Relaxed); }
    pub fn walker_octave_range(&self) -> u8       { self.state.walker.octave_range.load(Ordering::Relaxed) }
    pub fn set_walker_division(&self, d: u8)     { self.state.walker.division.store(d, Ordering::Relaxed); }
    pub fn walker_division(&self) -> u8           { self.state.walker.division.load(Ordering::Relaxed) }
    pub fn set_walker_gate(&self, v: f32)        { self.state.walker.gate.set(v); }
    pub fn walker_gate(&self) -> f32              { self.state.walker.gate.value() }
    pub fn set_walker_bpm(&self, bpm: f32)       { self.state.walker.bpm.set(bpm); }
    pub fn walker_bpm(&self) -> f32               { self.state.walker.bpm.value() }

    // =======================================================================
    // Event methods — channel-routed, track 0
    // =======================================================================

    /// Trigger a note. Sends `ControlEvent::NoteOn { .., track: 0 }`.
    pub fn note_on(&self, pitch: u8, velocity: u8) {
        let _ = self.control.try_send(ControlEvent::NoteOn { pitch, velocity, track: 0 });
    }

    /// Release a note.
    pub fn note_off(&self, pitch: u8) {
        let _ = self.control.try_send(ControlEvent::NoteOff { pitch, track: 0 });
    }

    /// Release every currently-held note.
    ///
    /// Stage 1 implementation sends one `NoteOff` per MIDI pitch. Simple and
    /// correct; a dedicated `AllNotesOff` channel variant can be added later
    /// if this becomes a hot path.
    pub fn all_notes_off(&self) {
        for pitch in 0u8..=127 {
            let _ = self.control.try_send(ControlEvent::NoteOff { pitch, track: 0 });
        }
    }

    /// Latch a chord into the arpeggiator.
    pub fn chord_hold(&self, notes: &[u8]) {
        let _ = self.control.try_send(ControlEvent::ChordHold { track: 0, notes: notes.to_vec() });
    }

    /// Restart arpeggiator timing.
    pub fn arp_restart(&self) {
        let _ = self.control.try_send(ControlEvent::ArpRestart { track: 0 });
    }

    /// Restart scale-walker timing.
    pub fn walker_restart(&self) {
        let _ = self.control.try_send(ControlEvent::WalkerRestart { track: 0 });
    }

    // =======================================================================
    // Readback — atomic reads, safe from any thread
    // =======================================================================

    /// Current amp-envelope cursor for a voice. Encoding: 0=idle, 1.x=attack,
    /// 2.x=decay, 3.0=sustain, 4.x=release. Returns 0.0 if voice index is
    /// out of range.
    pub fn amp_cursor(&self, voice: usize) -> f32 {
        self.state.amp_cursors.get(voice).map(|s| s.value()).unwrap_or(0.0)
    }

    /// Current filter-envelope cursor for a voice. Same encoding as amp cursor.
    pub fn fenv_cursor(&self, voice: usize) -> f32 {
        self.state.fenv_cursors.get(voice).map(|s| s.value()).unwrap_or(0.0)
    }

    /// Peak left-channel level (linear, post-limiter, post-tanh).
    pub fn peak_l(&self) -> f32 {
        f32::from_bits(self.state.peak_l.load(Ordering::Relaxed))
    }

    /// Peak right-channel level.
    pub fn peak_r(&self) -> f32 {
        f32::from_bits(self.state.peak_r.load(Ordering::Relaxed))
    }

    /// Last measured round-trip note→audio latency in microseconds.
    pub fn last_latency_us(&self) -> u32 {
        self.state.last_latency_us.load(Ordering::Relaxed)
    }

    /// Audio sample rate in Hz (written by the stream on first callback).
    pub fn sample_rate(&self) -> u32 {
        self.state.sample_rate.load(Ordering::Relaxed)
    }

    /// Audio buffer size in frames (written by the stream on first callback).
    pub fn buffer_frames(&self) -> u32 {
        self.state.buffer_frames.load(Ordering::Relaxed)
    }

    // =======================================================================
    // Generic dispatch — apply(Command)
    // =======================================================================

    /// Execute a `Command`. Equivalent to calling the matching typed
    /// setter or event method.
    ///
    /// For `SetParam`, `value` is cast to the parameter's native
    /// representation (u8 clamp + round for discrete, `!= 0.0` for bool,
    /// direct for f32-backed).
    pub fn apply(&self, cmd: Command) {
        match cmd {
            Command::SetParam { id, value }    => self.set_by_id(id, value),
            Command::NoteOn { pitch, velocity } => self.note_on(pitch, velocity),
            Command::NoteOff { pitch }          => self.note_off(pitch),
            Command::AllNotesOff                => self.all_notes_off(),
            Command::ChordHold(notes)           => self.chord_hold(&notes),
            Command::ArpRestart                 => self.arp_restart(),
            Command::WalkerRestart              => self.walker_restart(),
            _ => {} // non_exhaustive
        }
    }

    /// Generic parameter write keyed by `ParamId`. One arm per variant.
    fn set_by_id(&self, id: ParamId, v: f32) {
        // Helper for u8-backed discrete casts.
        #[inline] fn u8c(v: f32, max: u8) -> u8 {
            v.clamp(0.0, max as f32).round() as u8
        }
        #[inline] fn b(v: f32) -> bool { v != 0.0 }

        match id {
            // -- Oscillator bank --
            ParamId::OscWave(osc)            => self.set_osc_wave(osc, u8c(v, 3)),
            ParamId::OscFreqMult(osc)        => self.set_osc_freq_mult(osc, v),
            ParamId::OscVol(osc)             => self.set_osc_vol(osc, v),
            ParamId::OscPulseWidth(osc)      => self.set_osc_pulse_width(osc, v),
            ParamId::OscUnisonDetune(osc, c) => self.set_osc_unison_detune(osc, c, v),
            ParamId::OscUnisonVol(osc, c)    => self.set_osc_unison_vol(osc, c, v),
            ParamId::HardSyncEnabled          => self.set_hard_sync_enabled(b(v)),
            ParamId::FmDepth                  => self.set_fm_depth(v),
            ParamId::RingDepth                => self.set_ring_depth(v),
            ParamId::NoiseVol                 => self.set_noise_vol(v),

            // -- Filter --
            ParamId::FilterCutoff             => self.set_filter_cutoff(v),
            ParamId::FilterResonance          => self.set_filter_resonance(v),
            ParamId::FilterEnvAmount          => self.set_filter_env_amount(v),
            ParamId::FenvAttack               => self.set_fenv_attack(v),
            ParamId::FenvDecay                => self.set_fenv_decay(v),
            ParamId::FenvSustain              => self.set_fenv_sustain(v),
            ParamId::FenvRelease              => self.set_fenv_release(v),

            // -- LFO 1 --
            ParamId::LfoRate                  => self.set_lfo_rate(v),
            ParamId::LfoDepth                 => self.set_lfo_depth(v),
            ParamId::LfoShape                 => self.set_lfo_shape(u8c(v, 2)),
            ParamId::LfoDest                  => self.set_lfo_dest(u8c(v, 2)),
            ParamId::LfoSync                  => self.set_lfo_sync(u8c(v, 1)),
            ParamId::LfoDivision              => self.set_lfo_division(u8c(v, 15)),
            ParamId::LfoPitchMult             => self.set_lfo_pitch_mult(v),

            // -- LFO 2 --
            ParamId::Lfo2Rate                 => self.set_lfo2_rate(v),
            ParamId::Lfo2Depth                => self.set_lfo2_depth(v),
            ParamId::Lfo2Shape                => self.set_lfo2_shape(u8c(v, 2)),
            ParamId::Lfo2Dest                 => self.set_lfo2_dest(u8c(v, 2)),

            // -- Amp envelope + glide + master --
            ParamId::AmpAttack                => self.set_amp_attack(v),
            ParamId::AmpDecay                 => self.set_amp_decay(v),
            ParamId::AmpSustain               => self.set_amp_sustain(v),
            ParamId::AmpRelease               => self.set_amp_release(v),
            ParamId::GlideTime                => self.set_glide_time(v),
            ParamId::MasterVolume             => self.set_master_volume(v),
            ParamId::GlobalVolume             => self.set_global_volume(v),
            ParamId::LimiterEnabled           => self.set_limiter_enabled(b(v)),
            ParamId::LimiterThreshold         => self.set_limiter_threshold(v),

            // -- FX chain --
            ParamId::FxOverdriveDrive         => self.set_fx_overdrive_drive(v),
            ParamId::FxOverdriveMix           => self.set_fx_overdrive_mix(v),
            ParamId::FxOverdriveTone          => self.set_fx_overdrive_tone(v),
            ParamId::FxOverdriveAsym          => self.set_fx_overdrive_asym(v),
            ParamId::FxDistortionDrive        => self.set_fx_distortion_drive(v),
            ParamId::FxDistortionMix          => self.set_fx_distortion_mix(v),
            ParamId::FxDistortionTone         => self.set_fx_distortion_tone(v),
            ParamId::FxDistortionPre          => self.set_fx_distortion_pre(v),
            ParamId::FxChorusRate             => self.set_fx_chorus_rate(v),
            ParamId::FxChorusDepth            => self.set_fx_chorus_depth(v),
            ParamId::FxChorusMix              => self.set_fx_chorus_mix(v),
            ParamId::FxDelayTime              => self.set_fx_delay_time(v),
            ParamId::FxDelayFeedback          => self.set_fx_delay_feedback(v),
            ParamId::FxDelayMix               => self.set_fx_delay_mix(v),
            ParamId::FxDelaySync              => self.set_fx_delay_sync(u8c(v, 1)),
            ParamId::FxDelayDivision          => self.set_fx_delay_division(u8c(v, 15)),
            ParamId::FxReverbSize             => self.set_fx_reverb_size(v),
            ParamId::FxReverbDamp             => self.set_fx_reverb_damp(v),
            ParamId::FxReverbMix              => self.set_fx_reverb_mix(v),
            ParamId::FxReverbPredelay         => self.set_fx_reverb_predelay(v),
            ParamId::FxReverbType             => self.set_fx_reverb_type(u8c(v, 2)),
            ParamId::StereoSpread             => self.set_stereo_spread(v),
            ParamId::StereoWidth              => self.set_stereo_width(v),

            // -- Shimmer --
            ParamId::ShimmerSize              => self.set_shimmer_size(v),
            ParamId::ShimmerDamp              => self.set_shimmer_damp(v),
            ParamId::ShimmerMix               => self.set_shimmer_mix(v),
            ParamId::ShimmerAmount            => self.set_shimmer_amount(v),
            ParamId::ShimmerWidth             => self.set_shimmer_width(v),
            ParamId::ShimmerSpread            => self.set_shimmer_spread(v),
            ParamId::ShimmerPitch             => self.set_shimmer_pitch(u8c(v, 2)),

            // -- Crystallizer --
            ParamId::CrystalGrain             => self.set_crystal_grain(v),
            ParamId::CrystalScatter           => self.set_crystal_scatter(v),
            ParamId::CrystalFeedback          => self.set_crystal_feedback(v),
            ParamId::CrystalDelay             => self.set_crystal_delay(v),
            ParamId::CrystalMix               => self.set_crystal_mix(v),
            ParamId::CrystalPitch             => self.set_crystal_pitch(u8c(v, 4)),

            // -- Arp --
            ParamId::ArpEnabled               => self.set_arp_enabled(b(v)),
            ParamId::ArpMode                  => self.set_arp_mode(u8c(v, 4)),
            ParamId::ArpDivision              => self.set_arp_division(u8c(v, 15)),
            ParamId::ArpOctaveRange           => self.set_arp_octave_range(u8c(v, 4)),
            ParamId::ArpGate                  => self.set_arp_gate(v),
            ParamId::ArpHold                  => self.set_arp_hold(b(v)),
            ParamId::ArpBpm                   => self.set_arp_bpm(v),

            // -- Walker --
            ParamId::WalkerEnabled            => self.set_walker_enabled(b(v)),
            ParamId::WalkerScale              => self.set_walker_scale(u8c(v, 7)),
            ParamId::WalkerRoot               => self.set_walker_root(u8c(v, 127)),
            ParamId::WalkerOctaveRange        => self.set_walker_octave_range(u8c(v, 3)),
            ParamId::WalkerDivision           => self.set_walker_division(u8c(v, 15)),
            ParamId::WalkerGate               => self.set_walker_gate(v),
            ParamId::WalkerBpm                => self.set_walker_bpm(v),

            _ => {} // non_exhaustive — future ParamIds silently ignored
        }
    }

    /// Generic parameter read keyed by `ParamId`. Returns `None` for
    /// unsupported or readback-only identifiers.
    pub fn get_by_id(&self, id: ParamId) -> Option<f32> {
        let v = match id {
            ParamId::OscWave(o)            => self.osc_wave(o) as f32,
            ParamId::OscFreqMult(o)        => self.osc_freq_mult(o),
            ParamId::OscVol(o)             => self.osc_vol(o),
            ParamId::OscPulseWidth(o)      => self.osc_pulse_width(o),
            ParamId::OscUnisonDetune(o, c) => self.osc_unison_detune(o, c),
            ParamId::OscUnisonVol(o, c)    => self.osc_unison_vol(o, c),
            ParamId::HardSyncEnabled       => bf(self.hard_sync_enabled()),
            ParamId::FmDepth               => self.fm_depth(),
            ParamId::RingDepth             => self.ring_depth(),
            ParamId::NoiseVol              => self.noise_vol(),
            ParamId::FilterCutoff          => self.filter_cutoff(),
            ParamId::FilterResonance       => self.filter_resonance(),
            ParamId::FilterEnvAmount       => self.filter_env_amount(),
            ParamId::FenvAttack            => self.fenv_attack(),
            ParamId::FenvDecay             => self.fenv_decay(),
            ParamId::FenvSustain           => self.fenv_sustain(),
            ParamId::FenvRelease           => self.fenv_release(),
            ParamId::LfoRate               => self.lfo_rate(),
            ParamId::LfoDepth              => self.lfo_depth(),
            ParamId::LfoShape              => self.lfo_shape() as f32,
            ParamId::LfoDest               => self.lfo_dest() as f32,
            ParamId::LfoSync               => self.lfo_sync() as f32,
            ParamId::LfoDivision           => self.lfo_division() as f32,
            ParamId::LfoPitchMult          => self.lfo_pitch_mult(),
            ParamId::Lfo2Rate              => self.lfo2_rate(),
            ParamId::Lfo2Depth             => self.lfo2_depth(),
            ParamId::Lfo2Shape             => self.lfo2_shape() as f32,
            ParamId::Lfo2Dest              => self.lfo2_dest() as f32,
            ParamId::AmpAttack             => self.amp_attack(),
            ParamId::AmpDecay              => self.amp_decay(),
            ParamId::AmpSustain            => self.amp_sustain(),
            ParamId::AmpRelease            => self.amp_release(),
            ParamId::GlideTime             => self.glide_time(),
            ParamId::MasterVolume          => self.master_volume(),
            ParamId::GlobalVolume          => self.global_volume(),
            ParamId::LimiterEnabled        => bf(self.limiter_enabled()),
            ParamId::LimiterThreshold      => self.limiter_threshold(),
            ParamId::FxOverdriveDrive      => self.fx_overdrive_drive(),
            ParamId::FxOverdriveMix        => self.fx_overdrive_mix(),
            ParamId::FxOverdriveTone       => self.fx_overdrive_tone(),
            ParamId::FxOverdriveAsym       => self.fx_overdrive_asym(),
            ParamId::FxDistortionDrive     => self.fx_distortion_drive(),
            ParamId::FxDistortionMix       => self.fx_distortion_mix(),
            ParamId::FxDistortionTone      => self.fx_distortion_tone(),
            ParamId::FxDistortionPre       => self.fx_distortion_pre(),
            ParamId::FxChorusRate          => self.fx_chorus_rate(),
            ParamId::FxChorusDepth         => self.fx_chorus_depth(),
            ParamId::FxChorusMix           => self.fx_chorus_mix(),
            ParamId::FxDelayTime           => self.fx_delay_time(),
            ParamId::FxDelayFeedback       => self.fx_delay_feedback(),
            ParamId::FxDelayMix            => self.fx_delay_mix(),
            ParamId::FxDelaySync           => self.fx_delay_sync() as f32,
            ParamId::FxDelayDivision       => self.fx_delay_division() as f32,
            ParamId::FxReverbSize          => self.fx_reverb_size(),
            ParamId::FxReverbDamp          => self.fx_reverb_damp(),
            ParamId::FxReverbMix           => self.fx_reverb_mix(),
            ParamId::FxReverbPredelay      => self.fx_reverb_predelay(),
            ParamId::FxReverbType          => self.fx_reverb_type() as f32,
            ParamId::StereoSpread          => self.stereo_spread(),
            ParamId::StereoWidth           => self.stereo_width(),
            ParamId::ShimmerSize           => self.shimmer_size(),
            ParamId::ShimmerDamp           => self.shimmer_damp(),
            ParamId::ShimmerMix            => self.shimmer_mix(),
            ParamId::ShimmerAmount         => self.shimmer_amount(),
            ParamId::ShimmerWidth          => self.shimmer_width(),
            ParamId::ShimmerSpread         => self.shimmer_spread(),
            ParamId::ShimmerPitch          => self.shimmer_pitch() as f32,
            ParamId::CrystalGrain          => self.crystal_grain(),
            ParamId::CrystalScatter        => self.crystal_scatter(),
            ParamId::CrystalFeedback       => self.crystal_feedback(),
            ParamId::CrystalDelay          => self.crystal_delay(),
            ParamId::CrystalMix            => self.crystal_mix(),
            ParamId::CrystalPitch          => self.crystal_pitch() as f32,
            ParamId::ArpEnabled            => bf(self.arp_enabled()),
            ParamId::ArpMode               => self.arp_mode() as f32,
            ParamId::ArpDivision           => self.arp_division() as f32,
            ParamId::ArpOctaveRange        => self.arp_octave_range() as f32,
            ParamId::ArpGate               => self.arp_gate(),
            ParamId::ArpHold               => bf(self.arp_hold()),
            ParamId::ArpBpm                => self.arp_bpm(),
            ParamId::WalkerEnabled         => bf(self.walker_enabled()),
            ParamId::WalkerScale           => self.walker_scale() as f32,
            ParamId::WalkerRoot            => self.walker_root() as f32,
            ParamId::WalkerOctaveRange     => self.walker_octave_range() as f32,
            ParamId::WalkerDivision        => self.walker_division() as f32,
            ParamId::WalkerGate            => self.walker_gate(),
            ParamId::WalkerBpm             => self.walker_bpm(),
            _ => return None,
        };
        Some(v)
    }
}

#[inline] fn bf(b: bool) -> f32 { if b { 1.0 } else { 0.0 } }
