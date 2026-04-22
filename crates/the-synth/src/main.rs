//! The Synth — unified MiniMoog-style synthesizer
//! Run with: cargo run

#![allow(clippy::precedence)]

mod audio;
mod patch;
mod recorder;
mod sequencer;
mod ui;

use audio::{AudioEngine, AudioState};
use synth_control::midi::{MidiEngine, MidiEvent};
use synth_control::{ControlEvent, ControlSender};
use patch::{Patch, default_patches};
use eframe::egui;
use sequencer::{
    ChordKbState, SequencerHandle, spawn_sequencer,
};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use ui::layout::{AppMode, StudioTab};
use ui::frame::SynthFrame;

fn main() -> eframe::Result {
    let recorder_sink = Arc::new(Mutex::new(None));
    let engine = AudioEngine::new(Arc::clone(&recorder_sink)).expect("Failed to start audio");
    let state = Arc::clone(&engine.state);
    let control_tx = engine.control_tx.clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 860.0])
            .with_title("The Synth"),
        ..Default::default()
    };

    eframe::run_native(
        "The Synth",
        options,
        Box::new(move |_cc| Ok(Box::new(SynthApp::new(state, engine, control_tx, recorder_sink)))),
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
    pub(crate) state: Arc<AudioState>,
    pub(crate) midi: MidiEngine,
    pub(crate) control: ControlSender,
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

    // Noise
    pub(crate) noise_vol: f32,

    // LFO 1
    pub(crate) lfo_enabled: bool,
    pub(crate) lfo_rate: f32,
    pub(crate) lfo_depth: f32,
    pub(crate) lfo_shape: usize,    // 0=sin 1=tri 2=saw
    pub(crate) lfo_dest: usize,     // 0=pitch 1=filter 2=amp
    pub(crate) lfo_sync: bool,
    pub(crate) lfo_division: usize,

    // LFO 2
    pub(crate) lfo2_enabled: bool,
    pub(crate) lfo2_rate: f32,
    pub(crate) lfo2_depth: f32,
    pub(crate) lfo2_shape: usize,
    pub(crate) lfo2_dest: usize,

    pub(crate) filter_enabled: bool,

    // Filter
    pub(crate) filter_cutoff: f32,
    pub(crate) filter_q: f32,
    pub(crate) filter_env_amount: f32,
    pub(crate) fenv_adsr: [f32; 4],

    // Amp ADSR
    pub(crate) amp_adsr: [f32; 4],

    // Glide + master
    pub(crate) glide_time: f32,
    pub(crate) master_vol: f32,  // OSC mix level — pre-FX
    pub(crate) global_vol: f32,  // Final output — post all FX

    // Keyboard
    pub(crate) piano_octave: i32,
    pub(crate) piano_held_midi: std::collections::HashSet<u8>,
    pub(crate) piano_mouse_midi: Option<u8>,
    pub(crate) kb_chord_mode: bool,  // true = chord pads, false = standard piano
    /// When true, NoteOffs are suppressed; notes keep sounding until a new chord/note is played.
    pub(crate) kb_freeze: bool,
    /// MIDI notes currently sustained by freeze (key lifted but NoteOff suppressed).
    pub(crate) frozen_notes: std::collections::HashSet<u8>,

    // Peak meter
    pub(crate) peak_display: f32,
    pub(crate) peak_hold: f32,
    pub(crate) peak_hold_timer: f32,

    // Limiter
    pub(crate) limiter_enabled: bool,
    pub(crate) limiter_threshold: f32,
    pub(crate) window_focused: bool,

    // Global tempo / sync
    pub(crate) global_bpm: u32,     // master tempo — source of truth when components are synced
    pub(crate) global_sync: bool,   // when true, all components are forced to BPM sync
    pub(crate) arp_sync: bool,      // per-component sync toggle for arpeggiator
    pub(crate) walker_sync: bool,   // per-component sync toggle for scale walker
    pub(crate) seq_sync: bool,      // per-component sync toggle for sequencer

    // Sequencer — handle shared with the dedicated sequencer thread
    pub(crate) seq: Arc<SequencerHandle>,

    // Sequencer — chord keyboard (live, not threaded)
    pub(crate) chord_kb: ChordKbState,

    // Arpeggiator UI state
    pub(crate) arp_bpm: f32,
    pub(crate) arp_mode: u8,
    pub(crate) arp_division: u8,
    pub(crate) arp_octave_range: u8,
    pub(crate) arp_gate: f32,

    // Scale walker UI state
    pub(crate) walker_scale: u8,
    pub(crate) walker_root: u8,
    pub(crate) walker_oct: u8,
    pub(crate) walker_division: u8,
    pub(crate) walker_gate: f32,
    pub(crate) walker_bpm: f32,

    // Oscilloscope
    pub(crate) scope_height: f32,
    pub(crate) scope_x_scale: f32,
    pub(crate) scope_y_scale: f32,

    // Patch system
    pub(crate) patch_name: String,
    pub(crate) patch_library: Vec<Patch>,
    pub(crate) patch_browser_open: bool,
    pub(crate) patch_browser_category: String,
    pub(crate) patch_browser_model: String,
    pub(crate) patch_load_fx: bool, // if false, loading a patch leaves the FX chain untouched

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
    pub(crate) fx_delay_sync: bool,        // if true, delay_time is derived from BPM
    pub(crate) fx_delay_division: usize,   // index into DELAY_DIVISIONS
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

    /// Shared WAV recorder sink — `Some` while recording, `None` otherwise.
    pub(crate) recorder_sink: Arc<Mutex<Option<recorder::Recorder>>>,
}

impl SynthApp {
    fn new(state: Arc<AudioState>, audio: AudioEngine, control: ControlSender, recorder_sink: Arc<Mutex<Option<recorder::Recorder>>>) -> Self {
        let mut midi = MidiEngine::new();
        midi.list_ports(); // populate port list at startup

        // Restore persisted layout (theme + panel visibility).
        let saved = ui::layout::load_layout();
        let theme = ui::theme::builtin_themes()
            .into_iter()
            .find(|t| t.name == saved.theme_name)
            .unwrap_or_else(ui::theme::midnight);
        let panels = PanelVisibility::from_state(&saved.panels);
        let app_mode   = saved.app_mode;
        let studio_tab = saved.studio_tab;

        let seq = Arc::new(SequencerHandle::new());
        spawn_sequencer(
            Arc::clone(&seq),
            control.clone(),
            Arc::clone(&state.note_on_time),
        );

        Self {
            _audio: audio,
            state,
            midi,
            control,
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
            noise_vol: 0.0,
            lfo_enabled: false,
            lfo_rate: 2.0,
            lfo_depth: 0.0,
            lfo_shape: 0,
            lfo_dest: 1,
            lfo_sync: false,
            lfo_division: 4,
            lfo2_enabled: false,
            lfo2_rate: 0.3,
            lfo2_depth: 0.0,
            lfo2_shape: 0,
            lfo2_dest: 2, // amp (tremolo)
            filter_enabled: true,
            filter_cutoff: 3000.0,
            filter_q: 0.3,
            filter_env_amount: 0.3,
            fenv_adsr: [0.01, 0.3, 0.0, 0.2],
            amp_adsr: [0.01, 0.15, 0.7, 0.4],
            glide_time: 0.0,
            master_vol: 0.8,
            global_vol: 0.8,
            piano_octave: 4,
            kb_chord_mode: false,
            kb_freeze: false,
            frozen_notes: std::collections::HashSet::new(),
            piano_held_midi: std::collections::HashSet::new(),
            piano_mouse_midi: None,
            peak_display: 0.0,
            peak_hold: 0.0,
            peak_hold_timer: 0.0,
            limiter_enabled: true,
            limiter_threshold: 0.95,
            window_focused: true,
            global_bpm: 120,
            global_sync: false,
            arp_sync: true,
            walker_sync: true,
            seq_sync: true,
            seq,
            chord_kb: ChordKbState::new(),
            arp_bpm: 120.0,
            arp_mode: 0,
            arp_division: 1, // 1/8 default
            arp_octave_range: 1,
            arp_gate: 0.7,
            walker_scale: 0,
            walker_root: 60,
            walker_oct: 2,
            walker_division: 1,
            walker_gate: 0.6,
            walker_bpm: 120.0,
            scope_height: 140.0,
            scope_x_scale: 1.0,
            scope_y_scale: 2.5,
            patch_name: "Init".into(),
            patch_library: default_patches(),
            patch_browser_open: false,
            patch_browser_category: "All".into(),
            patch_browser_model: "All".into(),
            patch_load_fx: false,
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
        let _ = self.control.try_send(ControlEvent::ArpRestart { track: 0 });
        let _ = self.control.try_send(ControlEvent::WalkerRestart { track: 0 });
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
        if self.arp_sync_active() && bar_quantize && playing {
            self.seq.arp_restart.store(true, Ordering::Relaxed);
        } else {
            let _ = self.control.try_send(ControlEvent::ArpRestart { track: 0 });
        }
    }

    pub(crate) fn schedule_or_restart_walker(&mut self) {
        let playing = self.seq.playing.load(Ordering::Relaxed);
        let bar_quantize = self.seq.bar_quantize.load(Ordering::Relaxed);
        if self.walker_sync_active() && bar_quantize && playing {
            self.seq.walker_restart.store(true, Ordering::Relaxed);
        } else {
            let _ = self.control.try_send(ControlEvent::WalkerRestart { track: 0 });
        }
    }

    pub(crate) fn apply_clock_sync(&mut self) {
        let global = self.global_bpm as f32;
        if self.seq_sync_active() {
            self.seq.bpm.store(self.global_bpm, Ordering::Relaxed);
        }
        if self.arp_sync_active() && (self.arp_bpm - global).abs() > f32::EPSILON {
            self.arp_bpm = global;
            self.state.arp.bpm.set(global);
        }
        if self.walker_sync_active() && (self.walker_bpm - global).abs() > f32::EPSILON {
            self.walker_bpm = global;
            self.state.walker.bpm.set(global);
        }
        if self.lfo_sync_active() {
            let rate = ui::modulation::lfo_synced_rate(global, self.lfo_division);
            if (self.lfo_rate - rate).abs() > f32::EPSILON {
                self.lfo_rate = rate;
                self.state.lfo_rate.set(rate);
            }
        }
    }

    /// Push a NoteOn event into the audio thread's control queue.
    pub(crate) fn push_note_on(&mut self, midi: u8) {
        if let Ok(mut t) = self.state.note_on_time.lock() {
            *t = Some(std::time::Instant::now());
        }
        let _ = self.control.try_send(ControlEvent::NoteOn { pitch: midi, velocity: 100, track: 0 });
    }

    /// Push a NoteOff event into the audio thread's control queue.
    pub(crate) fn push_note_off(&mut self, midi: u8) {
        let _ = self.control.try_send(ControlEvent::NoteOff { pitch: midi, track: 0 });
    }

    /// Silence all voices, reset all FX tails, and clear all note-tracking state.
    pub(crate) fn all_notes_off(&mut self) {
        // Zero all voice gates — kills currently-playing voices via ADSR release
        for g in &self.state.voice_gates { g.set(0.0); }
        // Send NoteOff for every MIDI note that might be held
        let held: Vec<u8> = self.piano_held_midi.drain().collect();
        for n in held { self.push_note_off(n); }
        // Signal sequencer thread to stop; it will NoteOff its prev_notes itself.
        self.seq.playing.store(false, Ordering::Relaxed);
        let frozen: Vec<u8> = self.frozen_notes.drain().collect();
        for n in frozen { self.push_note_off(n); }
        self.chord_kb.held_pad = None;
        self.chord_kb.kb_held.clear();
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
                MidiEvent::NoteOn { note, velocity, .. } => {
                    let _ = velocity;
                    self.push_note_on(note);
                }
                MidiEvent::NoteOff { note, .. } => {
                    self.push_note_off(note);
                }
                MidiEvent::CC { cc, value, .. } => {
                    let v = value as f32 / 127.0;
                    match cc {
                        1  => { // Mod wheel → LFO depth
                            self.lfo_depth = v;
                            self.state.lfo_depth.set(v);
                        }
                        7  => { // Volume → master vol
                            self.master_vol = v;
                            self.state.master_vol.set(v);
                        }
                        71 => { // Resonance
                            let q = v * 0.95;
                            self.filter_q = q;
                            self.state.resonance.set(q);
                        }
                        74 => { // Cutoff (brightness)
                            let hz = 80.0 * (18000.0_f32 / 80.0).powf(v);
                            self.filter_cutoff = hz;
                            self.state.cutoff.set(hz);
                        }
                        64 => { // Sustain pedal → freeze
                            let pedal_down = value >= 64;
                            if pedal_down && !self.kb_freeze {
                                self.kb_freeze = true;
                            } else if !pedal_down && self.kb_freeze {
                                self.kb_freeze = false;
                                let frozen: Vec<u8> = self.frozen_notes.drain().collect();
                                for n in frozen { self.push_note_off(n); }
                            }
                        }
                        _ => {}
                    }
                }
                MidiEvent::PitchBend { value, .. } => {
                    let semitones = value * 2.0;
                    self.state.lfo_pitch_mult.set(2_f32.powf(semitones / 12.0));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Main update
// ---------------------------------------------------------------------------

impl eframe::App for SynthApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let state = ui::layout::LayoutState {
            theme_name: self.theme.name.clone(),
            panels:     self.panels.to_state(),
            app_mode:   self.app_mode,
            studio_tab: self.studio_tab,
        };
        ui::layout::save_layout(&state);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
        self.tick_keyboard_input(ctx);

        // Patch browser is a floating window — must be shown before panels.
        self.ui_patch_browser(ctx);

        // ── Zone 1: global bar (top, always visible) ──────────────────────────
        egui::TopBottomPanel::top("global_bar")
            .frame(SynthFrame::bar(&self.theme))
            .show(ctx, |ui| { self.ui_global_bar(ui); });

        // ── Zone 5b: keyboard strip (bottom-most) ─────────────────────────────
        egui::TopBottomPanel::bottom("keyboard_strip")
            .frame(SynthFrame::transport(&self.theme))
            .show(ctx, |ui| { self.ui_keyboard_panel(ui); });

        // ── Zone 5a: FX mini strip (above keyboard, always visible) ───────────
        egui::TopBottomPanel::bottom("fx_mini_strip")
            .frame(SynthFrame::transport(&self.theme))
            .show(ctx, |ui| { self.ui_fx_mini_strip(ui); });

        // ── Zones 2 + 3: central editing area (dock in Studio, placeholder in Live) ──
        egui::CentralPanel::default()
            .frame(SynthFrame::app_bg(&self.theme))
            .show(ctx, |ui| {
                match self.app_mode {
                    AppMode::Studio => {
                        if self.reset_layout_pending {
                            self.dock_state = ui::dock::default_dock_state();
                            self.reset_layout_pending = false;
                        }
                        let mut dock_state = std::mem::replace(
                            &mut self.dock_state,
                            egui_dock::DockState::new(vec![]),
                        );
                        let mut dock_style = egui_dock::Style::from_egui(ui.style());
                        dock_style.separator.width = 6.0;
                        dock_style.separator.color_idle    = egui::Color32::TRANSPARENT;
                        dock_style.separator.color_hovered = egui::Color32::from_black_alpha(60);
                        dock_style.separator.color_dragged = egui::Color32::from_black_alpha(100);
                        dock_style.dock_area_padding = Some(egui::Margin::same(6.0));
                        let rm = self.theme.rounding_md;
                        let r_top = egui::Rounding { nw: rm, ne: rm, sw: 0.0, se: 0.0 };
                        let r_body = egui::Rounding { nw: 0.0, ne: rm, sw: rm, se: rm };
                        let bg_app     = self.theme.c(&self.theme.bg_app);
                        let bg_surface = self.theme.c(&self.theme.bg_surface);
                        let border     = self.theme.c(&self.theme.border);
                        let text_pri   = self.theme.c(&self.theme.text_primary);
                        let text_sec   = self.theme.c(&self.theme.text_secondary);
                        let accent     = self.theme.c(&self.theme.accent);
                        // Tab body: rounded corners + subtle border
                        dock_style.tab.tab_body.rounding = r_body;
                        dock_style.tab.tab_body.stroke   = egui::Stroke::new(self.theme.stroke_ui, border);
                        // Tab bar: transparent so the panel background shows through
                        dock_style.tab_bar.bg_fill     = egui::Color32::TRANSPARENT;
                        dock_style.tab_bar.hline_color = egui::Color32::TRANSPARENT;
                        dock_style.tab_bar.rounding    = r_body;
                        dock_style.tab_bar.height      = 28.0;
                        // Active tab: raised (bg_surface), top-only rounding, accent outline
                        dock_style.tab.active.rounding      = r_top;
                        dock_style.tab.active.bg_fill       = bg_surface;
                        dock_style.tab.active.text_color    = text_pri;
                        dock_style.tab.active.outline_color = accent;
                        // Focused: same as active with accent text
                        dock_style.tab.focused.rounding      = r_top;
                        dock_style.tab.focused.bg_fill       = bg_surface;
                        dock_style.tab.focused.text_color    = accent;
                        dock_style.tab.focused.outline_color = accent;
                        // Inactive: transparent, dimmed text
                        dock_style.tab.inactive.rounding      = r_top;
                        dock_style.tab.inactive.bg_fill       = egui::Color32::TRANSPARENT;
                        dock_style.tab.inactive.text_color    = text_sec;
                        dock_style.tab.inactive.outline_color = egui::Color32::TRANSPARENT;
                        // Hovered: slightly raised
                        dock_style.tab.hovered.rounding      = r_top;
                        dock_style.tab.hovered.bg_fill       = bg_surface;
                        dock_style.tab.hovered.text_color    = text_pri;
                        dock_style.tab.hovered.outline_color = border;
                        egui_dock::DockArea::new(&mut dock_state)
                            .style(dock_style)
                            .show_inside(ui, &mut ui::dock::SynthTabViewer { app: self });
                        self.dock_state = dock_state;
                    }
                    AppMode::Live => {
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                egui::RichText::new("LIVE MODE — coming soon.")
                                    .size(18.0)
                                    .color(self.theme.c(&self.theme.text_secondary)),
                            );
                        });
                    }
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
        Patch {
            name: self.patch_name.clone(),
            category: "User".into(),
            osc_wave:           self.osc_wave,
            osc_octave:         self.osc_octave,
            osc_detune:         self.osc_detune,
            osc_vol:            self.osc_vol,
            osc_enabled:        self.osc_enabled,
            osc_pulse_width:    self.osc_pulse_width,
            osc_pw_enabled:     self.osc_pw_enabled,
            osc_unison_enabled: self.osc_unison_enabled,
            osc_unison_count:   self.osc_unison_count,
            osc_unison_spread:  self.osc_unison_spread,
            hard_sync:          self.hard_sync,
            fm_enabled:         self.fm_enabled,
            fm_depth:           self.fm_depth,
            ring_enabled:       self.ring_enabled,
            ring_depth:         self.ring_depth,
            noise_vol:          self.noise_vol,
            lfo_enabled:        self.lfo_enabled,
            lfo_rate:           self.lfo_rate,
            lfo_depth:          self.lfo_depth,
            lfo_shape:          self.lfo_shape,
            lfo_dest:           self.lfo_dest,
            lfo_sync:           self.lfo_sync,
            lfo_division:       self.lfo_division,
            lfo2_enabled:       self.lfo2_enabled,
            lfo2_rate:          self.lfo2_rate,
            lfo2_depth:         self.lfo2_depth,
            lfo2_shape:         self.lfo2_shape,
            lfo2_dest:          self.lfo2_dest,
            filter_enabled:     self.filter_enabled,
            filter_cutoff:      self.filter_cutoff,
            filter_q:           self.filter_q,
            filter_env_amount:  self.filter_env_amount,
            fenv_adsr:          self.fenv_adsr,
            amp_adsr:           self.amp_adsr,
            glide_time:         self.glide_time,
            master_vol:         self.master_vol,
            global_vol:         self.global_vol,
            limiter_enabled:    self.limiter_enabled,
            limiter_threshold:  self.limiter_threshold,
            synth_model:        String::new(),
            fx_overdrive_on:    self.fx_overdrive_on,
            fx_overdrive_drive: self.fx_overdrive_drive,
            fx_overdrive_mix:   self.fx_overdrive_mix,
            fx_overdrive_tone:  self.fx_overdrive_tone,
            fx_overdrive_asym:  self.fx_overdrive_asym,
            fx_distortion_on:   self.fx_distortion_on,
            fx_distortion_drive: self.fx_distortion_drive,
            fx_distortion_mix:  self.fx_distortion_mix,
            fx_distortion_tone: self.fx_distortion_tone,
            fx_distortion_pre:  self.fx_distortion_pre,
            fx_chorus_on:       self.fx_chorus_on,
            fx_chorus_rate:     self.fx_chorus_rate,
            fx_chorus_depth:    self.fx_chorus_depth,
            fx_chorus_mix:      self.fx_chorus_mix,
            fx_delay_on:        self.fx_delay_on,
            fx_delay_time:      self.fx_delay_time,
            fx_delay_feedback:  self.fx_delay_feedback,
            fx_delay_mix:       self.fx_delay_mix,
            fx_delay_sync:      self.fx_delay_sync,
            fx_delay_division:  self.fx_delay_division,
            fx_reverb_on:       self.fx_reverb_on,
            fx_reverb_size:     self.fx_reverb_size,
            fx_reverb_damp:     self.fx_reverb_damp,
            fx_reverb_mix:      self.fx_reverb_mix,
            fx_reverb_predelay: self.fx_reverb_predelay,
            fx_reverb_type:     self.fx_reverb_type,
            stereo_spread: self.stereo_spread,
            stereo_width:  self.stereo_width,
            fx_shimmer_on:      self.fx_shimmer_on,
            fx_shimmer_size:    self.fx_shimmer_size,
            fx_shimmer_damp:    self.fx_shimmer_damp,
            fx_shimmer_mix:     self.fx_shimmer_mix,
            fx_shimmer_amt:     self.fx_shimmer_amt,
            fx_shimmer_width:   self.fx_shimmer_width,
            fx_shimmer_spread:  self.fx_shimmer_spread,
            fx_shimmer_pitch:   self.fx_shimmer_pitch,
            fx_crystal_on:      self.fx_crystal_on,
            fx_crystal_mix:     self.fx_crystal_mix,
            fx_crystal_grain_ms:self.fx_crystal_grain_ms,
            fx_crystal_scatter: self.fx_crystal_scatter,
            fx_crystal_feedback:self.fx_crystal_feedback,
            fx_crystal_delay_ms:self.fx_crystal_delay_ms,
            fx_crystal_pitch:   self.fx_crystal_pitch,
        }
    }

    pub(crate) fn apply_patch(&mut self, p: Patch) {
        // Silence all voices before changing parameters to prevent Moog filter blowup.
        self.all_notes_off();

        self.patch_name         = p.name;
        self.osc_wave           = p.osc_wave;
        self.osc_octave         = p.osc_octave;
        self.osc_detune         = p.osc_detune;
        self.osc_vol            = p.osc_vol;
        self.osc_enabled        = p.osc_enabled;
        self.osc_pulse_width    = p.osc_pulse_width;
        self.osc_pw_enabled     = p.osc_pw_enabled;
        self.osc_unison_enabled = p.osc_unison_enabled;
        self.osc_unison_count   = p.osc_unison_count;
        self.osc_unison_spread  = p.osc_unison_spread;
        self.hard_sync          = p.hard_sync;
        self.fm_enabled         = p.fm_enabled;
        self.fm_depth           = p.fm_depth;
        self.ring_enabled       = p.ring_enabled;
        self.ring_depth         = p.ring_depth;
        self.noise_vol          = p.noise_vol;
        self.lfo_enabled        = p.lfo_enabled;
        self.lfo_rate           = p.lfo_rate;
        self.lfo_depth          = p.lfo_depth;
        self.lfo_shape          = p.lfo_shape;
        self.lfo_dest           = p.lfo_dest;
        self.lfo_sync           = p.lfo_sync;
        self.lfo_division       = p.lfo_division;
        self.lfo2_enabled       = p.lfo2_enabled;
        self.lfo2_rate          = p.lfo2_rate;
        self.lfo2_depth         = p.lfo2_depth;
        self.lfo2_shape         = p.lfo2_shape;
        self.lfo2_dest          = p.lfo2_dest;
        self.filter_enabled     = p.filter_enabled;
        self.filter_cutoff      = p.filter_cutoff;
        self.filter_q           = p.filter_q;
        self.filter_env_amount  = p.filter_env_amount;
        self.fenv_adsr          = p.fenv_adsr;
        self.amp_adsr           = p.amp_adsr;
        self.glide_time         = p.glide_time;
        self.master_vol         = p.master_vol;
        self.global_vol         = p.global_vol;
        self.limiter_enabled    = p.limiter_enabled;
        self.limiter_threshold  = p.limiter_threshold;

        // Push everything to AudioState Shareds
        let s = &self.state;
        for i in 0..3 {
            s.osc_wave[i].store(self.osc_wave[i] as u8, std::sync::atomic::Ordering::Relaxed);
            s.osc_vol[i].set(if self.osc_enabled[i] { self.osc_vol[i] } else { 0.0 });
            s.osc_pulse_width[i].set(self.osc_pulse_width[i]);
            self.update_freq_mult(i);
            self.update_unison(i);
        }
        s.hard_sync_enabled.store(self.hard_sync, std::sync::atomic::Ordering::Relaxed);
        s.fm_depth.set(if self.fm_enabled { self.fm_depth } else { 0.0 });
        s.ring_depth.set(if self.ring_enabled { self.ring_depth } else { 0.0 });
        s.noise_vol.set(self.noise_vol);
        s.lfo_rate.set(self.lfo_rate);
        s.lfo_depth.set(if self.lfo_enabled { self.lfo_depth } else { 0.0 });
        s.lfo_shape.store(self.lfo_shape as u8, std::sync::atomic::Ordering::Relaxed);
        s.lfo_dest.store(self.lfo_dest as u8, std::sync::atomic::Ordering::Relaxed);
        s.lfo2_rate.set(self.lfo2_rate);
        s.lfo2_depth.set(if self.lfo2_enabled { self.lfo2_depth } else { 0.0 });
        s.lfo2_shape.store(self.lfo2_shape as u8, std::sync::atomic::Ordering::Relaxed);
        s.lfo2_dest.store(self.lfo2_dest as u8, std::sync::atomic::Ordering::Relaxed);
        s.cutoff.set(if self.filter_enabled { self.filter_cutoff } else { 18000.0 });
        s.resonance.set(if self.filter_enabled { self.filter_q } else { 0.0 });
        s.filter_env_amount.set(self.filter_env_amount);
        s.fenv_attack.set(self.fenv_adsr[0]);
        s.fenv_decay.set(self.fenv_adsr[1]);
        s.fenv_sustain.set(self.fenv_adsr[2]);
        s.fenv_release.set(self.fenv_adsr[3]);
        s.adsr_attack.set(self.amp_adsr[0]);
        s.adsr_decay.set(self.amp_adsr[1]);
        s.adsr_sustain.set(self.amp_adsr[2]);
        s.adsr_release.set(self.amp_adsr[3]);
        s.glide_time.set(self.glide_time);
        s.master_vol.set(self.master_vol);
        s.global_vol.set(self.global_vol);
        s.limiter_enabled.store(self.limiter_enabled, std::sync::atomic::Ordering::Relaxed);
        s.limiter_threshold.set(self.limiter_threshold);

        // FX chain — only applied when "Load FX" is enabled in the patch browser
        if self.patch_load_fx {
            self.fx_overdrive_on    = p.fx_overdrive_on;
            self.fx_overdrive_drive = p.fx_overdrive_drive;
            self.fx_overdrive_mix   = p.fx_overdrive_mix;
            self.fx_overdrive_tone  = p.fx_overdrive_tone;
            self.fx_overdrive_asym  = p.fx_overdrive_asym;
            self.fx_distortion_on   = p.fx_distortion_on;
            self.fx_distortion_drive = p.fx_distortion_drive;
            self.fx_distortion_mix  = p.fx_distortion_mix;
            self.fx_distortion_tone = p.fx_distortion_tone;
            self.fx_distortion_pre  = p.fx_distortion_pre;
            self.fx_chorus_on       = p.fx_chorus_on;
            self.fx_chorus_rate     = p.fx_chorus_rate;
            self.fx_chorus_depth    = p.fx_chorus_depth;
            self.fx_chorus_mix      = p.fx_chorus_mix;
            self.fx_delay_on        = p.fx_delay_on;
            self.fx_delay_time      = p.fx_delay_time;
            self.fx_delay_feedback  = p.fx_delay_feedback;
            self.fx_delay_mix       = p.fx_delay_mix;
            self.fx_delay_sync      = p.fx_delay_sync;
            self.fx_delay_division  = p.fx_delay_division;
            self.fx_reverb_on       = p.fx_reverb_on;
            self.fx_reverb_size     = p.fx_reverb_size;
            self.fx_reverb_damp     = p.fx_reverb_damp;
            self.fx_reverb_mix      = p.fx_reverb_mix;
            self.fx_reverb_predelay = p.fx_reverb_predelay;
            self.fx_reverb_type     = p.fx_reverb_type;
            self.stereo_spread = p.stereo_spread;
            self.stereo_width  = p.stereo_width;
            self.fx_shimmer_on      = p.fx_shimmer_on;
            self.fx_shimmer_size    = p.fx_shimmer_size;
            self.fx_shimmer_damp    = p.fx_shimmer_damp;
            self.fx_shimmer_mix     = p.fx_shimmer_mix;
            self.fx_shimmer_amt     = p.fx_shimmer_amt;
            self.fx_shimmer_width   = p.fx_shimmer_width;
            self.fx_shimmer_spread  = p.fx_shimmer_spread;
            self.fx_shimmer_pitch   = p.fx_shimmer_pitch;
            self.fx_crystal_on      = p.fx_crystal_on;
            self.fx_crystal_mix     = p.fx_crystal_mix;
            self.fx_crystal_grain_ms = p.fx_crystal_grain_ms;
            self.fx_crystal_scatter = p.fx_crystal_scatter;
            self.fx_crystal_feedback = p.fx_crystal_feedback;
            self.fx_crystal_delay_ms = p.fx_crystal_delay_ms;
            self.fx_crystal_pitch   = p.fx_crystal_pitch;
            s.fx_overdrive_drive.set_value(self.fx_overdrive_drive);
            s.fx_overdrive_mix.set_value(if self.fx_overdrive_on { self.fx_overdrive_mix } else { 0.0 });
            s.fx_overdrive_tone.set_value(self.fx_overdrive_tone);
            s.fx_overdrive_asym.set_value(self.fx_overdrive_asym);
            s.fx_distortion_drive.set_value(self.fx_distortion_drive);
            s.fx_distortion_mix.set_value(if self.fx_distortion_on { self.fx_distortion_mix } else { 0.0 });
            s.fx_distortion_tone.set_value(self.fx_distortion_tone);
            s.fx_distortion_pre.set_value(self.fx_distortion_pre);
            s.fx_chorus_rate.set_value(self.fx_chorus_rate);
            s.fx_chorus_depth.set_value(self.fx_chorus_depth);
            s.fx_chorus_mix.set_value(if self.fx_chorus_on { self.fx_chorus_mix } else { 0.0 });
            s.fx_delay_time.set_value(self.fx_delay_time);
            s.fx_delay_feedback.set_value(self.fx_delay_feedback);
            s.fx_delay_mix.set_value(if self.fx_delay_on { self.fx_delay_mix } else { 0.0 });
            s.fx_reverb_size.set_value(self.fx_reverb_size);
            s.fx_reverb_damp.set_value(self.fx_reverb_damp);
            s.fx_reverb_mix.set_value(if self.fx_reverb_on { self.fx_reverb_mix } else { 0.0 });
            s.fx_reverb_predelay.set_value(self.fx_reverb_predelay);
            s.fx_reverb_type.store(self.fx_reverb_type, std::sync::atomic::Ordering::Relaxed);
            s.stereo_spread.set_value(self.stereo_spread);
            s.stereo_width.set_value(self.stereo_width);
            s.fx_shimmer.size.set_value(self.fx_shimmer_size);
            s.fx_shimmer.damp.set_value(self.fx_shimmer_damp);
            s.fx_shimmer.shimmer.set_value(if self.fx_shimmer_on { self.fx_shimmer_amt } else { 0.0 });
            s.fx_shimmer.width.set_value(self.fx_shimmer_width);
            s.fx_shimmer.spread.set_value(self.fx_shimmer_spread);
            s.fx_shimmer.pitch.store(self.fx_shimmer_pitch, std::sync::atomic::Ordering::Relaxed);
            s.fx_shimmer.mix.set_value(if self.fx_shimmer_on { self.fx_shimmer_mix } else { 0.0 });
            s.fx_crystal.grain_ms.set_value(self.fx_crystal_grain_ms);
            s.fx_crystal.scatter.set_value(self.fx_crystal_scatter);
            s.fx_crystal.feedback.set_value(self.fx_crystal_feedback);
            s.fx_crystal.delay_ms.set_value(self.fx_crystal_delay_ms);
            s.fx_crystal.pitch.store(self.fx_crystal_pitch, std::sync::atomic::Ordering::Relaxed);
            s.fx_crystal.mix.set_value(if self.fx_crystal_on { self.fx_crystal_mix } else { 0.0 });
            // Propagate delay sync state (reads fx_delay_sync / fx_delay_division)
            self.apply_clock_sync();
        }
    }
}

// ---------------------------------------------------------------------------
// Layout B — zone UI methods
// ---------------------------------------------------------------------------

impl SynthApp {
    /// Zone 1: global bar — mode toggle, BPM, patch name, transport, settings.
    fn ui_global_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // ── Mode toggle ───────────────────────────────────────────────
            let studio_active = self.app_mode == AppMode::Studio;
            let live_active   = self.app_mode == AppMode::Live;

            let studio_col = if studio_active { self.theme.c(&self.theme.accent) }
                             else             { self.theme.c(&self.theme.text_secondary) };
            let live_col   = if live_active   { self.theme.c(&self.theme.accent) }
                             else             { self.theme.c(&self.theme.text_secondary) };

            if ui.add(egui::SelectableLabel::new(
                studio_active,
                egui::RichText::new("STUDIO").size(11.0).color(studio_col),
            )).clicked() {
                self.app_mode = AppMode::Studio;
            }
            if ui.add(egui::SelectableLabel::new(
                live_active,
                egui::RichText::new("LIVE").size(11.0).color(live_col),
            )).on_hover_text("Live performance view — coming soon.")
              .clicked()
            {
                self.app_mode = AppMode::Live;
            }

            ui.separator();

            // ── BPM ───────────────────────────────────────────────────────
            ui.label(
                egui::RichText::new("BPM")
                    .size(11.0)
                    .color(self.theme.c(&self.theme.text_secondary)),
            );
            if ui.add(
                egui::DragValue::new(&mut self.global_bpm)
                    .range(40..=600)
                    .speed(0.5),
            ).on_hover_text("Master tempo (40–600 BPM). Drag or scroll.")
             .changed()
            {
                self.apply_clock_sync();
            }

            // ── Sync controls ─────────────────────────────────────────────
            let sync_col = if self.global_sync { self.theme.c(&self.theme.accent) }
                           else                { self.theme.c(&self.theme.text_disabled) };
            if ui.add(egui::SelectableLabel::new(
                self.global_sync,
                egui::RichText::new("SYNC").size(11.0).color(sync_col),
            )).on_hover_text("Force all components (Seq, Arp, Walker, Delay) to follow Global BPM.")
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
                let bq_col = if bq { self.theme.c(&self.theme.accent_dim) }
                             else  { self.theme.c(&self.theme.text_disabled) };
                if ui.add(egui::SelectableLabel::new(
                    bq,
                    egui::RichText::new("BAR").size(11.0).color(bq_col),
                )).on_hover_text("Quantise Arp/Walker restart to next bar boundary.")
                  .clicked()
                {
                    self.seq.bar_quantize.store(!bq, Ordering::Relaxed);
                }
            });

            ui.separator();

            // ── Right-aligned items ───────────────────────────────────────
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Settings menu (File + Theme)
                ui.menu_button(egui::RichText::new("⚙").size(14.0), |ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("New Patch").clicked() {
                            self.patch_name = "Init".into();
                            ui.close_menu();
                        }
                        ui.separator();
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
                        if ui.button("Browse Library…").clicked() {
                            self.patch_browser_open = !self.patch_browser_open;
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("Theme", |ui| {
                        for t in ui::theme::builtin_themes() {
                            if ui.selectable_label(self.theme.name == t.name, &t.name).clicked() {
                                self.theme = t;
                                ui.close_menu();
                            }
                        }
                    });

                    ui.menu_button("View", |ui| {
                        for &tab in ui::dock::Tab::ALL {
                            let open = self.dock_state.find_tab(&tab).is_some();
                            if ui.selectable_label(open, tab.title()).clicked() {
                                if open {
                                    self.dock_state.remove_tab(
                                        self.dock_state.find_tab(&tab).unwrap(),
                                    );
                                } else {
                                    self.dock_state.push_to_focused_leaf(tab);
                                }
                                ui.close_menu();
                            }
                        }
                        ui.separator();
                        if ui.button("Reset Layout").clicked() {
                            self.reset_layout_pending = true;
                            ui.close_menu();
                        }
                    });

                    ui.separator();
                    if ui.button("Sync Now")
                       .on_hover_text("Reset phases for sequencer, arpeggiator, and walker.")
                       .clicked()
                    {
                        self.apply_clock_sync();
                        self.sync_transport_now();
                        ui.close_menu();
                    }
                });

                ui.separator();

                // Latency / CPU indicator
                ui::scope::draw_latency_bar(ui, &self.state, self.amp_adsr[0], &self.theme);

                ui.separator();

                // Record button
                let is_recording = self.recorder_sink.lock().map(|g| g.is_some()).unwrap_or(false);
                if is_recording {
                    let stop_label = egui::RichText::new("■ REC")
                        .size(11.0)
                        .color(egui::Color32::from_rgb(220, 60, 60));
                    if ui.button(stop_label)
                       .on_hover_text("Stop recording and save WAV file.")
                       .clicked()
                    {
                        if let Ok(mut guard) = self.recorder_sink.lock() {
                            if let Some(rec) = guard.take() {
                                let path = rec.path.clone();
                                match rec.stop() {
                                    Ok(())  => eprintln!("Recording saved: {path}"),
                                    Err(e)  => eprintln!("Recording stop error: {e}"),
                                }
                            }
                        }
                    }
                } else {
                    let rec_label = egui::RichText::new("⏺").size(13.0)
                        .color(self.theme.c(&self.theme.text_secondary));
                    if ui.button(rec_label)
                       .on_hover_text("Record stereo output to WAV.")
                       .clicked()
                    {
                        let sr = self.state.sample_rate.load(std::sync::atomic::Ordering::Relaxed);
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
                if ui.add(
                    egui::DragValue::new(&mut self.global_vol)
                        .range(0.0_f32..=1.0)
                        .speed(0.005)
                        .fixed_decimals(2),
                ).on_hover_text("Global output volume — applied after all FX.")
                 .changed()
                {
                    self.state.global_vol.set(self.global_vol);
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
                        .add(egui::Button::new(egui::RichText::new($label).size(11.0).color(col)).frame($on))
                        .clicked()
                    {
                        $toggle;
                    }
                }};
            }

            let on = self.fx_overdrive_on;
            fx_chip!(ui, "OD", on, self.theme.fx_overdrive, {
                self.fx_overdrive_on = !on;
                self.state.fx_overdrive_mix.set_value(if !on { self.fx_overdrive_mix } else { 0.0 });
            });

            let on = self.fx_distortion_on;
            fx_chip!(ui, "DIST", on, self.theme.fx_distortion, {
                self.fx_distortion_on = !on;
                self.state.fx_distortion_mix.set_value(if !on { self.fx_distortion_mix } else { 0.0 });
            });

            let on = self.fx_chorus_on;
            fx_chip!(ui, "CHOR", on, self.theme.fx_chorus, {
                self.fx_chorus_on = !on;
                self.state.fx_chorus_mix.set_value(if !on { self.fx_chorus_mix } else { 0.0 });
            });

            let on = self.fx_delay_on;
            fx_chip!(ui, "DLY", on, self.theme.fx_delay, {
                self.fx_delay_on = !on;
                self.state.fx_delay_mix.set_value(if !on { self.fx_delay_mix } else { 0.0 });
            });

            let on = self.fx_reverb_on;
            fx_chip!(ui, "REV", on, self.theme.fx_reverb, {
                self.fx_reverb_on = !on;
                self.state.fx_reverb_mix.set_value(if !on { self.fx_reverb_mix } else { 0.0 });
            });

            let on = self.fx_shimmer_on;
            fx_chip!(ui, "SHIM", on, self.theme.fx_shimmer, {
                self.fx_shimmer_on = !on;
                self.state.fx_shimmer.shimmer.set_value(if !on { self.fx_shimmer_amt } else { 0.0 });
                self.state.fx_shimmer.mix.set_value(if !on { self.fx_shimmer_mix } else { 0.0 });
            });

            let on = self.fx_crystal_on;
            fx_chip!(ui, "CRYST", on, self.theme.fx_crystallizer, {
                self.fx_crystal_on = !on;
                self.state.fx_crystal.mix.set_value(if !on { self.fx_crystal_mix } else { 0.0 });
            });
        });
    }

}
