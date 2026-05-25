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

use std::sync::atomic::Ordering;
use std::sync::Arc;

use forma_control::{Command, ControlEvent, ControlSender, ParamId};

use crate::audio::{AudioState, GatePattern};
use crate::patch::Patch;

#[inline]
fn gate_set_step(g: &GatePattern, step: u8, on: bool) {
    if step >= 16 {
        return;
    }
    let bit = 1u16 << step;
    let prev = g.pattern.load(Ordering::Relaxed);
    let next = if on { prev | bit } else { prev & !bit };
    g.pattern.store(next, Ordering::Relaxed);
}
#[inline]
fn gate_get_step(g: &GatePattern, step: u8) -> bool {
    if step >= 16 {
        return false;
    }
    (g.pattern.load(Ordering::Relaxed) >> step) & 1 != 0
}

/// Clonable, `Send + Sync` facade over the audio engine. Hand one of these
/// to the UI, the MIDI thread, the sequencer — each thread `.clone()`s its
/// own copy. All clones point to the same underlying atomics + channel.
#[derive(Clone)]
pub struct SynthEngineHandle {
    state: Arc<AudioState>,
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
        self.state
            .osc_wave
            .get(osc as usize)
            .map(|s| s.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    pub fn set_osc_freq_mult(&self, osc: u8, v: f32) {
        if let Some(s) = self.state.osc_freq_mult.get(osc as usize) {
            s.set(v);
        }
    }
    pub fn osc_freq_mult(&self, osc: u8) -> f32 {
        self.state
            .osc_freq_mult
            .get(osc as usize)
            .map(|s| s.value())
            .unwrap_or(1.0)
    }

    pub fn set_osc_vol(&self, osc: u8, v: f32) {
        if let Some(s) = self.state.osc_vol.get(osc as usize) {
            s.set(v);
        }
    }
    pub fn osc_vol(&self, osc: u8) -> f32 {
        self.state
            .osc_vol
            .get(osc as usize)
            .map(|s| s.value())
            .unwrap_or(0.0)
    }

    pub fn set_osc_pulse_width(&self, osc: u8, v: f32) {
        if let Some(s) = self.state.osc_pulse_width.get(osc as usize) {
            s.set(v);
        }
    }
    pub fn osc_pulse_width(&self, osc: u8) -> f32 {
        self.state
            .osc_pulse_width
            .get(osc as usize)
            .map(|s| s.value())
            .unwrap_or(0.5)
    }

    pub fn set_osc_unison_detune(&self, osc: u8, copy: u8, v: f32) {
        if let Some(row) = self.state.osc_unison_detune.get(osc as usize) {
            if let Some(s) = row.get(copy as usize) {
                s.set(v);
            }
        }
    }
    pub fn osc_unison_detune(&self, osc: u8, copy: u8) -> f32 {
        self.state
            .osc_unison_detune
            .get(osc as usize)
            .and_then(|row| row.get(copy as usize))
            .map(|s| s.value())
            .unwrap_or(1.0)
    }

    pub fn set_osc_unison_vol(&self, osc: u8, copy: u8, v: f32) {
        if let Some(row) = self.state.osc_unison_vol.get(osc as usize) {
            if let Some(s) = row.get(copy as usize) {
                s.set(v);
            }
        }
    }
    pub fn osc_unison_vol(&self, osc: u8, copy: u8) -> f32 {
        self.state
            .osc_unison_vol
            .get(osc as usize)
            .and_then(|row| row.get(copy as usize))
            .map(|s| s.value())
            .unwrap_or(0.0)
    }

    pub fn set_hard_sync_enabled(&self, on: bool) {
        self.state.hard_sync_enabled.store(on, Ordering::Relaxed);
    }
    pub fn hard_sync_enabled(&self) -> bool {
        self.state.hard_sync_enabled.load(Ordering::Relaxed)
    }

    pub fn set_fm_depth(&self, v: f32) {
        self.state.fm_depth.set(v);
    }
    pub fn fm_depth(&self) -> f32 {
        self.state.fm_depth.value()
    }

    pub fn set_ring_depth(&self, v: f32) {
        self.state.ring_depth.set(v);
    }
    pub fn ring_depth(&self) -> f32 {
        self.state.ring_depth.value()
    }

    pub fn set_noise_vol(&self, v: f32) {
        self.state.noise_vol.set(v);
    }
    pub fn noise_vol(&self) -> f32 {
        self.state.noise_vol.value()
    }

    // -- Filter --

    pub fn set_filter_cutoff(&self, hz: f32) {
        self.state.cutoff.set(hz);
    }
    pub fn filter_cutoff(&self) -> f32 {
        self.state.cutoff.value()
    }

    pub fn set_filter_resonance(&self, v: f32) {
        self.state.resonance.set(v);
    }
    pub fn filter_resonance(&self) -> f32 {
        self.state.resonance.value()
    }

    pub fn set_filter_drive(&self, v: f32) {
        self.state.filter_drive.set(v.max(1.0));
    }
    pub fn filter_drive(&self) -> f32 {
        self.state.filter_drive.value()
    }

    pub fn set_filter_key_track(&self, v: f32) {
        self.state.filter_key_track.set(v.clamp(0.0, 1.0));
    }
    pub fn filter_key_track(&self) -> f32 {
        self.state.filter_key_track.value()
    }
    pub fn set_vel_amp(&self, v: f32) {
        self.state.vel_amp.set(v.clamp(0.0, 1.0));
    }
    pub fn vel_amp(&self) -> f32 {
        self.state.vel_amp.value()
    }
    pub fn set_vel_filter(&self, v: f32) {
        self.state.vel_filter.set(v.clamp(0.0, 1.0));
    }
    pub fn vel_filter(&self) -> f32 {
        self.state.vel_filter.value()
    }
    pub fn set_mono_mode(&self, v: u8) {
        self.state.mono_mode.store(v.min(2), Ordering::Relaxed);
    }
    pub fn mono_mode(&self) -> u8 {
        self.state.mono_mode.load(Ordering::Relaxed)
    }
    pub fn set_mod_wheel_cutoff_add(&self, v: f32) {
        self.state.mod_wheel_cutoff_add.set(v.clamp(0.0, 8000.0));
    }

    pub fn set_mod_wheel(&self, v: f32) {
        self.state.mod_wheel.set(v.clamp(0.0, 1.0));
    }
    pub fn mod_wheel(&self) -> f32 {
        self.state.mod_wheel.value()
    }
    pub fn set_mod_wheel_dest(&self, d: u8) {
        self.state.mod_wheel_dest.store(d.min(3), Ordering::Relaxed);
    }
    pub fn mod_wheel_dest(&self) -> u8 {
        self.state.mod_wheel_dest.load(Ordering::Relaxed)
    }
    pub fn set_mod_wheel_depth(&self, v: f32) {
        self.state.mod_wheel_depth.set(v.clamp(0.0, 1.0));
    }
    pub fn mod_wheel_depth(&self) -> f32 {
        self.state.mod_wheel_depth.value()
    }

    pub fn set_aftertouch(&self, v: f32) {
        self.state.aftertouch.set(v.clamp(0.0, 1.0));
    }
    pub fn aftertouch(&self) -> f32 {
        self.state.aftertouch.value()
    }
    pub fn set_aftertouch_dest(&self, d: u8) {
        self.state
            .aftertouch_dest
            .store(d.min(3), Ordering::Relaxed);
    }
    pub fn aftertouch_dest(&self) -> u8 {
        self.state.aftertouch_dest.load(Ordering::Relaxed)
    }
    pub fn set_aftertouch_depth(&self, v: f32) {
        self.state.aftertouch_depth.set(v.clamp(0.0, 1.0));
    }
    pub fn aftertouch_depth(&self) -> f32 {
        self.state.aftertouch_depth.value()
    }

    pub fn set_mat_src(&self, slot: usize, src: u8) {
        if let Some(s) = self.state.mat_src.get(slot) {
            s.store(src.min(4), Ordering::Relaxed);
        }
    }
    pub fn mat_src(&self, slot: usize) -> u8 {
        self.state
            .mat_src
            .get(slot)
            .map_or(0, |s| s.load(Ordering::Relaxed))
    }
    pub fn set_mat_dst(&self, slot: usize, dst: u8) {
        if let Some(d) = self.state.mat_dst.get(slot) {
            d.store(dst.min(3), Ordering::Relaxed);
        }
    }
    pub fn mat_dst(&self, slot: usize) -> u8 {
        self.state
            .mat_dst
            .get(slot)
            .map_or(0, |d| d.load(Ordering::Relaxed))
    }
    pub fn set_mat_depth(&self, slot: usize, v: f32) {
        if let Some(d) = self.state.mat_depth.get(slot) {
            d.set(v.clamp(-1.0, 1.0));
        }
    }
    pub fn mat_depth(&self, slot: usize) -> f32 {
        self.state.mat_depth.get(slot).map_or(0.0, |d| d.value())
    }

    pub fn set_filter_env_amount(&self, v: f32) {
        self.state.filter_env_amount.set(v);
    }
    pub fn filter_env_amount(&self) -> f32 {
        self.state.filter_env_amount.value()
    }

    pub fn set_fenv_attack(&self, s: f32) {
        self.state.fenv_attack.set(s);
    }
    pub fn fenv_attack(&self) -> f32 {
        self.state.fenv_attack.value()
    }

    pub fn set_fenv_decay(&self, s: f32) {
        self.state.fenv_decay.set(s);
    }
    pub fn fenv_decay(&self) -> f32 {
        self.state.fenv_decay.value()
    }

    pub fn set_fenv_sustain(&self, v: f32) {
        self.state.fenv_sustain.set(v);
    }
    pub fn fenv_sustain(&self) -> f32 {
        self.state.fenv_sustain.value()
    }

    pub fn set_fenv_release(&self, s: f32) {
        self.state.fenv_release.set(s);
    }
    pub fn fenv_release(&self) -> f32 {
        self.state.fenv_release.value()
    }

    // -- LFO 1 --

    pub fn set_lfo_rate(&self, hz: f32) {
        self.state.lfo_rate.set(hz);
    }
    pub fn lfo_rate(&self) -> f32 {
        self.state.lfo_rate.value()
    }

    pub fn set_lfo_depth(&self, v: f32) {
        self.state.lfo_depth.set(v);
    }
    pub fn lfo_depth(&self) -> f32 {
        self.state.lfo_depth.value()
    }

    pub fn set_lfo_shape(&self, s: u8) {
        self.state.lfo_shape.store(s.min(2), Ordering::Relaxed);
    }
    pub fn lfo_shape(&self) -> u8 {
        self.state.lfo_shape.load(Ordering::Relaxed)
    }

    pub fn set_lfo_dest(&self, d: u8) {
        self.state.lfo_dest.store(d.min(2), Ordering::Relaxed);
    }
    pub fn lfo_dest(&self) -> u8 {
        self.state.lfo_dest.load(Ordering::Relaxed)
    }

    pub fn set_lfo_sync(&self, s: u8) {
        self.state.lfo_sync.store(s.min(1), Ordering::Relaxed);
    }
    pub fn lfo_sync(&self) -> u8 {
        self.state.lfo_sync.load(Ordering::Relaxed)
    }

    pub fn set_lfo_division(&self, d: u8) {
        self.state.lfo_division.store(d, Ordering::Relaxed);
    }
    pub fn lfo_division(&self) -> u8 {
        self.state.lfo_division.load(Ordering::Relaxed)
    }

    pub fn set_lfo_pitch_mult(&self, v: f32) {
        self.state.lfo_pitch_mult.set(v);
    }
    pub fn lfo_pitch_mult(&self) -> f32 {
        self.state.lfo_pitch_mult.value()
    }

    // -- LFO 2 --

    pub fn set_lfo2_rate(&self, hz: f32) {
        self.state.lfo2_rate.set(hz);
    }
    pub fn lfo2_rate(&self) -> f32 {
        self.state.lfo2_rate.value()
    }

    pub fn set_lfo2_depth(&self, v: f32) {
        self.state.lfo2_depth.set(v);
    }
    pub fn lfo2_depth(&self) -> f32 {
        self.state.lfo2_depth.value()
    }

    pub fn set_lfo2_shape(&self, s: u8) {
        self.state.lfo2_shape.store(s.min(2), Ordering::Relaxed);
    }
    pub fn lfo2_shape(&self) -> u8 {
        self.state.lfo2_shape.load(Ordering::Relaxed)
    }

    pub fn set_lfo2_dest(&self, d: u8) {
        self.state.lfo2_dest.store(d.min(2), Ordering::Relaxed);
    }
    pub fn lfo2_dest(&self) -> u8 {
        self.state.lfo2_dest.load(Ordering::Relaxed)
    }

    // -- Gate lanes (amp ducker "Pulse" + LFO1 retrigger + LFO2 retrigger) --
    //
    // Each lane shares the same five universal fields (enabled / pattern / length /
    // division / rate). Lane-specific extras (e.g. duck depth on the amp lane) are
    // separate setters. The `set_gate_*` family delegates to the same low-level
    // ops on a `&GatePattern`, just routed to a different lane.

    pub fn set_gate_aenv_enabled(&self, on: bool) {
        self.state.gate_aenv.enabled.store(on, Ordering::Relaxed);
    }
    pub fn gate_aenv_enabled(&self) -> bool {
        self.state.gate_aenv.enabled.load(Ordering::Relaxed)
    }
    pub fn set_gate_aenv_pattern(&self, mask: u16) {
        self.state.gate_aenv.pattern.store(mask, Ordering::Relaxed);
    }
    pub fn gate_aenv_pattern(&self) -> u16 {
        self.state.gate_aenv.pattern.load(Ordering::Relaxed)
    }
    pub fn set_gate_aenv_step(&self, step: u8, on: bool) {
        gate_set_step(&self.state.gate_aenv, step, on);
    }
    pub fn gate_aenv_step(&self, step: u8) -> bool {
        gate_get_step(&self.state.gate_aenv, step)
    }
    pub fn set_gate_aenv_length(&self, len: u8) {
        self.state
            .gate_aenv
            .length
            .store(len.clamp(1, 16), Ordering::Relaxed);
    }
    pub fn gate_aenv_length(&self) -> u8 {
        self.state.gate_aenv.length.load(Ordering::Relaxed)
    }
    pub fn set_gate_aenv_division(&self, d: u8) {
        self.state.gate_aenv.division.store(d, Ordering::Relaxed);
    }
    pub fn gate_aenv_division(&self) -> u8 {
        self.state.gate_aenv.division.load(Ordering::Relaxed)
    }
    pub fn set_gate_aenv_rate(&self, hz: f32) {
        self.state.gate_aenv.rate.set(hz);
    }
    pub fn gate_aenv_rate(&self) -> f32 {
        self.state.gate_aenv.rate.value()
    }
    pub fn set_gate_aenv_depth(&self, v: f32) {
        self.state.gate_aenv_depth.set(v);
    }
    pub fn gate_aenv_depth(&self) -> f32 {
        self.state.gate_aenv_depth.value()
    }

    // LFO1 retrigger lane
    pub fn set_gate_lfo1_enabled(&self, on: bool) {
        self.state.gate_lfo1.enabled.store(on, Ordering::Relaxed);
    }
    pub fn gate_lfo1_enabled(&self) -> bool {
        self.state.gate_lfo1.enabled.load(Ordering::Relaxed)
    }
    pub fn set_gate_lfo1_pattern(&self, mask: u16) {
        self.state.gate_lfo1.pattern.store(mask, Ordering::Relaxed);
    }
    pub fn gate_lfo1_pattern(&self) -> u16 {
        self.state.gate_lfo1.pattern.load(Ordering::Relaxed)
    }
    pub fn set_gate_lfo1_step(&self, step: u8, on: bool) {
        gate_set_step(&self.state.gate_lfo1, step, on);
    }
    pub fn gate_lfo1_step(&self, step: u8) -> bool {
        gate_get_step(&self.state.gate_lfo1, step)
    }
    pub fn set_gate_lfo1_length(&self, len: u8) {
        self.state
            .gate_lfo1
            .length
            .store(len.clamp(1, 16), Ordering::Relaxed);
    }
    pub fn gate_lfo1_length(&self) -> u8 {
        self.state.gate_lfo1.length.load(Ordering::Relaxed)
    }
    pub fn set_gate_lfo1_division(&self, d: u8) {
        self.state.gate_lfo1.division.store(d, Ordering::Relaxed);
    }
    pub fn gate_lfo1_division(&self) -> u8 {
        self.state.gate_lfo1.division.load(Ordering::Relaxed)
    }
    pub fn set_gate_lfo1_rate(&self, hz: f32) {
        self.state.gate_lfo1.rate.set(hz);
    }
    pub fn gate_lfo1_rate(&self) -> f32 {
        self.state.gate_lfo1.rate.value()
    }

    // LFO2 retrigger lane
    pub fn set_gate_lfo2_enabled(&self, on: bool) {
        self.state.gate_lfo2.enabled.store(on, Ordering::Relaxed);
    }
    pub fn gate_lfo2_enabled(&self) -> bool {
        self.state.gate_lfo2.enabled.load(Ordering::Relaxed)
    }
    pub fn set_gate_lfo2_pattern(&self, mask: u16) {
        self.state.gate_lfo2.pattern.store(mask, Ordering::Relaxed);
    }
    pub fn gate_lfo2_pattern(&self) -> u16 {
        self.state.gate_lfo2.pattern.load(Ordering::Relaxed)
    }
    pub fn set_gate_lfo2_step(&self, step: u8, on: bool) {
        gate_set_step(&self.state.gate_lfo2, step, on);
    }
    pub fn gate_lfo2_step(&self, step: u8) -> bool {
        gate_get_step(&self.state.gate_lfo2, step)
    }
    pub fn set_gate_lfo2_length(&self, len: u8) {
        self.state
            .gate_lfo2
            .length
            .store(len.clamp(1, 16), Ordering::Relaxed);
    }
    pub fn gate_lfo2_length(&self) -> u8 {
        self.state.gate_lfo2.length.load(Ordering::Relaxed)
    }
    pub fn set_gate_lfo2_division(&self, d: u8) {
        self.state.gate_lfo2.division.store(d, Ordering::Relaxed);
    }
    pub fn gate_lfo2_division(&self) -> u8 {
        self.state.gate_lfo2.division.load(Ordering::Relaxed)
    }
    pub fn set_gate_lfo2_rate(&self, hz: f32) {
        self.state.gate_lfo2.rate.set(hz);
    }
    pub fn gate_lfo2_rate(&self) -> f32 {
        self.state.gate_lfo2.rate.value()
    }

    // -- Amp envelope + glide + master --

    pub fn set_amp_attack(&self, s: f32) {
        self.state.adsr_attack.set(s);
    }
    pub fn amp_attack(&self) -> f32 {
        self.state.adsr_attack.value()
    }

    pub fn set_amp_decay(&self, s: f32) {
        self.state.adsr_decay.set(s);
    }
    pub fn amp_decay(&self) -> f32 {
        self.state.adsr_decay.value()
    }

    pub fn set_amp_sustain(&self, v: f32) {
        self.state.adsr_sustain.set(v);
    }
    pub fn amp_sustain(&self) -> f32 {
        self.state.adsr_sustain.value()
    }

    pub fn set_amp_release(&self, s: f32) {
        self.state.adsr_release.set(s);
    }
    pub fn amp_release(&self) -> f32 {
        self.state.adsr_release.value()
    }

    pub fn set_glide_time(&self, s: f32) {
        self.state.glide_time.set(s);
    }
    pub fn glide_time(&self) -> f32 {
        self.state.glide_time.value()
    }

    pub fn set_master_volume(&self, v: f32) {
        self.state.master_vol.set(v);
    }
    pub fn master_volume(&self) -> f32 {
        self.state.master_vol.value()
    }

    pub fn set_global_volume(&self, v: f32) {
        self.state.global_vol.set(v);
    }
    pub fn global_volume(&self) -> f32 {
        self.state.global_vol.value()
    }

    pub fn set_limiter_enabled(&self, on: bool) {
        self.state.limiter_enabled.store(on, Ordering::Relaxed);
    }
    pub fn limiter_enabled(&self) -> bool {
        self.state.limiter_enabled.load(Ordering::Relaxed)
    }

    pub fn set_limiter_threshold(&self, v: f32) {
        self.state.limiter_threshold.set(v);
    }
    pub fn limiter_threshold(&self) -> f32 {
        self.state.limiter_threshold.value()
    }

    // -- FX: Overdrive --

    pub fn set_fx_overdrive_drive(&self, v: f32) {
        self.state.fx_overdrive_drive.set(v);
    }
    pub fn fx_overdrive_drive(&self) -> f32 {
        self.state.fx_overdrive_drive.value()
    }
    pub fn set_fx_overdrive_mix(&self, v: f32) {
        self.state.fx_overdrive_mix.set(v);
    }
    pub fn fx_overdrive_mix(&self) -> f32 {
        self.state.fx_overdrive_mix.value()
    }
    pub fn set_fx_overdrive_tone(&self, v: f32) {
        self.state.fx_overdrive_tone.set(v);
    }
    pub fn fx_overdrive_tone(&self) -> f32 {
        self.state.fx_overdrive_tone.value()
    }
    pub fn set_fx_overdrive_asym(&self, v: f32) {
        self.state.fx_overdrive_asym.set(v);
    }
    pub fn fx_overdrive_asym(&self) -> f32 {
        self.state.fx_overdrive_asym.value()
    }

    // -- FX: Distortion --

    pub fn set_fx_distortion_drive(&self, v: f32) {
        self.state.fx_distortion_drive.set(v);
    }
    pub fn fx_distortion_drive(&self) -> f32 {
        self.state.fx_distortion_drive.value()
    }
    pub fn set_fx_distortion_mix(&self, v: f32) {
        self.state.fx_distortion_mix.set(v);
    }
    pub fn fx_distortion_mix(&self) -> f32 {
        self.state.fx_distortion_mix.value()
    }
    pub fn set_fx_distortion_tone(&self, v: f32) {
        self.state.fx_distortion_tone.set(v);
    }
    pub fn fx_distortion_tone(&self) -> f32 {
        self.state.fx_distortion_tone.value()
    }
    pub fn set_fx_distortion_pre(&self, v: f32) {
        self.state.fx_distortion_pre.set(v);
    }
    pub fn fx_distortion_pre(&self) -> f32 {
        self.state.fx_distortion_pre.value()
    }

    // -- FX: Chorus --

    pub fn set_fx_chorus_rate(&self, hz: f32) {
        self.state.fx_chorus_rate.set(hz);
    }
    pub fn fx_chorus_rate(&self) -> f32 {
        self.state.fx_chorus_rate.value()
    }
    pub fn set_fx_chorus_depth(&self, s: f32) {
        self.state.fx_chorus_depth.set(s);
    }
    pub fn fx_chorus_depth(&self) -> f32 {
        self.state.fx_chorus_depth.value()
    }
    pub fn set_fx_chorus_mix(&self, v: f32) {
        self.state.fx_chorus_mix.set(v);
    }
    pub fn fx_chorus_mix(&self) -> f32 {
        self.state.fx_chorus_mix.value()
    }

    // -- FX: Delay --

    pub fn set_fx_delay_time(&self, s: f32) {
        self.state.fx_delay_time.set(s);
    }
    pub fn fx_delay_time(&self) -> f32 {
        self.state.fx_delay_time.value()
    }
    pub fn set_fx_delay_feedback(&self, v: f32) {
        self.state.fx_delay_feedback.set(v);
    }
    pub fn fx_delay_feedback(&self) -> f32 {
        self.state.fx_delay_feedback.value()
    }
    pub fn set_fx_delay_mix(&self, v: f32) {
        self.state.fx_delay_mix.set(v);
    }
    pub fn fx_delay_mix(&self) -> f32 {
        self.state.fx_delay_mix.value()
    }
    pub fn set_fx_delay_sync(&self, s: u8) {
        self.state.fx_delay_sync.store(s.min(1), Ordering::Relaxed);
    }
    pub fn fx_delay_sync(&self) -> u8 {
        self.state.fx_delay_sync.load(Ordering::Relaxed)
    }
    pub fn set_fx_delay_division(&self, d: u8) {
        self.state.fx_delay_division.store(d, Ordering::Relaxed);
    }
    pub fn fx_delay_division(&self) -> u8 {
        self.state.fx_delay_division.load(Ordering::Relaxed)
    }

    // -- FX: Reverb --

    pub fn set_fx_reverb_size(&self, v: f32) {
        self.state.fx_reverb_size.set(v);
    }
    pub fn fx_reverb_size(&self) -> f32 {
        self.state.fx_reverb_size.value()
    }
    pub fn set_fx_reverb_damp(&self, v: f32) {
        self.state.fx_reverb_damp.set(v);
    }
    pub fn fx_reverb_damp(&self) -> f32 {
        self.state.fx_reverb_damp.value()
    }
    pub fn set_fx_reverb_mix(&self, v: f32) {
        self.state.fx_reverb_mix.set(v);
    }
    pub fn fx_reverb_mix(&self) -> f32 {
        self.state.fx_reverb_mix.value()
    }
    pub fn set_fx_reverb_predelay(&self, s: f32) {
        self.state.fx_reverb_predelay.set(s);
    }
    pub fn fx_reverb_predelay(&self) -> f32 {
        self.state.fx_reverb_predelay.value()
    }
    pub fn set_fx_reverb_type(&self, t: u8) {
        self.state.fx_reverb_type.store(t.min(2), Ordering::Relaxed);
    }
    pub fn fx_reverb_type(&self) -> u8 {
        self.state.fx_reverb_type.load(Ordering::Relaxed)
    }

    // -- Stereo --

    pub fn set_stereo_spread(&self, s: f32) {
        self.state.stereo_spread.set(s);
    }
    pub fn stereo_spread(&self) -> f32 {
        self.state.stereo_spread.value()
    }
    pub fn set_stereo_width(&self, v: f32) {
        self.state.stereo_width.set(v);
    }
    pub fn stereo_width(&self) -> f32 {
        self.state.stereo_width.value()
    }

    // -- Shimmer --

    pub fn set_shimmer_size(&self, v: f32) {
        self.state.fx_shimmer.size.set(v);
    }
    pub fn shimmer_size(&self) -> f32 {
        self.state.fx_shimmer.size.value()
    }
    pub fn set_shimmer_damp(&self, v: f32) {
        self.state.fx_shimmer.damp.set(v);
    }
    pub fn shimmer_damp(&self) -> f32 {
        self.state.fx_shimmer.damp.value()
    }
    pub fn set_shimmer_mix(&self, v: f32) {
        self.state.fx_shimmer.mix.set(v);
    }
    pub fn shimmer_mix(&self) -> f32 {
        self.state.fx_shimmer.mix.value()
    }
    pub fn set_shimmer_amount(&self, v: f32) {
        self.state.fx_shimmer.shimmer.set(v);
    }
    pub fn shimmer_amount(&self) -> f32 {
        self.state.fx_shimmer.shimmer.value()
    }
    pub fn set_shimmer_width(&self, v: f32) {
        self.state.fx_shimmer.width.set(v);
    }
    pub fn shimmer_width(&self) -> f32 {
        self.state.fx_shimmer.width.value()
    }
    pub fn set_shimmer_spread(&self, v: f32) {
        self.state.fx_shimmer.spread.set(v);
    }
    pub fn shimmer_spread(&self) -> f32 {
        self.state.fx_shimmer.spread.value()
    }
    pub fn set_shimmer_pitch(&self, p: u8) {
        self.state
            .fx_shimmer
            .pitch
            .store(p.min(2), Ordering::Relaxed);
    }
    pub fn shimmer_pitch(&self) -> u8 {
        self.state.fx_shimmer.pitch.load(Ordering::Relaxed)
    }

    // -- Crystallizer --

    pub fn set_crystal_grain(&self, ms: f32) {
        self.state.fx_crystal.grain_ms.set(ms);
    }
    pub fn crystal_grain(&self) -> f32 {
        self.state.fx_crystal.grain_ms.value()
    }
    pub fn set_crystal_scatter(&self, v: f32) {
        self.state.fx_crystal.scatter.set(v);
    }
    pub fn crystal_scatter(&self) -> f32 {
        self.state.fx_crystal.scatter.value()
    }
    pub fn set_crystal_feedback(&self, v: f32) {
        self.state.fx_crystal.feedback.set(v);
    }
    pub fn crystal_feedback(&self) -> f32 {
        self.state.fx_crystal.feedback.value()
    }
    pub fn set_crystal_delay(&self, ms: f32) {
        self.state.fx_crystal.delay_ms.set(ms);
    }
    pub fn crystal_delay(&self) -> f32 {
        self.state.fx_crystal.delay_ms.value()
    }
    pub fn set_crystal_mix(&self, v: f32) {
        self.state.fx_crystal.mix.set(v);
    }
    pub fn crystal_mix(&self) -> f32 {
        self.state.fx_crystal.mix.value()
    }
    pub fn set_crystal_pitch(&self, p: u8) {
        self.state
            .fx_crystal
            .pitch
            .store(p.min(4), Ordering::Relaxed);
    }
    pub fn crystal_pitch(&self) -> u8 {
        self.state.fx_crystal.pitch.load(Ordering::Relaxed)
    }

    // -- Bit crusher --

    pub fn set_fx_bitcrush_bits(&self, v: f32) {
        self.state.fx_bitcrush_bits.set(v.clamp(1.0, 16.0));
    }
    pub fn fx_bitcrush_bits(&self) -> f32 {
        self.state.fx_bitcrush_bits.value()
    }
    pub fn set_fx_bitcrush_rate(&self, v: f32) {
        self.state.fx_bitcrush_rate.set(v.clamp(1.0, 32.0));
    }
    pub fn fx_bitcrush_rate(&self) -> f32 {
        self.state.fx_bitcrush_rate.value()
    }
    pub fn set_fx_bitcrush_mix(&self, v: f32) {
        self.state.fx_bitcrush_mix.set(v.clamp(0.0, 1.0));
    }
    pub fn fx_bitcrush_mix(&self) -> f32 {
        self.state.fx_bitcrush_mix.value()
    }

    // -- Tape saturation --

    pub fn set_fx_tape_drive(&self, v: f32) {
        self.state.fx_tape_drive.set(v.clamp(0.0, 1.0));
    }
    pub fn fx_tape_drive(&self) -> f32 {
        self.state.fx_tape_drive.value()
    }
    pub fn set_fx_tape_tone(&self, v: f32) {
        self.state.fx_tape_tone.set(v.clamp(0.0, 1.0));
    }
    pub fn fx_tape_tone(&self) -> f32 {
        self.state.fx_tape_tone.value()
    }
    pub fn set_fx_tape_bias(&self, v: f32) {
        self.state.fx_tape_bias.set(v.clamp(0.0, 1.0));
    }
    pub fn fx_tape_bias(&self) -> f32 {
        self.state.fx_tape_bias.value()
    }
    pub fn set_fx_tape_mix(&self, v: f32) {
        self.state.fx_tape_mix.set(v.clamp(0.0, 1.0));
    }
    pub fn fx_tape_mix(&self) -> f32 {
        self.state.fx_tape_mix.value()
    }

    // -- Phaser --

    pub fn set_fx_phaser_rate(&self, v: f32) {
        self.state.fx_phaser_rate.set(v.clamp(0.05, 10.0));
    }
    pub fn fx_phaser_rate(&self) -> f32 {
        self.state.fx_phaser_rate.value()
    }
    pub fn set_fx_phaser_depth(&self, v: f32) {
        self.state.fx_phaser_depth.set(v.clamp(0.0, 1.0));
    }
    pub fn fx_phaser_depth(&self) -> f32 {
        self.state.fx_phaser_depth.value()
    }
    pub fn set_fx_phaser_feedback(&self, v: f32) {
        self.state.fx_phaser_feedback.set(v.clamp(-0.9, 0.9));
    }
    pub fn fx_phaser_feedback(&self) -> f32 {
        self.state.fx_phaser_feedback.value()
    }
    pub fn set_fx_phaser_center(&self, v: f32) {
        self.state.fx_phaser_center.set(v.clamp(100.0, 8000.0));
    }
    pub fn fx_phaser_center(&self) -> f32 {
        self.state.fx_phaser_center.value()
    }
    pub fn set_fx_phaser_stages(&self, v: u8) {
        self.state
            .fx_phaser_stages
            .store(v.clamp(2, 8), Ordering::Relaxed);
    }
    pub fn fx_phaser_stages(&self) -> u8 {
        self.state.fx_phaser_stages.load(Ordering::Relaxed)
    }
    pub fn set_fx_phaser_mix(&self, v: f32) {
        self.state.fx_phaser_mix.set(v.clamp(0.0, 1.0));
    }
    pub fn fx_phaser_mix(&self) -> f32 {
        self.state.fx_phaser_mix.value()
    }

    // -- Arpeggiator --

    pub fn set_arp_enabled(&self, on: bool) {
        self.state.arp.enabled.store(on, Ordering::Relaxed);
    }
    pub fn arp_enabled(&self) -> bool {
        self.state.arp.enabled.load(Ordering::Relaxed)
    }
    pub fn set_arp_mode(&self, m: u8) {
        self.state.arp.mode.store(m, Ordering::Relaxed);
    }
    pub fn arp_mode(&self) -> u8 {
        self.state.arp.mode.load(Ordering::Relaxed)
    }
    pub fn set_arp_division(&self, d: u8) {
        self.state.arp.division.store(d, Ordering::Relaxed);
    }
    pub fn arp_division(&self) -> u8 {
        self.state.arp.division.load(Ordering::Relaxed)
    }
    pub fn set_arp_octave_range(&self, o: u8) {
        self.state
            .arp
            .octave_range
            .store(o.max(1), Ordering::Relaxed);
    }
    pub fn arp_octave_range(&self) -> u8 {
        self.state.arp.octave_range.load(Ordering::Relaxed)
    }
    pub fn set_arp_gate(&self, v: f32) {
        self.state.arp.gate.set(v);
    }
    pub fn arp_gate(&self) -> f32 {
        self.state.arp.gate.value()
    }
    pub fn set_arp_hold(&self, on: bool) {
        self.state.arp.hold.store(on, Ordering::Relaxed);
    }
    pub fn arp_hold(&self) -> bool {
        self.state.arp.hold.load(Ordering::Relaxed)
    }
    pub fn set_arp_bpm(&self, bpm: f32) {
        self.state.arp.bpm.set(bpm);
    }
    pub fn arp_bpm(&self) -> f32 {
        self.state.arp.bpm.value()
    }

    // -- Arp ring gate sequencer --

    pub fn set_arp_ring_enabled(&self, on: bool) {
        self.state.arp.ring_enabled.store(on, Ordering::Relaxed);
    }
    pub fn arp_ring_enabled(&self) -> bool {
        self.state.arp.ring_enabled.load(Ordering::Relaxed)
    }
    pub fn set_arp_ring_steps(&self, n: u8) {
        self.state
            .arp
            .ring_steps
            .store(n.clamp(2, 16), Ordering::Relaxed);
    }
    pub fn arp_ring_steps(&self) -> u8 {
        self.state.arp.ring_steps.load(Ordering::Relaxed)
    }
    pub fn set_arp_ring_pattern(&self, p: u32) {
        self.state.arp.ring_pattern.store(p, Ordering::Relaxed);
    }
    pub fn arp_ring_pattern(&self) -> u32 {
        self.state.arp.ring_pattern.load(Ordering::Relaxed)
    }
    pub fn arp_ring_pos(&self) -> u8 {
        self.state.arp.ring_pos.load(Ordering::Relaxed)
    }

    // -- Scale walker --

    pub fn set_walker_enabled(&self, on: bool) {
        self.state.walker.enabled.store(on, Ordering::Relaxed);
    }
    pub fn walker_enabled(&self) -> bool {
        self.state.walker.enabled.load(Ordering::Relaxed)
    }
    pub fn set_walker_scale(&self, s: u8) {
        self.state.walker.scale.store(s, Ordering::Relaxed);
    }
    pub fn walker_scale(&self) -> u8 {
        self.state.walker.scale.load(Ordering::Relaxed)
    }
    pub fn set_walker_root(&self, r: u8) {
        self.state.walker.root.store(r, Ordering::Relaxed);
    }
    pub fn walker_root(&self) -> u8 {
        self.state.walker.root.load(Ordering::Relaxed)
    }
    pub fn set_walker_octave_range(&self, o: u8) {
        self.state
            .walker
            .octave_range
            .store(o.max(1), Ordering::Relaxed);
    }
    pub fn walker_octave_range(&self) -> u8 {
        self.state.walker.octave_range.load(Ordering::Relaxed)
    }
    pub fn set_walker_division(&self, d: u8) {
        self.state.walker.division.store(d, Ordering::Relaxed);
    }
    pub fn walker_division(&self) -> u8 {
        self.state.walker.division.load(Ordering::Relaxed)
    }
    pub fn set_walker_gate(&self, v: f32) {
        self.state.walker.gate.set(v);
    }
    pub fn walker_gate(&self) -> f32 {
        self.state.walker.gate.value()
    }
    pub fn set_walker_bpm(&self, bpm: f32) {
        self.state.walker.bpm.set(bpm);
    }
    pub fn walker_bpm(&self) -> f32 {
        self.state.walker.bpm.value()
    }

    // =======================================================================
    // Event methods — channel-routed, track 0
    // =======================================================================

    /// Trigger a note. Sends `ControlEvent::NoteOn { .., track: 0 }` and
    /// records a timestamp that the audio callback uses to compute
    /// `last_latency_us`. The mutex write never contends with the audio
    /// thread (which only `try_lock`s), so UI / sequencer / MIDI threads
    /// may call this freely.
    pub fn note_on(&self, pitch: u8, velocity: u8) {
        if let Ok(mut t) = self.state.note_on_time.lock() {
            *t = Some(std::time::Instant::now());
        }
        let _ = self.control.try_send(ControlEvent::NoteOn {
            pitch,
            velocity,
            track: 0,
        });
    }

    /// Release a note.
    pub fn note_off(&self, pitch: u8) {
        let _ = self
            .control
            .try_send(ControlEvent::NoteOff { pitch, track: 0 });
    }

    /// Release every currently-held note.
    ///
    /// Stage 1 implementation sends one `NoteOff` per MIDI pitch. Simple and
    /// correct; a dedicated `AllNotesOff` channel variant can be added later
    /// if this becomes a hot path.
    pub fn all_notes_off(&self) {
        for pitch in 0u8..=127 {
            let _ = self
                .control
                .try_send(ControlEvent::NoteOff { pitch, track: 0 });
        }
    }

    /// Panic: zero every voice gate right now, bypassing the event channel.
    /// Voices go through ADSR release on the next audio sample. Intended
    /// for hard silence (mode changes, patch loads, emergency stop) where
    /// waiting for a channel round-trip would leak audio.
    pub fn silence_all_voices(&self) {
        for gate in self.state.voice_gates.iter() {
            gate.set(0.0);
        }
    }

    /// Flush all FX tail buffers (delay, reverb, shimmer, crystallizer, pre-delay,
    /// Haas widener). Runs on the next audio callback tick — no allocation or locking.
    /// Call on patch load or when enabling an effect to prevent old signal bleeding in.
    pub fn reset_fx_tails(&self) {
        self.state.fx_clear_requested.store(true, Ordering::Relaxed);
    }

    /// Latch a chord into the arpeggiator.
    pub fn chord_hold(&self, notes: &[u8]) {
        let _ = self.control.try_send(ControlEvent::ChordHold {
            track: 0,
            notes: notes.to_vec(),
        });
    }

    /// Restart arpeggiator timing.
    pub fn arp_restart(&self) {
        let _ = self.control.try_send(ControlEvent::ArpRestart { track: 0 });
    }

    /// Restart scale-walker timing.
    pub fn walker_restart(&self) {
        let _ = self
            .control
            .try_send(ControlEvent::WalkerRestart { track: 0 });
    }

    // =======================================================================
    // Readback — atomic reads, safe from any thread
    // =======================================================================

    /// Current amp-envelope cursor for a voice. Encoding: 0=idle, 1.x=attack,
    /// 2.x=decay, 3.0=sustain, 4.x=release. Returns 0.0 if voice index is
    /// out of range.
    pub fn amp_cursor(&self, voice: usize) -> f32 {
        self.state
            .amp_cursors
            .get(voice)
            .map(|s| s.value())
            .unwrap_or(0.0)
    }

    /// Snapshot every voice's amp-envelope cursor in one call. Same encoding
    /// as [`Self::amp_cursor`]. Allocates a `Vec` on the caller thread (UI
    /// rate — fine). Length is always `crate::audio::VOICE_COUNT`.
    pub fn amp_cursors(&self) -> Vec<f32> {
        self.state.amp_cursors.iter().map(|s| s.value()).collect()
    }

    /// Snapshot every voice's gate value (1.0 = held, 0.0 = released).
    /// Length is always `crate::audio::VOICE_COUNT`.
    pub fn voice_gates(&self) -> Vec<f32> {
        self.state.voice_gates.iter().map(|s| s.value()).collect()
    }

    /// Snapshot every voice's current oscillator frequency in Hz.
    /// Length is always `crate::audio::VOICE_COUNT`. Returns 0.0 for idle voices.
    pub fn voice_freqs(&self) -> Vec<f32> {
        self.state.voice_freqs.iter().map(|s| s.value()).collect()
    }

    /// Current filter-envelope cursor for a voice. Same encoding as amp cursor.
    pub fn fenv_cursor(&self, voice: usize) -> f32 {
        self.state
            .fenv_cursors
            .get(voice)
            .map(|s| s.value())
            .unwrap_or(0.0)
    }

    /// Snapshot every voice's filter-envelope cursor in one call.
    pub fn fenv_cursors(&self) -> Vec<f32> {
        self.state.fenv_cursors.iter().map(|s| s.value()).collect()
    }

    /// Clone the oscilloscope ring buffer for display. Briefly acquires the
    /// buffer mutex — the audio callback uses `try_lock` so this never
    /// stalls the audio thread. Call from the UI thread at frame rate.
    pub fn scope_buffer_snapshot(&self) -> Vec<f32> {
        self.state
            .osc_buffer
            .lock()
            .map(|b| b.clone())
            .unwrap_or_default()
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
            Command::SetParam { id, value } => self.set_by_id(id, value),
            Command::NoteOn { pitch, velocity } => self.note_on(pitch, velocity),
            Command::NoteOff { pitch } => self.note_off(pitch),
            Command::AllNotesOff => self.all_notes_off(),
            Command::ChordHold(notes) => self.chord_hold(&notes),
            Command::ArpRestart => self.arp_restart(),
            Command::WalkerRestart => self.walker_restart(),
            _ => {} // non_exhaustive
        }
    }

    /// Generic parameter write keyed by `ParamId`. One arm per variant.
    pub fn set_by_id(&self, id: ParamId, v: f32) {
        // Helper for u8-backed discrete casts.
        #[inline]
        fn u8c(v: f32, max: u8) -> u8 {
            v.clamp(0.0, max as f32).round() as u8
        }
        #[inline]
        fn b(v: f32) -> bool {
            v != 0.0
        }

        match id {
            // -- Oscillator bank --
            ParamId::OscWave(osc) => self.set_osc_wave(osc, u8c(v, 3)),
            ParamId::OscFreqMult(osc) => self.set_osc_freq_mult(osc, v),
            ParamId::OscVol(osc) => self.set_osc_vol(osc, v),
            ParamId::OscPulseWidth(osc) => self.set_osc_pulse_width(osc, v),
            ParamId::OscUnisonDetune(osc, c) => self.set_osc_unison_detune(osc, c, v),
            ParamId::OscUnisonVol(osc, c) => self.set_osc_unison_vol(osc, c, v),
            ParamId::HardSyncEnabled => self.set_hard_sync_enabled(b(v)),
            ParamId::FmDepth => self.set_fm_depth(v),
            ParamId::RingDepth => self.set_ring_depth(v),
            ParamId::NoiseVol => self.set_noise_vol(v),

            // -- Filter --
            ParamId::FilterCutoff => self.set_filter_cutoff(v),
            ParamId::FilterResonance => self.set_filter_resonance(v),
            ParamId::FilterEnvAmount => self.set_filter_env_amount(v),
            ParamId::FenvAttack => self.set_fenv_attack(v),
            ParamId::FenvDecay => self.set_fenv_decay(v),
            ParamId::FenvSustain => self.set_fenv_sustain(v),
            ParamId::FenvRelease => self.set_fenv_release(v),

            // -- LFO 1 --
            ParamId::LfoRate => self.set_lfo_rate(v),
            ParamId::LfoDepth => self.set_lfo_depth(v),
            ParamId::LfoShape => self.set_lfo_shape(u8c(v, 2)),
            ParamId::LfoDest => self.set_lfo_dest(u8c(v, 2)),
            ParamId::LfoSync => self.set_lfo_sync(u8c(v, 1)),
            ParamId::LfoDivision => self.set_lfo_division(u8c(v, 15)),
            ParamId::LfoPitchMult => self.set_lfo_pitch_mult(v),

            // -- LFO 2 --
            ParamId::Lfo2Rate => self.set_lfo2_rate(v),
            ParamId::Lfo2Depth => self.set_lfo2_depth(v),
            ParamId::Lfo2Shape => self.set_lfo2_shape(u8c(v, 2)),
            ParamId::Lfo2Dest => self.set_lfo2_dest(u8c(v, 2)),

            // -- Gate lanes (amp ducker + LFO1 retrigger + LFO2 retrigger) --
            ParamId::GateAenvEnabled => self.set_gate_aenv_enabled(b(v)),
            ParamId::GateAenvPattern => {
                let mask = (v.round().clamp(0.0, 65535.0) as u32 & 0xFFFF) as u16;
                self.set_gate_aenv_pattern(mask);
            }
            ParamId::GateAenvLength => self.set_gate_aenv_length(u8c(v.max(1.0), 16)),
            ParamId::GateAenvDivision => self.set_gate_aenv_division(u8c(v, 13)),
            ParamId::GateAenvRate => self.set_gate_aenv_rate(v),
            ParamId::GateAenvDepth => self.set_gate_aenv_depth(v),
            ParamId::GateLfo1Enabled => self.set_gate_lfo1_enabled(b(v)),
            ParamId::GateLfo1Pattern => {
                let mask = (v.round().clamp(0.0, 65535.0) as u32 & 0xFFFF) as u16;
                self.set_gate_lfo1_pattern(mask);
            }
            ParamId::GateLfo1Length => self.set_gate_lfo1_length(u8c(v.max(1.0), 16)),
            ParamId::GateLfo1Division => self.set_gate_lfo1_division(u8c(v, 13)),
            ParamId::GateLfo1Rate => self.set_gate_lfo1_rate(v),
            ParamId::GateLfo2Enabled => self.set_gate_lfo2_enabled(b(v)),
            ParamId::GateLfo2Pattern => {
                let mask = (v.round().clamp(0.0, 65535.0) as u32 & 0xFFFF) as u16;
                self.set_gate_lfo2_pattern(mask);
            }
            ParamId::GateLfo2Length => self.set_gate_lfo2_length(u8c(v.max(1.0), 16)),
            ParamId::GateLfo2Division => self.set_gate_lfo2_division(u8c(v, 13)),
            ParamId::GateLfo2Rate => self.set_gate_lfo2_rate(v),

            // -- Amp envelope + glide + master --
            ParamId::AmpAttack => self.set_amp_attack(v),
            ParamId::AmpDecay => self.set_amp_decay(v),
            ParamId::AmpSustain => self.set_amp_sustain(v),
            ParamId::AmpRelease => self.set_amp_release(v),
            ParamId::GlideTime => self.set_glide_time(v),
            ParamId::MasterVolume => self.set_master_volume(v),
            ParamId::GlobalVolume => self.set_global_volume(v),
            ParamId::LimiterEnabled => self.set_limiter_enabled(b(v)),
            ParamId::LimiterThreshold => self.set_limiter_threshold(v),

            // -- FX chain --
            ParamId::FxOverdriveDrive => self.set_fx_overdrive_drive(v),
            ParamId::FxOverdriveMix => self.set_fx_overdrive_mix(v),
            ParamId::FxOverdriveTone => self.set_fx_overdrive_tone(v),
            ParamId::FxOverdriveAsym => self.set_fx_overdrive_asym(v),
            ParamId::FxDistortionDrive => self.set_fx_distortion_drive(v),
            ParamId::FxDistortionMix => self.set_fx_distortion_mix(v),
            ParamId::FxDistortionTone => self.set_fx_distortion_tone(v),
            ParamId::FxDistortionPre => self.set_fx_distortion_pre(v),
            ParamId::FxChorusRate => self.set_fx_chorus_rate(v),
            ParamId::FxChorusDepth => self.set_fx_chorus_depth(v),
            ParamId::FxChorusMix => self.set_fx_chorus_mix(v),
            ParamId::FxDelayTime => self.set_fx_delay_time(v),
            ParamId::FxDelayFeedback => self.set_fx_delay_feedback(v),
            ParamId::FxDelayMix => self.set_fx_delay_mix(v),
            ParamId::FxDelaySync => self.set_fx_delay_sync(u8c(v, 1)),
            ParamId::FxDelayDivision => self.set_fx_delay_division(u8c(v, 15)),
            ParamId::FxReverbSize => self.set_fx_reverb_size(v),
            ParamId::FxReverbDamp => self.set_fx_reverb_damp(v),
            ParamId::FxReverbMix => self.set_fx_reverb_mix(v),
            ParamId::FxReverbPredelay => self.set_fx_reverb_predelay(v),
            ParamId::FxReverbType => self.set_fx_reverb_type(u8c(v, 2)),
            ParamId::StereoSpread => self.set_stereo_spread(v),
            ParamId::StereoWidth => self.set_stereo_width(v),

            // -- Shimmer --
            ParamId::ShimmerSize => self.set_shimmer_size(v),
            ParamId::ShimmerDamp => self.set_shimmer_damp(v),
            ParamId::ShimmerMix => self.set_shimmer_mix(v),
            ParamId::ShimmerAmount => self.set_shimmer_amount(v),
            ParamId::ShimmerWidth => self.set_shimmer_width(v),
            ParamId::ShimmerSpread => self.set_shimmer_spread(v),
            ParamId::ShimmerPitch => self.set_shimmer_pitch(u8c(v, 2)),

            // -- Crystallizer --
            ParamId::CrystalGrain => self.set_crystal_grain(v),
            ParamId::CrystalScatter => self.set_crystal_scatter(v),
            ParamId::CrystalFeedback => self.set_crystal_feedback(v),
            ParamId::CrystalDelay => self.set_crystal_delay(v),
            ParamId::CrystalMix => self.set_crystal_mix(v),
            ParamId::CrystalPitch => self.set_crystal_pitch(u8c(v, 4)),

            // -- Arp --
            ParamId::ArpEnabled => self.set_arp_enabled(b(v)),
            ParamId::ArpMode => self.set_arp_mode(u8c(v, 4)),
            ParamId::ArpDivision => self.set_arp_division(u8c(v, 15)),
            ParamId::ArpOctaveRange => self.set_arp_octave_range(u8c(v, 4)),
            ParamId::ArpGate => self.set_arp_gate(v),
            ParamId::ArpHold => self.set_arp_hold(b(v)),
            ParamId::ArpBpm => self.set_arp_bpm(v),

            // -- Walker --
            ParamId::WalkerEnabled => self.set_walker_enabled(b(v)),
            ParamId::WalkerScale => self.set_walker_scale(u8c(v, 7)),
            ParamId::WalkerRoot => self.set_walker_root(u8c(v, 127)),
            ParamId::WalkerOctaveRange => self.set_walker_octave_range(u8c(v, 3)),
            ParamId::WalkerDivision => self.set_walker_division(u8c(v, 15)),
            ParamId::WalkerGate => self.set_walker_gate(v),
            ParamId::WalkerBpm => self.set_walker_bpm(v),

            _ => {} // non_exhaustive — future ParamIds silently ignored
        }
    }

    /// Generic parameter read keyed by `ParamId`. Returns `None` for
    /// unsupported or readback-only identifiers.
    pub fn get_by_id(&self, id: ParamId) -> Option<f32> {
        let v = match id {
            ParamId::OscWave(o) => self.osc_wave(o) as f32,
            ParamId::OscFreqMult(o) => self.osc_freq_mult(o),
            ParamId::OscVol(o) => self.osc_vol(o),
            ParamId::OscPulseWidth(o) => self.osc_pulse_width(o),
            ParamId::OscUnisonDetune(o, c) => self.osc_unison_detune(o, c),
            ParamId::OscUnisonVol(o, c) => self.osc_unison_vol(o, c),
            ParamId::HardSyncEnabled => bf(self.hard_sync_enabled()),
            ParamId::FmDepth => self.fm_depth(),
            ParamId::RingDepth => self.ring_depth(),
            ParamId::NoiseVol => self.noise_vol(),
            ParamId::FilterCutoff => self.filter_cutoff(),
            ParamId::FilterResonance => self.filter_resonance(),
            ParamId::FilterEnvAmount => self.filter_env_amount(),
            ParamId::FenvAttack => self.fenv_attack(),
            ParamId::FenvDecay => self.fenv_decay(),
            ParamId::FenvSustain => self.fenv_sustain(),
            ParamId::FenvRelease => self.fenv_release(),
            ParamId::LfoRate => self.lfo_rate(),
            ParamId::LfoDepth => self.lfo_depth(),
            ParamId::LfoShape => self.lfo_shape() as f32,
            ParamId::LfoDest => self.lfo_dest() as f32,
            ParamId::LfoSync => self.lfo_sync() as f32,
            ParamId::LfoDivision => self.lfo_division() as f32,
            ParamId::LfoPitchMult => self.lfo_pitch_mult(),
            ParamId::Lfo2Rate => self.lfo2_rate(),
            ParamId::Lfo2Depth => self.lfo2_depth(),
            ParamId::Lfo2Shape => self.lfo2_shape() as f32,
            ParamId::Lfo2Dest => self.lfo2_dest() as f32,
            ParamId::GateAenvEnabled => bf(self.gate_aenv_enabled()),
            ParamId::GateAenvPattern => self.gate_aenv_pattern() as f32,
            ParamId::GateAenvLength => self.gate_aenv_length() as f32,
            ParamId::GateAenvDivision => self.gate_aenv_division() as f32,
            ParamId::GateAenvRate => self.gate_aenv_rate(),
            ParamId::GateAenvDepth => self.gate_aenv_depth(),
            ParamId::GateLfo1Enabled => bf(self.gate_lfo1_enabled()),
            ParamId::GateLfo1Pattern => self.gate_lfo1_pattern() as f32,
            ParamId::GateLfo1Length => self.gate_lfo1_length() as f32,
            ParamId::GateLfo1Division => self.gate_lfo1_division() as f32,
            ParamId::GateLfo1Rate => self.gate_lfo1_rate(),
            ParamId::GateLfo2Enabled => bf(self.gate_lfo2_enabled()),
            ParamId::GateLfo2Pattern => self.gate_lfo2_pattern() as f32,
            ParamId::GateLfo2Length => self.gate_lfo2_length() as f32,
            ParamId::GateLfo2Division => self.gate_lfo2_division() as f32,
            ParamId::GateLfo2Rate => self.gate_lfo2_rate(),
            ParamId::AmpAttack => self.amp_attack(),
            ParamId::AmpDecay => self.amp_decay(),
            ParamId::AmpSustain => self.amp_sustain(),
            ParamId::AmpRelease => self.amp_release(),
            ParamId::GlideTime => self.glide_time(),
            ParamId::MasterVolume => self.master_volume(),
            ParamId::GlobalVolume => self.global_volume(),
            ParamId::LimiterEnabled => bf(self.limiter_enabled()),
            ParamId::LimiterThreshold => self.limiter_threshold(),
            ParamId::FxOverdriveDrive => self.fx_overdrive_drive(),
            ParamId::FxOverdriveMix => self.fx_overdrive_mix(),
            ParamId::FxOverdriveTone => self.fx_overdrive_tone(),
            ParamId::FxOverdriveAsym => self.fx_overdrive_asym(),
            ParamId::FxDistortionDrive => self.fx_distortion_drive(),
            ParamId::FxDistortionMix => self.fx_distortion_mix(),
            ParamId::FxDistortionTone => self.fx_distortion_tone(),
            ParamId::FxDistortionPre => self.fx_distortion_pre(),
            ParamId::FxChorusRate => self.fx_chorus_rate(),
            ParamId::FxChorusDepth => self.fx_chorus_depth(),
            ParamId::FxChorusMix => self.fx_chorus_mix(),
            ParamId::FxDelayTime => self.fx_delay_time(),
            ParamId::FxDelayFeedback => self.fx_delay_feedback(),
            ParamId::FxDelayMix => self.fx_delay_mix(),
            ParamId::FxDelaySync => self.fx_delay_sync() as f32,
            ParamId::FxDelayDivision => self.fx_delay_division() as f32,
            ParamId::FxReverbSize => self.fx_reverb_size(),
            ParamId::FxReverbDamp => self.fx_reverb_damp(),
            ParamId::FxReverbMix => self.fx_reverb_mix(),
            ParamId::FxReverbPredelay => self.fx_reverb_predelay(),
            ParamId::FxReverbType => self.fx_reverb_type() as f32,
            ParamId::StereoSpread => self.stereo_spread(),
            ParamId::StereoWidth => self.stereo_width(),
            ParamId::ShimmerSize => self.shimmer_size(),
            ParamId::ShimmerDamp => self.shimmer_damp(),
            ParamId::ShimmerMix => self.shimmer_mix(),
            ParamId::ShimmerAmount => self.shimmer_amount(),
            ParamId::ShimmerWidth => self.shimmer_width(),
            ParamId::ShimmerSpread => self.shimmer_spread(),
            ParamId::ShimmerPitch => self.shimmer_pitch() as f32,
            ParamId::CrystalGrain => self.crystal_grain(),
            ParamId::CrystalScatter => self.crystal_scatter(),
            ParamId::CrystalFeedback => self.crystal_feedback(),
            ParamId::CrystalDelay => self.crystal_delay(),
            ParamId::CrystalMix => self.crystal_mix(),
            ParamId::CrystalPitch => self.crystal_pitch() as f32,
            ParamId::ArpEnabled => bf(self.arp_enabled()),
            ParamId::ArpMode => self.arp_mode() as f32,
            ParamId::ArpDivision => self.arp_division() as f32,
            ParamId::ArpOctaveRange => self.arp_octave_range() as f32,
            ParamId::ArpGate => self.arp_gate(),
            ParamId::ArpHold => bf(self.arp_hold()),
            ParamId::ArpBpm => self.arp_bpm(),
            ParamId::WalkerEnabled => bf(self.walker_enabled()),
            ParamId::WalkerScale => self.walker_scale() as f32,
            ParamId::WalkerRoot => self.walker_root() as f32,
            ParamId::WalkerOctaveRange => self.walker_octave_range() as f32,
            ParamId::WalkerDivision => self.walker_division() as f32,
            ParamId::WalkerGate => self.walker_gate(),
            ParamId::WalkerBpm => self.walker_bpm(),
            _ => return None,
        };
        Some(v)
    }

    // =======================================================================
    // Patch I/O
    // =======================================================================

    /// Write every engine-relevant field from `p` into the live engine
    /// state. Respects the UI-side bypass flags (`osc_enabled`, `fm_enabled`,
    /// `filter_enabled`, `lfo_enabled`, `fx_*_on`, …) by zeroing the
    /// corresponding engine parameter instead of exposing a separate enable
    /// bit — the engine has no such bit.
    ///
    /// Does NOT silence voices. Callers should invoke `all_notes_off` first
    /// if they want to avoid filter blow-up on sudden parameter jumps.
    pub fn apply_patch(&self, p: &Patch) {
        // -- Oscillator bank --
        for i in 0..3 {
            self.set_osc_wave(i as u8, p.osc_wave[i] as u8);
            self.set_osc_vol(i as u8, if p.osc_enabled[i] { p.osc_vol[i] } else { 0.0 });
            self.set_osc_pulse_width(i as u8, p.osc_pulse_width[i]);
            // Freq mult combines octave + detune.
            let mult = 2_f32.powf(p.osc_octave[i] as f32 + p.osc_detune[i] / 1200.0);
            self.set_osc_freq_mult(i as u8, mult);
            apply_unison_from_patch(self, i, p);
        }
        self.set_hard_sync_enabled(p.hard_sync);
        self.set_fm_depth(if p.fm_enabled { p.fm_depth } else { 0.0 });
        self.set_ring_depth(if p.ring_enabled { p.ring_depth } else { 0.0 });
        self.set_noise_vol(p.noise_vol);

        // -- Filter --
        self.set_filter_cutoff(if p.filter_enabled {
            p.filter_cutoff
        } else {
            18_000.0
        });
        self.set_filter_resonance(if p.filter_enabled { p.filter_q } else { 0.0 });
        self.set_filter_drive(p.filter_drive);
        self.set_filter_key_track(p.filter_key_track);
        self.set_vel_amp(p.vel_amp);
        self.set_vel_filter(p.vel_filter);
        self.set_mono_mode(p.mono_mode);
        self.set_mod_wheel_dest(p.mod_wheel_dest);
        self.set_mod_wheel_depth(p.mod_wheel_depth);
        self.set_aftertouch_dest(p.aftertouch_dest);
        self.set_aftertouch_depth(p.aftertouch_depth);
        for i in 0..4 {
            self.set_mat_src(i, p.mat_src[i]);
            self.set_mat_dst(i, p.mat_dst[i]);
            self.set_mat_depth(i, p.mat_depth[i]);
        }
        self.set_filter_env_amount(p.filter_env_amount);
        self.set_fenv_attack(p.fenv_adsr[0]);
        self.set_fenv_decay(p.fenv_adsr[1]);
        self.set_fenv_sustain(p.fenv_adsr[2]);
        self.set_fenv_release(p.fenv_adsr[3]);

        // -- LFO 1 --
        self.set_lfo_rate(p.lfo_rate);
        self.set_lfo_depth(if p.lfo_enabled { p.lfo_depth } else { 0.0 });
        self.set_lfo_shape(p.lfo_shape as u8);
        self.set_lfo_dest(p.lfo_dest as u8);
        self.set_lfo_sync(if p.lfo_sync { 1 } else { 0 });
        self.set_lfo_division(p.lfo_division as u8);

        // -- LFO 2 --
        self.set_lfo2_rate(p.lfo2_rate);
        self.set_lfo2_depth(if p.lfo2_enabled { p.lfo2_depth } else { 0.0 });
        self.set_lfo2_shape(p.lfo2_shape as u8);
        self.set_lfo2_dest(p.lfo2_dest as u8);

        // -- Gate lanes (amp ducker + LFO1 retrigger + LFO2 retrigger) --
        // Rate is recomputed UI-side from BPM + division when the patch loads;
        // here we only restore the user's choice (division) and the pattern state.
        self.set_gate_aenv_enabled(p.gate_aenv_enabled);
        self.set_gate_aenv_pattern(p.gate_aenv_pattern);
        self.set_gate_aenv_length(p.gate_aenv_length);
        self.set_gate_aenv_division(p.gate_aenv_division as u8);
        self.set_gate_aenv_depth(p.gate_aenv_depth);
        self.set_gate_lfo1_enabled(p.gate_lfo1_enabled);
        self.set_gate_lfo1_pattern(p.gate_lfo1_pattern);
        self.set_gate_lfo1_length(p.gate_lfo1_length);
        self.set_gate_lfo1_division(p.gate_lfo1_division as u8);
        self.set_gate_lfo2_enabled(p.gate_lfo2_enabled);
        self.set_gate_lfo2_pattern(p.gate_lfo2_pattern);
        self.set_gate_lfo2_length(p.gate_lfo2_length);
        self.set_gate_lfo2_division(p.gate_lfo2_division as u8);

        // -- Arp ring sequencer --
        self.set_arp_ring_enabled(p.arp_ring_enabled);
        self.set_arp_ring_steps(p.arp_ring_steps);
        self.set_arp_ring_pattern(p.arp_ring_pattern);

        // -- Amp envelope + glide + master --
        self.set_amp_attack(p.amp_adsr[0]);
        self.set_amp_decay(p.amp_adsr[1]);
        self.set_amp_sustain(p.amp_adsr[2]);
        self.set_amp_release(p.amp_adsr[3]);
        self.set_glide_time(p.glide_time);
        self.set_master_volume(p.master_vol);
        self.set_global_volume(p.global_vol);
        self.set_limiter_enabled(p.limiter_enabled);
        self.set_limiter_threshold(p.limiter_threshold);

        // -- FX: Overdrive / Distortion / Chorus / Delay / Reverb / Stereo --
        self.set_fx_overdrive_drive(p.fx_overdrive_drive);
        self.set_fx_overdrive_mix(if p.fx_overdrive_on {
            p.fx_overdrive_mix
        } else {
            0.0
        });
        self.set_fx_overdrive_tone(p.fx_overdrive_tone);
        self.set_fx_overdrive_asym(p.fx_overdrive_asym);

        self.set_fx_distortion_drive(p.fx_distortion_drive);
        self.set_fx_distortion_mix(if p.fx_distortion_on {
            p.fx_distortion_mix
        } else {
            0.0
        });
        self.set_fx_distortion_tone(p.fx_distortion_tone);
        self.set_fx_distortion_pre(p.fx_distortion_pre);

        self.set_fx_chorus_rate(p.fx_chorus_rate);
        self.set_fx_chorus_depth(p.fx_chorus_depth);
        self.set_fx_chorus_mix(if p.fx_chorus_on { p.fx_chorus_mix } else { 0.0 });

        self.set_fx_delay_time(p.fx_delay_time);
        self.set_fx_delay_feedback(p.fx_delay_feedback);
        self.set_fx_delay_mix(if p.fx_delay_on { p.fx_delay_mix } else { 0.0 });
        self.set_fx_delay_sync(if p.fx_delay_sync { 1 } else { 0 });
        self.set_fx_delay_division(p.fx_delay_division as u8);

        self.set_fx_reverb_size(p.fx_reverb_size);
        self.set_fx_reverb_damp(p.fx_reverb_damp);
        self.set_fx_reverb_mix(if p.fx_reverb_on { p.fx_reverb_mix } else { 0.0 });
        self.set_fx_reverb_predelay(p.fx_reverb_predelay);
        self.set_fx_reverb_type(p.fx_reverb_type);

        self.set_stereo_spread(p.stereo_spread);
        self.set_stereo_width(p.stereo_width);

        // -- Shimmer --
        self.set_shimmer_size(p.fx_shimmer_size);
        self.set_shimmer_damp(p.fx_shimmer_damp);
        self.set_shimmer_amount(if p.fx_shimmer_on {
            p.fx_shimmer_amt
        } else {
            0.0
        });
        self.set_shimmer_width(p.fx_shimmer_width);
        self.set_shimmer_spread(p.fx_shimmer_spread);
        self.set_shimmer_mix(if p.fx_shimmer_on {
            p.fx_shimmer_mix
        } else {
            0.0
        });
        self.set_shimmer_pitch(p.fx_shimmer_pitch);

        // -- Crystallizer --
        self.set_crystal_grain(p.fx_crystal_grain_ms);
        self.set_crystal_scatter(p.fx_crystal_scatter);
        self.set_crystal_feedback(p.fx_crystal_feedback);
        self.set_crystal_delay(p.fx_crystal_delay_ms);
        self.set_crystal_mix(if p.fx_crystal_on {
            p.fx_crystal_mix
        } else {
            0.0
        });
        self.set_crystal_pitch(p.fx_crystal_pitch);

        // Bit crusher
        self.set_fx_bitcrush_bits(p.fx_bitcrush_bits);
        self.set_fx_bitcrush_rate(p.fx_bitcrush_rate);
        self.set_fx_bitcrush_mix(if p.fx_bitcrush_on {
            p.fx_bitcrush_mix
        } else {
            0.0
        });

        // Tape saturation
        self.set_fx_tape_drive(p.fx_tape_drive);
        self.set_fx_tape_tone(p.fx_tape_tone);
        self.set_fx_tape_bias(p.fx_tape_bias);
        self.set_fx_tape_mix(if p.fx_tape_on { p.fx_tape_mix } else { 0.0 });

        // Phaser
        self.set_fx_phaser_rate(p.fx_phaser_rate);
        self.set_fx_phaser_depth(p.fx_phaser_depth);
        self.set_fx_phaser_feedback(p.fx_phaser_feedback);
        self.set_fx_phaser_center(p.fx_phaser_center);
        self.set_fx_phaser_stages(p.fx_phaser_stages);
        self.set_fx_phaser_mix(if p.fx_phaser_on { p.fx_phaser_mix } else { 0.0 });
    }

    /// Read engine state into a fresh `Patch`. Metadata fields (`name`,
    /// `category`, `synth_model`) are left empty — callers supply them.
    ///
    /// UI-side bypass flags (`osc_enabled`, `fm_enabled`, …) are inferred
    /// from the engine: a non-zero mix / depth / volume is "on". This is
    /// lossy for the corner case "slider at 0 but enabled", but round-trips
    /// correctly for every practical scenario.
    pub fn snapshot_patch(&self) -> Patch {
        let osc_wave = [
            self.osc_wave(0) as usize,
            self.osc_wave(1) as usize,
            self.osc_wave(2) as usize,
        ];
        let (osc_octave, osc_detune) = {
            let mut oct = [0i32; 3];
            let mut det = [0f32; 3];
            for i in 0..3 {
                let (o, d) = freq_mult_to_oct_detune(self.osc_freq_mult(i as u8));
                oct[i] = o;
                det[i] = d;
            }
            (oct, det)
        };
        let osc_vol = [self.osc_vol(0), self.osc_vol(1), self.osc_vol(2)];
        let osc_pulse_width = [
            self.osc_pulse_width(0),
            self.osc_pulse_width(1),
            self.osc_pulse_width(2),
        ];
        let osc_enabled = [osc_vol[0] > 0.0, osc_vol[1] > 0.0, osc_vol[2] > 0.0];
        let osc_pw_enabled = [true, true, true];
        let (osc_unison_enabled, osc_unison_count, osc_unison_spread) = snapshot_unison(self);

        Patch {
            name: String::new(),
            category: String::new(),
            synth_model: String::new(),
            tags: Vec::new(),

            osc_wave,
            osc_octave,
            osc_detune,
            osc_vol,
            osc_enabled,
            osc_pulse_width,
            osc_pw_enabled,
            osc_unison_enabled,
            osc_unison_count,
            osc_unison_spread,
            hard_sync: self.hard_sync_enabled(),
            fm_enabled: self.fm_depth() > 0.0,
            fm_depth: self.fm_depth(),
            ring_enabled: self.ring_depth() > 0.0,
            ring_depth: self.ring_depth(),
            noise_vol: self.noise_vol(),

            lfo_enabled: self.lfo_depth() > 0.0,
            lfo_rate: self.lfo_rate(),
            lfo_depth: self.lfo_depth(),
            lfo_shape: self.lfo_shape() as usize,
            lfo_dest: self.lfo_dest() as usize,
            lfo_sync: self.lfo_sync() != 0,
            lfo_division: self.lfo_division() as usize,

            lfo2_enabled: self.lfo2_depth() > 0.0,
            lfo2_rate: self.lfo2_rate(),
            lfo2_depth: self.lfo2_depth(),
            lfo2_shape: self.lfo2_shape() as usize,
            lfo2_dest: self.lfo2_dest() as usize,

            gate_aenv_enabled: self.gate_aenv_enabled(),
            gate_aenv_pattern: self.gate_aenv_pattern(),
            gate_aenv_length: self.gate_aenv_length(),
            gate_aenv_division: self.gate_aenv_division() as usize,
            gate_aenv_depth: self.gate_aenv_depth(),
            gate_lfo1_enabled: self.gate_lfo1_enabled(),
            gate_lfo1_pattern: self.gate_lfo1_pattern(),
            gate_lfo1_length: self.gate_lfo1_length(),
            gate_lfo1_division: self.gate_lfo1_division() as usize,
            gate_lfo2_enabled: self.gate_lfo2_enabled(),
            gate_lfo2_pattern: self.gate_lfo2_pattern(),
            gate_lfo2_length: self.gate_lfo2_length(),
            gate_lfo2_division: self.gate_lfo2_division() as usize,

            filter_enabled: self.filter_cutoff() < 17_999.0 || self.filter_resonance() > 0.0,
            filter_cutoff: self.filter_cutoff(),
            filter_q: self.filter_resonance(),
            filter_drive: self.filter_drive(),
            filter_key_track: self.filter_key_track(),
            vel_amp: self.vel_amp(),
            vel_filter: self.vel_filter(),
            mono_mode: self.mono_mode(),
            mod_wheel_dest: self.mod_wheel_dest(),
            mod_wheel_depth: self.mod_wheel_depth(),
            aftertouch_dest: self.aftertouch_dest(),
            aftertouch_depth: self.aftertouch_depth(),
            mat_src: std::array::from_fn(|i| self.mat_src(i)),
            mat_dst: std::array::from_fn(|i| self.mat_dst(i)),
            mat_depth: std::array::from_fn(|i| self.mat_depth(i)),
            filter_env_amount: self.filter_env_amount(),
            fenv_adsr: [
                self.fenv_attack(),
                self.fenv_decay(),
                self.fenv_sustain(),
                self.fenv_release(),
            ],

            amp_adsr: [
                self.amp_attack(),
                self.amp_decay(),
                self.amp_sustain(),
                self.amp_release(),
            ],

            glide_time: self.glide_time(),
            master_vol: self.master_volume(),
            global_vol: self.global_volume(),
            limiter_enabled: self.limiter_enabled(),
            limiter_threshold: self.limiter_threshold(),

            fx_overdrive_on: self.fx_overdrive_mix() > 0.0,
            fx_overdrive_drive: self.fx_overdrive_drive(),
            fx_overdrive_mix: self.fx_overdrive_mix(),
            fx_overdrive_tone: self.fx_overdrive_tone(),
            fx_overdrive_asym: self.fx_overdrive_asym(),
            fx_distortion_on: self.fx_distortion_mix() > 0.0,
            fx_distortion_drive: self.fx_distortion_drive(),
            fx_distortion_mix: self.fx_distortion_mix(),
            fx_distortion_tone: self.fx_distortion_tone(),
            fx_distortion_pre: self.fx_distortion_pre(),
            fx_chorus_on: self.fx_chorus_mix() > 0.0,
            fx_chorus_rate: self.fx_chorus_rate(),
            fx_chorus_depth: self.fx_chorus_depth(),
            fx_chorus_mix: self.fx_chorus_mix(),
            fx_delay_on: self.fx_delay_mix() > 0.0,
            fx_delay_time: self.fx_delay_time(),
            fx_delay_feedback: self.fx_delay_feedback(),
            fx_delay_mix: self.fx_delay_mix(),
            fx_delay_sync: self.fx_delay_sync() != 0,
            fx_delay_division: self.fx_delay_division() as usize,
            fx_reverb_on: self.fx_reverb_mix() > 0.0,
            fx_reverb_size: self.fx_reverb_size(),
            fx_reverb_damp: self.fx_reverb_damp(),
            fx_reverb_mix: self.fx_reverb_mix(),
            fx_reverb_predelay: self.fx_reverb_predelay(),
            fx_reverb_type: self.fx_reverb_type(),
            stereo_spread: self.stereo_spread(),
            stereo_width: self.stereo_width(),

            fx_shimmer_on: self.shimmer_mix() > 0.0,
            fx_shimmer_size: self.shimmer_size(),
            fx_shimmer_damp: self.shimmer_damp(),
            fx_shimmer_mix: self.shimmer_mix(),
            fx_shimmer_amt: self.shimmer_amount(),
            fx_shimmer_width: self.shimmer_width(),
            fx_shimmer_spread: self.shimmer_spread(),
            fx_shimmer_pitch: self.shimmer_pitch(),
            fx_crystal_on: self.crystal_mix() > 0.0,
            fx_crystal_mix: self.crystal_mix(),
            fx_crystal_grain_ms: self.crystal_grain(),
            fx_crystal_scatter: self.crystal_scatter(),
            fx_crystal_feedback: self.crystal_feedback(),
            fx_crystal_delay_ms: self.crystal_delay(),
            fx_crystal_pitch: self.crystal_pitch(),

            arp_ring_enabled: self.arp_ring_enabled(),
            arp_ring_steps: self.arp_ring_steps(),
            arp_ring_pattern: self.arp_ring_pattern(),

            note_seq_div: 1,  // 1/8 note default; UI overrides on save
            chord_seq_div: 4, // 1 bar default; UI overrides on save

            fx_bitcrush_on: self.fx_bitcrush_mix() > 0.0,
            fx_bitcrush_bits: self.fx_bitcrush_bits(),
            fx_bitcrush_rate: self.fx_bitcrush_rate(),
            fx_bitcrush_mix: self.fx_bitcrush_mix(),

            fx_tape_on: self.fx_tape_mix() > 0.0,
            fx_tape_drive: self.fx_tape_drive(),
            fx_tape_tone: self.fx_tape_tone(),
            fx_tape_bias: self.fx_tape_bias(),
            fx_tape_mix: self.fx_tape_mix(),

            fx_phaser_on: self.fx_phaser_mix() > 0.0,
            fx_phaser_rate: self.fx_phaser_rate(),
            fx_phaser_depth: self.fx_phaser_depth(),
            fx_phaser_feedback: self.fx_phaser_feedback(),
            fx_phaser_center: self.fx_phaser_center(),
            fx_phaser_stages: self.fx_phaser_stages(),
            fx_phaser_mix: self.fx_phaser_mix(),
        }
    }
}

#[inline]
fn bf(b: bool) -> f32 {
    if b {
        1.0
    } else {
        0.0
    }
}

/// Apply per-oscillator unison detune + volume from a patch, mirroring the
/// UI's `update_unison` logic so that patches behave identically whether
/// they came in through the UI or through `apply_patch`.
fn apply_unison_from_patch(h: &SynthEngineHandle, i: usize, p: &Patch) {
    let osc = i as u8;
    let count = p.osc_unison_count[i];
    let spread = p.osc_unison_spread[i];

    if !p.osc_unison_enabled[i] || count <= 1 {
        for c in 0..5 {
            h.set_osc_unison_detune(osc, c as u8, 1.0);
            h.set_osc_unison_vol(osc, c as u8, if c == 0 { 1.0 } else { 0.0 });
        }
        return;
    }

    let vol = 1.0 / count as f32;
    for c in 0..5 {
        if c < count {
            let t = if count > 1 {
                c as f32 / (count - 1) as f32
            } else {
                0.5
            };
            let cents = -spread * 0.5 + t * spread;
            let detune = 2_f32.powf(cents / 1200.0);
            h.set_osc_unison_detune(osc, c as u8, detune);
            h.set_osc_unison_vol(osc, c as u8, vol);
        } else {
            h.set_osc_unison_detune(osc, c as u8, 1.0);
            h.set_osc_unison_vol(osc, c as u8, 0.0);
        }
    }
}

/// Inverse of the UI's freq-mult computation: `mult = 2^(oct + cents/1200)`.
/// Returns `(octave, detune_cents)`. Snap cents near zero so clean octaves
/// round-trip without spurious detune drift.
fn freq_mult_to_oct_detune(mult: f32) -> (i32, f32) {
    if mult <= 0.0 {
        return (0, 0.0);
    }
    let log2 = mult.log2();
    let oct = log2.round() as i32;
    let residual = log2 - oct as f32;
    let cents = residual * 1200.0;
    let cents = if cents.abs() < 0.01 { 0.0 } else { cents };
    (oct, cents)
}

/// Infer per-oscillator unison config from the live detune / volume tables.
/// Heuristic: count the number of audible copies (vol > 0), and derive spread
/// from the cents delta of the outermost pair.
fn snapshot_unison(h: &SynthEngineHandle) -> ([bool; 3], [usize; 3], [f32; 3]) {
    let mut enabled = [false; 3];
    let mut count = [1usize; 3];
    let mut spread = [0f32; 3];
    for osc in 0..3u8 {
        let vols: [f32; 5] = [
            h.osc_unison_vol(osc, 0),
            h.osc_unison_vol(osc, 1),
            h.osc_unison_vol(osc, 2),
            h.osc_unison_vol(osc, 3),
            h.osc_unison_vol(osc, 4),
        ];
        let active = vols.iter().filter(|v| **v > 0.0).count();
        count[osc as usize] = active.max(1);
        enabled[osc as usize] = active > 1;
        if active > 1 {
            let first = h.osc_unison_detune(osc, 0);
            let last = h.osc_unison_detune(osc, (active - 1) as u8);
            let cents = (last / first).log2() * 1200.0;
            spread[osc as usize] = cents.abs();
        }
    }
    (enabled, count, spread)
}
