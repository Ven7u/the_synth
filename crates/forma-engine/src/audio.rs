//! Audio engine: fundsp synthesis graph and AudioState.
//!
//! Single unified poly graph: 3 OSCs per voice → filter → amp ADSR.
//! LFO is computed in the callback and modulates effective_cutoff via a Shared.

#![allow(clippy::precedence)]

use fundsp::prelude32::*;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;

use crate::gated_voice::GatedVoice;
use forma_common::ClockDivision;
use forma_dsp::crystallizer::{Crystallizer, CrystallizerShared};
use forma_dsp::envelope::LiveAdsr;
use forma_dsp::osc::{MultiWaveOsc, SyncRole};
use forma_dsp::shimmer::{ShimmerReverb, ShimmerShared};

pub const VOICE_COUNT: usize = 6;

/// Tempo-synced 16-step gate sequencer attached to a single modulation source.
///
/// Each lane carries the *universal* gate state: enable flag, 16-bit step mask,
/// active pattern length (1..=16), clock division, and current rate in Hz.
/// Lane-specific parameters (e.g. duck depth on the amp lane) live as sibling
/// fields on `AudioState`, not on `GatePattern`.
///
/// Rate is computed UI-side from global BPM + division (same idiom as
/// `lfo_rate` in synced mode); the audio callback reads `rate` once per buffer
/// and advances a phase accumulator at `rate / sr` per sample.
pub struct GatePattern {
    pub enabled: Arc<AtomicBool>,
    pub pattern: Arc<AtomicU16>,
    pub length: Arc<AtomicU8>,
    pub division: Arc<AtomicU8>,
    pub rate: Shared,
}

impl GatePattern {
    /// Build a disabled lane with an empty pattern at the given default
    /// division. `rate_hz` should match `division.hz(default_bpm)`.
    pub fn new_disabled(division: ClockDivision, rate_hz: f32) -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            pattern: Arc::new(AtomicU16::new(0)),
            length: Arc::new(AtomicU8::new(16)),
            division: Arc::new(AtomicU8::new(division.to_u8())),
            rate: shared(rate_hz),
        }
    }
}

pub struct AudioState {
    // OSC bank — 3 oscillators per voice
    pub osc_wave: [Arc<AtomicU8>; 3], // 0=sine 1=saw 2=square 3=triangle
    pub osc_freq_mult: [Shared; 3],   // octave+detune combined multiplier (1.0 = no change)
    pub osc_vol: [Shared; 3],         // 0.0..1.0 mix level
    pub osc_pulse_width: [Shared; 3], // 0.01..0.99, only used by Square
    // Unison: 5 copies max per OSC slot; inactive copies have vol=0.0
    pub osc_unison_detune: [[Shared; 5]; 3], // freq multiplier per copy (1.0 = no detune)
    pub osc_unison_vol: [[Shared; 5]; 3],    // mix weight per copy (sums to 1.0 when active)
    // Hard sync: OSC 1 → OSC 2. One generation counter per voice.
    // OSC 1 copy 0 increments on phase wrap; OSC 2 copies reset when they see a new generation.
    pub hard_sync_enabled: Arc<AtomicBool>,
    pub hard_sync_gen: Vec<Arc<AtomicU8>>, // one per voice

    // FM: OSC 2 audio output → OSC 1 frequency input (audio-rate FM).
    // fm_tap[vi] is written by OSC 2 copy 0 each sample; fm_depth scales the deviation.
    // deviation (Hz) = fm_tap × fm_depth × voice_freq × osc1_freq_mult
    pub fm_depth: Shared,    // 0.0 = off, ~1.0 = strong FM
    pub fm_tap: Vec<Shared>, // one per voice — written by OSC 2 copy 0

    // Ring modulation: OSC 1 × OSC 2 → added to voice mix.
    // ring_tap[vi] is written by OSC 1 copy 0; ring signal = ring_tap × fm_tap × ring_depth.
    // User mutes OSC 1/2 in mixer for pure ring mod sound.
    pub ring_depth: Shared,    // 0.0 = off
    pub ring_tap: Vec<Shared>, // one per voice — written by OSC 1 copy 0

    // Noise
    pub noise_vol: Shared, // 0.0..1.0

    // Filter
    pub cutoff: Shared,                 // base cutoff Hz (80..18000)
    pub resonance: Shared,              // Q (0.5..20)
    pub filter_drive: Shared,           // 1.0..10.0 — input saturation before moog
    pub filter_key_track: Shared,       // 0.0..1.0 — cutoff follows voice pitch (0=off, 1=full)
    pub mod_wheel_cutoff_add: Shared, // legacy — kept for backward compat; set to 0 by TrackProcessor
    pub mod_wheel: Shared,            // raw 0–1, set from MIDI CC 1 or on-screen strip
    pub mod_wheel_dest: Arc<AtomicU8>, // 0=Off 1=Filter 2=LFO-Depth 3=Amp
    pub mod_wheel_depth: Shared,      // 0–1
    pub aftertouch: Shared,           // channel pressure 0–1
    pub aftertouch_dest: Arc<AtomicU8>, // 0=Off 1=Filter 2=LFO-Depth 3=Amp
    pub aftertouch_depth: Shared,     // 0–1
    // Mod matrix — 4 free-routing slots. Each slot: source signal × depth → destination.
    //   src:  0=Off 1=LFO1 2=LFO2 3=ModWheel 4=Aftertouch
    //   dst:  0=Off 1=Filter 2=Amp 3=Pitch
    //   depth: −1.0..+1.0 (negative = inverse modulation)
    pub mat_src: [Arc<AtomicU8>; 4],
    pub mat_dst: [Arc<AtomicU8>; 4],
    pub mat_depth: [Shared; 4],
    pub filter_env_amount: Shared, // 0.0..1.0
    // Filter ADSR
    pub fenv_attack: Shared,
    pub fenv_decay: Shared,
    pub fenv_sustain: Shared,
    pub fenv_release: Shared,

    // LFO 1
    pub lfo_rate: Shared, // 0.1..20 Hz (free mode) or computed from division (synced)
    pub lfo_depth: Shared, // 0.0..1.0
    pub lfo_shape: Arc<AtomicU8>, // 0=sin 1=tri 2=saw
    pub lfo_dest: Arc<AtomicU8>, // 0=pitch 1=filter 2=amp
    /// 0 = free (Hz), 1 = BPM-synced. When synced the callback overwrites lfo_rate each buffer.
    pub lfo_sync: Arc<AtomicU8>,
    /// ClockDivision::to_u8() — active when lfo_sync == 1.
    pub lfo_division: Arc<AtomicU8>,
    // Written by callback each buffer; read by graph
    pub lfo_pitch_mult: Shared, // frequency multiplier (1.0 = no pitch mod)

    // LFO 2
    pub lfo2_rate: Shared,         // 0.01..20 Hz
    pub lfo2_depth: Shared,        // 0.0..1.0
    pub lfo2_shape: Arc<AtomicU8>, // 0=sin 1=tri 2=saw
    pub lfo2_dest: Arc<AtomicU8>,  // 0=pitch 1=filter 2=amp

    // Gate lanes — tempo-synced 16-step gate sequencers per modulation source.
    //   `gate_aenv` ducks the master output ("Pulse"): each "on" step fires a
    //   fast exponential duck on the post-FX, post-tanh signal, scaled by `gate_aenv_depth`.
    //   `gate_lfo1`/`gate_lfo2` retrigger their LFO's phase to 0 on each "on" step.
    pub gate_aenv: GatePattern,
    pub gate_aenv_depth: Shared, // 0.0..1.0 — ducks master by `depth` at peak (lane-specific)
    pub gate_lfo1: GatePattern,
    pub gate_lfo2: GatePattern,

    // Voice target frequencies — UI writes here; callback smooths to voice_freqs for glide
    pub voice_freq_targets: Vec<Shared>,

    // Amp ADSR
    pub adsr_attack: Shared,
    pub adsr_decay: Shared,
    pub adsr_sustain: Shared,
    pub adsr_release: Shared,

    // ADSR cursors — written by LiveAdsr each sample, read by UI for visualizer
    // Encoding: 0=idle, 1.x=attack, 2.x=decay, 3.0=sustain, 4.x=release (frac=progress)
    pub amp_cursors: Vec<Shared>,  // one per voice
    pub fenv_cursors: Vec<Shared>, // one per voice

    // Glide
    pub glide_time: Shared, // 0.0..0.5 s
    /// 0 = poly, 1 = mono (retrigger), 2 = legato (no retrigger while gate held)
    pub mono_mode: Arc<std::sync::atomic::AtomicU8>,

    // Master
    pub master_vol: Shared, // OSC mix level — pre-FX
    pub global_vol: Shared, // Final output — post all FX and tanh
    // 1/sqrt(active_voices) gain scaling — prevents polyphonic chords from
    // being louder than single notes. Smoothed by callback, read by graph.
    pub voice_gain_scale: Shared,

    // Polyphonic voice pool
    pub voice_freqs: Vec<Shared>,
    pub voice_gates: Vec<Shared>,
    pub voice_velocities: Vec<Shared>, // 0.0..1.0, set on NoteOn
    pub vel_amp: Shared,               // 0=ignore velocity, 1=full sensitivity
    pub vel_filter: Shared,            // 0=off, 1=velocity adds up to 8 kHz to cutoff
    /// Per-voice audibility flag, written by `VoiceAllocator::begin_buffer`.
    /// The DSP graph's `GatedVoice` wrapper skips the voice's sub-graph when
    /// this is `false` — the main CPU win at idle and under light load.
    /// Set to `true` whenever the voice's gate is held OR its amp envelope
    /// is not yet idle OR a retrigger countdown is pending.
    pub voice_audible: Vec<Arc<AtomicBool>>,

    // Internal: effective cutoff written by callback, read by graph
    pub effective_cutoff: Shared,

    // Oscilloscope
    pub osc_buffer: Arc<std::sync::Mutex<Vec<f32>>>,

    // Latency measurement
    // Buffer size in frames, written by audio callback on first call.
    pub buffer_frames: Arc<AtomicU32>,
    // Sample rate in Hz, written once during stream creation.
    pub sample_rate: Arc<AtomicU32>,
    // Timestamp of the last voice_on call — written by UI, cleared by audio callback.
    // Stored as a Mutex<Option<Instant>> so both sides can access without blocking
    // (callback uses try_lock so it never stalls the audio thread).
    pub note_on_time: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    // Last measured round-trip latency in microseconds, written by audio callback.
    pub last_latency_us: Arc<AtomicU32>,

    // Peak metering (pre-clip level, written by audio callback each buffer)
    pub peak_l: Arc<AtomicU32>, // f32 bits stored as u32
    pub peak_r: Arc<AtomicU32>,

    // Arpeggiator and scale walker config (UI-accessible atomics).
    // The matching ArpState / ScaleWalker live in the callback closure.
    pub arp: crate::arp::ArpShared,
    pub walker: crate::arp::ScaleWalkerShared,

    // Master limiter (envelope-follower, runs in callback before tanh)
    pub limiter_enabled: Arc<AtomicBool>,
    pub limiter_threshold: Shared, // 0.5..1.0

    // Set true by the UI thread to request a full FX tail flush (delay + reverb +
    // shimmer + crystallizer buffers zeroed). Checked and cleared by FxChain::tick()
    // so the clear runs on the audio thread without any allocation or locking.
    pub fx_clear_requested: Arc<AtomicBool>,

    // FX chain (post-mix, pre-output) — all wet/dry 0.0 = bypass
    pub fx_overdrive_drive: Shared,  // 1.0..10.0
    pub fx_overdrive_mix: Shared,    // 0.0..1.0
    pub fx_overdrive_tone: Shared,   // 0.0..1.0 — post-clipper LP (0=dark, 1=bright)
    pub fx_overdrive_asym: Shared,   // 0.0..1.0 — asymmetric bias (0=sym, 1=full asym)
    pub fx_distortion_drive: Shared, // 1.0..20.0
    pub fx_distortion_mix: Shared,
    pub fx_distortion_tone: Shared, // 0.0..1.0 — post-clipper LP
    pub fx_distortion_pre: Shared,  // 0.0..1.0 — pre-clipper HP (controls bass going in)
    pub fx_chorus_rate: Shared,     // 0.1..5.0 Hz
    pub fx_chorus_depth: Shared,    // 0.0..0.02 (seconds of modulation)
    pub fx_chorus_mix: Shared,
    pub fx_delay_time: Shared,     // 0.0..1.0 s  (free mode)
    pub fx_delay_feedback: Shared, // 0.0..0.95
    pub fx_delay_mix: Shared,
    /// 0 = free (use fx_delay_time), 1 = BPM-synced (use fx_delay_division).
    pub fx_delay_sync: Arc<AtomicU8>,
    /// ClockDivision::to_u8() — active when fx_delay_sync == 1.
    pub fx_delay_division: Arc<AtomicU8>,
    /// Written by the audio callback each buffer when sync is active;
    /// FxChain reads this instead of fx_delay_time. Units: seconds.
    pub fx_delay_synced_time: Shared,
    pub fx_reverb_size: Shared, // 0.0..1.0 (room size)
    pub fx_reverb_damp: Shared, // 0.0..1.0 (high-freq damping)
    pub fx_reverb_mix: Shared,
    pub fx_reverb_predelay: Shared, // 0.0..0.1 (seconds, 0–100 ms)
    /// 0 = Freeverb, 1 = Plate, 2 = FDN Hall
    pub fx_reverb_type: Arc<AtomicU8>,

    // Stereo widener
    pub stereo_spread: Shared, // 0.0..0.012 seconds (Haas delay on R channel)
    pub stereo_width: Shared,  // 0.0..2.0 (M/S width multiplier, 1.0 = unchanged)

    // Shimmer reverb (extends the standard reverb with pitch-shifted feedback)
    pub fx_shimmer: ShimmerShared,
    // Crystallizer (granular pitch-shift delay)
    pub fx_crystal: CrystallizerShared,

    // Bit crusher / sample-rate reducer
    pub fx_bitcrush_bits: Shared, // 1.0..16.0 bits
    pub fx_bitcrush_rate: Shared, // 1.0..32.0  sample-rate divisor
    pub fx_bitcrush_mix: Shared,

    // Tape saturation
    pub fx_tape_drive: Shared, // 0.0..1.0
    pub fx_tape_tone: Shared,  // 0.0..1.0  (post bandwidth: 0=dark, 1=full)
    pub fx_tape_bias: Shared,  // 0.0..1.0  (even-harmonic content)
    pub fx_tape_mix: Shared,

    // Phaser (8-stage all-pass, stereo-decorrelated LFO)
    pub fx_phaser_rate: Shared,          // 0.05..10.0 Hz
    pub fx_phaser_depth: Shared,         // 0.0..1.0
    pub fx_phaser_feedback: Shared,      // -0.9..0.9
    pub fx_phaser_center: Shared,        // 100..8000 Hz
    pub fx_phaser_stages: Arc<AtomicU8>, // 4 or 8
    pub fx_phaser_mix: Shared,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            osc_wave: [
                Arc::new(AtomicU8::new(1)), // OSC1: saw — needed for filter to have audible effect
                Arc::new(AtomicU8::new(0)),
                Arc::new(AtomicU8::new(0)),
            ],
            osc_freq_mult: [shared(1.0), shared(1.0), shared(1.0)],
            osc_vol: [shared(0.4), shared(0.3), shared(0.0)],
            osc_pulse_width: [shared(0.5), shared(0.5), shared(0.5)],
            // Unison off by default: copy 0 at full weight, copies 1-4 silent
            osc_unison_detune: [
                [
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                ],
                [
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                ],
                [
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                    shared(1.0),
                ],
            ],
            osc_unison_vol: [
                [
                    shared(1.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                ],
                [
                    shared(1.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                ],
                [
                    shared(1.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                    shared(0.0),
                ],
            ],
            noise_vol: shared(0.0),
            cutoff: shared(3000.0),
            resonance: shared(0.3),
            filter_drive: shared(1.0),
            filter_key_track: shared(0.0),
            mod_wheel_cutoff_add: shared(0.0),
            mod_wheel: shared(0.0),
            mod_wheel_dest: Arc::new(AtomicU8::new(1)), // Filter
            mod_wheel_depth: shared(0.5),
            aftertouch: shared(0.0),
            aftertouch_dest: Arc::new(AtomicU8::new(1)), // Filter
            aftertouch_depth: shared(0.3),
            mat_src: std::array::from_fn(|_| Arc::new(AtomicU8::new(0))),
            mat_dst: std::array::from_fn(|_| Arc::new(AtomicU8::new(0))),
            mat_depth: std::array::from_fn(|_| shared(0.0)),
            filter_env_amount: shared(0.3),
            fenv_attack: shared(0.01),
            fenv_decay: shared(0.3),
            fenv_sustain: shared(0.0), // matches UI default fenv_adsr[2]
            fenv_release: shared(0.2),
            lfo_rate: shared(2.0),
            lfo_depth: shared(0.0),
            lfo_shape: Arc::new(AtomicU8::new(0)), // sine
            lfo_dest: Arc::new(AtomicU8::new(1)),  // filter
            lfo_sync: Arc::new(AtomicU8::new(0)),
            lfo_division: Arc::new(AtomicU8::new(ClockDivision::Quarter.to_u8())),
            lfo_pitch_mult: shared(1.0),
            lfo2_rate: shared(0.3),
            lfo2_depth: shared(0.0),
            lfo2_shape: Arc::new(AtomicU8::new(0)),
            lfo2_dest: Arc::new(AtomicU8::new(2)), // amp (tremolo)
            // Gate lanes default to disabled with empty pattern; rate matches 1/8 @ 120 BPM.
            gate_aenv: GatePattern::new_disabled(ClockDivision::Eighth, 4.0),
            gate_aenv_depth: shared(0.0),
            gate_lfo1: GatePattern::new_disabled(ClockDivision::Eighth, 4.0),
            gate_lfo2: GatePattern::new_disabled(ClockDivision::Eighth, 4.0),
            voice_freq_targets: (0..VOICE_COUNT).map(|_| shared(440.0)).collect(),
            adsr_attack: shared(0.01),
            adsr_decay: shared(0.15),
            adsr_sustain: shared(0.7),
            adsr_release: shared(0.4),
            amp_cursors: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            fenv_cursors: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            glide_time: shared(0.0),
            mono_mode: Arc::new(AtomicU8::new(0)),
            master_vol: shared(0.8),
            global_vol: shared(0.8),
            voice_gain_scale: shared(1.0),
            hard_sync_enabled: Arc::new(AtomicBool::new(false)),
            hard_sync_gen: (0..VOICE_COUNT)
                .map(|_| Arc::new(AtomicU8::new(0)))
                .collect(),
            fm_depth: shared(0.0),
            fm_tap: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            ring_depth: shared(0.0),
            ring_tap: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            voice_freqs: (0..VOICE_COUNT).map(|_| shared(440.0)).collect(),
            voice_gates: (0..VOICE_COUNT).map(|_| shared(0.0)).collect(),
            voice_velocities: (0..VOICE_COUNT).map(|_| shared(1.0)).collect(),
            vel_amp: shared(1.0),
            vel_filter: shared(0.0),
            voice_audible: (0..VOICE_COUNT)
                .map(|_| Arc::new(AtomicBool::new(false)))
                .collect(),
            effective_cutoff: shared(3000.0),
            osc_buffer: Arc::new(std::sync::Mutex::new(vec![0.0f32; 1024])),
            buffer_frames: Arc::new(AtomicU32::new(0)),
            sample_rate: Arc::new(AtomicU32::new(0)),
            note_on_time: Arc::new(std::sync::Mutex::new(None)),
            last_latency_us: Arc::new(AtomicU32::new(0)),
            peak_l: Arc::new(AtomicU32::new(0)),
            peak_r: Arc::new(AtomicU32::new(0)),
            arp: crate::arp::ArpShared::new(),
            walker: crate::arp::ScaleWalkerShared::new(),
            limiter_enabled: Arc::new(AtomicBool::new(true)),
            limiter_threshold: shared(0.95),
            fx_clear_requested: Arc::new(AtomicBool::new(false)),
            fx_overdrive_drive: shared(3.0),
            fx_overdrive_mix: shared(0.0),
            fx_overdrive_tone: shared(0.8),
            fx_overdrive_asym: shared(0.0),
            fx_distortion_drive: shared(8.0),
            fx_distortion_mix: shared(0.0),
            fx_distortion_tone: shared(0.8),
            fx_distortion_pre: shared(0.0),
            fx_chorus_rate: shared(0.8),
            fx_chorus_depth: shared(0.008),
            fx_chorus_mix: shared(0.0),
            fx_delay_time: shared(0.35),
            fx_delay_feedback: shared(0.4),
            fx_delay_mix: shared(0.0),
            fx_delay_sync: Arc::new(AtomicU8::new(0)),
            fx_delay_division: Arc::new(AtomicU8::new(
                forma_common::ClockDivision::DottedEighth.to_u8(),
            )),
            fx_delay_synced_time: shared(0.375), // dotted 8th at 120 BPM
            fx_reverb_size: shared(0.6),
            fx_reverb_damp: shared(0.5),
            fx_reverb_mix: shared(0.0),
            fx_reverb_predelay: shared(0.0),
            fx_reverb_type: Arc::new(AtomicU8::new(0)),
            stereo_spread: shared(0.0),
            stereo_width: shared(1.0),
            fx_shimmer: ShimmerShared::new(),
            fx_crystal: CrystallizerShared::new(),
            fx_bitcrush_bits: shared(16.0),
            fx_bitcrush_rate: shared(1.0),
            fx_bitcrush_mix: shared(0.0),
            fx_tape_drive: shared(0.5),
            fx_tape_tone: shared(0.7),
            fx_tape_bias: shared(0.2),
            fx_tape_mix: shared(0.0),
            fx_phaser_rate: shared(0.5),
            fx_phaser_depth: shared(0.7),
            fx_phaser_feedback: shared(0.5),
            fx_phaser_center: shared(1200.0),
            fx_phaser_stages: Arc::new(AtomicU8::new(8)),
            fx_phaser_mix: shared(0.0),
        }
    }
}

impl Default for AudioState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// DSP graph builder
// ---------------------------------------------------------------------------

/// Asymmetric soft-clip node for warm filter drive.
///
/// Formula: `tanh(x * pregain + BIAS) - tanh(BIAS)`
/// - `pregain = drive * 2.0` so saturation is clearly audible at moderate settings.
/// - `BIAS` shifts the curve off-centre → even harmonics (2nd especially) → warmth.
/// - Subtracting `tanh(BIAS)` re-centres output so DC doesn't build up in the filter.
/// Output is bounded to roughly (−1, +1).
#[derive(Clone)]
struct DriveNode {
    drive: Shared,
}

impl AudioNode for DriveNode {
    const ID: u64 = 0x66696c74_72647276; // "filtrdvr"
    type Inputs = U1;
    type Outputs = U1;

    #[inline]
    fn tick(&mut self, input: &Frame<f32, U1>) -> Frame<f32, U1> {
        const BIAS: f32 = 0.3;
        let pregain = self.drive.value().max(1.0) * 2.0;
        let out = (input[0] * pregain + BIAS).tanh() - BIAS.tanh();
        Frame::from([out])
    }
}

/// Build the unified 6-voice poly graph.
/// Each voice: 3 OSCs + noise → lowpass(effective_cutoff) → amp ADSR
pub fn build_synth_graph(state: &AudioState, sr: f64) -> Box<dyn AudioUnit + Send> {
    let make_voice = |vi: usize| {
        let vf = &state.voice_freqs[vi];
        let vg = &state.voice_gates[vi];

        // Each OSC slot: 5 unison copies, inactive ones have vol=0.0.
        // Phases spread evenly across [0, 1) to avoid phase coherence and beating artifacts.
        // Hard sync: OSC 0 copy 0 is master, all OSC 1 copies are slaves.
        let sync_enabled = Arc::clone(&state.hard_sync_enabled);
        let sync_gen = Arc::clone(&state.hard_sync_gen[vi]);

        // LFO pitch modulation: applied at the voice frequency level so all OSCs track together.
        // lfo_pitch_mult is 1.0 when LFO dest != pitch.
        let vf_lfo = var(vf) * var(&state.lfo_pitch_mult);

        // FM: frequency deviation added to OSC 1's input (pitch-tracking).
        let osc0 = {
            let fm = var(&state.fm_tap[vi])
                * var(&state.fm_depth)
                * vf_lfo.clone()
                * var(&state.osc_freq_mult[0]);
            let c0 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[0])
                * var(&state.osc_unison_detune[0][0])
                + fm.clone()
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[0]),
                    state.osc_pulse_width[0].clone(),
                    sr as f32,
                    0.0 / 5.0,
                    SyncRole::Master {
                        sync_enabled: Arc::clone(&sync_enabled),
                        gen: Arc::clone(&sync_gen),
                    },
                    Some(state.ring_tap[vi].clone()),
                )))
                * var(&state.osc_unison_vol[0][0]);
            let c1 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[0])
                * var(&state.osc_unison_detune[0][1])
                + fm.clone()
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[0]),
                    state.osc_pulse_width[0].clone(),
                    sr as f32,
                    1.0 / 5.0,
                    SyncRole::None,
                    None,
                )))
                * var(&state.osc_unison_vol[0][1]);
            let c2 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[0])
                * var(&state.osc_unison_detune[0][2])
                + fm.clone()
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[0]),
                    state.osc_pulse_width[0].clone(),
                    sr as f32,
                    2.0 / 5.0,
                    SyncRole::None,
                    None,
                )))
                * var(&state.osc_unison_vol[0][2]);
            let c3 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[0])
                * var(&state.osc_unison_detune[0][3])
                + fm.clone()
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[0]),
                    state.osc_pulse_width[0].clone(),
                    sr as f32,
                    3.0 / 5.0,
                    SyncRole::None,
                    None,
                )))
                * var(&state.osc_unison_vol[0][3]);
            let c4 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[0])
                * var(&state.osc_unison_detune[0][4])
                + fm
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[0]),
                    state.osc_pulse_width[0].clone(),
                    sr as f32,
                    4.0 / 5.0,
                    SyncRole::None,
                    None,
                )))
                * var(&state.osc_unison_vol[0][4]);
            (c0 + c1 + c2 + c3 + c4) * var(&state.osc_vol[0])
        };
        let osc1 = {
            let c0 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[1])
                * var(&state.osc_unison_detune[1][0])
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[1]),
                    state.osc_pulse_width[1].clone(),
                    sr as f32,
                    0.0 / 5.0,
                    SyncRole::Slave {
                        sync_enabled: Arc::clone(&sync_enabled),
                        gen: Arc::clone(&sync_gen),
                        last_gen: 0,
                    },
                    Some(state.fm_tap[vi].clone()),
                )))
                * var(&state.osc_unison_vol[1][0]);
            let c1 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[1])
                * var(&state.osc_unison_detune[1][1])
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[1]),
                    state.osc_pulse_width[1].clone(),
                    sr as f32,
                    1.0 / 5.0,
                    SyncRole::Slave {
                        sync_enabled: Arc::clone(&sync_enabled),
                        gen: Arc::clone(&sync_gen),
                        last_gen: 0,
                    },
                    None,
                )))
                * var(&state.osc_unison_vol[1][1]);
            let c2 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[1])
                * var(&state.osc_unison_detune[1][2])
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[1]),
                    state.osc_pulse_width[1].clone(),
                    sr as f32,
                    2.0 / 5.0,
                    SyncRole::Slave {
                        sync_enabled: Arc::clone(&sync_enabled),
                        gen: Arc::clone(&sync_gen),
                        last_gen: 0,
                    },
                    None,
                )))
                * var(&state.osc_unison_vol[1][2]);
            let c3 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[1])
                * var(&state.osc_unison_detune[1][3])
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[1]),
                    state.osc_pulse_width[1].clone(),
                    sr as f32,
                    3.0 / 5.0,
                    SyncRole::Slave {
                        sync_enabled: Arc::clone(&sync_enabled),
                        gen: Arc::clone(&sync_gen),
                        last_gen: 0,
                    },
                    None,
                )))
                * var(&state.osc_unison_vol[1][3]);
            let c4 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[1])
                * var(&state.osc_unison_detune[1][4])
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[1]),
                    state.osc_pulse_width[1].clone(),
                    sr as f32,
                    4.0 / 5.0,
                    SyncRole::Slave {
                        sync_enabled: Arc::clone(&sync_enabled),
                        gen: Arc::clone(&sync_gen),
                        last_gen: 0,
                    },
                    None,
                )))
                * var(&state.osc_unison_vol[1][4]);
            (c0 + c1 + c2 + c3 + c4) * var(&state.osc_vol[1])
        };
        let osc2 = {
            let c0 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[2])
                * var(&state.osc_unison_detune[2][0])
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[2]),
                    state.osc_pulse_width[2].clone(),
                    sr as f32,
                    0.0 / 5.0,
                    SyncRole::None,
                    None,
                )))
                * var(&state.osc_unison_vol[2][0]);
            let c1 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[2])
                * var(&state.osc_unison_detune[2][1])
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[2]),
                    state.osc_pulse_width[2].clone(),
                    sr as f32,
                    1.0 / 5.0,
                    SyncRole::None,
                    None,
                )))
                * var(&state.osc_unison_vol[2][1]);
            let c2 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[2])
                * var(&state.osc_unison_detune[2][2])
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[2]),
                    state.osc_pulse_width[2].clone(),
                    sr as f32,
                    2.0 / 5.0,
                    SyncRole::None,
                    None,
                )))
                * var(&state.osc_unison_vol[2][2]);
            let c3 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[2])
                * var(&state.osc_unison_detune[2][3])
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[2]),
                    state.osc_pulse_width[2].clone(),
                    sr as f32,
                    3.0 / 5.0,
                    SyncRole::None,
                    None,
                )))
                * var(&state.osc_unison_vol[2][3]);
            let c4 = (vf_lfo.clone()
                * var(&state.osc_freq_mult[2])
                * var(&state.osc_unison_detune[2][4])
                >> An(MultiWaveOsc::with_sync(
                    Arc::clone(&state.osc_wave[2]),
                    state.osc_pulse_width[2].clone(),
                    sr as f32,
                    4.0 / 5.0,
                    SyncRole::None,
                    None,
                )))
                * var(&state.osc_unison_vol[2][4]);
            (c0 + c1 + c2 + c3 + c4) * var(&state.osc_vol[2])
        };

        // Ring mod: OSC1 × OSC2 added to the mix.
        let ring = var(&state.ring_tap[vi]) * var(&state.fm_tap[vi]) * var(&state.ring_depth);
        let noise = noise() * var(&state.noise_vol);
        let osc = osc0 + osc1 + osc2 + ring + noise;

        // Input saturation: soft-clip before the filter.
        // drive=1 is transparent (tanh(x/1)*1 ≈ x for small x); higher values
        // push the signal into the tanh knee for analog-style warmth.
        let driven = osc
            >> An(DriveNode {
                drive: state.filter_drive.clone(),
            });

        // Moog lowpass filter with per-voice filter ADSR (fully live-parametric).
        let fenv = var(vg)
            >> An(LiveAdsr::new(
                state.fenv_attack.clone(),
                state.fenv_decay.clone(),
                state.fenv_sustain.clone(),
                state.fenv_release.clone(),
                Some(state.fenv_cursors[vi].clone()),
                sr as f32,
            ));
        // Filter env sweep: additive in Hz with a fixed max range so the sweep covers
        // musically useful territory regardless of base cutoff.
        // env_amount=1.0 adds up to 12 kHz above base (≈2–3 octaves); at 0.3 it adds ~3.6 kHz.
        // Velocity → filter: adds up to 8 kHz at vel_filter=1.0 and full velocity.
        let dyn_cutoff = var(&state.effective_cutoff)
            + fenv * var(&state.filter_env_amount) * dc(12000.0_f32)
            + var(&state.voice_velocities[vi]) * var(&state.vel_filter) * dc(8000.0_f32);
        let filtered = (driven | dyn_cutoff | var(&state.resonance)) >> moog();

        // Amp ADSR envelope (fully live-parametric).
        let env = var(vg)
            >> An(LiveAdsr::new(
                state.adsr_attack.clone(),
                state.adsr_decay.clone(),
                state.adsr_sustain.clone(),
                state.adsr_release.clone(),
                Some(state.amp_cursors[vi].clone()),
                sr as f32,
            ));
        // Velocity → amplitude: lerp(1.0, velocity, vel_amp).
        // vel_amp=0 → always full volume; vel_amp=1 → velocity directly scales output.
        let vel_scale =
            dc(1.0) - var(&state.vel_amp) + var(&state.vel_amp) * var(&state.voice_velocities[vi]);
        filtered * env * vel_scale
    };

    // Wrap each voice in a `GatedVoice` so silent voices short-circuit their
    // sub-graph (3×5 oscillators + Moog filter + 2 ADSRs). The audibility
    // flag is updated once per audio buffer by `VoiceAllocator`.
    let gate = |vi: usize, voice| An(GatedVoice::new(voice, Arc::clone(&state.voice_audible[vi])));
    let v0 = gate(0, make_voice(0).0);
    let v1 = gate(1, make_voice(1).0);
    let v2 = gate(2, make_voice(2).0);
    let v3 = gate(3, make_voice(3).0);
    let v4 = gate(4, make_voice(4).0);
    let v5 = gate(5, make_voice(5).0);

    let voice_mix = v0 + v1 + v2 + v3 + v4 + v5;

    let chain = (voice_mix * var(&state.master_vol) * var(&state.voice_gain_scale))
        >> An(FxChain::new(state, sr as f32));

    let mut g: Box<dyn AudioUnit + Send> = Box::new(chain);
    g.set_sample_rate(sr);
    g.allocate();
    g
}

// ---------------------------------------------------------------------------
// FX chain — custom AudioNode (tick-based, plain f32)
// ---------------------------------------------------------------------------

/// One-pole exponential smoother wrapping a `Shared` parameter.
/// Prevents audio artifacts (clicks/pops) when a parameter is changed live.
/// `tau_s` is the smoothing time constant in seconds (63% convergence time).
#[derive(Clone)]
struct SmoothedParam {
    shared: Shared,
    current: f32,
    coeff: f32, // recomputed on sample-rate change
    tau_s: f32,
}

impl SmoothedParam {
    fn new(shared: Shared, tau_s: f32, sr: f32) -> Self {
        let current = shared.value();
        let coeff = (-1.0_f32 / (tau_s * sr)).exp();
        Self {
            shared,
            current,
            coeff,
            tau_s,
        }
    }

    fn set_sample_rate(&mut self, sr: f32) {
        self.coeff = (-1.0_f32 / (self.tau_s * sr)).exp();
    }

    fn reset(&mut self) {
        self.current = self.shared.value();
    }

    #[inline]
    fn next(&mut self) -> f32 {
        let target = self.shared.value();
        self.current = target + self.coeff * (self.current - target);
        self.current
    }
}

/// All effects in a single sample-accurate node.
/// Input is mono, output is stereo with decorrelated reverb/shimmer tails.
#[derive(Clone)]
struct FxChain {
    // Plain Shared — no smoothing needed (tone/asym affect filter coefficients gradually)
    od_tone: Shared,
    od_asym: Shared,
    dist_tone: Shared,
    dist_pre: Shared,
    // Smoothed — prevents zipper noise when moving sliders live
    od_drive: SmoothedParam,
    dist_drive: SmoothedParam,
    cho_rate: Shared,
    cho_depth: Shared,
    del_feedback: Shared,
    del_sync: Arc<AtomicU8>, // 0=free, 1=BPM-synced
    #[allow(dead_code)] // value is forwarded via del_synced_time_smooth
    del_synced_time: Shared, // written by callback; seconds
    // Smoothed params — use SmoothedParam to prevent clicks/artifacts
    od_mix: SmoothedParam,                 // 5 ms — pop-free toggle
    dist_mix: SmoothedParam,               // 5 ms
    cho_mix: SmoothedParam,                // 5 ms
    del_time: SmoothedParam, // 20 ms — prevents pitch-jump noise on slider move (free mode)
    del_synced_time_smooth: SmoothedParam, // 20 ms — synced mode
    del_mix: SmoothedParam,  // 5 ms
    // Reverb params (plain reverb, shimmer_amt always 0)
    rev_size: SmoothedParam, // 50 ms
    rev_damp: SmoothedParam, // 50 ms
    rev_mix: SmoothedParam,  // 5 ms
    rev_predelay: Shared,
    rev_pre_buf: Vec<f32>,
    rev_pre_pos: usize,
    rev_type: std::sync::Arc<std::sync::atomic::AtomicU8>,
    // Stereo widener
    stereo_spread: SmoothedParam,
    stereo_width: SmoothedParam,
    haas_buf: Vec<f32>, // ring buffer for Haas delay on R (max 12 ms)
    haas_pos: usize,
    // Shimmer params (independent instance — own size/damp/shimmer/pitch/mix)
    shim_size: SmoothedParam,
    shim_damp: SmoothedParam,
    shim_mix: SmoothedParam,
    shim_amt: SmoothedParam,
    shim_width: SmoothedParam,
    shim_spread: SmoothedParam,
    shim_pitch: std::sync::Arc<std::sync::atomic::AtomicU8>,
    // Crystallizer params (granular pitch-shift delay)
    crys_grain: SmoothedParam,
    crys_scatter: SmoothedParam,
    crys_feedback: SmoothedParam,
    crys_delay: SmoothedParam,
    crys_mix: SmoothedParam,
    crys_pitch: std::sync::Arc<std::sync::atomic::AtomicU8>,
    // Bit crusher
    bc_bits: SmoothedParam,
    bc_rate: Shared,
    bc_mix: SmoothedParam,
    bc_hold: f32,    // last held sample (decimation)
    bc_counter: f32, // counts samples until next hold update
    // Tape saturation
    tape_drive: SmoothedParam,
    tape_tone: Shared,
    tape_bias: Shared,
    tape_mix: SmoothedParam,
    tape_lp_z: f32, // post-sat LP filter state
    tape_hp_z: f32, // pre-sat HP filter state (pre-emphasis)
    tape_hb_z: f32, // head-bump LP state (low-shelf warmth)
    // Phaser
    ph_rate: Shared,
    ph_depth: Shared,
    ph_feedback: Shared,
    ph_center: Shared,
    ph_stages: std::sync::Arc<std::sync::atomic::AtomicU8>,
    ph_mix: SmoothedParam,
    ph_phase: f32,          // LFO phase 0..1
    ph_ap_l: [[f32; 2]; 8], // [stage][v_prev, unused] — per-stage state L
    ph_ap_r: [[f32; 2]; 8], // same for R (90° offset LFO)
    ph_fb_l: f32,           // feedback memory L
    ph_fb_r: f32,           // feedback memory R
    // Internal state
    cho_phase: f32,
    del_buf: Vec<f32>,
    del_pos: usize,
    od_tone_z: f32,
    dist_tone_z: f32,
    dist_pre_z: f32,
    rev_l: ShimmerReverb,
    rev_r: ShimmerReverb,
    shim_l: ShimmerReverb,
    shim_r: ShimmerReverb,
    crys_l: Crystallizer,
    crys_r: Crystallizer,
    sr: f32,
    clear_requested: Arc<AtomicBool>,
}

impl FxChain {
    fn new(state: &AudioState, sr: f32) -> Self {
        const MIX_TAU: f32 = 0.005; // 5 ms — pop-free mix/toggle transitions
        const DEL_TAU: f32 = 0.020; // 20 ms — delay time, prevents pitch-jump noise
        const REV_TAU: f32 = 0.050; // 50 ms — reverb room/damp, smooth tail changes
        let buf_len = (sr * 1.1) as usize;
        Self {
            od_tone: state.fx_overdrive_tone.clone(),
            od_asym: state.fx_overdrive_asym.clone(),
            dist_tone: state.fx_distortion_tone.clone(),
            dist_pre: state.fx_distortion_pre.clone(),
            od_drive: SmoothedParam::new(state.fx_overdrive_drive.clone(), DEL_TAU, sr),
            dist_drive: SmoothedParam::new(state.fx_distortion_drive.clone(), DEL_TAU, sr),
            cho_rate: state.fx_chorus_rate.clone(),
            cho_depth: state.fx_chorus_depth.clone(),
            del_feedback: state.fx_delay_feedback.clone(),
            del_sync: Arc::clone(&state.fx_delay_sync),
            del_synced_time: state.fx_delay_synced_time.clone(),
            od_mix: SmoothedParam::new(state.fx_overdrive_mix.clone(), MIX_TAU, sr),
            dist_mix: SmoothedParam::new(state.fx_distortion_mix.clone(), MIX_TAU, sr),
            cho_mix: SmoothedParam::new(state.fx_chorus_mix.clone(), MIX_TAU, sr),
            del_time: SmoothedParam::new(state.fx_delay_time.clone(), DEL_TAU, sr),
            del_synced_time_smooth: SmoothedParam::new(
                state.fx_delay_synced_time.clone(),
                DEL_TAU,
                sr,
            ),
            del_mix: SmoothedParam::new(state.fx_delay_mix.clone(), MIX_TAU, sr),
            rev_size: SmoothedParam::new(state.fx_reverb_size.clone(), REV_TAU, sr),
            rev_damp: SmoothedParam::new(state.fx_reverb_damp.clone(), REV_TAU, sr),
            rev_mix: SmoothedParam::new(state.fx_reverb_mix.clone(), MIX_TAU, sr),
            rev_predelay: state.fx_reverb_predelay.clone(),
            rev_type: std::sync::Arc::clone(&state.fx_reverb_type),
            rev_pre_buf: vec![0.0_f32; (sr * 0.105) as usize], // 105 ms max
            rev_pre_pos: 0,
            stereo_spread: SmoothedParam::new(state.stereo_spread.clone(), MIX_TAU, sr),
            stereo_width: SmoothedParam::new(state.stereo_width.clone(), MIX_TAU, sr),
            haas_buf: vec![0.0_f32; (sr * 0.015) as usize], // 15 ms max
            haas_pos: 0,
            shim_size: SmoothedParam::new(state.fx_shimmer.size.clone(), REV_TAU, sr),
            shim_damp: SmoothedParam::new(state.fx_shimmer.damp.clone(), REV_TAU, sr),
            shim_mix: SmoothedParam::new(state.fx_shimmer.mix.clone(), MIX_TAU, sr),
            shim_amt: SmoothedParam::new(state.fx_shimmer.shimmer.clone(), REV_TAU, sr),
            shim_width: SmoothedParam::new(state.fx_shimmer.width.clone(), REV_TAU, sr),
            shim_spread: SmoothedParam::new(state.fx_shimmer.spread.clone(), REV_TAU, sr),
            shim_pitch: std::sync::Arc::clone(&state.fx_shimmer.pitch),
            crys_grain: SmoothedParam::new(state.fx_crystal.grain_ms.clone(), REV_TAU, sr),
            crys_scatter: SmoothedParam::new(state.fx_crystal.scatter.clone(), REV_TAU, sr),
            crys_feedback: SmoothedParam::new(state.fx_crystal.feedback.clone(), REV_TAU, sr),
            crys_delay: SmoothedParam::new(state.fx_crystal.delay_ms.clone(), REV_TAU, sr),
            crys_mix: SmoothedParam::new(state.fx_crystal.mix.clone(), MIX_TAU, sr),
            crys_pitch: std::sync::Arc::clone(&state.fx_crystal.pitch),
            bc_bits: SmoothedParam::new(state.fx_bitcrush_bits.clone(), MIX_TAU, sr),
            bc_rate: state.fx_bitcrush_rate.clone(),
            bc_mix: SmoothedParam::new(state.fx_bitcrush_mix.clone(), MIX_TAU, sr),
            bc_hold: 0.0,
            bc_counter: 0.0,
            tape_drive: SmoothedParam::new(state.fx_tape_drive.clone(), MIX_TAU, sr),
            tape_tone: state.fx_tape_tone.clone(),
            tape_bias: state.fx_tape_bias.clone(),
            tape_mix: SmoothedParam::new(state.fx_tape_mix.clone(), MIX_TAU, sr),
            tape_lp_z: 0.0,
            tape_hp_z: 0.0,
            tape_hb_z: 0.0,
            ph_rate: state.fx_phaser_rate.clone(),
            ph_depth: state.fx_phaser_depth.clone(),
            ph_feedback: state.fx_phaser_feedback.clone(),
            ph_center: state.fx_phaser_center.clone(),
            ph_stages: std::sync::Arc::clone(&state.fx_phaser_stages),
            ph_mix: SmoothedParam::new(state.fx_phaser_mix.clone(), MIX_TAU, sr),
            ph_phase: 0.0,
            ph_ap_l: [[0.0; 2]; 8],
            ph_ap_r: [[0.0; 2]; 8],
            ph_fb_l: 0.0,
            ph_fb_r: 0.0,
            cho_phase: 0.0,
            del_buf: vec![0.0f32; buf_len],
            del_pos: 0,
            od_tone_z: 0.0,
            dist_tone_z: 0.0,
            dist_pre_z: 0.0,
            rev_l: ShimmerReverb::new(sr),
            rev_r: ShimmerReverb::new(sr),
            shim_l: ShimmerReverb::new(sr),
            shim_r: ShimmerReverb::new(sr),
            crys_l: Crystallizer::new(sr),
            crys_r: Crystallizer::new(sr),
            sr,
            clear_requested: Arc::clone(&state.fx_clear_requested),
        }
    }
}

impl AudioNode for FxChain {
    const ID: u64 = 0x7468655F_78636861; // "the_xcha"
    type Inputs = U1;
    type Outputs = U2;

    fn reset(&mut self) {
        self.cho_phase = 0.0;
        self.del_buf.fill(0.0);
        self.del_pos = 0;
        self.rev_pre_buf.fill(0.0);
        self.rev_pre_pos = 0;
        self.haas_buf.fill(0.0);
        self.haas_pos = 0;
        self.od_drive.reset();
        self.od_mix.reset();
        self.dist_drive.reset();
        self.dist_mix.reset();
        self.cho_mix.reset();
        self.del_time.reset();
        self.del_synced_time_smooth.reset();
        self.del_mix.reset();
        self.rev_size.reset();
        self.rev_damp.reset();
        self.rev_mix.reset();
        self.shim_size.reset();
        self.shim_damp.reset();
        self.shim_mix.reset();
        self.shim_amt.reset();
        self.shim_width.reset();
        self.shim_spread.reset();
        self.crys_grain.reset();
        self.crys_scatter.reset();
        self.crys_feedback.reset();
        self.crys_delay.reset();
        self.crys_mix.reset();
        self.bc_bits.reset();
        self.bc_mix.reset();
        self.bc_hold = 0.0;
        self.bc_counter = 0.0;
        self.tape_drive.reset();
        self.tape_mix.reset();
        self.tape_lp_z = 0.0;
        self.tape_hp_z = 0.0;
        self.tape_hb_z = 0.0;
        self.ph_mix.reset();
        self.ph_phase = 0.0;
        self.ph_ap_l = [[0.0; 2]; 8];
        self.ph_ap_r = [[0.0; 2]; 8];
        self.ph_fb_l = 0.0;
        self.ph_fb_r = 0.0;
        self.rev_l.reset();
        self.rev_r.reset();
        self.shim_l.reset();
        self.shim_r.reset();
        self.crys_l.reset();
        self.crys_r.reset();
    }

    fn set_sample_rate(&mut self, sr: f64) {
        self.sr = sr as f32;
        let buf_len = (self.sr * 1.1) as usize;
        self.del_buf = vec![0.0f32; buf_len];
        self.del_pos = 0;
        self.od_drive.set_sample_rate(self.sr);
        self.od_mix.set_sample_rate(self.sr);
        self.dist_drive.set_sample_rate(self.sr);
        self.dist_mix.set_sample_rate(self.sr);
        self.cho_mix.set_sample_rate(self.sr);
        self.del_time.set_sample_rate(self.sr);
        self.del_synced_time_smooth.set_sample_rate(self.sr);
        self.del_mix.set_sample_rate(self.sr);
        self.rev_size.set_sample_rate(self.sr);
        self.rev_damp.set_sample_rate(self.sr);
        self.rev_mix.set_sample_rate(self.sr);
        self.shim_size.set_sample_rate(self.sr);
        self.shim_damp.set_sample_rate(self.sr);
        self.shim_mix.set_sample_rate(self.sr);
        self.shim_amt.set_sample_rate(self.sr);
        self.shim_width.set_sample_rate(self.sr);
        self.shim_spread.set_sample_rate(self.sr);
        self.crys_grain.set_sample_rate(self.sr);
        self.crys_scatter.set_sample_rate(self.sr);
        self.crys_feedback.set_sample_rate(self.sr);
        self.crys_delay.set_sample_rate(self.sr);
        self.crys_mix.set_sample_rate(self.sr);
        self.bc_bits.set_sample_rate(self.sr);
        self.bc_mix.set_sample_rate(self.sr);
        self.tape_drive.set_sample_rate(self.sr);
        self.tape_mix.set_sample_rate(self.sr);
        self.ph_mix.set_sample_rate(self.sr);
        self.rev_l.set_sample_rate(self.sr);
        self.rev_r.set_sample_rate(self.sr);
        self.shim_l.set_sample_rate(self.sr);
        self.shim_r.set_sample_rate(self.sr);
        self.crys_l.set_sample_rate(self.sr);
        self.crys_r.set_sample_rate(self.sr);
    }

    #[inline]
    fn tick(&mut self, input: &Frame<f32, U1>) -> Frame<f32, U2> {
        // Flush all FX tails when requested (e.g. on patch load or effect toggle).
        // swap(false) clears the flag and runs the clear only once, on the audio thread.
        if self.clear_requested.swap(false, Ordering::Relaxed) {
            self.del_buf.fill(0.0);
            self.del_pos = 0;
            self.rev_pre_buf.fill(0.0);
            self.rev_pre_pos = 0;
            self.haas_buf.fill(0.0);
            self.haas_pos = 0;
            self.rev_l.reset();
            self.rev_r.reset();
            self.shim_l.reset();
            self.shim_r.reset();
            self.crys_l.reset();
            self.crys_r.reset();
        }

        let dry = input[0];

        // ── Bit crusher / sample-rate reducer ──────────────────────────────
        let bc_mix = self.bc_mix.next();
        let bc_wet = if bc_mix > 0.0001 {
            let bits = self.bc_bits.next().clamp(1.0, 16.0);
            let rate_div = self.bc_rate.value().clamp(1.0, 32.0);
            // Sample-rate decimation: hold the sample for rate_div samples.
            self.bc_counter += 1.0;
            if self.bc_counter >= rate_div {
                self.bc_counter = 0.0;
                // Bit-depth quantization.
                let steps = (2.0_f32.powf(bits - 1.0)).max(1.0);
                self.bc_hold = (dry * steps).round() / steps;
            }
            self.bc_hold
        } else {
            dry
        };
        let bc_theta = bc_mix * std::f32::consts::FRAC_PI_2;
        let s0 = bc_theta.cos() * dry + bc_theta.sin() * bc_wet;

        // ── Overdrive (tanh soft clip) ──────────────────────────────────────
        let od_drive = self.od_drive.next().max(1.0);
        let od_mix = self.od_mix.next();
        let od_tone = self.od_tone.value();
        let od_asym = self.od_asym.value();
        let od_wet = if od_mix > 0.0001 {
            // Asymmetric bias: scaled to match the driven signal level so it
            // actually shifts the clipping point. bias up to ±2.0 in tanh space.
            let driven_signal = s0 * od_drive * 5.0;
            let bias = od_asym * 2.0;
            let clipped = (driven_signal + bias).tanh() - bias.tanh();
            // Post-clipper tone LP: 0 = 400 Hz (dark/muffled), 1 = 8 kHz (bright).
            // Narrower range than before so movement is always audible.
            let fc = 400.0_f32 * (8000.0_f32 / 400.0).powf(od_tone);
            let lp_coeff = (-std::f32::consts::TAU * fc / self.sr).exp();
            self.od_tone_z = (1.0 - lp_coeff) * clipped + lp_coeff * self.od_tone_z;
            self.od_tone_z
        } else {
            dry
        };
        // Equal-power crossfade: cos(θ)·dry + sin(θ)·wet where θ = mix·π/2
        let od_theta = od_mix * std::f32::consts::FRAC_PI_2;
        let s1 = od_theta.cos() * s0 + od_theta.sin() * od_wet;

        // ── Distortion (hard clip) ──────────────────────────────────────────
        let dist_drive = self.dist_drive.next().max(1.0);
        let dist_mix = self.dist_mix.next();
        let dist_tone = self.dist_tone.value();
        let dist_pre = self.dist_pre.value();
        let dist_wet = if dist_mix > 0.0001 {
            // Pre-clipper HP: removes bass before clipping to avoid mud.
            // Maps 0→1 to 20 Hz → 800 Hz. HP = input - LP(input).
            let hp_fc = 20.0_f32 + dist_pre * 780.0;
            let hp_coeff = (-std::f32::consts::TAU * hp_fc / self.sr).exp();
            self.dist_pre_z = (1.0 - hp_coeff) * s1 + hp_coeff * self.dist_pre_z;
            let hp_out = s1 - self.dist_pre_z;
            // Hard clip
            let clipped = (hp_out * dist_drive * 10.0).clamp(-1.0, 1.0);
            // Post-clipper tone LP: rolls off harsh high harmonics
            let fc = 400.0_f32 * (18000.0_f32 / 400.0).powf(dist_tone);
            let lp_coeff = (-std::f32::consts::TAU * fc / self.sr).exp();
            self.dist_tone_z = (1.0 - lp_coeff) * clipped + lp_coeff * self.dist_tone_z;
            self.dist_tone_z
        } else {
            s1
        };
        let dist_theta = dist_mix * std::f32::consts::FRAC_PI_2;
        let s2 = dist_theta.cos() * s1 + dist_theta.sin() * dist_wet;

        // ── Tape saturation ─────────────────────────────────────────────────
        // Algorithm: pre-emphasis HP → tanh saturation + even harmonic bias →
        // de-emphasis LP (bandwidth limiting) → head-bump low shelf warmth.
        // Inspired by Airwindows ToTape; simplified for real-time per-sample use.
        let tape_mix = self.tape_mix.next();
        let tape_wet = if tape_mix > 0.0001 {
            let drive = self.tape_drive.next().clamp(0.0, 1.0);
            let tone = self.tape_tone.value().clamp(0.0, 1.0);
            let bias = self.tape_bias.value().clamp(0.0, 1.0);

            // Pre-emphasis: gentle HP at 120 Hz (tape sees sharp transients).
            let hp_coeff = (-std::f32::consts::TAU * 120.0 / self.sr).exp();
            self.tape_hp_z = (1.0 - hp_coeff) * s2 + hp_coeff * self.tape_hp_z;
            let pre_emphasized = s2 + drive * 0.4 * (s2 - self.tape_hp_z);

            // Saturation: normalized tanh + even harmonic (asymmetric bias).
            let d = 1.0 + drive * 3.5;
            let d_tanh = d.tanh();
            let biased = pre_emphasized + bias * 0.12 * pre_emphasized * pre_emphasized.abs();
            let saturated = (biased * d).tanh() / d_tanh;

            // De-emphasis LP: simulates tape bandwidth limiting.
            // tone=1 → 22 kHz (transparent), tone=0 → 2 kHz (vintage tape warmth).
            let lp_fc = 2000.0_f32 * (22000.0_f32 / 2000.0).powf(tone);
            let lp_coeff = (-std::f32::consts::TAU * lp_fc / self.sr).exp();
            self.tape_lp_z = (1.0 - lp_coeff) * saturated + lp_coeff * self.tape_lp_z;
            let de_emphasized = self.tape_lp_z;

            // Head bump: adds low shelf warmth around 80 Hz (the signature tape
            // resonance from the head gap and reproduce electronics).
            let hb_coeff = (-std::f32::consts::TAU * 80.0 / self.sr).exp();
            self.tape_hb_z = (1.0 - hb_coeff) * de_emphasized + hb_coeff * self.tape_hb_z;
            de_emphasized + drive * 0.25 * self.tape_hb_z
        } else {
            s2
        };
        let tape_theta = tape_mix * std::f32::consts::FRAC_PI_2;
        let s3 = tape_theta.cos() * s2 + tape_theta.sin() * tape_wet;

        // ── Chorus (LFO-modulated short delay) ─────────────────────────────
        let cho_mix = self.cho_mix.next();
        let buf_len = self.del_buf.len();
        self.del_buf[self.del_pos] = s3;

        let cho_wet = if cho_mix > 0.0001 {
            let rate = self.cho_rate.value();
            let depth = self.cho_depth.value();
            self.cho_phase = (self.cho_phase + rate / self.sr).fract();
            let lfo = (self.cho_phase * std::f32::consts::TAU).sin();
            let delay_smp = ((0.01 + depth * lfo) * self.sr).max(0.0);
            let read = (self.del_pos as f32 - delay_smp).rem_euclid(buf_len as f32);
            let i0 = read as usize % buf_len;
            let i1 = (i0 + 1) % buf_len;
            self.del_buf[i0] * (1.0 - read.fract()) + self.del_buf[i1] * read.fract()
        } else {
            s3
        };
        let cho_theta = cho_mix * std::f32::consts::FRAC_PI_2;
        let s4 = cho_theta.cos() * s3 + cho_theta.sin() * cho_wet;

        // ── Phaser (8-stage all-pass, stereo-decorrelated LFO) ─────────────
        // Two independent all-pass chains (L=0°, R=90°) give a lush stereo
        // spread. Their mid sum feeds the delay/reverb chain; their side
        // difference is injected into the final stereo output.
        let ph_mix = self.ph_mix.next();
        let (ph_wet_l, ph_wet_r) = if ph_mix > 0.0001 {
            let rate = self.ph_rate.value();
            let depth = self.ph_depth.value().clamp(0.0, 0.95);
            let feedback = self.ph_feedback.value().clamp(-0.95, 0.95);
            let center = self.ph_center.value().clamp(60.0, 8000.0);
            let n_stages = self.ph_stages.load(std::sync::atomic::Ordering::Relaxed) as usize;
            let n = n_stages.clamp(2, 8);

            self.ph_phase = (self.ph_phase + rate / self.sr).fract();
            let lfo_l = (self.ph_phase * std::f32::consts::TAU).sin();
            // R channel: 90° offset for stereo width.
            let lfo_r = ((self.ph_phase + 0.25) * std::f32::consts::TAU).sin();

            // Helper: first-order all-pass coefficient for a given frequency.
            // Uses the Schroeder form: v[n] = x - a*v[n-1], y = a*v + v[n-1].
            let ap_coeff = |fc: f32| -> f32 {
                let w = (std::f32::consts::PI * fc / self.sr).tan();
                (w - 1.0) / (w + 1.0)
            };

            let fc_l = (center * (1.0 + depth * lfo_l)).clamp(60.0, self.sr * 0.49);
            let fc_r = (center * (1.0 + depth * lfo_r)).clamp(60.0, self.sr * 0.49);
            let a_l = ap_coeff(fc_l);
            let a_r = ap_coeff(fc_r);

            // Run L chain with feedback.
            let mut sig_l = s4 + self.ph_fb_l * feedback;
            for i in 0..n {
                let v_prev = self.ph_ap_l[i][0];
                let v = sig_l - a_l * v_prev;
                let y = a_l * v + v_prev;
                self.ph_ap_l[i][0] = v;
                sig_l = y;
            }
            self.ph_fb_l = sig_l;

            // Run R chain with feedback.
            let mut sig_r = s4 + self.ph_fb_r * feedback;
            for i in 0..n {
                let v_prev = self.ph_ap_r[i][0];
                let v = sig_r - a_r * v_prev;
                let y = a_r * v + v_prev;
                self.ph_ap_r[i][0] = v;
                sig_r = y;
            }
            self.ph_fb_r = sig_r;

            (sig_l, sig_r)
        } else {
            (s4, s4)
        };
        // Mid sum feeds the rest of the mono chain (delay, reverb process it).
        let ph_mid = (ph_wet_l + ph_wet_r) * 0.5;
        let ph_theta = ph_mix * std::f32::consts::FRAC_PI_2;
        let s5 = ph_theta.cos() * s4 + ph_theta.sin() * ph_mid;

        // ── Delay ──────────────────────────────────────────────────────────
        let del_mix = self.del_mix.next();
        // Advance both smoothers every sample so they track correctly when mode switches.
        let del_time_free = self.del_time.next();
        let del_time_synced = self.del_synced_time_smooth.next();
        let del_time = if self.del_sync.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            del_time_synced
        } else {
            del_time_free
        };
        let del_feedback = self.del_feedback.value();
        let del_feedback = del_feedback.clamp(0.0, 0.95);
        let del_wet = if del_mix > 0.0001 {
            let delay_smp = (del_time * self.sr).clamp(1.0, (buf_len - 2) as f32);
            let read_f = (self.del_pos as f32 - delay_smp).rem_euclid(buf_len as f32);
            let i0 = read_f as usize % buf_len;
            let i1 = (i0 + 1) % buf_len;
            let delayed =
                self.del_buf[i0] * (1.0 - read_f.fract()) + self.del_buf[i1] * read_f.fract();
            self.del_buf[self.del_pos] = (s5 + delayed * del_feedback).tanh();
            delayed
        } else {
            0.0
        };
        let s6_raw = s5 + del_mix * del_wet;
        let s6 = if s6_raw.is_finite() { s6_raw } else { 0.0 };

        self.del_pos = (self.del_pos + 1) % buf_len;

        // Stereo field controls shared by reverb + shimmer wet tails.
        let shim_width = self.shim_width.next().clamp(0.5, 2.0);
        let shim_spread = self.shim_spread.next().clamp(0.0, 0.3);
        let size_spread = 0.5 * shim_spread;
        let damp_spread = 0.5 * shim_spread;

        // ── Reverb pre-delay ────────────────────────────────────────────────
        let pre_secs = self.rev_predelay.value().clamp(0.0, 0.1);
        let pre_len = self.rev_pre_buf.len();
        let pre_samples = std::cmp::min((pre_secs * self.sr) as usize, pre_len.saturating_sub(1));
        self.rev_pre_buf[self.rev_pre_pos] = s6;
        let read_pos = (self.rev_pre_pos + pre_len - pre_samples) % pre_len;
        let s6_predelayed = self.rev_pre_buf[read_pos];
        self.rev_pre_pos = (self.rev_pre_pos + 1) % pre_len;

        // ── Reverb (stereo-decorrelated) ────────────────────────────────────
        let rev_mix = self.rev_mix.next();
        let rev_size = self.rev_size.next();
        let rev_damp = self.rev_damp.next();
        let rev_type = self.rev_type.load(std::sync::atomic::Ordering::Relaxed);
        let (rev_wet_l, rev_wet_r) = if rev_mix > 0.0001 {
            let size_l = (rev_size * (1.0 - size_spread)).clamp(0.0, 1.0);
            let size_r = (rev_size * (1.0 + size_spread)).clamp(0.0, 1.0);
            let damp_l = (rev_damp + damp_spread).clamp(0.0, 1.0);
            let damp_r = (rev_damp - damp_spread).clamp(0.0, 1.0);
            (
                self.rev_l
                    .tick(s6_predelayed, size_l, damp_l, 0.0, 0, rev_type),
                self.rev_r
                    .tick(s6_predelayed, size_r, damp_r, 0.0, 0, rev_type),
            )
        } else {
            (0.0, 0.0)
        };

        // ── Shimmer reverb (stereo-decorrelated) ───────────────────────────
        let shim_mix = self.shim_mix.next();
        let shim_size = self.shim_size.next();
        let shim_damp = self.shim_damp.next();
        let shim_amt = self.shim_amt.next();
        let shim_pitch = self.shim_pitch.load(std::sync::atomic::Ordering::Relaxed);
        let (shim_wet_l, shim_wet_r) = if shim_mix > 0.0001 {
            let size_l = (shim_size * (1.0 - size_spread)).clamp(0.0, 1.0);
            let size_r = (shim_size * (1.0 + size_spread)).clamp(0.0, 1.0);
            let damp_l = (shim_damp + damp_spread).clamp(0.0, 1.0);
            let damp_r = (shim_damp - damp_spread).clamp(0.0, 1.0);
            (
                self.shim_l
                    .tick(s6, size_l, damp_l, shim_amt, shim_pitch, rev_type),
                self.shim_r
                    .tick(s6, size_r, damp_r, shim_amt, shim_pitch, rev_type),
            )
        } else {
            (0.0, 0.0)
        };

        // ── Crystallizer (stereo-decorrelated) ─────────────────────────────
        let crys_mix = self.crys_mix.next();
        let crys_grain = self.crys_grain.next();
        let crys_scatter = self.crys_scatter.next();
        let crys_feedback = self.crys_feedback.next();
        let crys_delay = self.crys_delay.next();
        let crys_pitch = self.crys_pitch.load(std::sync::atomic::Ordering::Relaxed);
        let (crys_wet_l, crys_wet_r) = if crys_mix > 0.0001 {
            (
                self.crys_l.tick(
                    s6,
                    crys_grain * 0.92,
                    (crys_scatter + 0.05).clamp(0.0, 1.0),
                    crys_feedback,
                    crys_delay * 0.95,
                    crys_pitch,
                ),
                self.crys_r.tick(
                    s6,
                    crys_grain * 1.08,
                    (crys_scatter - 0.05).clamp(0.0, 1.0),
                    crys_feedback,
                    crys_delay * 1.05,
                    crys_pitch,
                ),
            )
        } else {
            (0.0, 0.0)
        };

        // Equal-power wet balance: cos(θ)·dry so total energy stays constant
        // when wet is increased. θ = total_wet · π/2.
        let wet_total = (rev_mix + shim_mix + crys_mix).clamp(0.0, 1.0);
        let dry_bal = s6 * (wet_total * std::f32::consts::FRAC_PI_2).cos();

        // Wet side gain: sin(θ) completes the equal-power pair with cos(θ) dry.
        let wet_gain = (wet_total * std::f32::consts::FRAC_PI_2).sin();
        let wet_scale = if wet_total > 0.0001 {
            wet_gain / wet_total
        } else {
            1.0
        };
        let wet_l = (rev_mix * rev_wet_l + shim_mix * shim_wet_l) * wet_scale;
        let wet_r = (rev_mix * rev_wet_r + shim_mix * shim_wet_r) * wet_scale;

        // Extra width in the wet field only, leaving dry center stable.
        let wet_mid = 0.5 * (wet_l + wet_r);
        let wet_side = 0.5 * (wet_l - wet_r);
        let wet_wide_l = wet_mid + wet_side * shim_width;
        let wet_wide_r = wet_mid - wet_side * shim_width;

        // ── Stereo widener ──────────────────────────────────────────────────
        // Haas: write current dry into ring buffer, read with fractional delay for R.
        // Fractional interpolation prevents clicks when spread changes — reading between
        // two adjacent samples gives a smooth Doppler-like transition instead of a step.
        let spread_secs = self.stereo_spread.next().clamp(0.0, 0.012);
        let haas_len = self.haas_buf.len();
        let spread_f = (spread_secs * self.sr).clamp(0.0, (haas_len - 2) as f32);
        self.haas_buf[self.haas_pos] = dry_bal;
        let read_f = self.haas_pos as f32 - spread_f;
        let read_int = read_f.floor() as isize;
        let frac = read_f - read_int as f32;
        let i0 = ((read_int).rem_euclid(haas_len as isize)) as usize;
        let i1 = ((read_int + 1).rem_euclid(haas_len as isize)) as usize;
        let dry_r = self.haas_buf[i0] * (1.0 - frac) + self.haas_buf[i1] * frac;
        self.haas_pos = (self.haas_pos + 1) % haas_len;

        // Phaser stereo side: inject the L-R difference into the final output
        // to restore the stereo width lost when taking the mid-sum above.
        let ph_side = ph_theta.sin() * (ph_wet_l - ph_wet_r) * 0.5;

        let crys_theta = crys_mix * std::f32::consts::FRAC_PI_2;
        let raw_l = dry_bal + wet_wide_l + crys_theta.sin() * crys_wet_l + ph_side;
        let raw_r = dry_r + wet_wide_r + crys_theta.sin() * crys_wet_r - ph_side;

        // M/S width on final output
        let width = self.stereo_width.next().clamp(0.0, 2.0);
        let mid = 0.5 * (raw_l + raw_r);
        let side = 0.5 * (raw_l - raw_r);
        let out_l = mid + side * width;
        let out_r = mid - side * width;

        Frame::from([out_l, out_r])
    }
}
