//! The Synth — unified MiniMoog-style synthesizer
//! Run with: cargo run

#![allow(clippy::precedence)]

mod audio;
mod patch;
mod recorder;
mod scene;
mod sequencer;
mod ui;

use audio::{AudioEngine, DrumEngineAtomics, TrackMixerAtomics, TRACK_COUNT};
use eframe::egui;
use forma_control::midi::{MidiEngine, MidiEvent};
use patch::{default_patches, Patch};
use sequencer::{spawn_sequencer, ChordKbState, SequencerHandle};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use ui::drum_machine_ui::DrumMachineState;
use ui::frame::SynthFrame;
use ui::layout::{AppMode, StudioTab};

fn main() -> eframe::Result {
    let recorder_sink = Arc::new(Mutex::new(None));
    let audio = AudioEngine::new(Arc::clone(&recorder_sink)).expect("Failed to start audio");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 860.0])
            .with_title("The Synth"),
        ..Default::default()
    };

    eframe::run_native(
        "The Synth",
        options,
        Box::new(move |cc| {
            if let Some(wgpu_state) = cc.wgpu_render_state.as_ref() {
                let resources = ui::scope_wgpu::ScopeGpuResources::new(
                    &wgpu_state.device,
                    wgpu_state.target_format,
                );
                wgpu_state
                    .renderer
                    .write()
                    .callback_resources
                    .insert(resources);
            }
            Ok(Box::new(SynthApp::new(audio, recorder_sink)))
        }),
    )
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub(crate) struct PanelVisibility {
    pub oscillators: bool,
    pub modulation: bool,
    pub keyboard: bool,
    pub sequencer: bool,
    pub arp_walker: bool,
    pub fx_chain: bool,
    pub scope: bool,
    pub midi: bool,
}

impl Default for PanelVisibility {
    fn default() -> Self {
        Self {
            oscillators: true,
            modulation: true,
            keyboard: true,
            sequencer: true,
            arp_walker: true,
            fx_chain: true,
            scope: true,
            midi: true,
        }
    }
}

impl PanelVisibility {
    pub fn to_state(&self) -> ui::layout::PanelVisibilityState {
        ui::layout::PanelVisibilityState {
            oscillators: self.oscillators,
            modulation: self.modulation,
            keyboard: self.keyboard,
            sequencer: self.sequencer,
            arp_walker: self.arp_walker,
            fx_chain: self.fx_chain,
            scope: self.scope,
            midi: self.midi,
        }
    }

    pub fn from_state(s: &ui::layout::PanelVisibilityState) -> Self {
        Self {
            oscillators: s.oscillators,
            modulation: s.modulation,
            keyboard: s.keyboard,
            sequencer: s.sequencer,
            arp_walker: s.arp_walker,
            fx_chain: s.fx_chain,
            scope: s.scope,
            midi: s.midi,
        }
    }
}

pub(crate) struct SynthApp {
    pub(crate) _audio: AudioEngine, // keeps cpal stream alive
    /// Typed engine facade. The only way UI code talks to the engine —
    /// all parameter writes, event dispatch, and readback flow through this.
    pub(crate) engine: forma_engine::SynthEngineHandle,
    pub(crate) midi: MidiEngine,
    pub(crate) theme: ui::theme::SynthTheme,
    pub(crate) panels: PanelVisibility,
    pub(crate) reset_layout_pending: bool,
    pub(crate) dock_state: egui_dock::DockState<ui::dock::Tab>,

    // Layout B state
    pub(crate) app_mode: AppMode,
    pub(crate) studio_tab: StudioTab,

    // OSC bank
    pub(crate) osc_wave: [usize; 3], // 0=sine 1=saw 2=square 3=triangle
    pub(crate) osc_octave: [i32; 3], // -2..+2
    pub(crate) osc_detune: [f32; 3], // -100..+100 cents
    pub(crate) osc_vol: [f32; 3],
    pub(crate) osc_enabled: [bool; 3],
    pub(crate) osc_pulse_width: [f32; 3],
    pub(crate) osc_pw_enabled: [bool; 3],
    pub(crate) osc_unison_enabled: [bool; 3],
    pub(crate) osc_unison_count: [usize; 3], // 2..5
    pub(crate) osc_unison_spread: [f32; 3],  // 0..50 cents total
    pub(crate) hard_sync: bool,              // OSC 1 → OSC 2 hard sync
    pub(crate) fm_enabled: bool,             // OSC 2 → OSC 1 frequency modulation
    pub(crate) fm_depth: f32,                // FM depth (0 = off, ~1 = strong)
    pub(crate) ring_enabled: bool,           // ring modulation OSC 1 × OSC 2
    pub(crate) ring_depth: f32,              // ring mod depth
    pub(crate) osc1_mod_view: bool,          // OSC 1 card flipped to MOD back face

    // Noise — volume lives in engine; no UI mirror.

    // Mod wheel / aftertouch routing (mirrored from patch; runtime raw values are engine-only)
    pub(crate) mod_wheel_dest: usize, // 0=Off 1=Filter 2=LFO Depth 3=Amp
    pub(crate) mod_wheel_depth: f32,
    pub(crate) aftertouch_dest: usize,
    pub(crate) aftertouch_depth: f32,

    // LFO 1
    pub(crate) lfo_enabled: bool,
    pub(crate) lfo_rate: f32,
    pub(crate) lfo_depth: f32,
    pub(crate) lfo_shape: usize, // 0=sin 1=tri 2=saw
    pub(crate) lfo_dest: usize,  // 0=pitch 1=filter 2=amp
    pub(crate) lfo_sync: bool,
    pub(crate) lfo_division: usize,

    // LFO 2
    pub(crate) lfo2_enabled: bool,
    pub(crate) lfo2_rate: f32,
    pub(crate) lfo2_depth: f32,
    pub(crate) lfo2_shape: usize,
    pub(crate) lfo2_dest: usize,

    // Pulse (master ducker gate-lane). Pattern + length + division are UI-side mirrors of
    // the engine atomics; rate is derived from global_bpm + division and pushed via apply_clock_sync.
    pub(crate) pulse_enabled: bool,
    pub(crate) pulse_pattern: u16,
    pub(crate) pulse_length: u8,
    pub(crate) pulse_division: usize, // ClockDivision::to_u8() value
    pub(crate) pulse_depth: f32,

    // LFO1 / LFO2 retrigger gate lanes — same shape as Pulse, no depth (retrigger is binary).
    pub(crate) lfo1_gate_enabled: bool,
    pub(crate) lfo1_gate_pattern: u16,
    pub(crate) lfo1_gate_length: u8,
    pub(crate) lfo1_gate_division: usize,
    pub(crate) lfo2_gate_enabled: bool,
    pub(crate) lfo2_gate_pattern: u16,
    pub(crate) lfo2_gate_length: u8,
    pub(crate) lfo2_gate_division: usize,

    pub(crate) filter_enabled: bool,

    // Filter — cutoff/q are kept because the UI wants to remember their
    // pre-bypass value when filter_enabled is toggled off.
    pub(crate) filter_cutoff: f32,
    pub(crate) filter_q: f32,
    // filter_env_amount, fenv_adsr, amp_adsr, glide_time, master_vol, global_vol
    // live in the engine; UI reads via handle getters.

    // Keyboard
    pub(crate) piano_octave: i32,
    pub(crate) piano_velocity: u8,
    pub(crate) piano_pitch_bend: i8, // -2, -1, 0, +1, +2 semitones
    pub(crate) piano_mod_wheel: u8,  // 0–5: keys 3(off)–8(max); maps to 0–8000 Hz filter offset
    pub(crate) piano_held_midi: std::collections::HashSet<u8>,
    pub(crate) piano_mouse_midi: Option<u8>,
    pub(crate) kb_chord_mode: bool, // true = chord pads, false = standard piano
    /// When true, NoteOffs are suppressed; notes keep sounding until a new chord/note is played.
    pub(crate) kb_freeze: bool,
    /// MIDI notes currently sustained by freeze (key lifted but NoteOff suppressed).
    pub(crate) frozen_notes: std::collections::HashSet<u8>,

    // Peak meter
    pub(crate) peak_display: f32,
    pub(crate) peak_hold: f32,
    pub(crate) peak_hold_timer: f32,

    // Limiter — threshold lives in engine; only the UI toggle is mirrored.
    pub(crate) limiter_enabled: bool,
    pub(crate) window_focused: bool,

    // Global tempo / sync
    pub(crate) global_bpm: u32, // master tempo — source of truth when components are synced
    pub(crate) global_sync: bool, // when true, all components are forced to BPM sync
    pub(crate) arp_sync: bool,  // per-component sync toggle for arpeggiator
    pub(crate) walker_sync: bool, // per-component sync toggle for scale walker
    pub(crate) seq_sync: bool,  // per-component sync toggle for sequencer

    // Sequencer — focused-track handle (shorthand clone of track_seq[focused_track]).
    // All call sites use this; switch_focused_track swaps it to the new track's handle.
    pub(crate) seq: Arc<SequencerHandle>,

    // Per-track sequencer handles — each runs its own background thread.
    pub(crate) track_seq: [Arc<SequencerHandle>; TRACK_COUNT],

    // Per-track arp/seq sync flags (the single self.arp_sync/seq_sync/seq_pending_start
    // hold the focused track's current values and are saved here on focus switch).
    pub(crate) track_arp_sync: [bool; TRACK_COUNT],
    pub(crate) track_seq_sync: [bool; TRACK_COUNT],
    pub(crate) track_seq_pending: [bool; TRACK_COUNT],
    pub(crate) track_arp_pending: [bool; TRACK_COUNT],

    // Sequencer — chord keyboard (live, not threaded)
    pub(crate) chord_kb: ChordKbState,

    // Arp ring gate sequencer — mirrored here for patch save/load and UI
    pub(crate) arp_ring_enabled: bool,
    pub(crate) arp_ring_steps: u8,
    pub(crate) arp_ring_pattern: u32,
    pub(crate) arp_ring_k: u8, // euclidean K input (UI-only, not persisted to patch)

    // Per-sequencer clock division mirrors (index into SeqClockDiv::LABELS)
    pub(crate) note_seq_div: u8,
    pub(crate) chord_seq_div: u8,

    /// When bar-quantize is on, Play defers start until the next bar boundary.
    pub(crate) seq_pending_start: bool,
    /// When bar-quantize is on, Arp enable/RST defers restart until the next bar boundary.
    pub(crate) arp_pending_start: bool,

    // Oscilloscope
    pub(crate) scope_fullscreen: bool,
    pub(crate) scope_x_scale: f32,
    pub(crate) scope_y_scale: f32,
    pub(crate) show_voice_debug: bool,
    pub(crate) viz_mode: ui::scope_wgpu::VizMode,
    pub(crate) harm_phase: f64, // slow-drift animation phase for harmonograph
    pub(crate) vor_time: f64,   // elapsed seconds for voronoi seed orbits

    // Patch system
    pub(crate) patch_name: String,
    pub(crate) patch_library: Vec<Patch>,
    pub(crate) patch_browser_open: bool,
    pub(crate) patch_browser_category: String,
    pub(crate) patch_browser_model: String,
    pub(crate) patch_load_fx: bool,
    pub(crate) patch_search: String,
    pub(crate) patch_active_tags: std::collections::HashSet<String>,
    pub(crate) patch_favorites: std::collections::HashSet<String>,
    pub(crate) patch_recent: Vec<String>,

    // Metronome
    pub(crate) show_metronome: bool,
    pub(crate) metro_enabled: bool,
    pub(crate) metro_beats: u8,      // beats per bar (numerator): 2–8
    pub(crate) metro_denom: u8,      // beat unit: 4 = quarter, 8 = eighth
    pub(crate) metro_phase: f64,     // current position in beats [0, beats)
    pub(crate) metro_last_time: f64, // egui time at last frame, for delta

    // FX chain — per-effect enable + saved mix value
    pub(crate) fx_overdrive_on: bool,
    pub(crate) fx_overdrive_drive: f32,
    pub(crate) fx_overdrive_mix: f32,
    pub(crate) fx_overdrive_tone: f32,
    pub(crate) fx_overdrive_asym: f32,
    pub(crate) fx_distortion_on: bool,
    pub(crate) fx_distortion_drive: f32,
    pub(crate) fx_distortion_mix: f32,
    pub(crate) fx_distortion_tone: f32,
    pub(crate) fx_distortion_pre: f32,
    pub(crate) fx_chorus_on: bool,
    pub(crate) fx_chorus_rate: f32,
    pub(crate) fx_chorus_depth: f32,
    pub(crate) fx_chorus_mix: f32,
    pub(crate) fx_delay_on: bool,
    pub(crate) fx_delay_time: f32,
    pub(crate) fx_delay_feedback: f32,
    pub(crate) fx_delay_mix: f32,
    pub(crate) fx_delay_sync: bool, // if true, delay_time is derived from BPM
    pub(crate) fx_delay_division: usize, // index into DELAY_DIVISIONS
    pub(crate) fx_reverb_on: bool,
    pub(crate) fx_reverb_size: f32,
    pub(crate) fx_reverb_damp: f32,
    pub(crate) fx_reverb_mix: f32,
    pub(crate) fx_reverb_predelay: f32,
    pub(crate) fx_reverb_type: u8,
    pub(crate) stereo_spread: f32,
    pub(crate) stereo_width: f32,

    // Shimmer reverb (independent from plain reverb)
    pub(crate) fx_shimmer_on: bool,
    pub(crate) fx_shimmer_size: f32,
    pub(crate) fx_shimmer_damp: f32,
    pub(crate) fx_shimmer_mix: f32,
    pub(crate) fx_shimmer_amt: f32,
    pub(crate) fx_shimmer_width: f32,
    pub(crate) fx_shimmer_spread: f32,
    pub(crate) fx_shimmer_pitch: u8, // 0=unison, 1=+12st, 2=+24st
    // Crystallizer (granular pitch-shift delay)
    pub(crate) fx_crystal_on: bool,
    pub(crate) fx_crystal_mix: f32,
    pub(crate) fx_crystal_grain_ms: f32,
    pub(crate) fx_crystal_scatter: f32,
    pub(crate) fx_crystal_feedback: f32,
    pub(crate) fx_crystal_delay_ms: f32,
    pub(crate) fx_crystal_pitch: u8, // 0=0.5x, 1=1x, 2=2x, 3=4x

    // Multi-track rig — all 4 engine handles + mixer atomics
    pub(crate) track_engines: [forma_engine::SynthEngineHandle; TRACK_COUNT],
    pub(crate) track_mixer: [std::sync::Arc<TrackMixerAtomics>; TRACK_COUNT],
    /// Which track the UI is currently editing (0–3). Track 0 = default.
    pub(crate) focused_track: usize,
    /// Per-track name labels.
    pub(crate) track_names: [String; TRACK_COUNT],
    /// Last-known patch for each track — used to restore UI mirrors on focus switch.
    pub(crate) track_patches: [patch::Patch; TRACK_COUNT],

    // Drum machine — UI state + audio engine atomics
    pub(crate) drums: DrumMachineState,
    pub(crate) drum_engine: std::sync::Arc<DrumEngineAtomics>,

    // Mixer panel visibility (LIVE mode)
    pub(crate) show_mixer: bool,

    // Scene management
    pub(crate) scene_library: Vec<scene::Scene>,
    pub(crate) scene_name: String,
    pub(crate) scene_browser_open: bool,

    // Keyboard split (per-track MIDI note range, inclusive)
    pub(crate) track_key_lo: [u8; TRACK_COUNT],
    pub(crate) track_key_hi: [u8; TRACK_COUNT],
    // MIDI channel routing: 0 = omni, 1–16 = specific channel
    pub(crate) track_midi_ch: [u8; TRACK_COUNT],

    // Scene chain (auto-advance through scenes on bar boundaries)
    pub(crate) scene_chain: Vec<usize>, // indices into scene_library
    pub(crate) scene_chain_bars: u32,   // bars per step
    pub(crate) scene_chain_pos: usize,  // current step index
    pub(crate) scene_chain_active: bool,
    pub(crate) scene_chain_elapsed_s: f32,

    /// Shared WAV recorder sink — `Some` while recording, `None` otherwise.
    pub(crate) recorder_sink: Arc<Mutex<Option<recorder::Recorder>>>,
}

impl SynthApp {
    fn new(audio: AudioEngine, recorder_sink: Arc<Mutex<Option<recorder::Recorder>>>) -> Self {
        let mut midi = MidiEngine::new();
        midi.list_ports(); // populate port list at startup

        // Track 0 is the UI's active engine — existing UI code uses self.engine
        // which always points to the focused track's handle. Phase 2 will add
        // focus switching; for now track 0 is permanently focused.
        let engine = audio.handles[0].clone();
        let track_engines = [
            audio.handles[0].clone(),
            audio.handles[1].clone(),
            audio.handles[2].clone(),
            audio.handles[3].clone(),
        ];
        let track_mixer = [
            std::sync::Arc::clone(&audio.mixers[0]),
            std::sync::Arc::clone(&audio.mixers[1]),
            std::sync::Arc::clone(&audio.mixers[2]),
            std::sync::Arc::clone(&audio.mixers[3]),
        ];
        // Extract drum engine Arc before audio is moved.
        let drum_engine = std::sync::Arc::clone(&audio.drum);

        // Snapshot each engine's initial patch state (all "Init" on fresh start).
        let track_patches = [
            audio.handles[0].snapshot_patch(),
            audio.handles[1].snapshot_patch(),
            audio.handles[2].snapshot_patch(),
            audio.handles[3].snapshot_patch(),
        ];

        // Restore persisted layout (theme + panel visibility).
        let saved = ui::layout::load_layout();
        let theme = ui::theme::builtin_themes()
            .into_iter()
            .find(|t| t.name == saved.theme_name)
            .unwrap_or_else(ui::theme::midnight);
        let panels = PanelVisibility::from_state(&saved.panels);
        let app_mode = saved.app_mode;
        let studio_tab = saved.studio_tab;

        // One sequencer thread per track — each wired to its own engine clone so
        // all 4 can run and produce notes independently and simultaneously.
        let track_seq: [Arc<SequencerHandle>; TRACK_COUNT] = std::array::from_fn(|t| {
            let handle = Arc::new(SequencerHandle::new());
            spawn_sequencer(Arc::clone(&handle), track_engines[t].clone());
            handle
        });
        // self.seq is always a clone of the focused track's handle (initially track 0).
        let seq = Arc::clone(&track_seq[0]);

        Self {
            _audio: audio,
            engine,
            midi,
            theme,
            panels,
            reset_layout_pending: true,
            dock_state: ui::dock::default_dock_state(),
            app_mode,
            studio_tab,
            osc_wave: [1, 0, 0], // OSC1=saw, OSC2=sine, OSC3=sine
            osc_octave: [0, 0, 0],
            osc_detune: [0.0, 0.0, 0.0],
            osc_vol: [0.4, 0.3, 0.5],
            osc_enabled: [true, true, false],
            osc_pulse_width: [0.5, 0.5, 0.5],
            osc_pw_enabled: [false, false, false],
            osc_unison_enabled: [false, false, false],
            osc_unison_count: [2, 2, 2],
            osc_unison_spread: [20.0, 20.0, 20.0],
            hard_sync: false,
            fm_enabled: false,
            fm_depth: 1.0,
            ring_enabled: false,
            ring_depth: 1.0,
            osc1_mod_view: false,
            lfo_enabled: false,
            lfo_rate: 2.0,
            lfo_depth: 0.0,
            lfo_shape: 0,
            mod_wheel_dest: 1,
            mod_wheel_depth: 0.5,
            aftertouch_dest: 1,
            aftertouch_depth: 0.3,
            lfo_dest: 1,
            lfo_sync: false,
            lfo_division: 4,
            lfo2_enabled: false,
            lfo2_rate: 0.3,
            lfo2_depth: 0.0,
            lfo2_shape: 0,
            lfo2_dest: 2, // amp (tremolo)
            pulse_enabled: false,
            pulse_pattern: 0,
            pulse_length: 16,
            pulse_division: forma_common::ClockDivision::Eighth.to_u8() as usize,
            pulse_depth: 0.0,
            lfo1_gate_enabled: false,
            lfo1_gate_pattern: 0,
            lfo1_gate_length: 16,
            lfo1_gate_division: forma_common::ClockDivision::Eighth.to_u8() as usize,
            lfo2_gate_enabled: false,
            lfo2_gate_pattern: 0,
            lfo2_gate_length: 16,
            lfo2_gate_division: forma_common::ClockDivision::Eighth.to_u8() as usize,
            filter_enabled: true,
            filter_cutoff: 3000.0,
            filter_q: 0.3,
            piano_octave: 4,
            piano_velocity: 100,
            piano_pitch_bend: 0,
            piano_mod_wheel: 0,
            kb_chord_mode: false,
            kb_freeze: false,
            frozen_notes: std::collections::HashSet::new(),
            piano_held_midi: std::collections::HashSet::new(),
            piano_mouse_midi: None,
            peak_display: 0.0,
            peak_hold: 0.0,
            peak_hold_timer: 0.0,
            limiter_enabled: true,
            window_focused: true,
            global_bpm: 120,
            global_sync: false,
            arp_sync: true,
            walker_sync: true,
            seq_sync: true,
            arp_ring_enabled: false,
            arp_ring_steps: 8,
            arp_ring_pattern: 0xFF,
            arp_ring_k: 3,
            note_seq_div: 1,  // 1/8 note
            chord_seq_div: 4, // 1 bar
            seq_pending_start: false,
            arp_pending_start: false,
            seq,
            track_seq,
            track_arp_sync: [true; TRACK_COUNT],
            track_seq_sync: [true; TRACK_COUNT],
            track_seq_pending: [false; TRACK_COUNT],
            track_arp_pending: [false; TRACK_COUNT],
            chord_kb: ChordKbState::new(),
            scope_fullscreen: false,
            scope_x_scale: 1.0,
            scope_y_scale: 2.5,
            show_voice_debug: false,
            viz_mode: ui::scope_wgpu::VizMode::Scope,
            harm_phase: 0.0,
            vor_time: 0.0,
            patch_name: "Init".into(),
            patch_library: default_patches(),
            patch_browser_open: false,
            patch_browser_category: "All".into(),
            patch_browser_model: "All".into(),
            patch_load_fx: false,
            patch_search: String::new(),
            patch_active_tags: std::collections::HashSet::new(),
            patch_favorites: saved.patch_favorites.into_iter().collect(),
            patch_recent: saved.patch_recent,
            show_metronome: false,
            metro_enabled: false,
            metro_beats: 4,
            metro_denom: 4,
            metro_phase: 0.0,
            metro_last_time: 0.0,
            fx_overdrive_on: false,
            fx_overdrive_drive: 3.0,
            fx_overdrive_mix: 0.5,
            fx_overdrive_tone: 0.8,
            fx_overdrive_asym: 0.0,
            fx_distortion_on: false,
            fx_distortion_drive: 8.0,
            fx_distortion_mix: 0.5,
            fx_distortion_tone: 0.8,
            fx_distortion_pre: 0.0,
            fx_chorus_on: false,
            fx_chorus_rate: 0.8,
            fx_chorus_depth: 0.008,
            fx_chorus_mix: 0.4,
            fx_delay_on: false,
            fx_delay_time: 0.35,
            fx_delay_feedback: 0.4,
            fx_delay_mix: 0.4,
            fx_delay_sync: false,
            fx_delay_division: 2, // default: 1/4 note
            fx_reverb_on: false,
            fx_reverb_size: 0.6,
            fx_reverb_damp: 0.5,
            fx_reverb_mix: 0.4,
            fx_reverb_predelay: 0.0,
            fx_reverb_type: 0,
            stereo_spread: 0.0,
            stereo_width: 1.0,
            fx_shimmer_on: false,
            fx_shimmer_size: 0.7,
            fx_shimmer_damp: 0.4,
            fx_shimmer_mix: 0.4,
            fx_shimmer_amt: 0.5,
            fx_shimmer_width: 1.35,
            fx_shimmer_spread: 0.10,
            fx_shimmer_pitch: 1,
            fx_crystal_on: false,
            fx_crystal_mix: 0.35,
            fx_crystal_grain_ms: 120.0,
            fx_crystal_scatter: 0.25,
            fx_crystal_feedback: 0.35,
            fx_crystal_delay_ms: 260.0,
            fx_crystal_pitch: 2,
            track_engines,
            track_mixer,
            focused_track: 0,
            track_names: ["Lead".into(), "Pad".into(), "Bass".into(), "Keys".into()],
            track_patches,
            drums: DrumMachineState::default(),
            drum_engine,
            show_mixer: false,
            scene_library: scene::load_scenes(),
            scene_name: "Scene 1".into(),
            scene_browser_open: false,
            track_key_lo: [0u8; TRACK_COUNT],
            track_key_hi: [127u8; TRACK_COUNT],
            track_midi_ch: [0u8; TRACK_COUNT],
            scene_chain: Vec::new(),
            scene_chain_bars: 4,
            scene_chain_pos: 0,
            scene_chain_active: false,
            scene_chain_elapsed_s: 0.0,
            recorder_sink,
        }
    }
}

// ---------------------------------------------------------------------------
// Voice management
// ---------------------------------------------------------------------------

impl SynthApp {
    pub(crate) fn sync_transport_now(&mut self) {
        self.seq.current_step.store(0, Ordering::Relaxed);
        self.seq.arp_restart.store(false, Ordering::Relaxed);
        self.seq.walker_restart.store(false, Ordering::Relaxed);
        self.engine.arp_restart();
        self.engine.walker_restart();
        self.metro_reset();
    }

    pub(crate) fn arp_sync_active(&self) -> bool {
        self.global_sync || self.arp_sync
    }

    pub(crate) fn walker_sync_active(&self) -> bool {
        self.global_sync || self.walker_sync
    }

    pub(crate) fn seq_sync_active(&self) -> bool {
        self.global_sync || self.seq_sync
    }

    pub(crate) fn delay_sync_active(&self) -> bool {
        self.global_sync || self.fx_delay_sync
    }

    pub(crate) fn lfo_sync_active(&self) -> bool {
        self.global_sync || self.lfo_sync
    }

    pub(crate) fn schedule_or_restart_arp(&mut self) {
        let playing = self.seq.playing.load(Ordering::Relaxed);
        let bar_quantize = self.seq.bar_quantize.load(Ordering::Relaxed);
        if self.arp_sync_active() && bar_quantize {
            if playing {
                // Sequencer is the clock master — it fires arp restart at next bar boundary.
                self.seq.arp_restart.store(true, Ordering::Relaxed);
            } else {
                // No running sequencer — defer via metro bar-wrap.
                self.arp_pending_start = true;
            }
        } else {
            self.engine.arp_restart();
        }
    }

    pub(crate) fn schedule_or_restart_walker(&mut self) {
        let playing = self.seq.playing.load(Ordering::Relaxed);
        let bar_quantize = self.seq.bar_quantize.load(Ordering::Relaxed);
        if self.walker_sync_active() && bar_quantize && playing {
            self.seq.walker_restart.store(true, Ordering::Relaxed);
        } else {
            self.engine.walker_restart();
        }
    }

    pub(crate) fn apply_clock_sync(&mut self) {
        let global = self.global_bpm as f32;
        // Broadcast BPM to all track sequencers — they share the master tempo.
        if self.seq_sync_active() {
            for t in 0..TRACK_COUNT {
                self.track_seq[t]
                    .bpm
                    .store(self.global_bpm, Ordering::Relaxed);
            }
        }
        if self.arp_sync_active() && (self.engine.arp_bpm() - global).abs() > f32::EPSILON {
            self.engine.set_arp_bpm(global);
        }
        if self.walker_sync_active() && (self.engine.walker_bpm() - global).abs() > f32::EPSILON {
            self.engine.set_walker_bpm(global);
        }
        if self.lfo_sync_active() {
            let rate = ui::modulation::lfo_synced_rate(global, self.lfo_division);
            if (self.lfo_rate - rate).abs() > f32::EPSILON {
                self.lfo_rate = rate;
                self.engine.set_lfo_rate(rate);
            }
        }
        // Gate lanes are always tempo-synced — recompute step rate from BPM + division.
        let pulse_rate = forma_common::ClockDivision::from_u8(self.pulse_division as u8).hz(global);
        if (self.engine.gate_aenv_rate() - pulse_rate).abs() > f32::EPSILON {
            self.engine.set_gate_aenv_rate(pulse_rate);
        }
        let lfo1_gate_rate =
            forma_common::ClockDivision::from_u8(self.lfo1_gate_division as u8).hz(global);
        if (self.engine.gate_lfo1_rate() - lfo1_gate_rate).abs() > f32::EPSILON {
            self.engine.set_gate_lfo1_rate(lfo1_gate_rate);
        }
        let lfo2_gate_rate =
            forma_common::ClockDivision::from_u8(self.lfo2_gate_division as u8).hz(global);
        if (self.engine.gate_lfo2_rate() - lfo2_gate_rate).abs() > f32::EPSILON {
            self.engine.set_gate_lfo2_rate(lfo2_gate_rate);
        }
    }

    /// Push a NoteOn from the on-screen keyboard — always routes to the focused
    /// track only (tracks are independent synths; the piano controls the one you see).
    pub(crate) fn push_note_on(&mut self, midi: u8) {
        self.engine.note_on(midi, self.piano_velocity);
    }

    /// Route a NoteOn from a hardware MIDI device using per-track channel + split filters.
    /// In LIVE mode each track acts as an independent synth: hardware MIDI is the only
    /// path that fans out, and only when a track's channel/range matches.
    /// `channel`: 0-based MIDI channel (0–15).
    pub(crate) fn route_note_on(&mut self, midi: u8, channel: u8) {
        if self.app_mode == crate::ui::layout::AppMode::Live {
            for t in 0..TRACK_COUNT {
                let ch_ok = self.track_midi_ch[t] == 0 || self.track_midi_ch[t] == channel + 1;
                let key_ok = midi >= self.track_key_lo[t] && midi <= self.track_key_hi[t];
                if ch_ok && key_ok {
                    self.track_engines[t].note_on(midi, self.piano_velocity);
                }
            }
        } else {
            self.engine.note_on(midi, self.piano_velocity);
        }
    }

    /// Push a NoteOff from the on-screen keyboard — focused track only.
    pub(crate) fn push_note_off(&mut self, midi: u8) {
        self.engine.note_off(midi);
    }

    /// Silence all voices, reset all FX tails, and clear all note-tracking state.
    /// Push DrumMachineState → DrumEngineAtomics and read back current_step.
    pub(crate) fn tick_drums_sync(&mut self) {
        let d = &self.drums;
        let e = &self.drum_engine;
        e.enabled
            .store(d.enabled, std::sync::atomic::Ordering::Relaxed);
        e.set_bpm(self.global_bpm as f32);
        e.set_swing(d.swing);
        for ch in 0..audio::DRUM_CHANNELS {
            let mut pattern: u16 = 0;
            for step in 0..16 {
                if d.steps[ch][step] {
                    pattern |= 1 << step;
                }
            }
            e.step_patterns[ch].store(pattern, std::sync::atomic::Ordering::Relaxed);
            e.channel_muted[ch].store(d.muted[ch], std::sync::atomic::Ordering::Relaxed);
            e.set_channel_volume(ch, d.channel_volume[ch]);
        }
        self.drums.current_step = e.current_step.load(std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn all_notes_off(&mut self) {
        // Silence the focused engine and release held piano notes.
        // In LIVE mode each track is independent — other tracks' arps/seqs
        // keep running; only the focused track's keyboard input is cleared.
        self.engine.silence_all_voices();
        let held: Vec<u8> = self.piano_held_midi.drain().collect();
        for n in held {
            self.engine.note_off(n);
        }
        if self.app_mode == crate::ui::layout::AppMode::Live {
            for t in 0..TRACK_COUNT {
                self.track_seq[t].playing.store(false, Ordering::Relaxed);
                self.track_seq[t].current_step.store(0, Ordering::Relaxed);
            }
        } else {
            self.seq.playing.store(false, Ordering::Relaxed);
        }
        let frozen: Vec<u8> = self.frozen_notes.drain().collect();
        for n in frozen {
            self.engine.note_off(n);
        }
        self.chord_kb.held_pad = None;
        self.chord_kb.kb_held.clear();
    }

    /// Full panic stop: silence all voices, halt all transport (seq, arp, walker, drums),
    /// clear every pending state, keyboard freeze, and flush FX tails.
    pub(crate) fn stop_all(&mut self) {
        // Silence voices and clear all keyboard/note state
        self.all_notes_off();

        // Stop arp and clear its chord
        self.engine.set_arp_enabled(false);
        self.engine.chord_hold(&[]);
        self.arp_pending_start = false;
        self.kb_freeze = false;
        self.frozen_notes.clear();

        // Stop walker
        self.engine.set_walker_enabled(false);

        // Stop drums
        self.drum_engine.enabled.store(false, Ordering::Relaxed);
        self.drums.enabled = false;

        // Clear pending bar-quantize starts
        self.seq_pending_start = false;
        self.seq.playing.store(false, Ordering::Relaxed);

        // Flush FX tails (delay, reverb, shimmer, etc.)
        self.engine.reset_fx_tails();
    }
}

// ---------------------------------------------------------------------------
// Scene chain tick
// ---------------------------------------------------------------------------

impl SynthApp {
    pub(crate) fn tick_scene_chain(&mut self, dt: f32) {
        if !self.scene_chain_active || self.scene_chain.is_empty() {
            return;
        }
        let seconds_per_bar = 4.0 * 60.0 / (self.global_bpm as f32);
        let step_duration = seconds_per_bar * self.scene_chain_bars as f32;
        self.scene_chain_elapsed_s += dt;
        if self.scene_chain_elapsed_s >= step_duration {
            self.scene_chain_elapsed_s -= step_duration;
            self.scene_chain_pos = (self.scene_chain_pos + 1) % self.scene_chain.len();
            let idx = self.scene_chain[self.scene_chain_pos];
            if idx < self.scene_library.len() {
                let scene = self.scene_library[idx].clone();
                self.load_scene(scene);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MIDI tick — drain events from the MIDI thread each frame
// ---------------------------------------------------------------------------

impl SynthApp {
    fn tick_midi(&mut self) {
        let events = self.midi.drain();
        for ev in events {
            match ev {
                MidiEvent::NoteOn {
                    note,
                    velocity,
                    channel,
                } => {
                    let _ = velocity;
                    self.route_note_on(note, channel);
                }
                MidiEvent::NoteOff { note, .. } => {
                    self.push_note_off(note);
                }
                MidiEvent::Aftertouch { value, .. } => {
                    self.engine.set_aftertouch(value as f32 / 127.0);
                }
                MidiEvent::CC { cc, value, .. } => {
                    let v = value as f32 / 127.0;
                    match cc {
                        1 => {
                            // Mod wheel — routed by mod_wheel_dest in the engine
                            self.piano_mod_wheel = (v * 5.0).round() as u8;
                            self.engine.set_mod_wheel(v);
                        }
                        7 => {
                            // Volume → master vol
                            self.engine.set_master_volume(v);
                        }
                        71 => {
                            // Resonance
                            let q = v * 0.95;
                            self.filter_q = q;
                            self.engine.set_filter_resonance(q);
                        }
                        74 => {
                            // Cutoff (brightness)
                            let hz = 80.0 * (18000.0_f32 / 80.0).powf(v);
                            self.filter_cutoff = hz;
                            self.engine.set_filter_cutoff(hz);
                        }
                        64 => {
                            // Sustain pedal → freeze
                            let pedal_down = value >= 64;
                            if pedal_down && !self.kb_freeze {
                                self.kb_freeze = true;
                            } else if !pedal_down && self.kb_freeze {
                                self.kb_freeze = false;
                                let frozen: Vec<u8> = self.frozen_notes.drain().collect();
                                for n in frozen {
                                    self.push_note_off(n);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                MidiEvent::PitchBend { value, .. } => {
                    let semitones = value * 2.0;
                    self.engine.set_lfo_pitch_mult(2_f32.powf(semitones / 12.0));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Main update
// ---------------------------------------------------------------------------

impl eframe::App for SynthApp {
    fn on_exit(&mut self) {
        let state = ui::layout::LayoutState {
            theme_name: self.theme.name.clone(),
            panels: self.panels.to_state(),
            app_mode: self.app_mode,
            studio_tab: self.studio_tab,
            patch_favorites: self.patch_favorites.iter().cloned().collect(),
            patch_recent: self.patch_recent.clone(),
        };
        ui::layout::save_layout(&state);
    }

    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = _ui.ctx().clone();
        let ctx = &ctx;
        // Apply theme to egui Visuals + Style every frame — cheap struct copies.
        self.theme.apply_to_egui(ctx);

        // Release all notes when the window loses focus so keys/MIDI can't get stuck.
        if ctx.input(|i| i.focused) != self.window_focused {
            self.window_focused = ctx.input(|i| i.focused);
            if !self.window_focused {
                self.all_notes_off();
            }
        }

        self.tick_midi();
        self.apply_clock_sync();
        self.tick_drums_sync();
        self.tick_keyboard_input(ctx);
        let dt = ctx.input(|i| i.unstable_dt).min(0.1);
        self.tick_scene_chain(dt);

        // Advance metronome phase each frame.
        self.tick_metronome(ctx);

        // Floating windows — must be shown before panels.
        self.ui_patch_browser(ctx);
        self.ui_metronome_window(ctx);
        self.ui_scope_fullscreen(ctx);
        self.ui_scene_browser(ctx);

        // ── Zone 1: global bar (top, always visible) ──────────────────────────
        egui::TopBottomPanel::top("global_bar")
            .frame(SynthFrame::bar(&self.theme))
            .show(ctx, |ui| {
                self.ui_global_bar(ui);
            });

        // ── Zone 5b: keyboard strip (bottom-most) ─────────────────────────────
        egui::TopBottomPanel::bottom("keyboard_strip")
            .frame(SynthFrame::transport(&self.theme))
            .show(ctx, |ui| {
                self.ui_keyboard_panel(ui);
            });

        // ── Zone 5a: FX mini strip (above keyboard, always visible) ───────────
        egui::TopBottomPanel::bottom("fx_mini_strip")
            .frame(SynthFrame::transport(&self.theme))
            .show(ctx, |ui| {
                self.ui_fx_mini_strip(ui);
            });

        // ── Zones 2 + 3: central editing area (dock in Studio, placeholder in Live) ──
        egui::CentralPanel::default()
            .frame(SynthFrame::app_bg(&self.theme))
            .show(ctx, |ui| match self.app_mode {
                AppMode::Studio => {
                    self.ui_synth_dock(ui);
                }
                #[cfg(feature = "live_rig")]
                AppMode::DrumMachine => {
                    self.ui_drum_machine(ui);
                }
                #[cfg(feature = "live_rig")]
                AppMode::Live => {
                    self.ui_live_view(ui);
                }
                #[cfg(not(feature = "live_rig"))]
                _ => {
                    self.ui_synth_dock(ui);
                }
            });

        ctx.request_repaint();
    }
}

// ---------------------------------------------------------------------------
// Patch capture / apply
// ---------------------------------------------------------------------------

impl SynthApp {
    pub(crate) fn capture_patch(&self) -> Patch {
        // Start with a snapshot of engine state, then overlay the UI-owned
        // fields that either live only on the UI mirror (enable flags,
        // "remembered" pre-bypass slider positions, derived decompositions
        // of engine params) or whose UI truth outranks engine truth.
        let mut p = self.engine.snapshot_patch();
        p.name = self.patch_name.clone();
        p.category = "User".into();

        // Oscillator bank: UI owns the (osc_vol, *_enabled, osc_pw_enabled,
        // unison_*, osc_octave, osc_detune) decomposition.
        p.osc_wave = self.osc_wave;
        p.osc_octave = self.osc_octave;
        p.osc_detune = self.osc_detune;
        p.osc_vol = self.osc_vol;
        p.osc_enabled = self.osc_enabled;
        p.osc_pulse_width = self.osc_pulse_width;
        p.osc_pw_enabled = self.osc_pw_enabled;
        p.osc_unison_enabled = self.osc_unison_enabled;
        p.osc_unison_count = self.osc_unison_count;
        p.osc_unison_spread = self.osc_unison_spread;
        p.hard_sync = self.hard_sync;

        // Global/bypass-paired fields.
        p.fm_enabled = self.fm_enabled;
        p.fm_depth = self.fm_depth;
        p.ring_enabled = self.ring_enabled;
        p.ring_depth = self.ring_depth;
        p.mod_wheel_dest = self.mod_wheel_dest as u8;
        p.mod_wheel_depth = self.mod_wheel_depth;
        p.aftertouch_dest = self.aftertouch_dest as u8;
        p.aftertouch_depth = self.aftertouch_depth;
        p.lfo_enabled = self.lfo_enabled;
        p.lfo_rate = self.lfo_rate;
        p.lfo_depth = self.lfo_depth;
        p.lfo_shape = self.lfo_shape;
        p.lfo_dest = self.lfo_dest;
        p.lfo_sync = self.lfo_sync;
        p.lfo_division = self.lfo_division;
        p.lfo2_enabled = self.lfo2_enabled;
        p.lfo2_rate = self.lfo2_rate;
        p.lfo2_depth = self.lfo2_depth;
        p.lfo2_shape = self.lfo2_shape;
        p.lfo2_dest = self.lfo2_dest;
        p.gate_aenv_enabled = self.pulse_enabled;
        p.gate_aenv_pattern = self.pulse_pattern;
        p.gate_aenv_length = self.pulse_length;
        p.gate_aenv_division = self.pulse_division;
        p.gate_aenv_depth = self.pulse_depth;
        p.gate_lfo1_enabled = self.lfo1_gate_enabled;
        p.gate_lfo1_pattern = self.lfo1_gate_pattern;
        p.gate_lfo1_length = self.lfo1_gate_length;
        p.gate_lfo1_division = self.lfo1_gate_division;
        p.gate_lfo2_enabled = self.lfo2_gate_enabled;
        p.gate_lfo2_pattern = self.lfo2_gate_pattern;
        p.gate_lfo2_length = self.lfo2_gate_length;
        p.gate_lfo2_division = self.lfo2_gate_division;
        p.arp_ring_enabled = self.arp_ring_enabled;
        p.arp_ring_steps = self.arp_ring_steps;
        p.arp_ring_pattern = self.arp_ring_pattern;
        p.note_seq_div = self.note_seq_div;
        p.chord_seq_div = self.chord_seq_div;
        p.filter_enabled = self.filter_enabled;
        p.filter_cutoff = self.filter_cutoff;
        p.filter_q = self.filter_q;
        p.limiter_enabled = self.limiter_enabled;

        // FX chain (mirror still lives on SynthApp; future batches may move
        // these into pure engine-read territory).
        p.fx_overdrive_on = self.fx_overdrive_on;
        p.fx_overdrive_drive = self.fx_overdrive_drive;
        p.fx_overdrive_mix = self.fx_overdrive_mix;
        p.fx_overdrive_tone = self.fx_overdrive_tone;
        p.fx_overdrive_asym = self.fx_overdrive_asym;
        p.fx_distortion_on = self.fx_distortion_on;
        p.fx_distortion_drive = self.fx_distortion_drive;
        p.fx_distortion_mix = self.fx_distortion_mix;
        p.fx_distortion_tone = self.fx_distortion_tone;
        p.fx_distortion_pre = self.fx_distortion_pre;
        p.fx_chorus_on = self.fx_chorus_on;
        p.fx_chorus_rate = self.fx_chorus_rate;
        p.fx_chorus_depth = self.fx_chorus_depth;
        p.fx_chorus_mix = self.fx_chorus_mix;
        p.fx_delay_on = self.fx_delay_on;
        p.fx_delay_time = self.fx_delay_time;
        p.fx_delay_feedback = self.fx_delay_feedback;
        p.fx_delay_mix = self.fx_delay_mix;
        p.fx_delay_sync = self.fx_delay_sync;
        p.fx_delay_division = self.fx_delay_division;
        p.fx_reverb_on = self.fx_reverb_on;
        p.fx_reverb_size = self.fx_reverb_size;
        p.fx_reverb_damp = self.fx_reverb_damp;
        p.fx_reverb_mix = self.fx_reverb_mix;
        p.fx_reverb_predelay = self.fx_reverb_predelay;
        p.fx_reverb_type = self.fx_reverb_type;
        p.stereo_spread = self.stereo_spread;
        p.stereo_width = self.stereo_width;
        p.fx_shimmer_on = self.fx_shimmer_on;
        p.fx_shimmer_size = self.fx_shimmer_size;
        p.fx_shimmer_damp = self.fx_shimmer_damp;
        p.fx_shimmer_mix = self.fx_shimmer_mix;
        p.fx_shimmer_amt = self.fx_shimmer_amt;
        p.fx_shimmer_width = self.fx_shimmer_width;
        p.fx_shimmer_spread = self.fx_shimmer_spread;
        p.fx_shimmer_pitch = self.fx_shimmer_pitch;
        p.fx_crystal_on = self.fx_crystal_on;
        p.fx_crystal_mix = self.fx_crystal_mix;
        p.fx_crystal_grain_ms = self.fx_crystal_grain_ms;
        p.fx_crystal_scatter = self.fx_crystal_scatter;
        p.fx_crystal_feedback = self.fx_crystal_feedback;
        p.fx_crystal_delay_ms = self.fx_crystal_delay_ms;
        p.fx_crystal_pitch = self.fx_crystal_pitch;
        p
    }

    pub(crate) fn apply_patch(&mut self, p: Patch) {
        // Record in recents (deduplicate, keep newest first, cap at 12)
        let rname = p.name.clone();
        self.patch_recent.retain(|n| n != &rname);
        self.patch_recent.insert(0, rname);
        self.patch_recent.truncate(12);

        // Silence all voices before changing parameters to prevent Moog filter blowup.
        self.all_notes_off();
        // Clear FX tail buffers so old reverb/delay from the previous patch does not
        // bleed into the new sound. Runs on the next audio callback tick.
        self.engine.reset_fx_tails();

        // -- Sync UI mirror fields from the patch. Only the fields still
        // living on the UI mirror get copied. Fields that the engine
        // authoritatively owns (ADSRs, glide, master, global, noise,
        // limiter threshold, filter env amount, arp/walker state) are
        // restored by `engine.apply_patch` below.
        self.patch_name = p.name.clone();
        self.osc_wave = p.osc_wave;
        self.osc_octave = p.osc_octave;
        self.osc_detune = p.osc_detune;
        self.osc_vol = p.osc_vol;
        self.osc_enabled = p.osc_enabled;
        self.osc_pulse_width = p.osc_pulse_width;
        self.osc_pw_enabled = p.osc_pw_enabled;
        self.osc_unison_enabled = p.osc_unison_enabled;
        self.osc_unison_count = p.osc_unison_count;
        self.osc_unison_spread = p.osc_unison_spread;
        self.hard_sync = p.hard_sync;
        self.fm_enabled = p.fm_enabled;
        self.fm_depth = p.fm_depth;
        self.ring_enabled = p.ring_enabled;
        self.ring_depth = p.ring_depth;
        self.mod_wheel_dest = p.mod_wheel_dest as usize;
        self.mod_wheel_depth = p.mod_wheel_depth;
        self.aftertouch_dest = p.aftertouch_dest as usize;
        self.aftertouch_depth = p.aftertouch_depth;
        self.lfo_enabled = p.lfo_enabled;
        self.lfo_rate = p.lfo_rate;
        self.lfo_depth = p.lfo_depth;
        self.lfo_shape = p.lfo_shape;
        self.lfo_dest = p.lfo_dest;
        self.lfo_sync = p.lfo_sync;
        self.lfo_division = p.lfo_division;
        self.lfo2_enabled = p.lfo2_enabled;
        self.lfo2_rate = p.lfo2_rate;
        self.lfo2_depth = p.lfo2_depth;
        self.lfo2_shape = p.lfo2_shape;
        self.lfo2_dest = p.lfo2_dest;
        self.pulse_enabled = p.gate_aenv_enabled;
        self.pulse_pattern = p.gate_aenv_pattern;
        self.pulse_length = p.gate_aenv_length;
        self.pulse_division = p.gate_aenv_division;
        self.pulse_depth = p.gate_aenv_depth;
        self.lfo1_gate_enabled = p.gate_lfo1_enabled;
        self.lfo1_gate_pattern = p.gate_lfo1_pattern;
        self.lfo1_gate_length = p.gate_lfo1_length;
        self.lfo1_gate_division = p.gate_lfo1_division;
        self.lfo2_gate_enabled = p.gate_lfo2_enabled;
        self.lfo2_gate_pattern = p.gate_lfo2_pattern;
        self.lfo2_gate_length = p.gate_lfo2_length;
        self.lfo2_gate_division = p.gate_lfo2_division;
        self.arp_ring_enabled = p.arp_ring_enabled;
        self.arp_ring_steps = p.arp_ring_steps;
        self.arp_ring_pattern = p.arp_ring_pattern;
        self.engine.set_arp_ring_enabled(p.arp_ring_enabled);
        self.engine.set_arp_ring_steps(p.arp_ring_steps);
        self.engine.set_arp_ring_pattern(p.arp_ring_pattern);
        self.note_seq_div = p.note_seq_div;
        self.chord_seq_div = p.chord_seq_div;
        self.seq.note_div.store(p.note_seq_div, Ordering::Relaxed);
        self.seq.chord_div.store(p.chord_seq_div, Ordering::Relaxed);
        self.filter_enabled = p.filter_enabled;
        self.filter_cutoff = p.filter_cutoff;
        self.filter_q = p.filter_q;
        self.limiter_enabled = p.limiter_enabled;

        if self.patch_load_fx {
            // Sync the FX mirror fields too.
            self.fx_overdrive_on = p.fx_overdrive_on;
            self.fx_overdrive_drive = p.fx_overdrive_drive;
            self.fx_overdrive_mix = p.fx_overdrive_mix;
            self.fx_overdrive_tone = p.fx_overdrive_tone;
            self.fx_overdrive_asym = p.fx_overdrive_asym;
            self.fx_distortion_on = p.fx_distortion_on;
            self.fx_distortion_drive = p.fx_distortion_drive;
            self.fx_distortion_mix = p.fx_distortion_mix;
            self.fx_distortion_tone = p.fx_distortion_tone;
            self.fx_distortion_pre = p.fx_distortion_pre;
            self.fx_chorus_on = p.fx_chorus_on;
            self.fx_chorus_rate = p.fx_chorus_rate;
            self.fx_chorus_depth = p.fx_chorus_depth;
            self.fx_chorus_mix = p.fx_chorus_mix;
            self.fx_delay_on = p.fx_delay_on;
            self.fx_delay_time = p.fx_delay_time;
            self.fx_delay_feedback = p.fx_delay_feedback;
            self.fx_delay_mix = p.fx_delay_mix;
            self.fx_delay_sync = p.fx_delay_sync;
            self.fx_delay_division = p.fx_delay_division;
            self.fx_reverb_on = p.fx_reverb_on;
            self.fx_reverb_size = p.fx_reverb_size;
            self.fx_reverb_damp = p.fx_reverb_damp;
            self.fx_reverb_mix = p.fx_reverb_mix;
            self.fx_reverb_predelay = p.fx_reverb_predelay;
            self.fx_reverb_type = p.fx_reverb_type;
            self.stereo_spread = p.stereo_spread;
            self.stereo_width = p.stereo_width;
            self.fx_shimmer_on = p.fx_shimmer_on;
            self.fx_shimmer_size = p.fx_shimmer_size;
            self.fx_shimmer_damp = p.fx_shimmer_damp;
            self.fx_shimmer_mix = p.fx_shimmer_mix;
            self.fx_shimmer_amt = p.fx_shimmer_amt;
            self.fx_shimmer_width = p.fx_shimmer_width;
            self.fx_shimmer_spread = p.fx_shimmer_spread;
            self.fx_shimmer_pitch = p.fx_shimmer_pitch;
            self.fx_crystal_on = p.fx_crystal_on;
            self.fx_crystal_mix = p.fx_crystal_mix;
            self.fx_crystal_grain_ms = p.fx_crystal_grain_ms;
            self.fx_crystal_scatter = p.fx_crystal_scatter;
            self.fx_crystal_feedback = p.fx_crystal_feedback;
            self.fx_crystal_delay_ms = p.fx_crystal_delay_ms;
            self.fx_crystal_pitch = p.fx_crystal_pitch;
        }

        // -- Push engine state through the typed handle.
        //
        // `apply_patch` always writes the sound-generating half of the patch
        // (oscillators, filter, LFOs, envelopes, master, limiter). The FX
        // chain is only written if the user has "Load FX" enabled — to keep
        // it off, patch over just the FX fields with a zero-mix view.
        if self.patch_load_fx {
            self.engine.apply_patch(&p);
        } else {
            let mut core = p.clone();
            // Wipe FX and stereo so apply_patch doesn't clobber the user's
            // current FX settings. Use the live handle values.
            core.fx_overdrive_on = self.fx_overdrive_on;
            core.fx_overdrive_drive = self.fx_overdrive_drive;
            core.fx_overdrive_mix = self.fx_overdrive_mix;
            core.fx_overdrive_tone = self.fx_overdrive_tone;
            core.fx_overdrive_asym = self.fx_overdrive_asym;
            core.fx_distortion_on = self.fx_distortion_on;
            core.fx_distortion_drive = self.fx_distortion_drive;
            core.fx_distortion_mix = self.fx_distortion_mix;
            core.fx_distortion_tone = self.fx_distortion_tone;
            core.fx_distortion_pre = self.fx_distortion_pre;
            core.fx_chorus_on = self.fx_chorus_on;
            core.fx_chorus_rate = self.fx_chorus_rate;
            core.fx_chorus_depth = self.fx_chorus_depth;
            core.fx_chorus_mix = self.fx_chorus_mix;
            core.fx_delay_on = self.fx_delay_on;
            core.fx_delay_time = self.fx_delay_time;
            core.fx_delay_feedback = self.fx_delay_feedback;
            core.fx_delay_mix = self.fx_delay_mix;
            core.fx_delay_sync = self.fx_delay_sync;
            core.fx_delay_division = self.fx_delay_division;
            core.fx_reverb_on = self.fx_reverb_on;
            core.fx_reverb_size = self.fx_reverb_size;
            core.fx_reverb_damp = self.fx_reverb_damp;
            core.fx_reverb_mix = self.fx_reverb_mix;
            core.fx_reverb_predelay = self.fx_reverb_predelay;
            core.fx_reverb_type = self.fx_reverb_type;
            core.stereo_spread = self.stereo_spread;
            core.stereo_width = self.stereo_width;
            core.fx_shimmer_on = self.fx_shimmer_on;
            core.fx_shimmer_size = self.fx_shimmer_size;
            core.fx_shimmer_damp = self.fx_shimmer_damp;
            core.fx_shimmer_mix = self.fx_shimmer_mix;
            core.fx_shimmer_amt = self.fx_shimmer_amt;
            core.fx_shimmer_width = self.fx_shimmer_width;
            core.fx_shimmer_spread = self.fx_shimmer_spread;
            core.fx_shimmer_pitch = self.fx_shimmer_pitch;
            core.fx_crystal_on = self.fx_crystal_on;
            core.fx_crystal_mix = self.fx_crystal_mix;
            core.fx_crystal_grain_ms = self.fx_crystal_grain_ms;
            core.fx_crystal_scatter = self.fx_crystal_scatter;
            core.fx_crystal_feedback = self.fx_crystal_feedback;
            core.fx_crystal_delay_ms = self.fx_crystal_delay_ms;
            core.fx_crystal_pitch = self.fx_crystal_pitch;
            self.engine.apply_patch(&core);
        }

        // Propagate delay-sync state.
        if self.patch_load_fx {
            self.apply_clock_sync();
        }
    }
}

// ---------------------------------------------------------------------------
// Multi-track focus management
// ---------------------------------------------------------------------------

impl SynthApp {
    /// Switch the UI to edit a different track without stopping any notes.
    /// Saves the current track's UI state, swaps the engine reference, restores
    /// the new track's UI state.
    pub(crate) fn switch_focused_track(&mut self, new: usize) {
        if new >= TRACK_COUNT || new == self.focused_track {
            return;
        }
        let old = self.focused_track;

        // Release any piano-held notes on the old track before switching so they
        // don't sustain indefinitely (each track is an independent synth).
        let held: Vec<u8> = self.piano_held_midi.iter().copied().collect();
        for n in held {
            self.track_engines[old].note_off(n);
        }

        // Save current track's patch and sync flags.
        self.track_patches[old] = self.capture_patch();
        self.track_arp_sync[old] = self.arp_sync;
        self.track_seq_sync[old] = self.seq_sync;
        self.track_seq_pending[old] = self.seq_pending_start;
        self.track_arp_pending[old] = self.arp_pending_start;

        // Switch engine + sequencer handle to the new track.
        // The old track's sequencer thread keeps running independently.
        self.focused_track = new;
        self.engine = self.track_engines[new].clone();
        self.seq = Arc::clone(&self.track_seq[new]);

        // Restore new track's sync flags.
        self.arp_sync = self.track_arp_sync[new];
        self.seq_sync = self.track_seq_sync[new];
        self.seq_pending_start = self.track_seq_pending[new];
        self.arp_pending_start = self.track_arp_pending[new];

        // Restore new track state: sync UI mirrors AND push params to the engine.
        let p = self.track_patches[new].clone();
        self.apply_ui_mirrors_only(p);
        self.engine.apply_patch(&self.track_patches[new]);
        self.apply_clock_sync();
    }

    /// Copy all UI-mirror fields from a patch without touching the audio engine
    /// or stopping notes. Used when switching focused track.
    pub(crate) fn apply_ui_mirrors_only(&mut self, p: patch::Patch) {
        self.patch_name = p.name.clone();
        self.osc_wave = p.osc_wave;
        self.osc_octave = p.osc_octave;
        self.osc_detune = p.osc_detune;
        self.osc_vol = p.osc_vol;
        self.osc_enabled = p.osc_enabled;
        self.osc_pulse_width = p.osc_pulse_width;
        self.osc_pw_enabled = p.osc_pw_enabled;
        self.osc_unison_enabled = p.osc_unison_enabled;
        self.osc_unison_count = p.osc_unison_count;
        self.osc_unison_spread = p.osc_unison_spread;
        self.hard_sync = p.hard_sync;
        self.fm_enabled = p.fm_enabled;
        self.fm_depth = p.fm_depth;
        self.ring_enabled = p.ring_enabled;
        self.ring_depth = p.ring_depth;
        self.mod_wheel_dest = p.mod_wheel_dest as usize;
        self.mod_wheel_depth = p.mod_wheel_depth;
        self.aftertouch_dest = p.aftertouch_dest as usize;
        self.aftertouch_depth = p.aftertouch_depth;
        self.lfo_enabled = p.lfo_enabled;
        self.lfo_rate = p.lfo_rate;
        self.lfo_depth = p.lfo_depth;
        self.lfo_shape = p.lfo_shape;
        self.lfo_dest = p.lfo_dest;
        self.lfo_sync = p.lfo_sync;
        self.lfo_division = p.lfo_division;
        self.lfo2_enabled = p.lfo2_enabled;
        self.lfo2_rate = p.lfo2_rate;
        self.lfo2_depth = p.lfo2_depth;
        self.lfo2_shape = p.lfo2_shape;
        self.lfo2_dest = p.lfo2_dest;
        self.pulse_enabled = p.gate_aenv_enabled;
        self.pulse_pattern = p.gate_aenv_pattern;
        self.pulse_length = p.gate_aenv_length;
        self.pulse_division = p.gate_aenv_division;
        self.pulse_depth = p.gate_aenv_depth;
        self.lfo1_gate_enabled = p.gate_lfo1_enabled;
        self.lfo1_gate_pattern = p.gate_lfo1_pattern;
        self.lfo1_gate_length = p.gate_lfo1_length;
        self.lfo1_gate_division = p.gate_lfo1_division;
        self.lfo2_gate_enabled = p.gate_lfo2_enabled;
        self.lfo2_gate_pattern = p.gate_lfo2_pattern;
        self.lfo2_gate_length = p.gate_lfo2_length;
        self.lfo2_gate_division = p.gate_lfo2_division;
        self.arp_ring_enabled = p.arp_ring_enabled;
        self.arp_ring_steps = p.arp_ring_steps;
        self.arp_ring_pattern = p.arp_ring_pattern;
        self.note_seq_div = p.note_seq_div;
        self.chord_seq_div = p.chord_seq_div;
        self.filter_enabled = p.filter_enabled;
        self.filter_cutoff = p.filter_cutoff;
        self.filter_q = p.filter_q;
        self.limiter_enabled = p.limiter_enabled;
        self.fx_overdrive_on = p.fx_overdrive_on;
        self.fx_overdrive_drive = p.fx_overdrive_drive;
        self.fx_overdrive_mix = p.fx_overdrive_mix;
        self.fx_overdrive_tone = p.fx_overdrive_tone;
        self.fx_overdrive_asym = p.fx_overdrive_asym;
        self.fx_distortion_on = p.fx_distortion_on;
        self.fx_distortion_drive = p.fx_distortion_drive;
        self.fx_distortion_mix = p.fx_distortion_mix;
        self.fx_distortion_tone = p.fx_distortion_tone;
        self.fx_distortion_pre = p.fx_distortion_pre;
        self.fx_chorus_on = p.fx_chorus_on;
        self.fx_chorus_rate = p.fx_chorus_rate;
        self.fx_chorus_depth = p.fx_chorus_depth;
        self.fx_chorus_mix = p.fx_chorus_mix;
        self.fx_delay_on = p.fx_delay_on;
        self.fx_delay_time = p.fx_delay_time;
        self.fx_delay_feedback = p.fx_delay_feedback;
        self.fx_delay_mix = p.fx_delay_mix;
        self.fx_delay_sync = p.fx_delay_sync;
        self.fx_delay_division = p.fx_delay_division;
        self.fx_reverb_on = p.fx_reverb_on;
        self.fx_reverb_size = p.fx_reverb_size;
        self.fx_reverb_damp = p.fx_reverb_damp;
        self.fx_reverb_mix = p.fx_reverb_mix;
        self.fx_reverb_predelay = p.fx_reverb_predelay;
        self.fx_reverb_type = p.fx_reverb_type;
        self.stereo_spread = p.stereo_spread;
        self.stereo_width = p.stereo_width;
        self.fx_shimmer_on = p.fx_shimmer_on;
        self.fx_shimmer_size = p.fx_shimmer_size;
        self.fx_shimmer_damp = p.fx_shimmer_damp;
        self.fx_shimmer_mix = p.fx_shimmer_mix;
        self.fx_shimmer_amt = p.fx_shimmer_amt;
        self.fx_shimmer_width = p.fx_shimmer_width;
        self.fx_shimmer_spread = p.fx_shimmer_spread;
        self.fx_shimmer_pitch = p.fx_shimmer_pitch;
        self.fx_crystal_on = p.fx_crystal_on;
        self.fx_crystal_mix = p.fx_crystal_mix;
        self.fx_crystal_grain_ms = p.fx_crystal_grain_ms;
        self.fx_crystal_scatter = p.fx_crystal_scatter;
        self.fx_crystal_feedback = p.fx_crystal_feedback;
        self.fx_crystal_delay_ms = p.fx_crystal_delay_ms;
        self.fx_crystal_pitch = p.fx_crystal_pitch;
    }
}

// ---------------------------------------------------------------------------
// Scene management
// ---------------------------------------------------------------------------

impl SynthApp {
    /// Snapshot the complete rig state into a `Scene`.
    pub(crate) fn capture_scene(&self) -> scene::Scene {
        // Save the current track's live state first (same as patch capture).
        let mut track_patches = self.track_patches.clone();
        track_patches[self.focused_track] = self.capture_patch();

        scene::Scene {
            name: self.scene_name.clone(),
            global_bpm: self.global_bpm,
            track_names: self.track_names.clone(),
            track_patches,
            track_volumes: std::array::from_fn(|t| self.track_mixer[t].volume()),
            track_pans: std::array::from_fn(|t| self.track_mixer[t].pan()),
            track_muted: std::array::from_fn(|t| self.track_mixer[t].muted()),
            drums: self.drums.clone(),
            track_key_lo: self.track_key_lo,
            track_key_hi: self.track_key_hi,
            track_midi_ch: self.track_midi_ch,
        }
    }

    /// Restore a complete rig state from a `Scene`.
    pub(crate) fn load_scene(&mut self, s: scene::Scene) {
        self.scene_name = s.name.clone();
        self.global_bpm = s.global_bpm;
        self.track_names = s.track_names.clone();

        // Push mixer state to atomics.
        for t in 0..TRACK_COUNT {
            self.track_mixer[t].set_volume(s.track_volumes[t]);
            self.track_mixer[t].set_pan(s.track_pans[t]);
            self.track_mixer[t].set_muted(s.track_muted[t]);
        }

        // Push each track's patch to its engine (no notes-off: tracks keep playing).
        for t in 0..TRACK_COUNT {
            self.track_engines[t].apply_patch(&s.track_patches[t]);
        }

        // Store patches and update UI mirrors for the focused track.
        self.track_patches = s.track_patches.clone();
        let focused_patch = self.track_patches[self.focused_track].clone();
        self.apply_ui_mirrors_only(focused_patch);
        self.apply_clock_sync();

        self.drums = s.drums;
        self.track_key_lo = s.track_key_lo;
        self.track_key_hi = s.track_key_hi;
        self.track_midi_ch = s.track_midi_ch;
    }
}

// ---------------------------------------------------------------------------
// Layout B — zone UI methods
// ---------------------------------------------------------------------------

impl SynthApp {
    /// Zone 1: global bar — mode toggle, BPM, patch name, transport, settings.
    fn ui_global_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // ── Mode toggle: STUDIO | DRUM MACHINE | LIVE ─────────────────
            #[cfg(feature = "live_rig")]
            let mode_entries: &[(AppMode, &str, &str)] = &[
                (AppMode::Studio, "STUDIO", "Single-synth deep editing."),
                (
                    AppMode::DrumMachine,
                    "DRUMS",
                    "Drum machine — step grid + voice editor.",
                ),
                (AppMode::Live, "LIVE", "Rig performance view."),
            ];
            #[cfg(not(feature = "live_rig"))]
            let mode_entries: &[(AppMode, &str, &str)] =
                &[(AppMode::Studio, "STUDIO", "Single-synth deep editing.")];
            for (mode, label, hover) in mode_entries.iter().copied() {
                let active = self.app_mode == mode;
                let col = if active {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.text_secondary)
                };
                if ui
                    .add(egui::SelectableLabel::new(
                        active,
                        egui::RichText::new(label).size(11.0).color(col),
                    ))
                    .on_hover_text(hover)
                    .clicked()
                {
                    self.app_mode = mode;
                }
            }

            ui.separator();

            // ── BPM ───────────────────────────────────────────────────────
            ui.label(
                egui::RichText::new("BPM")
                    .size(11.0)
                    .color(self.theme.c(&self.theme.text_secondary)),
            );
            if ui
                .add(
                    egui::DragValue::new(&mut self.global_bpm)
                        .range(40..=600)
                        .speed(0.5),
                )
                .on_hover_text("Master tempo (40–600 BPM). Drag or scroll.")
                .changed()
            {
                self.apply_clock_sync();
            }

            // ── Sync controls ─────────────────────────────────────────────
            let sync_col = if self.global_sync {
                self.theme.c(&self.theme.accent)
            } else {
                self.theme.c(&self.theme.text_disabled)
            };
            if ui
                .add(egui::SelectableLabel::new(
                    self.global_sync,
                    egui::RichText::new("SYNC").size(11.0).color(sync_col),
                ))
                .on_hover_text(
                    "Force all components (Seq, Arp, Walker, Delay) to follow Global BPM.",
                )
                .clicked()
            {
                self.global_sync = !self.global_sync;
                if self.global_sync {
                    self.apply_clock_sync();
                    self.sync_transport_now();
                } else {
                    self.seq.arp_restart.store(false, Ordering::Relaxed);
                    self.seq.walker_restart.store(false, Ordering::Relaxed);
                }
            }

            let any_sync = self.global_sync || self.seq_sync || self.arp_sync || self.walker_sync;
            ui.add_enabled_ui(any_sync, |ui| {
                let bq = self.seq.bar_quantize.load(Ordering::Relaxed);
                let bq_col = if bq {
                    self.theme.c(&self.theme.accent_dim)
                } else {
                    self.theme.c(&self.theme.text_disabled)
                };
                if ui
                    .add(egui::SelectableLabel::new(
                        bq,
                        egui::RichText::new("BAR").size(11.0).color(bq_col),
                    ))
                    .on_hover_text("Quantise Arp/Walker restart to next bar boundary.")
                    .clicked()
                {
                    self.seq.bar_quantize.store(!bq, Ordering::Relaxed);
                }
            });

            // ── BPM display + beat indicator ──────────────────────────────
            // Clicking opens/closes the metronome window.
            {
                let seq_playing = self.seq.playing.load(Ordering::Relaxed);
                let drums_running = self.drum_engine.enabled.load(Ordering::Relaxed);
                let metro_active = self.metro_enabled
                    || self.seq_pending_start
                    || self.arp_pending_start
                    || seq_playing
                    || drums_running;
                let beat_idx = self.metro_phase as usize;
                let beat_frac = self.metro_phase.fract() as f32;

                // Accent dot pulses on beat 1; beat dot pulses on beats 2+.
                let accent_t = if metro_active && beat_idx == 0 {
                    (1.0_f32 - beat_frac).powf(2.2)
                } else {
                    0.0
                };
                let beat_t = if metro_active && beat_idx > 0 {
                    (1.0_f32 - beat_frac).powf(2.2)
                } else {
                    0.0
                };

                const DOT_R: f32 = 3.5;
                // Fixed layout: 30px BPM text + 5px gap + dot + 4px gap + dot
                let total_w = 30.0 + 5.0 + DOT_R * 2.0 + 4.0 + DOT_R * 2.0;
                let (rect, resp) = ui.allocate_exact_size(
                    egui::Vec2::new(total_w, ui.available_height()),
                    egui::Sense::click(),
                );
                if resp.clicked() {
                    self.show_metronome = !self.show_metronome;
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                resp.on_hover_text("Click to open metronome / time signature settings.");

                if ui.is_rect_visible(rect) {
                    let painter = ui.painter();
                    let cy = rect.center().y;

                    // Time signature label (e.g. "4/4")
                    let sig_col = if self.show_metronome {
                        self.theme.c(&self.theme.accent)
                    } else {
                        self.theme.c(&self.theme.text_secondary)
                    };
                    painter.text(
                        egui::Pos2::new(rect.left() + 15.0, cy),
                        egui::Align2::CENTER_CENTER,
                        format!("{}/{}", self.metro_beats, self.metro_denom),
                        egui::FontId::monospace(10.0),
                        sig_col,
                    );

                    // Helper: lerp between two Color32s
                    let lerp_col = |a: egui::Color32, b: egui::Color32, t: f32| {
                        let t = t.clamp(0.0, 1.0);
                        egui::Color32::from_rgb(
                            (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
                            (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
                            (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
                        )
                    };

                    // Accent dot (beat 1) — accent colour
                    let accent_full = self.theme.c(&self.theme.accent);
                    let accent_dim = egui::Color32::from_rgb(
                        (accent_full.r() as f32 * 0.18) as u8,
                        (accent_full.g() as f32 * 0.18) as u8,
                        (accent_full.b() as f32 * 0.18) as u8,
                    );
                    let dot1_x = rect.left() + 30.0 + 5.0 + DOT_R;
                    painter.circle_filled(
                        egui::Pos2::new(dot1_x, cy),
                        DOT_R,
                        lerp_col(accent_dim, accent_full, accent_t),
                    );

                    // Beat dot (beats 2+) — cool blue
                    let beat_full = egui::Color32::from_rgb(100, 170, 220);
                    let beat_dim = egui::Color32::from_rgb(15, 30, 45);
                    let dot2_x = dot1_x + DOT_R * 2.0 + 4.0;
                    painter.circle_filled(
                        egui::Pos2::new(dot2_x, cy),
                        DOT_R,
                        lerp_col(beat_dim, beat_full, beat_t),
                    );
                }
            }

            // ── STOP ─────────────────────────────────────────────────────
            if ui
                .add(egui::Button::new(
                    egui::RichText::new("■")
                        .size(13.0)
                        .color(egui::Color32::from_rgb(220, 80, 70)),
                ))
                .on_hover_text(
                    "Panic stop — silence all voices, stop sequencer / arp / walker / drums, clear frozen notes and flush FX tails.",
                )
                .clicked()
            {
                self.stop_all();
            }

            ui.separator();

            // ── Track breadcrumb ──────────────────────────────────────────
            if self.app_mode != AppMode::DrumMachine {
                let crumb = format!(
                    "T{}  {}  ·  {}",
                    self.focused_track + 1,
                    self.track_names[self.focused_track],
                    self.patch_name,
                );
                ui.label(
                    egui::RichText::new(crumb)
                        .size(11.0)
                        .color(self.theme.c(&self.theme.text_secondary)),
                );
            }

            // ── Right-aligned items ───────────────────────────────────────
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Settings menu — flat single-level list to avoid submenu overlap issues.
                ui.menu_button(egui::RichText::new("⚙").size(14.0), |ui| {
                    ui.set_min_width(160.0);

                    // ── Patch ──────────────────────────────────────────────
                    ui.label(egui::RichText::new("PATCH").small().weak());
                    if ui.button("New Patch").clicked() {
                        self.patch_name = "Init".into();
                        ui.close_menu();
                    }
                    if ui.button("Save Patch…").clicked() {
                        let p = self.capture_patch();
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name(&format!("{}.json", p.name))
                            .add_filter("Patch", &["json"])
                            .save_file()
                        {
                            if let Ok(json) = serde_json::to_string_pretty(&p) {
                                let _ = std::fs::write(path, json);
                            }
                        }
                        ui.close_menu();
                    }
                    if ui.button("Load Patch…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Patch", &["json"])
                            .pick_file()
                        {
                            if let Ok(json) = std::fs::read_to_string(path) {
                                if let Ok(p) = serde_json::from_str::<patch::Patch>(&json) {
                                    self.apply_patch(p);
                                }
                            }
                        }
                        ui.close_menu();
                    }

                    ui.separator();

                    // ── Theme ──────────────────────────────────────────────
                    ui.label(egui::RichText::new("THEME").small().weak());
                    for t in ui::theme::builtin_themes() {
                        if ui
                            .selectable_label(self.theme.name == t.name, &t.name)
                            .clicked()
                        {
                            self.theme = t;
                            ui.close_menu();
                        }
                    }

                    ui.separator();

                    // ── View ───────────────────────────────────────────────
                    ui.label(egui::RichText::new("VIEW").small().weak());
                    for &tab in ui::dock::Tab::ALL {
                        let open = self.dock_state.find_tab(&tab).is_some();
                        if ui.selectable_label(open, tab.title()).clicked() {
                            if open {
                                self.dock_state
                                    .remove_tab(self.dock_state.find_tab(&tab).unwrap());
                            } else {
                                self.dock_state.push_to_focused_leaf(tab);
                            }
                            ui.close_menu();
                        }
                    }
                    if ui.button("Reset Layout").clicked() {
                        self.reset_layout_pending = true;
                        ui.close_menu();
                    }

                    ui.separator();

                    // ── Transport ──────────────────────────────────────────
                    if ui
                        .button("Sync Now")
                        .on_hover_text("Reset phases for sequencer, arpeggiator, and walker.")
                        .clicked()
                    {
                        self.apply_clock_sync();
                        self.sync_transport_now();
                        ui.close_menu();
                    }
                });

                // Metronome toggle button
                let metro_col = if self.show_metronome {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.text_secondary)
                };
                if ui
                    .button(egui::RichText::new("♩").size(11.0).color(metro_col))
                    .on_hover_text(
                        "Metronome — visual beat indicator with configurable time signature.",
                    )
                    .clicked()
                {
                    self.show_metronome = !self.show_metronome;
                }

                // Patch library button — direct access, no submenu navigation needed
                let lib_col = if self.patch_browser_open {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.text_secondary)
                };
                if ui
                    .button(egui::RichText::new("LIB").size(11.0).color(lib_col))
                    .on_hover_text("Patch Library — browse and load factory patches.")
                    .clicked()
                {
                    self.patch_browser_open = !self.patch_browser_open;
                }

                // Scene browser button
                let scene_col = if self.scene_browser_open {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.text_secondary)
                };
                if ui
                    .button(egui::RichText::new("SCENE").size(11.0).color(scene_col))
                    .on_hover_text("Scene manager — save and load complete rig states.")
                    .clicked()
                {
                    self.scene_browser_open = !self.scene_browser_open;
                }

                ui.separator();

                // Latency / CPU indicator
                ui::scope::draw_latency_bar(
                    ui,
                    &self.engine,
                    self.engine.amp_attack(),
                    &self.theme,
                );

                ui.separator();

                // Record button
                let is_recording = self
                    .recorder_sink
                    .lock()
                    .map(|g| g.is_some())
                    .unwrap_or(false);
                if is_recording {
                    let stop_label = egui::RichText::new("■ REC")
                        .size(11.0)
                        .color(egui::Color32::from_rgb(220, 60, 60));
                    if ui
                        .button(stop_label)
                        .on_hover_text("Stop recording and save WAV file.")
                        .clicked()
                    {
                        if let Ok(mut guard) = self.recorder_sink.lock() {
                            if let Some(rec) = guard.take() {
                                let path = rec.path.clone();
                                match rec.stop() {
                                    Ok(()) => eprintln!("Recording saved: {path}"),
                                    Err(e) => eprintln!("Recording stop error: {e}"),
                                }
                            }
                        }
                    }
                } else {
                    let rec_label = egui::RichText::new("⏺ REC")
                        .size(11.0)
                        .color(self.theme.c(&self.theme.text_secondary));
                    if ui
                        .button(rec_label)
                        .on_hover_text("Record stereo output to WAV.")
                        .clicked()
                    {
                        let sr = self.engine.sample_rate();
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("Save recording as")
                            .set_file_name("recording.wav")
                            .add_filter("WAV audio", &["wav"])
                            .save_file()
                        {
                            let path_str = path.to_string_lossy().into_owned();
                            match recorder::Recorder::start(path_str, sr) {
                                Ok(rec) => {
                                    if let Ok(mut guard) = self.recorder_sink.lock() {
                                        *guard = Some(rec);
                                    }
                                }
                                Err(e) => eprintln!("Failed to start recording: {e}"),
                            }
                        }
                    }
                }

                ui.separator();

                // Global volume
                ui.label(
                    egui::RichText::new("VOL")
                        .size(10.0)
                        .color(self.theme.c(&self.theme.text_disabled)),
                );
                let mut global_vol = self.engine.global_volume();
                if ui
                    .add(
                        egui::DragValue::new(&mut global_vol)
                            .range(0.0_f32..=1.0)
                            .speed(0.005)
                            .fixed_decimals(2),
                    )
                    .on_hover_text("Global output volume — applied after all FX.")
                    .changed()
                {
                    self.engine.set_global_volume(global_vol);
                }

                ui.separator();

                // Patch name
                ui.add(
                    egui::TextEdit::singleline(&mut self.patch_name)
                        .desired_width(100.0)
                        .font(egui::TextStyle::Monospace),
                );
                ui.label(
                    egui::RichText::new("PATCH")
                        .size(10.0)
                        .color(self.theme.c(&self.theme.text_disabled)),
                );
            });
        });
    }

    /// Zone 5a: FX mini strip — always-visible compact FX toggle row.
    fn ui_fx_mini_strip(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("FX")
                    .size(10.0)
                    .color(self.theme.c(&self.theme.text_disabled)),
            );
            ui.separator();

            macro_rules! fx_chip {
                ($ui:expr, $label:expr, $on:expr, $color:expr, $toggle:expr) => {{
                    let col = if $on {
                        self.theme.c(&$color)
                    } else {
                        self.theme.c(&self.theme.text_disabled)
                    };
                    if $ui
                        .add(
                            egui::Button::new(egui::RichText::new($label).size(11.0).color(col))
                                .frame($on),
                        )
                        .clicked()
                    {
                        $toggle;
                    }
                }};
            }

            let on = self.fx_overdrive_on;
            fx_chip!(ui, "OD", on, self.theme.fx_overdrive, {
                self.fx_overdrive_on = !on;
                self.engine
                    .set_fx_overdrive_mix(if !on { self.fx_overdrive_mix } else { 0.0 });
            });

            let on = self.fx_distortion_on;
            fx_chip!(ui, "DIST", on, self.theme.fx_distortion, {
                self.fx_distortion_on = !on;
                self.engine
                    .set_fx_distortion_mix(if !on { self.fx_distortion_mix } else { 0.0 });
            });

            let on = self.fx_chorus_on;
            fx_chip!(ui, "CHOR", on, self.theme.fx_chorus, {
                self.fx_chorus_on = !on;
                self.engine
                    .set_fx_chorus_mix(if !on { self.fx_chorus_mix } else { 0.0 });
            });

            let on = self.fx_delay_on;
            fx_chip!(ui, "DLY", on, self.theme.fx_delay, {
                self.fx_delay_on = !on;
                self.engine
                    .set_fx_delay_mix(if !on { self.fx_delay_mix } else { 0.0 });
            });

            let on = self.fx_reverb_on;
            fx_chip!(ui, "REV", on, self.theme.fx_reverb, {
                self.fx_reverb_on = !on;
                self.engine
                    .set_fx_reverb_mix(if !on { self.fx_reverb_mix } else { 0.0 });
            });

            let on = self.fx_shimmer_on;
            fx_chip!(ui, "SHIM", on, self.theme.fx_shimmer, {
                self.fx_shimmer_on = !on;
                self.engine
                    .set_shimmer_amount(if !on { self.fx_shimmer_amt } else { 0.0 });
                self.engine
                    .set_shimmer_mix(if !on { self.fx_shimmer_mix } else { 0.0 });
            });

            let on = self.fx_crystal_on;
            fx_chip!(ui, "CRYST", on, self.theme.fx_crystallizer, {
                self.fx_crystal_on = !on;
                self.engine
                    .set_crystal_mix(if !on { self.fx_crystal_mix } else { 0.0 });
            });
        });
    }
}
