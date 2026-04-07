//! The Synth — unified MiniMoog-style synthesizer
//! Run with: cargo run

#![allow(clippy::precedence)]

mod audio;
mod patch;
mod sequencer;

use audio::{AudioEngine, AudioState};
use synth_control::midi::{MidiEngine, MidiEvent};
use synth_control::{ControlEvent, ControlSender};
use patch::{Patch, default_patches};
use eframe::egui;
use egui::{Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use sequencer::{
    ChordKbState, ChordSeqState, NoteSeqState, SeqMode, ScaleType,
    NOTE_NAMES, DEGREE_LABELS, chord_name, chord_quality,
};
use std::sync::atomic::Ordering;
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

struct SynthApp {
    _audio: AudioEngine, // keeps cpal stream alive
    state: Arc<AudioState>,
    midi: MidiEngine,
    control: ControlSender,

    // OSC bank
    osc_wave: [usize; 3], // 0=sine 1=saw 2=square 3=triangle
    osc_octave: [i32; 3], // -2..+2
    osc_detune: [f32; 3], // -100..+100 cents
    osc_vol: [f32; 3],
    osc_enabled: [bool; 3],
    osc_pulse_width: [f32; 3],
    osc_pw_enabled: [bool; 3],
    osc_unison_enabled: [bool; 3],
    osc_unison_count: [usize; 3], // 2..5
    osc_unison_spread: [f32; 3],  // 0..50 cents total
    hard_sync: bool,              // OSC 1 → OSC 2 hard sync
    fm_enabled: bool,             // OSC 2 → OSC 1 frequency modulation
    fm_depth: f32,                // FM depth (0 = off, ~1 = strong)
    ring_enabled: bool,           // ring modulation OSC 1 × OSC 2
    ring_depth: f32,              // ring mod depth

    // Noise
    noise_vol: f32,

    // LFO
    lfo_enabled: bool,
    lfo_rate: f32,
    lfo_depth: f32,
    lfo_shape: usize, // 0=sin 1=tri 2=saw
    lfo_dest: usize,  // 0=pitch 1=filter 2=amp

    filter_enabled: bool,

    // Filter
    filter_cutoff: f32,
    filter_q: f32,
    filter_env_amount: f32,
    fenv_adsr: [f32; 4],

    // Amp ADSR
    amp_adsr: [f32; 4],

    // Glide + master
    glide_time: f32,
    master_vol: f32,

    // Keyboard
    piano_octave: i32,
    piano_held_midi: std::collections::HashSet<u8>,
    piano_mouse_midi: Option<u8>,

    // Peak meter
    peak_display: f32,
    peak_hold: f32,
    peak_hold_timer: f32,

    // Limiter
    limiter_enabled: bool,
    limiter_threshold: f32,

    // Sequencer — shared timing
    seq_playing: bool,
    seq_bpm: u32,
    seq_current_step: usize,
    seq_last_tick: std::time::Instant,
    seq_prev_notes: Vec<u8>, // notes playing from last step (supports chords)

    // Sequencer — mode + per-mode state
    seq_mode: SeqMode,
    note_seq: NoteSeqState,
    chord_seq: ChordSeqState,
    chord_kb: ChordKbState,

    // Oscilloscope
    scope_height: f32,
    scope_x_scale: f32,
    scope_y_scale: f32,

    // Patch system
    patch_name: String,
    patch_library: Vec<Patch>,
    patch_browser_open: bool,
    patch_browser_category: String,
    patch_browser_model: String,
    patch_load_fx: bool, // if false, loading a patch leaves the FX chain untouched

    // FX chain — per-effect enable + saved mix value
    fx_overdrive_on: bool,
    fx_overdrive_drive: f32,
    fx_overdrive_mix: f32,
    fx_overdrive_tone: f32,
    fx_overdrive_asym: f32,
    fx_distortion_on: bool,
    fx_distortion_drive: f32,
    fx_distortion_mix: f32,
    fx_distortion_tone: f32,
    fx_distortion_pre: f32,
    fx_chorus_on: bool,
    fx_chorus_rate: f32,
    fx_chorus_depth: f32,
    fx_chorus_mix: f32,
    fx_delay_on: bool,
    fx_delay_time: f32,
    fx_delay_feedback: f32,
    fx_delay_mix: f32,
    fx_delay_sync: bool,        // if true, delay_time is derived from BPM
    fx_delay_division: usize,   // index into DELAY_DIVISIONS
    fx_reverb_on: bool,
    fx_reverb_size: f32,
    fx_reverb_damp: f32,
    fx_reverb_mix: f32,
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
            master_vol: 0.5,
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
            seq_current_step: 0,
            seq_last_tick: std::time::Instant::now(),
            seq_prev_notes: Vec::new(),
            seq_mode: SeqMode::NoteSeq,
            note_seq: NoteSeqState::new(),
            chord_seq: ChordSeqState::new(),
            chord_kb: ChordKbState::new(),
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
        }
    }
}

// ---------------------------------------------------------------------------
// Voice management
// ---------------------------------------------------------------------------

impl SynthApp {
    /// Push a NoteOn event into the audio thread's control queue.
    fn push_note_on(&mut self, midi: u8) {
        // Stamp the time before enqueuing — audio callback measures how long it takes
        // to consume this event (round-trip latency indicator).
        if let Ok(mut t) = self.state.note_on_time.lock() {
            *t = Some(std::time::Instant::now());
        }
        let _ = self.control.try_send(ControlEvent::NoteOn { pitch: midi, velocity: 100, track: 0 });
    }

    /// Push a NoteOff event into the audio thread's control queue.
    fn push_note_off(&mut self, midi: u8) {
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
                    // Scale velocity to master volume modulation is left for later.
                    // For now just trigger the note.
                    let _ = velocity;
                    self.push_note_on(note);
                }
                MidiEvent::NoteOff { note, .. } => {
                    self.push_note_off(note);
                }
                MidiEvent::CC { cc, value, .. } => {
                    // Normalised 0..1 value for most CCs
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
                        64 => { // Sustain pedal — hold all notes (simple: ignore for now)
                        }
                        _ => {}
                    }
                }
                MidiEvent::PitchBend { value, .. } => {
                    // ±2 semitones pitch bend — applied as LFO pitch mult for simplicity
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

        // Release all notes from the previous step
        let prev: Vec<u8> = self.seq_prev_notes.drain(..).collect();
        for m in prev { self.push_note_off(m); }

        let seq_length = match self.seq_mode {
            SeqMode::NoteSeq  => self.note_seq.length,
            SeqMode::ChordSeq => self.chord_seq.length,
            SeqMode::ChordKb  => return, // ChordKb has no sequencer tick
        };
        self.seq_current_step = (self.seq_current_step + 1) % seq_length;

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
        self.tick_sequencer(ctx);

        self.ui_patch_browser(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            // Patch bar
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| { self.ui_patch_bar(ui); });
            });

            ui.add_space(2.0);

            // Row 1: OSC bank + mixer
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

            // Row 2: LFO + Filter + Filter ADSR + Amp ADSR
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

            // Row 3: Keyboard
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("KEYBOARD").strong().small());
            });
            egui::Frame::group(ui.style()).show(ui, |ui| {
                self.ui_keyboard_panel(ui);
            });

            ui.add_space(4.0);

            // Row 4: Sequencer (full width)
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("SEQUENCER").strong().small());
            });
            egui::Frame::group(ui.style()).show(ui, |ui| {
                self.ui_sequencer_panel(ui);
            });

            ui.add_space(4.0);

            // Row 5: FX Chain
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("FX CHAIN").strong().small());
            });
            egui::Frame::group(ui.style()).show(ui, |ui| {
                self.ui_fx_chain(ui);
            });

            ui.add_space(4.0);

            // MIDI + Latency row
            ui.horizontal(|ui| {
                self.ui_midi_panel(ui);
                ui.separator();
                draw_latency_bar(ui, &self.state, self.amp_adsr[0]);
            });

            // Oscilloscope footer
            self.ui_oscilloscope(ui);
        });

        ctx.request_repaint();
    }
}

// ---------------------------------------------------------------------------
// OSC panel
// ---------------------------------------------------------------------------

const WAVE_LABELS: &[&str] = &["Sin", "Saw", "Sqr", "Tri"];

/// Delay note divisions: (label, beats relative to a quarter-note pulse).
/// beats = 1.0 → quarter note, 0.5 → eighth note, etc.
const DELAY_DIVISIONS: &[(&str, f32)] = &[
    ("1/1",  4.0),
    ("1/2",  2.0),
    ("1/4",  1.0),
    ("1/8",  0.5),
    ("1/16", 0.25),
    ("3/8",  1.5),  // dotted quarter
    ("3/16", 0.75), // dotted eighth
];

impl SynthApp {
    fn ui_osc_panel(&mut self, ui: &mut egui::Ui, i: usize) {
        // Header: label + on/off toggle
        ui.horizontal(|ui| {
            let label = egui::RichText::new(format!("OSC {}", i + 1)).strong();
            let on = self.osc_enabled[i];
            let text = if on {
                label.color(Color32::from_rgb(0, 220, 160))
            } else {
                label.color(Color32::GRAY)
            };
            if ui.button(text).on_hover_text("Toggle this oscillator on/off").clicked() {
                self.osc_enabled[i] = !on;
                let vol = if self.osc_enabled[i] { self.osc_vol[i] } else { 0.0 };
                self.state.osc_vol[i].set(vol);
            }
        });

        // Controls greyed out when disabled
        ui.add_enabled_ui(self.osc_enabled[i], |ui| {
            // Waveform selector
            ui.horizontal(|ui| {
                let tips = [
                    "Sine — pure tone, no harmonics. Smooth and soft.",
                    "Sawtooth — all harmonics, bright buzz. Classic for brass and strings.",
                    "Square — odd harmonics only, hollow and woody. Supports pulse width.",
                    "Triangle — odd harmonics, softer than square. Alias-free.",
                ];
                for (w, &label) in WAVE_LABELS.iter().enumerate() {
                    if ui.selectable_label(self.osc_wave[i] == w, label)
                        .on_hover_text(tips[w])
                        .clicked()
                    {
                        self.osc_wave[i] = w;
                        self.state.osc_wave[i].store(w as u8, Ordering::Relaxed);
                    }
                }
            });

            // Octave
            ui.horizontal(|ui| {
                ui.label("Oct:").on_hover_text("Shift pitch in octave steps relative to the played note (−2 to +2).");
                if ui.small_button("−").on_hover_text("One octave down").clicked() && self.osc_octave[i] > -2 {
                    self.osc_octave[i] -= 1;
                    self.update_freq_mult(i);
                }
                ui.label(format!("{:+}", self.osc_octave[i]));
                if ui.small_button("+").on_hover_text("One octave up").clicked() && self.osc_octave[i] < 2 {
                    self.osc_octave[i] += 1;
                    self.update_freq_mult(i);
                }
            });

            // Detune
            ui.horizontal(|ui| {
                ui.label("Det:").on_hover_text("Fine-tune pitch in cents (1/100 of a semitone). ±100 ¢ = ±1 semitone.");
                if ui
                    .add(
                        egui::Slider::new(&mut self.osc_detune[i], -100.0..=100.0)
                            .text("¢")
                            .fixed_decimals(0),
                    )
                    .on_hover_text("Fine-tune pitch in cents. Use small values to fatten the sound when combined with another OSC.")
                    .changed()
                {
                    self.update_freq_mult(i);
                }
            });

            // Pulse width — only shown when Square is selected
            if self.osc_wave[i] == 2 {
                ui.horizontal(|ui| {
                    let pw_on = self.osc_pw_enabled[i];
                    let label = egui::RichText::new("PW").small().color(if pw_on {
                        Color32::from_rgb(0, 220, 160)
                    } else {
                        Color32::GRAY
                    });
                    if ui.button(label)
                        .on_hover_text("Pulse Width — vary the duty cycle of the square wave. 0.5 = standard square. Narrower = thinner, nasal tone.")
                        .clicked()
                    {
                        self.osc_pw_enabled[i] = !pw_on;
                        if !self.osc_pw_enabled[i] {
                            self.osc_pulse_width[i] = 0.5;
                            self.state.osc_pulse_width[i].set(0.5);
                        }
                    }
                    ui.add_enabled_ui(self.osc_pw_enabled[i], |ui| {
                        if ui
                            .add(
                                egui::Slider::new(&mut self.osc_pulse_width[i], 0.01..=0.99)
                                    .fixed_decimals(2),
                            )
                            .on_hover_text("Duty cycle: 0.5 = square, 0.1 = thin/nasal, 0.9 = thin/nasal (mirrored).")
                            .changed()
                        {
                            self.state.osc_pulse_width[i].set(self.osc_pulse_width[i]);
                        }
                    });
                });
            }

            // Unison
            ui.horizontal(|ui| {
                let uni_on = self.osc_unison_enabled[i];
                let label = egui::RichText::new("Uni").small().color(if uni_on {
                    Color32::from_rgb(0, 220, 160)
                } else {
                    Color32::GRAY
                });
                if ui.button(label)
                    .on_hover_text("Unison — stack multiple detuned copies of this oscillator for a thick, wide sound.")
                    .clicked()
                {
                    self.osc_unison_enabled[i] = !uni_on;
                    self.update_unison(i);
                }
                ui.add_enabled_ui(self.osc_unison_enabled[i], |ui| {
                    let mut changed = false;
                    changed |= ui
                        .add(egui::Slider::new(&mut self.osc_unison_count[i], 2..=5).text("v"))
                        .on_hover_text("Number of simultaneous copies (2–5). More copies = thicker sound.")
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut self.osc_unison_spread[i], 0.0..=50.0)
                                .text("¢")
                                .fixed_decimals(0),
                        )
                        .on_hover_text("Total pitch spread across all copies in cents. Higher = wider detune, more chorus effect.")
                        .changed();
                    if changed {
                        self.update_unison(i);
                    }
                });
            });

            // Hard sync, FM, Ring mod — only on OSC 1
            if i == 0 {
                ui.horizontal(|ui| {
                    let on = self.hard_sync;
                    let label = egui::RichText::new("Sync→2").small()
                        .color(if on { Color32::from_rgb(255, 180, 0) } else { Color32::GRAY });
                    if ui.button(label)
                        .on_hover_text("Hard Sync — OSC 1 resets OSC 2's phase on every cycle. Creates a complex, harmonically rich timbre. Sweep OSC 2's pitch for the classic sync sweep sound.")
                        .clicked()
                    {
                        self.hard_sync = !on;
                        self.state.hard_sync_enabled.store(self.hard_sync, std::sync::atomic::Ordering::Relaxed);
                    }
                    ui.label(egui::RichText::new("OSC1 → OSC2").weak().small());
                });

                ui.horizontal(|ui| {
                    let on = self.fm_enabled;
                    let label = egui::RichText::new("FM").small()
                        .color(if on { Color32::from_rgb(120, 180, 255) } else { Color32::GRAY });
                    if ui.button(label)
                        .on_hover_text("Frequency Modulation — OSC 2 modulates OSC 1's pitch at audio rate. Low depth = warmth. High depth = metallic, DX7-style timbres.")
                        .clicked()
                    {
                        self.fm_enabled = !on;
                        let depth = if self.fm_enabled { self.fm_depth } else { 0.0 };
                        self.state.fm_depth.set(depth);
                    }
                    ui.add_enabled_ui(self.fm_enabled, |ui| {
                        if ui.add(egui::Slider::new(&mut self.fm_depth, 0.0..=10.0)
                            .text("depth").fixed_decimals(1))
                            .on_hover_text("FM depth (modulation index). ~1 = subtle. 3–5 = DX7 bells. 8+ = chaotic sidebands.")
                            .changed()
                        {
                            self.state.fm_depth.set(self.fm_depth);
                        }
                    });
                });

                ui.horizontal(|ui| {
                    let on = self.ring_enabled;
                    let label = egui::RichText::new("Ring").small()
                        .color(if on { Color32::from_rgb(255, 130, 200) } else { Color32::GRAY });
                    if ui.button(label)
                        .on_hover_text("Ring Modulation — multiplies OSC 1 × OSC 2. Output contains sum and difference frequencies, not the originals. Metallic, bell-like, Dalek-style textures.")
                        .clicked()
                    {
                        self.ring_enabled = !on;
                        let depth = if self.ring_enabled { self.ring_depth } else { 0.0 };
                        self.state.ring_depth.set(depth);
                    }
                    ui.add_enabled_ui(self.ring_enabled, |ui| {
                        if ui.add(egui::Slider::new(&mut self.ring_depth, 0.0..=2.0)
                            .text("depth").fixed_decimals(2))
                            .on_hover_text("Ring mod level added to the mix. Mute OSC 1 and OSC 2 in the mixer for pure ring mod — only sum/difference tones remain.")
                            .changed()
                        {
                            self.state.ring_depth.set(self.ring_depth);
                        }
                    });
                });
            }
        });
    }

    fn update_freq_mult(&self, i: usize) {
        let oct = self.osc_octave[i] as f32;
        let cents = self.osc_detune[i];
        let mult = 2_f32.powf(oct + cents / 1200.0);
        self.state.osc_freq_mult[i].set(mult);
    }

    /// Push unison detune multipliers and volumes to the DSP graph.
    /// Voices are spread symmetrically: ±spread/2 cents across `count` copies.
    fn update_unison(&self, i: usize) {
        let count = self.osc_unison_count[i];
        let spread = self.osc_unison_spread[i];

        if !self.osc_unison_enabled[i] || count <= 1 {
            // Disabled: only copy 0 active at full weight, no detune
            for c in 0..5 {
                self.state.osc_unison_detune[i][c].set(1.0);
                self.state.osc_unison_vol[i][c].set(if c == 0 { 1.0 } else { 0.0 });
            }
            return;
        }

        let vol = 1.0 / count as f32;
        for c in 0..5 {
            if c < count {
                // Spread evenly from -spread/2 to +spread/2 cents
                let t = if count > 1 {
                    c as f32 / (count - 1) as f32
                } else {
                    0.5
                };
                let cents = -spread * 0.5 + t * spread;
                let detune = 2_f32.powf(cents / 1200.0);
                self.state.osc_unison_detune[i][c].set(detune);
                self.state.osc_unison_vol[i][c].set(vol);
            } else {
                self.state.osc_unison_detune[i][c].set(1.0);
                self.state.osc_unison_vol[i][c].set(0.0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Mixer panel
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_mixer_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("MIXER").strong());
        ui.horizontal(|ui| {
            for i in 0..3 {
                ui.vertical(|ui| {
                    ui.set_width(36.0);
                    if ui
                        .add(
                            egui::Slider::new(&mut self.osc_vol[i], 0.0..=1.0)
                                .vertical()
                                .text(format!("{}", i + 1)),
                        )
                        .on_hover_text(format!("OSC {} volume in the mix.", i + 1))
                        .changed()
                    {
                        // Only push to DSP if the oscillator is enabled; otherwise
                        // just save the value so it restores correctly on re-enable.
                        if self.osc_enabled[i] {
                            self.state.osc_vol[i].set(self.osc_vol[i]);
                        }
                    }
                });
            }
            ui.vertical(|ui| {
                ui.set_width(36.0);
                if ui
                    .add(
                        egui::Slider::new(&mut self.noise_vol, 0.0..=1.0)
                            .vertical()
                            .text("N"),
                    )
                    .on_hover_text("White noise volume. Adds breathiness, air, or full noise textures.")
                    .changed()
                {
                    self.state.noise_vol.set(self.noise_vol);
                }
            });
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Vol:").on_hover_text("Master output volume.");
            if ui
                .add(egui::Slider::new(&mut self.master_vol, 0.0..=1.0))
                .on_hover_text("Master output volume.")
                .changed()
            {
                self.state.master_vol.set(self.master_vol);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Glide:").on_hover_text("Portamento — glide pitch from previous note to next. 0 = instant.");
            if ui
                .add(egui::Slider::new(&mut self.glide_time, 0.0..=0.5).text("s"))
                .on_hover_text("Glide time in seconds. Higher = slower pitch slide between notes.")
                .changed()
            {
                self.state.glide_time.set(self.glide_time);
            }
        });

        // --- Limiter controls ---
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let label = if self.limiter_enabled {
                egui::RichText::new("LIM").color(Color32::GREEN)
            } else {
                egui::RichText::new("LIM").color(Color32::GRAY)
            };
            if ui.button(label)
                .on_hover_text("Limiter — prevents the output from clipping. Enable when the mix is too loud.")
                .clicked()
            {
                self.limiter_enabled = !self.limiter_enabled;
                self.state
                    .limiter_enabled
                    .store(self.limiter_enabled, Ordering::Relaxed);
            }
            ui.add_enabled(
                self.limiter_enabled,
                egui::Slider::new(&mut self.limiter_threshold, 0.5..=1.0).text("Thr"),
            ).on_hover_text("Threshold at which limiting kicks in. Lower = more compression.");
            if self.limiter_enabled {
                self.state.limiter_threshold.set(self.limiter_threshold);
            }
        });

        // --- Peak meter ---
        ui.add_space(4.0);
        let peak_raw = f32::from_bits(self.state.peak_l.load(Ordering::Relaxed));
        // Smooth decay for display
        self.peak_display = (self.peak_display * 0.85 + peak_raw * 0.15).max(peak_raw * 0.3);

        // Peak hold: remember highest value, decay after 1 second
        let dt = 1.0 / 60.0_f32; // approximate frame time
        if peak_raw > self.peak_hold {
            self.peak_hold = peak_raw;
            self.peak_hold_timer = 0.0;
        } else {
            self.peak_hold_timer += dt;
            if self.peak_hold_timer > 1.0 {
                self.peak_hold *= 0.95; // slow decay after hold
            }
        }

        draw_peak_meter(ui, self.peak_display, self.peak_hold);
    }
}

// ---------------------------------------------------------------------------
// LFO panel
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_lfo_panel(&mut self, ui: &mut egui::Ui) {
        // Header toggle
        ui.horizontal(|ui| {
            let on = self.lfo_enabled;
            let label = egui::RichText::new("LFO").strong()
                .color(if on { Color32::from_rgb(0, 220, 160) } else { Color32::GRAY });
            if ui.button(label)
                .on_hover_text("Low Frequency Oscillator — a slow (sub-audio) wave that modulates pitch, filter cutoff, or amplitude. Creates vibrato, filter wobble, or tremolo.")
                .clicked()
            {
                self.lfo_enabled = !on;
                self.state.lfo_depth.set(if self.lfo_enabled { self.lfo_depth } else { 0.0 });
            }
        });

        ui.add_enabled_ui(self.lfo_enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Rate:").on_hover_text("LFO speed in Hz. Below ~20 Hz = slow modulation. At 20 Hz+ the effect becomes a subtle audio-rate wobble.");
                if ui.add(egui::Slider::new(&mut self.lfo_rate, 0.1..=20.0)
                    .text("Hz").logarithmic(true))
                    .on_hover_text("0.1 Hz = very slow sweep (~10s cycle). 5 Hz = fast vibrato. 20 Hz = enters audio range.")
                    .changed()
                {
                    self.state.lfo_rate.set(self.lfo_rate);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Depth:").on_hover_text("How strongly the LFO modulates its destination. 0 = no effect, 1 = full range.");
                if ui.add(egui::Slider::new(&mut self.lfo_depth, 0.0..=1.0))
                    .on_hover_text("Depth scales the mod amount. For pitch: ±2 semitones at 1.0. For filter: ±50% cutoff. For amp: full tremolo.")
                    .changed()
                {
                    self.state.lfo_depth.set(self.lfo_depth);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Shape:").on_hover_text("Waveform of the LFO. Affects the character of the modulation.");
                let shape_tips = [
                    "Sine — smooth, natural-sounding modulation. Classic vibrato.",
                    "Triangle — linear ramp up and down. Slightly sharper than sine.",
                    "Saw — ramps up then resets. Creates a rhythmic, one-directional sweep.",
                ];
                for (s, label) in [(0usize, "Sin"), (1, "Tri"), (2, "Saw")] {
                    if ui.selectable_label(self.lfo_shape == s, label)
                        .on_hover_text(shape_tips[s])
                        .clicked()
                    {
                        self.lfo_shape = s;
                        self.state.lfo_shape.store(s as u8, Ordering::Relaxed);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("→").on_hover_text("Destination: what the LFO modulates.");
                let dest_tips = [
                    "Pitch — vibrato. LFO wiggles the frequency of all oscillators.",
                    "Filter — filter wobble / wah effect. LFO sweeps the cutoff frequency.",
                    "Amp — tremolo. LFO pulses the output volume.",
                ];
                for (d, label) in [(0usize, "Pitch"), (1, "Filter"), (2, "Amp")] {
                    if ui.selectable_label(self.lfo_dest == d, label)
                        .on_hover_text(dest_tips[d])
                        .clicked()
                    {
                        self.lfo_dest = d;
                        self.state.lfo_dest.store(d as u8, Ordering::Relaxed);
                    }
                }
            });
        });
    }
}

// ---------------------------------------------------------------------------
// Filter panel
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_filter_panel(&mut self, ui: &mut egui::Ui) {
        // Header toggle
        ui.horizontal(|ui| {
            let on = self.filter_enabled;
            let label = egui::RichText::new("FILTER").strong()
                .color(if on { Color32::from_rgb(0, 220, 160) } else { Color32::GRAY });
            if ui.button(label)
                .on_hover_text("Moog-style 4-pole lowpass filter. Removes high frequencies, shaping the brightness and timbre of the sound. The classic 'sweep' sound of a synthesizer.")
                .clicked()
            {
                self.filter_enabled = !on;
                // Off: open filter fully (max cutoff, zero resonance) so it's transparent
                if self.filter_enabled {
                    self.state.cutoff.set(self.filter_cutoff);
                    self.state.resonance.set(self.filter_q);
                } else {
                    self.state.cutoff.set(18000.0);
                    self.state.resonance.set(0.0);
                }
            }
        });

        ui.add_enabled_ui(self.filter_enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Cut:").on_hover_text("Cutoff frequency — frequencies above this point are attenuated. Low = dark/muffled, high = bright/open.");
                if ui.add(egui::Slider::new(&mut self.filter_cutoff, 80.0..=18000.0)
                    .text("Hz").logarithmic(true))
                    .on_hover_text("80 Hz = very dark. 500–2000 Hz = classic filter sweep range. 18000 Hz = fully open.")
                    .changed()
                {
                    self.state.cutoff.set(self.filter_cutoff);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Res:").on_hover_text("Resonance — boosts frequencies near the cutoff, adding a peak. High resonance = squelchy, whistling quality. Near 1.0 = self-oscillation.");
                if ui.add(egui::Slider::new(&mut self.filter_q, 0.0..=0.95)
                    .text("Res").fixed_decimals(2))
                    .on_hover_text("0 = no resonance. 0.5 = prominent peak. 0.9+ = near self-oscillation (the filter sings on its own).")
                    .changed()
                {
                    self.state.resonance.set(self.filter_q);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Env:").on_hover_text("Filter envelope amount — how much the filter ADSR envelope opens the filter above the base cutoff on each note.");
                if ui.add(egui::Slider::new(&mut self.filter_env_amount, 0.0..=1.0))
                    .on_hover_text("0 = envelope has no effect. 1 = envelope sweeps up to +12 kHz above base cutoff. For 'pew': low cutoff, env=1, fast attack, short decay, sustain=0.")
                    .changed()
                {
                    self.state.filter_env_amount.set(self.filter_env_amount);
                }
            });
        });
    }
}

// ---------------------------------------------------------------------------
// ADSR panel (shared for filter env and amp env)
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_adsr_panel(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        _slots: &mut [usize; 4],
        is_filter: bool,
    ) {
        ui.label(egui::RichText::new(title).strong());

        let adsr = if is_filter {
            &mut self.fenv_adsr
        } else {
            &mut self.amp_adsr
        };
        let labels = ["A", "D", "S", "R"];
        let tips = [
            "Attack — time to reach full level after a note is pressed.",
            "Decay — time to fall from peak to sustain level.",
            "Sustain — level held while key is held (0 = silent, 1 = full).",
            "Release — time to fade out after key is released.",
        ];
        let ranges: [std::ops::RangeInclusive<f32>; 4] =
            [0.001..=2.0, 0.001..=2.0, 0.0..=1.0, 0.001..=4.0];

        ui.horizontal(|ui| {
            for i in 0..4 {
                ui.vertical(|ui| {
                    ui.set_width(28.0);
                    let log = i != 2;
                    let changed = ui
                        .add(
                            egui::Slider::new(&mut adsr[i], ranges[i].clone())
                                .vertical()
                                .logarithmic(log)
                                .text(labels[i]),
                        )
                        .on_hover_text(tips[i])
                        .changed();
                    if changed {
                        let v = adsr[i];
                        if is_filter {
                            match i {
                                0 => self.state.fenv_attack.set(v),
                                1 => self.state.fenv_decay.set(v),
                                2 => self.state.fenv_sustain.set(v),
                                _ => self.state.fenv_release.set(v),
                            }
                        } else {
                            match i {
                                0 => self.state.adsr_attack.set(v),
                                1 => self.state.adsr_decay.set(v),
                                2 => self.state.adsr_sustain.set(v),
                                _ => self.state.adsr_release.set(v),
                            }
                        }
                    }
                });
            }
        });

        // Collect cursor values from all voices
        let cursors: Vec<f32> = if is_filter {
            self.state.fenv_cursors.iter().map(|s| s.value()).collect()
        } else {
            self.state.amp_cursors.iter().map(|s| s.value()).collect()
        };
        draw_adsr_visualizer(ui, adsr, &cursors);
    }
}

// ---------------------------------------------------------------------------
// Keyboard panel
// ---------------------------------------------------------------------------

const WHITE_SEMITONES: &[i32] = &[0, 2, 4, 5, 7, 9, 11];
const BLACK_SEMITONES: &[Option<i32>] = &[Some(1), Some(3), None, Some(6), Some(8), Some(10), None];

const KEY_MAP: &[(egui::Key, i32)] = &[
    (egui::Key::A, 0),
    (egui::Key::W, 1),
    (egui::Key::S, 2),
    (egui::Key::E, 3),
    (egui::Key::D, 4),
    (egui::Key::F, 5),
    (egui::Key::T, 6),
    (egui::Key::G, 7),
    (egui::Key::Y, 8),
    (egui::Key::H, 9),
    (egui::Key::U, 10),
    (egui::Key::J, 11),
    (egui::Key::K, 12),
    (egui::Key::L, 14),
];

impl SynthApp {
    fn ui_keyboard_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Octave:").on_hover_text("Keyboard octave range (1–7). Shifts all keys up or down by one octave.");
            if ui.button("−").on_hover_text("One octave down").clicked() && self.piano_octave > 1 {
                self.piano_octave -= 1;
            }
            ui.label(format!("{}", self.piano_octave)).on_hover_text("Current keyboard octave.");
            if ui.button("+").on_hover_text("One octave up").clicked() && self.piano_octave < 7 {
                self.piano_octave += 1;
            }
            let hint = if self.seq_mode == SeqMode::ChordKb {
                "  a s d f g h j = chords I–VII"
            } else if self.seq_mode == SeqMode::ChordSeq && self.seq_playing {
                "  any key = set root note (live transpose)"
            } else {
                "  a–l = white keys, w e t y u = sharps"
            };
            ui.label(egui::RichText::new(hint).weak().small());
        });

        // Keyboard input — white keys A S D F G H J map to scale degrees I–VII in ChordKb mode,
        // or to chromatic semitones in Note/ChordSeq mode.
        // WHITE_KEYS_ORDERED lists the 7 white-key entries from KEY_MAP in degree order.
        const WHITE_KEYS: &[egui::Key] = &[
            egui::Key::A, egui::Key::S, egui::Key::D, egui::Key::F,
            egui::Key::G, egui::Key::H, egui::Key::J,
        ];

        if self.seq_mode == SeqMode::ChordSeq && self.seq_playing {
            // While chord sequencer is playing, keyboard keys change the root key live.
            // Any key press is interpreted as a new tonic — semitone only, octave ignored.
            // This lets you transpose the sequence in real time while it plays.
            let mut pressed_semitone: Option<u8> = None;
            ui.input(|inp| {
                for &(key, semitone) in KEY_MAP {
                    if inp.key_pressed(key) {
                        pressed_semitone = Some((semitone % 12) as u8);
                    }
                }
            });
            if let Some(semi) = pressed_semitone {
                self.chord_seq.root = semi;
                // Mirror to the toolbar combo so the UI stays in sync
            }
            // Clear held midi so notes don't bleed if mode was previously normal
            let prev: Vec<u8> = self.piano_held_midi.drain().collect();
            for m in prev { self.push_note_off(m); }
        } else if self.seq_mode == SeqMode::ChordKb {
            let mut current_degrees = std::collections::HashSet::<usize>::new();
            ui.input(|inp| {
                for (degree, &key) in WHITE_KEYS.iter().enumerate() {
                    if inp.key_down(key) { current_degrees.insert(degree); }
                }
            });
            // Press new degrees
            for &deg in &current_degrees {
                if !self.chord_kb.kb_held.contains(&deg) {
                    for m in self.chord_kb.chord_notes(deg) { self.push_note_on(m); }
                }
            }
            // Release removed degrees
            let released: Vec<usize> = self.chord_kb.kb_held.iter()
                .filter(|&&d| !current_degrees.contains(&d))
                .copied().collect();
            for deg in released {
                for m in self.chord_kb.chord_notes(deg) { self.push_note_off(m); }
            }
            self.chord_kb.kb_held = current_degrees;
            // Also clear piano_held_midi so notes don't leak when switching modes
            let prev_midi: Vec<u8> = self.piano_held_midi.drain().collect();
            for m in prev_midi { self.push_note_off(m); }
        } else {
            // Normal note mode — release any chord KB notes held from previous mode
            if !self.chord_kb.kb_held.is_empty() {
                let held: Vec<usize> = self.chord_kb.kb_held.drain().collect();
                for deg in held {
                    for m in self.chord_kb.chord_notes(deg) { self.push_note_off(m); }
                }
            }
            let mut current_held = std::collections::HashSet::<u8>::new();
            ui.input(|inp| {
                for &(key, semitone) in KEY_MAP {
                    if inp.key_down(key) {
                        current_held.insert((self.piano_octave * 12 + semitone) as u8);
                    }
                }
            });
            for &midi in &current_held {
                if !self.piano_held_midi.contains(&midi) { self.push_note_on(midi); }
            }
            let released: Vec<u8> = self.piano_held_midi.iter()
                .filter(|&&m| !current_held.contains(&m))
                .copied().collect();
            for midi in released { self.push_note_off(midi); }
            self.piano_held_midi = current_held;
        }

        self.draw_piano(ui);
    }

    fn draw_piano(&mut self, ui: &mut egui::Ui) {
        let white_w = 32.0_f32;
        let white_h = 90.0_f32;
        let black_w = 20.0_f32;
        let black_h = 56.0_f32;
        let num_white = 14;
        let total_width = white_w * num_white as f32;

        let (resp, painter) = ui.allocate_painter(
            Vec2::new(total_width, white_h + 4.0),
            Sense::click_and_drag(),
        );
        let origin = resp.rect.left_top();
        let pointer_pos = resp.interact_pointer_pos();
        let mut clicked_midi: Option<u8> = None;

        for oct in 0..2_i32 {
            for (wi, &semi) in WHITE_SEMITONES.iter().enumerate() {
                let x = (oct * 7 + wi as i32) as f32 * white_w;
                let rect = Rect::from_min_size(
                    origin + Vec2::new(x + 1.0, 1.0),
                    Vec2::new(white_w - 2.0, white_h - 2.0),
                );
                let midi = ((self.piano_octave + oct) * 12 + semi) as u8;
                let pressed =
                    self.piano_held_midi.contains(&midi) || self.piano_mouse_midi == Some(midi);
                let fill = if pressed {
                    Color32::from_rgb(100, 180, 255)
                } else {
                    Color32::WHITE
                };
                painter.rect_filled(rect, Rounding::same(3.0), fill);
                painter.rect_stroke(
                    rect,
                    Rounding::same(3.0),
                    Stroke::new(1.0, Color32::DARK_GRAY),
                );
                if let Some(pos) = pointer_pos {
                    if rect.contains(pos) {
                        clicked_midi = Some(midi);
                    }
                }
            }
        }
        for oct in 0..2_i32 {
            for (bi, semi_opt) in BLACK_SEMITONES.iter().enumerate() {
                let Some(semi) = semi_opt else { continue };
                let x = (oct * 7 + bi as i32) as f32 * white_w + white_w * 0.6;
                let rect =
                    Rect::from_min_size(origin + Vec2::new(x, 1.0), Vec2::new(black_w, black_h));
                let midi = ((self.piano_octave + oct) * 12 + semi) as u8;
                let pressed =
                    self.piano_held_midi.contains(&midi) || self.piano_mouse_midi == Some(midi);
                let fill = if pressed {
                    Color32::from_rgb(60, 120, 200)
                } else {
                    Color32::BLACK
                };
                painter.rect_filled(rect, Rounding::same(2.0), fill);
                if let Some(pos) = pointer_pos {
                    if rect.contains(pos) {
                        clicked_midi = Some(midi);
                    }
                }
            }
        }

        if resp.is_pointer_button_down_on() {
            if let Some(midi) = clicked_midi {
                if self.piano_mouse_midi != Some(midi) {
                    if let Some(old) = self.piano_mouse_midi {
                        self.push_note_off(old);
                    }
                    self.piano_mouse_midi = Some(midi);
                    self.push_note_on(midi);
                }
            }
        } else if let Some(midi) = self.piano_mouse_midi.take() {
            self.push_note_off(midi);
        }
    }
}

// ---------------------------------------------------------------------------
// Sequencer panel — dispatches to per-mode UI
// ---------------------------------------------------------------------------

// Full chromatic range C2–C6 for note sequencer.
const SEQ_CHROMATIC: &[u8] = &[
    36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
    48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59,
    60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71,
    72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83,
    84,
];

impl SynthApp {
    fn ui_sequencer_panel(&mut self, ui: &mut egui::Ui) {
        // --- Shared toolbar ---
        ui.horizontal(|ui| {
            // Mode tabs
            for &mode in &[SeqMode::NoteSeq, SeqMode::ChordSeq, SeqMode::ChordKb] {
                let active = self.seq_mode == mode;
                let label = egui::RichText::new(mode.label())
                    .color(if active { Color32::from_rgb(0, 220, 160) } else { Color32::GRAY })
                    .strong();
                let tip = match mode {
                    SeqMode::NoteSeq  => "Note Sequencer — step-sequence individual notes.",
                    SeqMode::ChordSeq => "Chord Sequencer — step-sequence chords from a diatonic scale.",
                    SeqMode::ChordKb  => "Chord Keyboard — play chords live via keyboard keys (A–J = degrees I–VII).",
                };
                if ui.button(label).on_hover_text(tip).clicked() && !active {
                    // Stop playback when switching modes
                    let prev: Vec<u8> = self.seq_prev_notes.drain(..).collect();
                    for m in prev { self.push_note_off(m); }
                    self.seq_playing = false;
                    self.seq_current_step = 0;
                    self.seq_mode = mode;
                }
            }

            ui.separator();

            // Play/Stop — only for sequencer modes
            if self.seq_mode != SeqMode::ChordKb {
                let btn = if self.seq_playing { "⏹ Stop" } else { "▶ Play" };
                if ui.button(btn).on_hover_text("Start or stop the sequencer.").clicked() {
                    self.seq_playing = !self.seq_playing;
                    if !self.seq_playing {
                        let prev: Vec<u8> = self.seq_prev_notes.drain(..).collect();
                        for m in prev { self.push_note_off(m); }
                    }
                }
                ui.label("BPM:").on_hover_text("Sequencer tempo in beats per minute.");
                ui.add(egui::Slider::new(&mut self.seq_bpm, 40..=600))
                    .on_hover_text("Sequencer tempo (40–600 BPM).");

                // Step length selector
                let cur_length = match self.seq_mode {
                    SeqMode::NoteSeq  => &mut self.note_seq.length,
                    SeqMode::ChordSeq => &mut self.chord_seq.length,
                    SeqMode::ChordKb  => unreachable!(),
                };
                ui.label("Steps:").on_hover_text("Number of steps in the sequencer pattern.");
                for &len in &[8usize, 16, 24] {
                    let active = *cur_length == len;
                    let label = egui::RichText::new(format!("{len}"))
                        .color(if active { Color32::from_rgb(0, 200, 130) } else { Color32::GRAY });
                    if ui.button(label).on_hover_text(format!("Set pattern length to {len} steps.")).clicked() {
                        *cur_length = len;
                        if self.seq_current_step >= len { self.seq_current_step = 0; }
                    }
                }

                // Random fill
                if ui.button("🎲").on_hover_text("Randomly fill all steps with notes.").clicked() {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut h = DefaultHasher::new();
                    std::time::SystemTime::now().hash(&mut h);
                    let seed = h.finish();
                    match self.seq_mode {
                        SeqMode::NoteSeq => {
                            let len = self.note_seq.length;
                            for i in 0..len {
                                self.note_seq.steps[i] = seed.wrapping_shr(i as u32) & 1 == 1;
                                self.note_seq.notes[i] = SEQ_CHROMATIC[
                                    (seed.wrapping_shr((i * 3) as u32) & 0xff) as usize
                                    % SEQ_CHROMATIC.len()
                                ];
                            }
                        }
                        SeqMode::ChordSeq => {
                            let len = self.chord_seq.length;
                            for i in 0..len {
                                self.chord_seq.steps[i] = seed.wrapping_shr(i as u32) & 1 == 1;
                                self.chord_seq.degrees[i] =
                                    (seed.wrapping_shr((i * 4) as u32) & 0xff) as usize % 7;
                            }
                        }
                        SeqMode::ChordKb => {}
                    }
                }
            }

            // Chord key/scale selector (ChordSeq and ChordKb)
            if self.seq_mode == SeqMode::ChordSeq || self.seq_mode == SeqMode::ChordKb {
                ui.separator();
                let (root, scale) = match self.seq_mode {
                    SeqMode::ChordSeq => (&mut self.chord_seq.root, &mut self.chord_seq.scale),
                    SeqMode::ChordKb  => (&mut self.chord_kb.root,  &mut self.chord_kb.scale),
                    _ => unreachable!(),
                };
                ui.label("Key:").on_hover_text("Root note for the chord scale.");
                egui::ComboBox::from_id_salt("chord_root")
                    .selected_text(NOTE_NAMES[*root as usize])
                    .show_ui(ui, |ui| {
                        for (i, name) in NOTE_NAMES.iter().enumerate() {
                            ui.selectable_value(root, i as u8, *name);
                        }
                    });
                ui.label("Scale:").on_hover_text("Diatonic scale used to build chords (Major = bright, Minor = dark).");
                for &sc in &[ScaleType::Major, ScaleType::Minor] {
                    let active = *scale == sc;
                    let label = egui::RichText::new(sc.label())
                        .color(if active { Color32::from_rgb(0, 200, 130) } else { Color32::GRAY });
                    if ui.button(label).on_hover_text(match sc {
                        ScaleType::Major => "Major scale — bright, happy feel.",
                        ScaleType::Minor => "Minor scale — dark, moody feel.",
                    }).clicked() { *scale = sc; }
                }
            }
        });

        ui.add_space(4.0);

        match self.seq_mode {
            SeqMode::NoteSeq  => self.ui_note_seq(ui),
            SeqMode::ChordSeq => self.ui_chord_seq(ui),
            SeqMode::ChordKb  => self.ui_chord_kb(ui),
        }
    }

    // -----------------------------------------------------------------------
    // Note sequencer grid
    // -----------------------------------------------------------------------
    fn ui_note_seq(&mut self, ui: &mut egui::Ui) {
        let bar_area_h = 64.0;
        let n = self.note_seq.length as f32;
        let spacing = ui.spacing().item_spacing.x;
        let step_w = ((ui.available_width() - spacing * (n - 1.0)) / n).max(28.0);
        let midi_min = *SEQ_CHROMATIC.first().unwrap() as f32;
        let midi_max = *SEQ_CHROMATIC.last().unwrap() as f32;

        ui.horizontal(|ui| {
            for i in 0..self.note_seq.length {
                ui.vertical(|ui| {
                    ui.set_width(step_w);
                    let is_current = self.seq_playing && self.seq_current_step == i;
                    let is_on = self.note_seq.steps[i];
                    let note = self.note_seq.notes[i] as f32;

                    // Pitch bar
                    let (bar_resp, painter) = ui.allocate_painter(
                        Vec2::new(step_w, bar_area_h), Sense::click_and_drag());
                    let r = bar_resp.rect;
                    painter.rect_filled(r, Rounding::same(4.0), Color32::from_rgb(25, 25, 35));
                    let t = (note - midi_min) / (midi_max - midi_min);
                    let bar_h = (t * (bar_area_h - 4.0)).max(4.0);
                    let bar_rect = egui::Rect::from_min_size(
                        egui::pos2(r.min.x + 2.0, r.max.y - bar_h - 2.0),
                        Vec2::new(step_w - 4.0, bar_h),
                    );
                    let bar_color = if is_current { Color32::from_rgb(255, 210, 60) }
                        else if is_on { Color32::from_rgb(0, 120, 80) }
                        else { Color32::from_rgb(40, 50, 55) };
                    painter.rect_filled(bar_rect, Rounding::same(3.0), bar_color);
                    painter.text(r.center(), egui::Align2::CENTER_CENTER,
                        midi_note_name(self.note_seq.notes[i]),
                        egui::FontId::monospace(10.0),
                        if is_on { Color32::WHITE } else { Color32::GRAY });

                    if bar_resp.dragged() {
                        self.note_seq.drag_accum[i] -= bar_resp.drag_delta().y;
                        let steps = self.note_seq.drag_accum[i] as i32;
                        if steps != 0 {
                            self.note_seq.drag_accum[i] -= steps as f32;
                            let pos = SEQ_CHROMATIC.iter()
                                .position(|&n| n == self.note_seq.notes[i]).unwrap_or(0) as i32;
                            let new_pos = (pos + steps).clamp(0, SEQ_CHROMATIC.len() as i32 - 1) as usize;
                            self.note_seq.notes[i] = SEQ_CHROMATIC[new_pos];
                        }
                    }
                    if bar_resp.drag_stopped() { self.note_seq.drag_accum[i] = 0.0; }

                    // Step button
                    let fill = if is_current { Color32::from_rgb(255, 200, 50) }
                        else if is_on { Color32::from_rgb(0, 180, 120) }
                        else { Color32::from_rgb(40, 40, 55) };
                    let (r, painter) = ui.allocate_painter(Vec2::new(step_w, 28.0), Sense::click());
                    painter.rect_filled(r.rect, Rounding::same(5.0), fill);
                    painter.rect_stroke(r.rect, Rounding::same(5.0),
                        Stroke::new(1.0, if is_current { Color32::WHITE } else { Color32::GRAY }));
                    if r.clicked() { self.note_seq.steps[i] = !self.note_seq.steps[i]; }
                });
            }
        });
    }

    // -----------------------------------------------------------------------
    // Chord sequencer grid
    // -----------------------------------------------------------------------
    fn ui_chord_seq(&mut self, ui: &mut egui::Ui) {
        let bar_area_h = 64.0;
        let n = self.chord_seq.length as f32;
        let spacing = ui.spacing().item_spacing.x;
        let step_w = ((ui.available_width() - spacing * (n - 1.0)) / n).max(28.0);

        ui.horizontal(|ui| {
            for i in 0..self.chord_seq.length {
                ui.vertical(|ui| {
                    ui.set_width(step_w);
                    let is_current = self.seq_playing && self.seq_current_step == i;
                    let is_on = self.chord_seq.steps[i];
                    let degree = self.chord_seq.degrees[i];

                    // Chord bar — height = degree / 6
                    let (bar_resp, painter) = ui.allocate_painter(
                        Vec2::new(step_w, bar_area_h), Sense::click_and_drag());
                    let r = bar_resp.rect;
                    painter.rect_filled(r, Rounding::same(4.0), Color32::from_rgb(25, 25, 35));
                    let t = degree as f32 / 6.0;
                    let bar_h = (t * (bar_area_h - 4.0)).max(4.0);
                    let bar_rect = egui::Rect::from_min_size(
                        egui::pos2(r.min.x + 2.0, r.max.y - bar_h - 2.0),
                        Vec2::new(step_w - 4.0, bar_h),
                    );
                    // Color by chord quality
                    let quality = chord_quality(self.chord_seq.scale, degree);
                    let bar_color = if is_current { Color32::from_rgb(255, 210, 60) }
                        else if !is_on { Color32::from_rgb(40, 50, 55) }
                        else if quality == "m" { Color32::from_rgb(60, 80, 140) }
                        else if quality == "°" { Color32::from_rgb(120, 50, 50) }
                        else { Color32::from_rgb(0, 100, 70) };
                    painter.rect_filled(bar_rect, Rounding::same(3.0), bar_color);

                    // Chord name + roman numeral
                    let cname = chord_name(self.chord_seq.root, self.chord_seq.scale, degree);
                    painter.text(egui::pos2(r.center().x, r.center().y - 6.0),
                        egui::Align2::CENTER_CENTER, &cname,
                        egui::FontId::monospace(9.0),
                        if is_on { Color32::WHITE } else { Color32::GRAY });
                    painter.text(egui::pos2(r.center().x, r.center().y + 7.0),
                        egui::Align2::CENTER_CENTER, DEGREE_LABELS[degree],
                        egui::FontId::monospace(8.0),
                        if is_on { Color32::from_rgb(180, 180, 180) } else { Color32::from_rgb(80,80,80) });

                    if bar_resp.dragged() {
                        self.chord_seq.drag_accum[i] -= bar_resp.drag_delta().y;
                        let steps = self.chord_seq.drag_accum[i] as i32;
                        if steps != 0 {
                            self.chord_seq.drag_accum[i] -= steps as f32;
                            self.chord_seq.degrees[i] =
                                (degree as i32 + steps).clamp(0, 6) as usize;
                        }
                    }
                    if bar_resp.drag_stopped() { self.chord_seq.drag_accum[i] = 0.0; }

                    // Step button
                    let fill = if is_current { Color32::from_rgb(255, 200, 50) }
                        else if is_on { Color32::from_rgb(0, 180, 120) }
                        else { Color32::from_rgb(40, 40, 55) };
                    let (r, painter) = ui.allocate_painter(Vec2::new(step_w, 28.0), Sense::click());
                    painter.rect_filled(r.rect, Rounding::same(5.0), fill);
                    painter.rect_stroke(r.rect, Rounding::same(5.0),
                        Stroke::new(1.0, if is_current { Color32::WHITE } else { Color32::GRAY }));
                    if r.clicked() { self.chord_seq.steps[i] = !self.chord_seq.steps[i]; }
                });
            }
        });
    }

    // -----------------------------------------------------------------------
    // Chord keyboard — 7 big buttons (I–VII), click/hold to play chord
    // -----------------------------------------------------------------------
    fn ui_chord_kb(&mut self, ui: &mut egui::Ui) {
        let spacing = ui.spacing().item_spacing.x;
        let btn_w = ((ui.available_width() - spacing * 6.0) / 7.0).max(40.0);
        let btn_h = 90.0;

        ui.horizontal(|ui| {
            for degree in 0..7 {
                let (resp, painter) = ui.allocate_painter(
                    Vec2::new(btn_w, btn_h), Sense::click_and_drag());
                let r = resp.rect;

                let is_held_mouse = self.chord_kb.held_degree == Some(degree);
                let is_held_kb    = self.chord_kb.kb_held.contains(&degree);
                let is_held = is_held_mouse || is_held_kb;
                let quality = chord_quality(self.chord_kb.scale, degree);
                let bg = if is_held { Color32::from_rgb(255, 210, 60) }
                    else if quality == "m" { Color32::from_rgb(40, 55, 100) }
                    else if quality == "°" { Color32::from_rgb(80, 35, 35) }
                    else { Color32::from_rgb(30, 80, 55) };
                painter.rect_filled(r, Rounding::same(8.0), bg);
                painter.rect_stroke(r, Rounding::same(8.0),
                    Stroke::new(if is_held { 2.0 } else { 1.0 },
                    if is_held { Color32::WHITE } else { Color32::from_gray(80) }));

                let cname = chord_name(self.chord_kb.root, self.chord_kb.scale, degree);
                painter.text(egui::pos2(r.center().x, r.center().y - 10.0),
                    egui::Align2::CENTER_CENTER, &cname,
                    egui::FontId::proportional(14.0), Color32::WHITE);
                painter.text(egui::pos2(r.center().x, r.center().y + 10.0),
                    egui::Align2::CENTER_CENTER, DEGREE_LABELS[degree],
                    egui::FontId::monospace(10.0), Color32::from_gray(180));

                // Mouse press — only trigger if mouse (not keyboard) activated it
                if resp.is_pointer_button_down_on() && !is_held_mouse {
                    // Release previous mouse-held chord if any
                    if let Some(prev) = self.chord_kb.held_degree {
                        for m in self.chord_kb.chord_notes(prev) { self.push_note_off(m); }
                    }
                    self.chord_kb.held_degree = Some(degree);
                    for m in self.chord_kb.chord_notes(degree) { self.push_note_on(m); }
                }
                // Mouse release — only fires if the mouse was the one holding this chord
                if !resp.is_pointer_button_down_on() && is_held_mouse {
                    self.chord_kb.held_degree = None;
                    for m in self.chord_kb.chord_notes(degree) { self.push_note_off(m); }
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Latency indicator
// ---------------------------------------------------------------------------

fn draw_latency_bar(ui: &mut egui::Ui, state: &AudioState, attack_s: f32) {
    use std::sync::atomic::Ordering;

    let sr = state.sample_rate.load(Ordering::Relaxed);
    let frames = state.buffer_frames.load(Ordering::Relaxed);
    let measured_us = state.last_latency_us.load(Ordering::Relaxed);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Latency:").weak().small());

        if sr == 0 || frames == 0 {
            ui.label(egui::RichText::new("measuring…").weak().small().italics());
            return;
        }

        let buffer_ms = frames as f32 / sr as f32 * 1000.0;
        let ui_ms = 1000.0 / 60.0;
        let attack_ms = attack_s * 1000.0;
        let est_ms = buffer_ms + ui_ms + attack_ms;

        // Estimated (always visible)
        let est_color = if est_ms < 20.0 {
            Color32::from_rgb(0, 180, 120)
        } else if est_ms < 40.0 {
            Color32::from_rgb(200, 180, 0)
        } else {
            Color32::from_rgb(200, 70, 50)
        };
        ui.label(
            egui::RichText::new(format!(
                "est ~{est_ms:.0}ms  (buf {buffer_ms:.1} + UI ~{ui_ms:.0} + atk {attack_ms:.0})"
            ))
            .small()
            .color(est_color),
        );

        // Real measurement (only after first note-on)
        if measured_us > 0 {
            let measured_ms = measured_us as f32 / 1000.0;
            let meas_color = if measured_ms < 20.0 {
                Color32::from_rgb(0, 220, 160)
            } else if measured_ms < 40.0 {
                Color32::from_rgb(220, 200, 0)
            } else {
                Color32::from_rgb(220, 80, 60)
            };
            ui.separator();
            ui.label(
                egui::RichText::new(format!("measured {measured_ms:.1}ms"))
                    .small()
                    .strong()
                    .color(meas_color),
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Oscilloscope
// ---------------------------------------------------------------------------

fn draw_peak_meter(ui: &mut egui::Ui, level: f32, peak_hold: f32) {
    let (resp, painter) =
        ui.allocate_painter(Vec2::new(ui.available_width(), 14.0), Sense::hover());
    let rect = resp.rect;
    painter.rect_filled(rect, Rounding::same(2.0), Color32::from_rgb(10, 15, 20));

    // Map level to bar width. Show up to 1.5x (anything above 1.0 is clipping).
    let max_display = 1.5_f32;
    let bar_frac = (level / max_display).clamp(0.0, 1.0);
    let bar_w = rect.width() * bar_frac;

    if bar_w > 0.5 {
        // Color: green -> yellow -> red
        let color = if level < 0.7 {
            Color32::from_rgb(0, 200, 80)
        } else if level < 1.0 {
            let t = (level - 0.7) / 0.3;
            Color32::from_rgb(
                (255.0 * t) as u8,
                (200.0 * (1.0 - t * 0.5)) as u8,
                (80.0 * (1.0 - t)) as u8,
            )
        } else {
            Color32::from_rgb(255, 50, 30)
        };
        let bar_rect = Rect::from_min_size(rect.min, Vec2::new(bar_w, rect.height()));
        painter.rect_filled(bar_rect, Rounding::same(2.0), color);
    }

    // 0 dB reference line (level = 1.0)
    let unity_x = rect.left() + rect.width() * (1.0 / max_display);
    painter.line_segment(
        [
            Pos2::new(unity_x, rect.top()),
            Pos2::new(unity_x, rect.bottom()),
        ],
        Stroke::new(1.0, Color32::from_rgba_premultiplied(255, 255, 255, 100)),
    );

    // Peak hold indicator
    if peak_hold > 0.01 {
        let hold_frac = (peak_hold / max_display).clamp(0.0, 1.0);
        let hold_x = rect.left() + rect.width() * hold_frac;
        let hold_color = if peak_hold >= 1.0 {
            Color32::from_rgb(255, 80, 50)
        } else {
            Color32::from_rgb(255, 255, 255)
        };
        painter.line_segment(
            [
                Pos2::new(hold_x, rect.top() + 1.0),
                Pos2::new(hold_x, rect.bottom() - 1.0),
            ],
            Stroke::new(2.0, hold_color),
        );
    }

    // Label
    let text = if level >= 1.0 {
        format!("{:+.1} dB CLIP", 20.0 * level.log10())
    } else if level > 0.001 {
        format!("{:+.1} dB", 20.0 * level.log10())
    } else {
        "-inf dB".to_string()
    };
    painter.text(
        Pos2::new(rect.left() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(10.0),
        Color32::WHITE,
    );
}


// ---------------------------------------------------------------------------
// Oscilloscope
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// MIDI panel
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Patch capture / apply
// ---------------------------------------------------------------------------

impl SynthApp {
    fn capture_patch(&self) -> Patch {
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
        }
    }

    fn apply_patch(&mut self, p: Patch) {
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
        }
    }
}

// ---------------------------------------------------------------------------
// Patch UI — toolbar bar + browser panel
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_patch_bar(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("PATCH").strong().small());

        // Patch name
        ui.add(egui::TextEdit::singleline(&mut self.patch_name).desired_width(120.0))
            .on_hover_text("Patch name. Used as filename when saving.");

        // Save to file
        if ui.button("💾 Save").on_hover_text("Save current patch to a JSON file.").clicked() {
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
        }

        // Load from file
        if ui.button("📂 Load").on_hover_text("Load a patch from a JSON file.").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Patch", &["json"])
                .pick_file()
            {
                if let Ok(json) = std::fs::read_to_string(path) {
                    if let Ok(p) = serde_json::from_str::<Patch>(&json) {
                        self.apply_patch(p);
                    }
                }
            }
        }

        // Library browser toggle
        let browser_label = egui::RichText::new("📚 Library")
            .color(if self.patch_browser_open { Color32::from_rgb(0, 220, 160) } else { Color32::WHITE });
        if ui.button(browser_label).on_hover_text("Browse and load factory patches organized by category and synth model.").clicked() {
            self.patch_browser_open = !self.patch_browser_open;
        }
    }

    fn ui_patch_browser(&mut self, ctx: &egui::Context) {
        if !self.patch_browser_open { return; }

        let mut open = self.patch_browser_open;
        egui::Window::new("Patch Library")
            .open(&mut open)
            .resizable(true)
            .default_size([440.0, 520.0])
            .show(ctx, |ui| {
                // --- Category filter ---
                let categories: Vec<String> = {
                    let mut cats = vec!["All".to_string()];
                    let mut seen = std::collections::HashSet::new();
                    for p in &self.patch_library {
                        if seen.insert(p.category.clone()) {
                            cats.push(p.category.clone());
                        }
                    }
                    cats
                };
                ui.label(egui::RichText::new("Category").weak().small());
                ui.horizontal_wrapped(|ui| {
                    for cat in &categories {
                        let active = &self.patch_browser_category == cat;
                        let label = egui::RichText::new(cat)
                            .color(if active { Color32::from_rgb(0, 220, 160) } else { Color32::GRAY });
                        if ui.button(label).clicked() {
                            self.patch_browser_category = cat.clone();
                        }
                    }
                });

                ui.add_space(4.0);

                // --- Synth model filter ---
                // Collect models visible under the current category filter
                let models: Vec<String> = {
                    let cat_filter = &self.patch_browser_category;
                    let mut models = vec!["All".to_string()];
                    let mut seen = std::collections::HashSet::new();
                    for p in &self.patch_library {
                        if cat_filter != "All" && &p.category != cat_filter { continue; }
                        let m = if p.synth_model.is_empty() { "Original".to_string() } else { p.synth_model.clone() };
                        if seen.insert(m.clone()) {
                            models.push(m);
                        }
                    }
                    models
                };
                // Reset model filter if it's no longer visible
                if !models.contains(&self.patch_browser_model) {
                    self.patch_browser_model = "All".into();
                }
                ui.label(egui::RichText::new("Synth").weak().small());
                ui.horizontal_wrapped(|ui| {
                    for m in &models {
                        let active = &self.patch_browser_model == m;
                        let label = egui::RichText::new(m)
                            .color(if active { Color32::from_rgb(100, 180, 255) } else { Color32::GRAY });
                        if ui.button(label).clicked() {
                            self.patch_browser_model = m.clone();
                        }
                    }
                });

                ui.separator();

                // --- Load FX toggle ---
                ui.horizontal(|ui| {
                    let label = egui::RichText::new("Load FX with patch")
                        .small()
                        .color(if self.patch_load_fx { Color32::from_rgb(255, 180, 60) } else { Color32::GRAY });
                    ui.checkbox(&mut self.patch_load_fx, label)
                        .on_hover_text("When enabled, loading a patch also restores its FX chain settings.\nWhen disabled, your current FX chain stays untouched.");
                });

                ui.separator();

                // --- Patch list ---
                let cat_filter  = self.patch_browser_category.clone();
                let model_filter = self.patch_browser_model.clone();
                let patches: Vec<(usize, String, String, String)> = self.patch_library.iter().enumerate()
                    .filter(|(_, p)| {
                        let cat_ok   = cat_filter == "All"   || p.category == cat_filter;
                        let model_display = if p.synth_model.is_empty() { "Original" } else { &p.synth_model };
                        let model_ok = model_filter == "All" || model_display == model_filter;
                        cat_ok && model_ok
                    })
                    .map(|(i, p)| {
                        let m = if p.synth_model.is_empty() { "Original".to_string() } else { p.synth_model.clone() };
                        (i, p.name.clone(), p.category.clone(), m)
                    })
                    .collect();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut load_idx: Option<usize> = None;
                    for (idx, name, cat, model) in &patches {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("[{cat}]")).weak().small().monospace());
                            ui.label(egui::RichText::new(format!("{model}")).color(Color32::from_rgb(100, 180, 255)).small().monospace());
                            if ui.selectable_label(false, name).clicked() {
                                load_idx = Some(*idx);
                            }
                        });
                    }
                    if let Some(idx) = load_idx {
                        let p = self.patch_library[idx].clone();
                        self.apply_patch(p);
                        self.patch_browser_open = false;
                    }
                });
            });
        self.patch_browser_open = open;
    }
}

// ---------------------------------------------------------------------------
// MIDI panel
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_midi_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("MIDI").strong().small());

        // Refresh port list button
        if ui.small_button("⟳").on_hover_text("Refresh MIDI device list").clicked() {
            self.midi.list_ports();
        }

        if self.midi.port_names.is_empty() {
            ui.label(egui::RichText::new("No MIDI devices found").weak().small());
            return;
        }

        // Device selector
        let connected = self.midi.connected_port;
        let current_label = connected
            .and_then(|i| self.midi.port_names.get(i))
            .map(|s| s.as_str())
            .unwrap_or("— disconnected —");

        egui::ComboBox::from_id_salt("midi_port")
            .selected_text(egui::RichText::new(current_label).small())
            .show_ui(ui, |ui| {
                // Disconnect option
                let selected = connected.is_none();
                if ui.selectable_label(selected, "— disconnected —")
                    .on_hover_text("Disconnect from all MIDI devices.")
                    .clicked() {
                    self.midi.disconnect();
                }
                // One entry per port
                let names: Vec<String> = self.midi.port_names.clone();
                for (i, name) in names.iter().enumerate() {
                    let selected = connected == Some(i);
                    if ui.selectable_label(selected, name)
                        .on_hover_text(format!("Connect to MIDI device: {name}"))
                        .clicked() && !selected {
                        if let Err(e) = self.midi.connect(i) {
                            eprintln!("MIDI connect error: {e}");
                        }
                    }
                }
            });

        // Status dot
        let (color, label) = if connected.is_some() {
            (Color32::from_rgb(0, 220, 120), "●")
        } else {
            (Color32::from_gray(80), "○")
        };
        ui.label(egui::RichText::new(label).color(color).small());
    }
}

// ---------------------------------------------------------------------------
// Oscilloscope
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_oscilloscope(&mut self, ui: &mut egui::Ui) {
        // Controls row
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("SCOPE")
                    .small()
                    .color(Color32::from_rgb(60, 100, 80)),
            );
            ui.add_space(8.0);
            ui.label(egui::RichText::new("X").small().color(Color32::from_rgb(100, 180, 140)))
                .on_hover_text("Horizontal zoom — drag to stretch or compress the waveform in time.");
            ui.add(
                egui::DragValue::new(&mut self.scope_x_scale)
                    .speed(0.02)
                    .range(0.25_f32..=8.0)
                    .suffix("×"),
            ).on_hover_text("Horizontal zoom (0.25–8×). Drag left/right to adjust.");
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Y").small().color(Color32::from_rgb(100, 180, 140)))
                .on_hover_text("Vertical zoom — drag to scale the waveform amplitude.");
            ui.add(
                egui::DragValue::new(&mut self.scope_y_scale)
                    .speed(0.02)
                    .range(0.25_f32..=8.0)
                    .suffix("×"),
            ).on_hover_text("Vertical zoom (0.25–8×). Drag left/right to adjust.");
        });

        let buf = self.state.osc_buffer.lock().unwrap().clone();
        let width = ui.available_width();

        // Main scope area
        let (resp, painter) =
            ui.allocate_painter(Vec2::new(width, self.scope_height), Sense::hover());
        let rect = resp.rect;

        // CRT background
        painter.rect_filled(rect, Rounding::same(4.0), Color32::from_rgb(4, 10, 7));

        if !buf.is_empty() {
            // Scanlines
            let mut sy = rect.top() + 2.0;
            while sy < rect.bottom() {
                painter.line_segment(
                    [Pos2::new(rect.left(), sy), Pos2::new(rect.right(), sy)],
                    Stroke::new(1.0, Color32::from_rgba_premultiplied(0, 0, 0, 22)),
                );
                sy += 3.0;
            }

            let mid_y = rect.center().y;
            let half_h = rect.height() * 0.45;

            // Zero line
            painter.line_segment(
                [Pos2::new(rect.left(), mid_y), Pos2::new(rect.right(), mid_y)],
                Stroke::new(1.0, Color32::from_rgb(12, 28, 18)),
            );

            // X scale: how many samples to show
            let samples_to_show =
                ((buf.len() as f32 / self.scope_x_scale) as usize).clamp(16, buf.len());
            let buf_slice = &buf[..samples_to_show];
            let step = rect.width() / buf_slice.len() as f32;
            let y_scale = self.scope_y_scale;

            let points: Vec<Pos2> = buf_slice
                .iter()
                .enumerate()
                .map(|(i, &s)| {
                    let amp = (s * half_h * y_scale).clamp(-half_h, half_h);
                    Pos2::new(rect.left() + i as f32 * step, mid_y - amp)
                })
                .collect();

            // CRT phosphor glow: outer → inner → core
            for &(stroke_w, color) in &[
                (5.0_f32, Color32::from_rgba_premultiplied(0, 160, 90, 14)),
                (3.0_f32, Color32::from_rgba_premultiplied(0, 210, 130, 45)),
                (1.2_f32, Color32::from_rgba_premultiplied(55, 255, 165, 230)),
            ] {
                for w in points.windows(2) {
                    painter.line_segment([w[0], w[1]], Stroke::new(stroke_w, color));
                }
            }
        }

        // Drag-to-resize handle
        let (handle_resp, handle_painter) =
            ui.allocate_painter(Vec2::new(width, 7.0), Sense::drag());
        if handle_resp.dragged() {
            self.scope_height = (self.scope_height + handle_resp.drag_delta().y).max(40.0);
        }
        // Cursor hint
        if handle_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        let grip_color = if handle_resp.hovered() || handle_resp.dragged() {
            Color32::from_rgba_premultiplied(80, 220, 140, 140)
        } else {
            Color32::from_rgba_premultiplied(25, 65, 40, 100)
        };
        let cx = handle_resp.rect.center();
        for dx in [-12.0_f32, -6.0, 0.0, 6.0, 12.0] {
            let x = cx.x + dx;
            handle_painter.line_segment(
                [Pos2::new(x, cx.y - 1.5), Pos2::new(x, cx.y + 1.5)],
                Stroke::new(2.0, grip_color),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// ADSR visualizer
// ---------------------------------------------------------------------------

fn draw_adsr_visualizer(ui: &mut egui::Ui, adsr: &[f32; 4], cursors: &[f32]) {
    let height = 48.0;
    let (resp, painter) =
        ui.allocate_painter(Vec2::new(ui.available_width(), height), Sense::hover());
    let rect = resp.rect;

    painter.rect_filled(rect, Rounding::same(3.0), Color32::from_rgb(8, 14, 10));

    let a = adsr[0];
    let d = adsr[1];
    let s = adsr[2];
    let r = adsr[3];

    // Give sustain a fixed visual width proportional to total time
    let total = a + d + r;
    let s_vis = total * 0.35;
    let span  = a + d + s_vis + r;

    let w = rect.width();
    let h = rect.height();
    let pad_y = 4.0;
    let usable_h = h - pad_y * 2.0;

    // Map time position → x, level → y
    let tx = |t: f32| rect.left() + (t / span) * w;
    let ly = |level: f32| rect.bottom() - pad_y - level * usable_h;

    // 5 key points
    let p0 = Pos2::new(rect.left(),    ly(0.0));
    let p1 = Pos2::new(tx(a),          ly(1.0));
    let p2 = Pos2::new(tx(a + d),      ly(s));
    let p3 = Pos2::new(tx(a + d + s_vis), ly(s));
    let p4 = Pos2::new(rect.right(),   ly(0.0));

    // Filled shape
    let fill_pts = vec![
        p0, p1, p2, p3, p4,
        Pos2::new(rect.right(), rect.bottom() - pad_y),
        Pos2::new(rect.left(),  rect.bottom() - pad_y),
    ];
    painter.add(egui::Shape::convex_polygon(
        fill_pts,
        Color32::from_rgba_premultiplied(0, 160, 100, 30),
        Stroke::NONE,
    ));

    // Outline
    let pts = vec![p0, p1, p2, p3, p4];
    let stroke = Stroke::new(1.5, Color32::from_rgb(0, 200, 130));
    for w in pts.windows(2) {
        painter.line_segment([w[0], w[1]], stroke);
    }

    // Stage labels
    let label_color = Color32::from_rgba_premultiplied(80, 160, 110, 180);
    let small = egui::FontId::proportional(9.0);
    for (label, x) in [
        ("A", tx(a * 0.5)),
        ("D", tx(a + d * 0.5)),
        ("S", tx(a + d + s_vis * 0.5)),
        ("R", tx(a + d + s_vis + r * 0.5)),
    ] {
        painter.text(
            Pos2::new(x, rect.bottom() - pad_y - 2.0),
            egui::Align2::CENTER_BOTTOM,
            label,
            small.clone(),
            label_color,
        );
    }

    // Live cursor dots — one per voice
    for &cursor in cursors {
        if cursor < 0.5 { continue; } // idle

        let phase    = cursor as u8;
        let progress = cursor.fract();

        let pos = match phase {
            1 => Pos2::new(tx(a * progress),                              ly(progress)),
            2 => Pos2::new(tx(a + d * progress),                          ly(1.0 - (1.0 - s) * progress)),
            3 => Pos2::new(tx(a + d + s_vis * 0.5),                       ly(s)),
            4 => Pos2::new(tx(a + d + s_vis + r * progress),              ly(s * (1.0 - progress))),
            _ => continue,
        };

        // Glow + core dot
        painter.circle_filled(pos, 5.0, Color32::from_rgba_premultiplied(0, 255, 160, 40));
        painter.circle_filled(pos, 2.5, Color32::from_rgb(0, 255, 160));
    }
}

// ---------------------------------------------------------------------------
// FX chain panel
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_fx_chain(&mut self, ui: &mut egui::Ui) {
        let col_od   = Color32::from_rgb(255, 140,  60); // orange
        let col_dist = Color32::from_rgb(220,  60,  60); // red
        let col_cho  = Color32::from_rgb( 80, 200, 140); // green
        let col_dly  = Color32::from_rgb( 80, 160, 255); // blue
        let col_rev  = Color32::from_rgb(170,  90, 240); // purple

        ui.horizontal(|ui| {
            // ---- Overdrive ----
            ui.group(|ui| {
                ui.set_min_width(110.0);
                ui.vertical(|ui| {
                    let on = &mut self.fx_overdrive_on;
                    let label = egui::RichText::new("OVERDRIVE").small().strong()
                        .color(if *on { col_od } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Toggle overdrive (soft-clip / tanh saturation).").clicked() {
                        *on = !*on;
                        self.state.fx_overdrive_mix.set_value(if *on { self.fx_overdrive_mix } else { 0.0 });
                    }
                    ui.add(egui::Slider::new(&mut self.fx_overdrive_drive, 1.0_f32..=10.0)
                        .text("Drive").clamp_to_range(true))
                        .on_hover_text("Drive — how hard the signal is pushed into tanh saturation.");
                    ui.add(egui::Slider::new(&mut self.fx_overdrive_tone, 0.0_f32..=1.0)
                        .text("Tone").clamp_to_range(true))
                        .on_hover_text("Tone — post-clipper low-pass: 0 = dark (400 Hz), 1 = bright (18 kHz).");
                    ui.add(egui::Slider::new(&mut self.fx_overdrive_asym, 0.0_f32..=1.0)
                        .text("Asym").clamp_to_range(true))
                        .on_hover_text("Asymmetry — DC bias before clipping adds even harmonics for a warmer, tube-like character.");
                    ui.add(egui::Slider::new(&mut self.fx_overdrive_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Wet/dry mix: 0 = dry, 1 = fully overdriven.");
                    self.state.fx_overdrive_drive.set_value(self.fx_overdrive_drive);
                    self.state.fx_overdrive_tone.set_value(self.fx_overdrive_tone);
                    self.state.fx_overdrive_asym.set_value(self.fx_overdrive_asym);
                    if self.fx_overdrive_on {
                        self.state.fx_overdrive_mix.set_value(self.fx_overdrive_mix);
                    }
                });
            });

            // ---- Distortion ----
            ui.group(|ui| {
                ui.set_min_width(110.0);
                ui.vertical(|ui| {
                    let on = &mut self.fx_distortion_on;
                    let label = egui::RichText::new("DISTORTION").small().strong()
                        .color(if *on { col_dist } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Toggle distortion (hard clipping).").clicked() {
                        *on = !*on;
                        self.state.fx_distortion_mix.set_value(if *on { self.fx_distortion_mix } else { 0.0 });
                    }
                    ui.add(egui::Slider::new(&mut self.fx_distortion_drive, 1.0_f32..=20.0)
                        .text("Drive").clamp_to_range(true))
                        .on_hover_text("Drive — pre-gain before hard clipping. Higher = more of the wave is squared off.");
                    ui.add(egui::Slider::new(&mut self.fx_distortion_pre, 0.0_f32..=1.0)
                        .text("Pre").clamp_to_range(true))
                        .on_hover_text("Pre — high-pass before clipper (0 = all bass in, 1 = 800 Hz cut). Removes mud from low-end distortion.");
                    ui.add(egui::Slider::new(&mut self.fx_distortion_tone, 0.0_f32..=1.0)
                        .text("Tone").clamp_to_range(true))
                        .on_hover_text("Tone — post-clipper low-pass: 0 = dark (400 Hz), 1 = bright (18 kHz). Rolls off harsh high harmonics.");
                    ui.add(egui::Slider::new(&mut self.fx_distortion_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Wet/dry mix: 0 = dry, 1 = fully distorted.");
                    self.state.fx_distortion_drive.set_value(self.fx_distortion_drive);
                    self.state.fx_distortion_pre.set_value(self.fx_distortion_pre);
                    self.state.fx_distortion_tone.set_value(self.fx_distortion_tone);
                    if self.fx_distortion_on {
                        self.state.fx_distortion_mix.set_value(self.fx_distortion_mix);
                    }
                });
            });

            // ---- Chorus ----
            ui.group(|ui| {
                ui.set_min_width(130.0);
                ui.vertical(|ui| {
                    let on = &mut self.fx_chorus_on;
                    let label = egui::RichText::new("CHORUS").small().strong()
                        .color(if *on { col_cho } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Toggle chorus (LFO-modulated delay for width/shimmer).").clicked() {
                        *on = !*on;
                        self.state.fx_chorus_mix.set_value(if *on { self.fx_chorus_mix } else { 0.0 });
                    }
                    ui.add(egui::Slider::new(&mut self.fx_chorus_rate, 0.1_f32..=5.0)
                        .text("Rate").suffix(" Hz").clamp_to_range(true))
                        .on_hover_text("LFO rate in Hz — how fast the chorus modulates.");
                    ui.add(egui::Slider::new(&mut self.fx_chorus_depth, 0.0_f32..=0.02)
                        .text("Depth").clamp_to_range(true))
                        .on_hover_text("Depth of LFO modulation in seconds (0–20 ms).");
                    ui.add(egui::Slider::new(&mut self.fx_chorus_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Wet/dry mix.");
                    self.state.fx_chorus_rate.set_value(self.fx_chorus_rate);
                    self.state.fx_chorus_depth.set_value(self.fx_chorus_depth);
                    if self.fx_chorus_on {
                        self.state.fx_chorus_mix.set_value(self.fx_chorus_mix);
                    }
                });
            });

            // ---- Delay ----
            ui.group(|ui| {
                ui.set_min_width(160.0);
                ui.vertical(|ui| {
                    // Enable toggle
                    let on = &mut self.fx_delay_on;
                    let label = egui::RichText::new("DELAY").small().strong()
                        .color(if *on { col_dly } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Toggle delay (echo effect with feedback).").clicked() {
                        *on = !*on;
                        self.state.fx_delay_mix.set_value(if *on { self.fx_delay_mix } else { 0.0 });
                    }

                    // BPM sync toggle
                    ui.horizontal(|ui| {
                        let sync_label = egui::RichText::new(if self.fx_delay_sync { "BPM sync: ON" } else { "BPM sync: OFF" })
                            .small()
                            .color(if self.fx_delay_sync { col_dly } else { Color32::GRAY });
                        if ui.button(sync_label).on_hover_text("Sync delay time to the sequencer BPM.").clicked() {
                            self.fx_delay_sync = !self.fx_delay_sync;
                        }
                    });

                    if self.fx_delay_sync {
                        // Note division selector
                        let bpm = self.seq_bpm as f32;
                        let beat_sec = 60.0 / bpm;
                        ui.horizontal_wrapped(|ui| {
                            for (i, (name, _)) in DELAY_DIVISIONS.iter().enumerate() {
                                let active = self.fx_delay_division == i;
                                let btn_label = egui::RichText::new(*name).small()
                                    .color(if active { col_dly } else { Color32::GRAY });
                                if ui.button(btn_label).on_hover_text(format!("Set delay to {} note ({:.0} BPM → {:.3}s)", name, bpm, beat_sec * DELAY_DIVISIONS[i].1)).clicked() {
                                    self.fx_delay_division = i;
                                }
                            }
                        });
                        // Compute and display synced time
                        let synced_time = (beat_sec * DELAY_DIVISIONS[self.fx_delay_division].1).clamp(0.01, 1.0);
                        self.fx_delay_time = synced_time;
                        ui.label(egui::RichText::new(format!("{:.3} s  @{}BPM", synced_time, self.seq_bpm)).small().color(Color32::DARK_GRAY))
                            .on_hover_text("Current delay time computed from BPM and selected note division.");
                    } else {
                        ui.add(egui::Slider::new(&mut self.fx_delay_time, 0.01_f32..=1.0)
                            .text("Time").suffix(" s").clamp_to_range(true))
                            .on_hover_text("Delay time in seconds (10 ms – 1 s).");
                    }

                    ui.add(egui::Slider::new(&mut self.fx_delay_feedback, 0.0_f32..=0.95)
                        .text("Feedback").clamp_to_range(true))
                        .on_hover_text("Feedback amount — how much of the delayed signal repeats.");
                    ui.add(egui::Slider::new(&mut self.fx_delay_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Wet/dry mix.");
                    self.state.fx_delay_time.set_value(self.fx_delay_time);
                    self.state.fx_delay_feedback.set_value(self.fx_delay_feedback);
                    if self.fx_delay_on {
                        self.state.fx_delay_mix.set_value(self.fx_delay_mix);
                    }
                });
            });

            // ---- Reverb ----
            ui.group(|ui| {
                ui.set_min_width(130.0);
                ui.vertical(|ui| {
                    let on = &mut self.fx_reverb_on;
                    let label = egui::RichText::new("REVERB").small().strong()
                        .color(if *on { col_rev } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Toggle reverb (Schroeder plate-style reverb).").clicked() {
                        *on = !*on;
                        self.state.fx_reverb_mix.set_value(if *on { self.fx_reverb_mix } else { 0.0 });
                    }
                    ui.add(egui::Slider::new(&mut self.fx_reverb_size, 0.0_f32..=1.0)
                        .text("Size").clamp_to_range(true))
                        .on_hover_text("Room size — controls reverb decay time.");
                    ui.add(egui::Slider::new(&mut self.fx_reverb_damp, 0.0_f32..=1.0)
                        .text("Damp").clamp_to_range(true))
                        .on_hover_text("High-frequency damping — 0 = bright, 1 = dark/muffled.");
                    ui.add(egui::Slider::new(&mut self.fx_reverb_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Wet/dry mix.");
                    self.state.fx_reverb_size.set_value(self.fx_reverb_size);
                    self.state.fx_reverb_damp.set_value(self.fx_reverb_damp);
                    if self.fx_reverb_on {
                        self.state.fx_reverb_mix.set_value(self.fx_reverb_mix);
                    }
                });
            });
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn midi_note_name(midi: u8) -> &'static str {
    match midi % 12 {
        0 => "C",
        1 => "C#",
        2 => "D",
        3 => "D#",
        4 => "E",
        5 => "F",
        6 => "F#",
        7 => "G",
        8 => "G#",
        9 => "A",
        10 => "A#",
        11 => "B",
        _ => "?",
    }
}
