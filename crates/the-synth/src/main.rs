//! The Synth — unified MiniMoog-style synthesizer
//! Run with: cargo run

#![allow(clippy::precedence)]

mod audio;
mod patch;
mod sequencer;
mod ui;

use audio::{AudioEngine, AudioState};
use synth_control::midi::{MidiEngine, MidiEvent};
use synth_control::{ControlEvent, ControlSender};
use patch::{Patch, default_patches};
use eframe::egui;
use sequencer::{
    ChordKbState, ChordSeqState, NoteSeqState, SeqMode,
};
use std::sync::Arc;

fn main() -> eframe::Result {
    let engine = AudioEngine::new().expect("Failed to start audio");
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
        Box::new(move |_cc| Ok(Box::new(SynthApp::new(state, engine, control_tx)))),
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

pub(crate) struct SynthApp {
    pub(crate) _audio: AudioEngine, // keeps cpal stream alive
    pub(crate) state: Arc<AudioState>,
    pub(crate) midi: MidiEngine,
    pub(crate) control: ControlSender,
    pub(crate) theme: ui::theme::SynthTheme,
    pub(crate) panels: PanelVisibility,

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

    // Noise
    pub(crate) noise_vol: f32,

    // LFO
    pub(crate) lfo_enabled: bool,
    pub(crate) lfo_rate: f32,
    pub(crate) lfo_depth: f32,
    pub(crate) lfo_shape: usize, // 0=sin 1=tri 2=saw
    pub(crate) lfo_dest: usize,  // 0=pitch 1=filter 2=amp

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
    pub(crate) master_vol: f32,

    // Keyboard
    pub(crate) piano_octave: i32,
    pub(crate) piano_held_midi: std::collections::HashSet<u8>,
    pub(crate) piano_mouse_midi: Option<u8>,

    // Peak meter
    pub(crate) peak_display: f32,
    pub(crate) peak_hold: f32,
    pub(crate) peak_hold_timer: f32,

    // Limiter
    pub(crate) limiter_enabled: bool,
    pub(crate) limiter_threshold: f32,

    // Sequencer — shared timing
    pub(crate) seq_playing: bool,
    pub(crate) seq_bpm: u32,
    pub(crate) seq_clock_sync: bool,
    pub(crate) seq_bar_quantize_start: bool,
    pub(crate) arp_restart_pending: bool,
    pub(crate) walker_restart_pending: bool,
    pub(crate) seq_current_step: usize,
    pub(crate) seq_last_tick: std::time::Instant,
    pub(crate) seq_prev_notes: Vec<u8>, // notes playing from last step (supports chords)

    // Sequencer — mode + per-mode state
    pub(crate) seq_mode: SeqMode,
    pub(crate) note_seq: NoteSeqState,
    pub(crate) chord_seq: ChordSeqState,
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
}

impl SynthApp {
    fn new(state: Arc<AudioState>, audio: AudioEngine, control: ControlSender) -> Self {
        let mut midi = MidiEngine::new();
        midi.list_ports(); // populate port list at startup
        Self {
            _audio: audio,
            state,
            midi,
            control,
            theme: ui::theme::midnight(),
            panels: PanelVisibility::default(),
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
            noise_vol: 0.0,
            lfo_enabled: false,
            lfo_rate: 2.0,
            lfo_depth: 0.0,
            lfo_shape: 0,
            lfo_dest: 1,
            filter_enabled: true,
            filter_cutoff: 3000.0,
            filter_q: 0.3,
            filter_env_amount: 0.3,
            fenv_adsr: [0.01, 0.3, 0.0, 0.2],
            amp_adsr: [0.01, 0.15, 0.7, 0.4],
            glide_time: 0.0,
            master_vol: 0.8,
            piano_octave: 4,
            piano_held_midi: std::collections::HashSet::new(),
            piano_mouse_midi: None,
            peak_display: 0.0,
            peak_hold: 0.0,
            peak_hold_timer: 0.0,
            limiter_enabled: true,
            limiter_threshold: 0.95,
            seq_playing: false,
            seq_bpm: 120,
            seq_clock_sync: true,
            seq_bar_quantize_start: false,
            arp_restart_pending: false,
            walker_restart_pending: false,
            seq_current_step: 0,
            seq_last_tick: std::time::Instant::now(),
            seq_prev_notes: Vec::new(),
            seq_mode: SeqMode::NoteSeq,
            note_seq: NoteSeqState::new(),
            chord_seq: ChordSeqState::new(),
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
        }
    }
}

// ---------------------------------------------------------------------------
// Voice management
// ---------------------------------------------------------------------------

impl SynthApp {
    pub(crate) fn sync_transport_now(&mut self) {
        self.seq_current_step = 0;
        self.seq_last_tick = std::time::Instant::now();
        self.arp_restart_pending = false;
        self.walker_restart_pending = false;

        let prev: Vec<u8> = self.seq_prev_notes.drain(..).collect();
        for m in prev {
            self.push_note_off(m);
        }

        let _ = self.control.try_send(ControlEvent::ArpRestart { track: 0 });
        let _ = self.control.try_send(ControlEvent::WalkerRestart { track: 0 });
    }

    pub(crate) fn schedule_or_restart_arp(&mut self) {
        if self.seq_clock_sync && self.seq_bar_quantize_start && self.seq_playing {
            self.arp_restart_pending = true;
        } else {
            let _ = self.control.try_send(ControlEvent::ArpRestart { track: 0 });
        }
    }

    pub(crate) fn schedule_or_restart_walker(&mut self) {
        if self.seq_clock_sync && self.seq_bar_quantize_start && self.seq_playing {
            self.walker_restart_pending = true;
        } else {
            let _ = self.control.try_send(ControlEvent::WalkerRestart { track: 0 });
        }
    }

    pub(crate) fn apply_clock_sync(&mut self) {
        if !self.seq_clock_sync {
            return;
        }
        let bpm = self.seq_bpm as f32;
        if (self.arp_bpm - bpm).abs() > f32::EPSILON {
            self.arp_bpm = bpm;
            self.state.arp.bpm.set(bpm);
        }
        if (self.walker_bpm - bpm).abs() > f32::EPSILON {
            self.walker_bpm = bpm;
            self.state.walker.bpm.set(bpm);
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
                        64 => { // Sustain pedal
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
// Sequencer tick
// ---------------------------------------------------------------------------

impl SynthApp {
    fn tick_sequencer(&mut self, ctx: &egui::Context) {
        if !self.seq_playing {
            return;
        }
        let step_dur = std::time::Duration::from_millis(60_000 / self.seq_bpm as u64 / 2);
        if self.seq_last_tick.elapsed() < step_dur {
            return;
        }
        self.seq_last_tick = std::time::Instant::now();

        let prev: Vec<u8> = self.seq_prev_notes.drain(..).collect();
        for m in prev { self.push_note_off(m); }

        let seq_length = match self.seq_mode {
            SeqMode::NoteSeq  => self.note_seq.length,
            SeqMode::ChordSeq => self.chord_seq.length,
            SeqMode::ChordKb  => return,
        };
        self.seq_current_step = (self.seq_current_step + 1) % seq_length;
        let bar_boundary = self.seq_current_step == 0;

        if self.arp_restart_pending && bar_boundary {
            let _ = self.control.try_send(ControlEvent::ArpRestart { track: 0 });
            self.arp_restart_pending = false;
        }
        if self.walker_restart_pending && bar_boundary {
            let _ = self.control.try_send(ControlEvent::WalkerRestart { track: 0 });
            self.walker_restart_pending = false;
        }

        let notes_to_play: Vec<u8> = match self.seq_mode {
            SeqMode::NoteSeq => {
                let i = self.seq_current_step;
                if self.note_seq.steps[i] { vec![self.note_seq.notes[i]] } else { vec![] }
            }
            SeqMode::ChordSeq => {
                let i = self.seq_current_step;
                if self.chord_seq.steps[i] {
                    self.chord_seq.step_notes(i).to_vec()
                } else {
                    vec![]
                }
            }
            SeqMode::ChordKb => vec![],
        };

        for m in notes_to_play {
            self.push_note_on(m);
            self.seq_prev_notes.push(m);
        }
        ctx.request_repaint_after(step_dur);
    }
}

// ---------------------------------------------------------------------------
// Main update
// ---------------------------------------------------------------------------

impl eframe::App for SynthApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick_midi();
        self.apply_clock_sync();
        self.tick_sequencer(ctx);

        self.ui_patch_browser(ctx);

        // --- Menu bar ---
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                // File menu
                ui.menu_button("File", |ui| {
                    if ui.button("New Patch").clicked() {
                        self.patch_name = "Init".into();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Save Patch...").clicked() {
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
                    if ui.button("Load Patch...").clicked() {
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
                    if ui.button("Browse Library...").clicked() {
                        self.patch_browser_open = !self.patch_browser_open;
                        ui.close_menu();
                    }
                });

                // View menu — panel toggles
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.panels.oscillators, "Oscillators");
                    ui.checkbox(&mut self.panels.modulation, "Modulation & Filter");
                    ui.checkbox(&mut self.panels.keyboard, "Keyboard");
                    ui.checkbox(&mut self.panels.sequencer, "Sequencer");
                    ui.checkbox(&mut self.panels.arp_walker, "Arpeggiator & Walker");
                    ui.checkbox(&mut self.panels.fx_chain, "FX Chain");
                    ui.checkbox(&mut self.panels.scope, "Oscilloscope");
                    ui.checkbox(&mut self.panels.midi, "MIDI & Latency");
                    ui.separator();
                    if ui.button("Show All").clicked() {
                        self.panels = PanelVisibility::default();
                        ui.close_menu();
                    }
                });

                // Theme menu
                ui.menu_button("Theme", |ui| {
                    for t in ui::theme::builtin_themes() {
                        if ui.selectable_label(self.theme.name == t.name, &t.name).clicked() {
                            self.theme = t;
                            ui.close_menu();
                        }
                    }
                });

                // MIDI in menu bar
                ui.menu_button("MIDI", |ui| {
                    self.ui_midi_panel(ui);
                });

                // Right-aligned patch name + latency
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui::scope::draw_latency_bar(ui, &self.state, self.amp_adsr[0], &self.theme);
                    ui.separator();
                    ui.add(egui::TextEdit::singleline(&mut self.patch_name).desired_width(100.0));
                });
            });
        });

        // --- Central panel ---
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {

            if self.panels.oscillators {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("OSCILLATORS").strong().small());
                });
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.columns(4, |cols| {
                        self.ui_osc_panel(&mut cols[0], 0);
                        self.ui_osc_panel(&mut cols[1], 1);
                        self.ui_osc_panel(&mut cols[2], 2);
                        self.ui_mixer_panel(&mut cols[3]);
                    });
                });
                ui.add_space(4.0);
            }

            if self.panels.modulation {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("MODULATION & FILTER").strong().small());
                });
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.columns(4, |cols| {
                        self.ui_lfo_panel(&mut cols[0]);
                        self.ui_filter_panel(&mut cols[1]);
                        self.ui_adsr_panel(&mut cols[2], "Filter Env", &mut [0usize, 1, 2, 3], true);
                        self.ui_adsr_panel(&mut cols[3], "Amp Env", &mut [0usize, 1, 2, 3], false);
                    });
                });
                ui.add_space(4.0);
            }

            if self.panels.keyboard {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("KEYBOARD").strong().small());
                });
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    self.ui_keyboard_panel(ui);
                });
                ui.add_space(4.0);
            }

            if self.panels.sequencer {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("SEQUENCER").strong().small());
                });
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    self.ui_sequencer_panel(ui);
                });
                ui.add_space(4.0);
            }

            if self.panels.arp_walker {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("ARPEGGIATOR & SCALE WALKER").strong().small());
                });
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.columns(2, |cols| {
                        self.ui_arp_panel(&mut cols[0]);
                        self.ui_walker_panel(&mut cols[1]);
                    });
                });
                ui.add_space(4.0);
            }

            if self.panels.fx_chain {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("FX CHAIN").strong().small());
                });
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    self.ui_fx_chain(ui);
                });
                ui.add_space(4.0);
            }

            if self.panels.midi {
                ui.horizontal(|ui| {
                    self.ui_midi_panel(ui);
                    ui.separator();
                    ui::scope::draw_latency_bar(ui, &self.state, self.amp_adsr[0], &self.theme);
                });
            }

            if self.panels.scope {
                self.ui_oscilloscope(ui);
            }

            }); // end ScrollArea
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
            filter_enabled:     self.filter_enabled,
            filter_cutoff:      self.filter_cutoff,
            filter_q:           self.filter_q,
            filter_env_amount:  self.filter_env_amount,
            fenv_adsr:          self.fenv_adsr,
            amp_adsr:           self.amp_adsr,
            glide_time:         self.glide_time,
            master_vol:         self.master_vol,
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
            fx_reverb_on:       self.fx_reverb_on,
            fx_reverb_size:     self.fx_reverb_size,
            fx_reverb_damp:     self.fx_reverb_damp,
            fx_reverb_mix:      self.fx_reverb_mix,
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
        self.filter_enabled     = p.filter_enabled;
        self.filter_cutoff      = p.filter_cutoff;
        self.filter_q           = p.filter_q;
        self.filter_env_amount  = p.filter_env_amount;
        self.fenv_adsr          = p.fenv_adsr;
        self.amp_adsr           = p.amp_adsr;
        self.glide_time         = p.glide_time;
        self.master_vol         = p.master_vol;

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
            self.fx_reverb_on       = p.fx_reverb_on;
            self.fx_reverb_size     = p.fx_reverb_size;
            self.fx_reverb_damp     = p.fx_reverb_damp;
            self.fx_reverb_mix      = p.fx_reverb_mix;
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
        }
    }
}
