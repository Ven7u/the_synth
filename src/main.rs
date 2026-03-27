//! The Synth — interactive DSP learning app
//! Run with: cargo run

#![allow(clippy::precedence)]

mod audio;

use audio::{AudioEngine, AudioState, VOICE_COUNT};
use eframe::egui;
use egui::{Color32, Painter, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use fundsp::prelude::midi_hz;
use std::sync::Arc;
use std::sync::atomic::Ordering;

fn main() -> eframe::Result {
    let engine = AudioEngine::new().expect("Failed to start audio");
    let state = Arc::clone(&engine.state);

    // Keep engine alive for the duration of the app
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([920.0, 620.0])
            .with_title("The Synth — DSP Learning"),
        ..Default::default()
    };

    eframe::run_native(
        "The Synth",
        options,
        Box::new(move |_cc| Ok(Box::new(SynthApp::new(state, engine)))),
    )
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct SynthApp {
    audio: AudioEngine,          // keeps stream alive
    state: Arc<AudioState>,
    active_tab: usize,

    // Waveform tab local UI state
    waveform_freq: f32,
    waveform_vol: f32,
    waveform_playing: bool,

    // Filter tab local UI state
    filter_cutoff: f32,
    filter_q: f32,

    // Piano tab local UI state
    piano_octave: i32,
    /// MIDI notes currently held by keyboard keys (multiple at once = polyphony)
    piano_held_midi: std::collections::HashSet<u8>,
    /// Which MIDI note each voice slot is playing (None = voice is free / in release)
    piano_voice_notes: [Option<u8>; VOICE_COUNT],
    /// Rotates through slots for voice stealing when all are busy
    piano_steal_idx: usize,
    /// MIDI note currently held by mouse click (None if mouse not pressed)
    piano_mouse_midi: Option<u8>,
    adsr: [f32; 4], // attack, decay, sustain, release

    // Sequencer tab local UI state
    seq_playing: bool,
    seq_bpm: u32,
    seq_steps: [bool; 8],
    seq_notes: [u8; 8],   // MIDI note numbers
    seq_current_step: usize,
    seq_selected_step: usize,
    seq_last_tick: std::time::Instant,
}

impl SynthApp {
    fn new(state: Arc<AudioState>, audio: AudioEngine) -> Self {
        // Pentatonic scale default pattern (C D E G A in octave 4)
        let seq_notes = [60u8, 62, 64, 67, 69, 72, 67, 64];
        Self {
            audio,
            state,
            active_tab: 0,
            waveform_freq: 440.0,
            waveform_vol: 0.4,
            waveform_playing: false,
            filter_cutoff: 1000.0,
            filter_q: 1.0,
            piano_octave: 4,
            piano_held_midi: std::collections::HashSet::new(),
            piano_voice_notes: [None; VOICE_COUNT],
            piano_steal_idx: 0,
            piano_mouse_midi: None,
            adsr: [0.01, 0.1, 0.7, 0.4],
            seq_playing: false,
            seq_bpm: 120,
            seq_steps: [true, false, true, false, true, true, false, true],
            seq_notes,
            seq_current_step: 0,
            seq_selected_step: 0,
            seq_last_tick: std::time::Instant::now(),
        }
    }
}

impl eframe::App for SynthApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Advance sequencer clock
        self.tick_sequencer(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            // Tab bar
            ui.horizontal(|ui| {
                for (i, label) in ["🌊 Waveform", "🎛 Filter", "🎹 Piano", "🥁 Sequencer"]
                    .iter()
                    .enumerate()
                {
                    if ui.selectable_label(self.active_tab == i, *label).clicked() {
                        self.active_tab = i;
                        self.state.active_tab.store(i as u8, Ordering::Relaxed);
                        // Stop any playing note when switching tabs
                        self.state.gate.set(0.0);
                        self.waveform_playing = false;
                    }
                }
            });
            ui.separator();

            match self.active_tab {
                0 => self.ui_waveform(ui),
                1 => self.ui_filter(ui),
                2 => self.ui_piano(ui),
                3 => self.ui_sequencer(ui),
                _ => {}
            }
        });

        // Request continuous repaints so oscilloscope updates
        ctx.request_repaint();
    }
}

// ---------------------------------------------------------------------------
// Oscilloscope widget (shared across tabs)
// ---------------------------------------------------------------------------

fn draw_oscilloscope(ui: &mut egui::Ui, buffer: &[f32], height: f32) {
    let (resp, painter) = ui.allocate_painter(
        Vec2::new(ui.available_width(), height),
        Sense::hover(),
    );
    let rect = resp.rect;
    painter.rect_filled(rect, Rounding::same(4.0), Color32::from_rgb(10, 15, 20));

    if buffer.is_empty() { return; }

    let mid_y = rect.center().y;
    let half_h = rect.height() * 0.45;
    let step = rect.width() / buffer.len() as f32;

    let points: Vec<Pos2> = buffer
        .iter()
        .enumerate()
        .map(|(i, &s)| Pos2::new(rect.left() + i as f32 * step, mid_y - s * half_h))
        .collect();

    // Draw zero line
    painter.line_segment(
        [Pos2::new(rect.left(), mid_y), Pos2::new(rect.right(), mid_y)],
        Stroke::new(1.0, Color32::from_rgb(30, 40, 50)),
    );

    // Draw waveform
    for w in points.windows(2) {
        painter.line_segment([w[0], w[1]], Stroke::new(1.5, Color32::from_rgb(0, 220, 160)));
    }
}

// ---------------------------------------------------------------------------
// Tab 0 — Waveform explorer
// ---------------------------------------------------------------------------

const WAVEFORM_LABELS: &[&str] = &["Sine", "Saw", "Square", "Triangle"];

impl SynthApp {
    fn ui_waveform(&mut self, ui: &mut egui::Ui) {
        ui.label("Select a waveform to hear and see how its shape differs.");
        ui.add_space(8.0);

        // Waveform buttons
        ui.horizontal(|ui| {
            let current = self.state.wave_type.load(Ordering::Relaxed) as usize;
            for (i, label) in WAVEFORM_LABELS.iter().enumerate() {
                if ui.selectable_label(current == i, *label).clicked() {
                    self.state.wave_type.store(i as u8, Ordering::Relaxed);
                }
            }
        });

        ui.add_space(8.0);

        // Frequency slider
        ui.horizontal(|ui| {
            ui.label("Frequency:");
            if ui.add(egui::Slider::new(&mut self.waveform_freq, 55.0..=880.0)
                .text("Hz")
                .logarithmic(true))
                .changed()
            {
                self.state.frequency.set(self.waveform_freq);
            }
        });

        // Volume slider
        ui.horizontal(|ui| {
            ui.label("Volume:    ");
            if ui.add(egui::Slider::new(&mut self.waveform_vol, 0.0..=1.0)).changed() {
                self.state.volume.set(self.waveform_vol);
            }
        });

        ui.add_space(8.0);

        // Play/stop toggle
        let btn_label = if self.waveform_playing { "⏹ Stop" } else { "▶ Play" };
        if ui.button(btn_label).clicked() {
            self.waveform_playing = !self.waveform_playing;
            self.state.gate.set(if self.waveform_playing { 1.0 } else { 0.0 });
            self.state.frequency.set(self.waveform_freq);
        }

        ui.add_space(12.0);

        // Oscilloscope
        ui.label("Oscilloscope:");
        let buf = self.state.osc_buffer.lock().unwrap().clone();
        draw_oscilloscope(ui, &buf, 200.0);

        ui.add_space(8.0);
        ui.label(egui::RichText::new(
            "💡 Tip: Sine = pure tone (one frequency). Saw/Square/Triangle have harmonics — \
             extra frequencies that give them their characteristic buzzy or hollow sound.",
        ).weak());
    }
}

// ---------------------------------------------------------------------------
// Tab 1 — Filter explorer
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_filter(&mut self, ui: &mut egui::Ui) {
        ui.label("A filter removes some frequencies and passes others. Drag the sliders to hear the effect.");
        ui.add_space(8.0);

        // Source selector
        ui.horizontal(|ui| {
            ui.label("Source:");
            let src = self.state.filter_source.load(Ordering::Relaxed);
            if ui.selectable_label(src == 0, "Saw drone").clicked() {
                self.state.filter_source.store(0, Ordering::Relaxed);
            }
            if ui.selectable_label(src == 1, "Pink noise").clicked() {
                self.state.filter_source.store(1, Ordering::Relaxed);
            }
        });

        // Filter type selector
        ui.horizontal(|ui| {
            ui.label("Filter: ");
            let ft = self.state.filter_type.load(Ordering::Relaxed);
            if ui.selectable_label(ft == 0, "Lowpass").clicked() {
                self.state.filter_type.store(0, Ordering::Relaxed);
            }
            if ui.selectable_label(ft == 1, "Highpass").clicked() {
                self.state.filter_type.store(1, Ordering::Relaxed);
            }
            if ui.selectable_label(ft == 2, "Bandpass").clicked() {
                self.state.filter_type.store(2, Ordering::Relaxed);
            }
        });

        ui.add_space(8.0);

        // Cutoff slider (logarithmic — frequency perception is logarithmic)
        ui.horizontal(|ui| {
            ui.label("Cutoff:    ");
            if ui.add(egui::Slider::new(&mut self.filter_cutoff, 80.0..=12000.0)
                .text("Hz")
                .logarithmic(true))
                .changed()
            {
                self.state.cutoff.set(self.filter_cutoff);
            }
        });

        // Resonance / Q slider
        ui.horizontal(|ui| {
            ui.label("Resonance:");
            if ui.add(egui::Slider::new(&mut self.filter_q, 0.5..=20.0)
                .text("Q")
                .logarithmic(true))
                .changed()
            {
                self.state.resonance.set(self.filter_q);
            }
        });

        ui.add_space(12.0);

        let buf = self.state.osc_buffer.lock().unwrap().clone();
        draw_oscilloscope(ui, &buf, 200.0);

        ui.add_space(8.0);
        ui.label(egui::RichText::new(
            "💡 Tip: Lowpass = keeps bass, removes treble. \
             Highpass = keeps treble, removes bass. \
             High Q adds a resonant peak at the cutoff frequency.",
        ).weak());
    }
}

// ---------------------------------------------------------------------------
// Tab 2 — Piano keyboard
// ---------------------------------------------------------------------------

// White key semitone offsets (C D E F G A B) repeated for drawing 2 octaves
const WHITE_SEMITONES: &[i32] = &[0, 2, 4, 5, 7, 9, 11];
// Black key semitone offsets per white-key slot (None = no black key at that gap)
const BLACK_SEMITONES: &[Option<i32>] = &[
    Some(1), Some(3), None, Some(6), Some(8), Some(10), None,
];

// Flat keyboard map: (key, semitone-offset-from-octave-base)
//   a s d f g h j k l  →  C D E F G A B C D  (white keys, k/l wrap to next octave)
//   w e   t y u        →  C# D# _ F# G# A#   (black keys)
const KEY_MAP: &[(egui::Key, i32)] = &[
    (egui::Key::A, 0),   // C
    (egui::Key::W, 1),   // C#
    (egui::Key::S, 2),   // D
    (egui::Key::E, 3),   // D#
    (egui::Key::D, 4),   // E
    (egui::Key::F, 5),   // F
    (egui::Key::T, 6),   // F#
    (egui::Key::G, 7),   // G
    (egui::Key::Y, 8),   // G#
    (egui::Key::H, 9),   // A
    (egui::Key::U, 10),  // A#
    (egui::Key::J, 11),  // B
    (egui::Key::K, 12),  // C (next octave)
    (egui::Key::L, 14),  // D (next octave)
];

impl SynthApp {
    /// Assign a free voice slot to this MIDI note and trigger it.
    /// If all slots are busy, steal the oldest one (round-robin).
    fn voice_on(&mut self, midi: u8) {
        // Don't double-trigger an already-playing note
        if self.piano_voice_notes.iter().any(|&n| n == Some(midi)) { return; }
        let slot = self.piano_voice_notes.iter().position(|n| n.is_none())
            .unwrap_or_else(|| {
                let s = self.piano_steal_idx % VOICE_COUNT;
                self.piano_steal_idx += 1;
                s
            });
        self.piano_voice_notes[slot] = Some(midi);
        self.state.voice_freqs[slot].set(midi_hz(midi as f64) as f32);
        self.state.voice_gates[slot].set(1.0);
    }

    /// Trigger the release phase of the voice holding this MIDI note.
    fn voice_off(&mut self, midi: u8) {
        for (slot, note) in self.piano_voice_notes.iter_mut().enumerate() {
            if *note == Some(midi) {
                *note = None;
                self.state.voice_gates[slot].set(0.0);
                return;
            }
        }
    }

    fn ui_piano(&mut self, ui: &mut egui::Ui) {
        // Waveform selector
        ui.horizontal(|ui| {
            ui.label("Waveform:");
            let current = self.state.wave_type.load(Ordering::Relaxed) as usize;
            for (i, label) in WAVEFORM_LABELS.iter().enumerate() {
                if ui.selectable_label(current == i, *label).clicked() {
                    self.state.wave_type.store(i as u8, Ordering::Relaxed);
                }
            }
            ui.separator();
            ui.label("Octave:");
            if ui.button("−").clicked() && self.piano_octave > 1 { self.piano_octave -= 1; }
            ui.label(format!("{}", self.piano_octave));
            if ui.button("+").clicked() && self.piano_octave < 7 { self.piano_octave += 1; }
        });

        ui.add_space(8.0);

        // Collect every key currently held this frame
        let mut current_held = std::collections::HashSet::<u8>::new();
        ui.input(|inp| {
            for &(key, semitone) in KEY_MAP {
                if inp.key_down(key) {
                    current_held.insert((self.piano_octave * 12 + semitone) as u8);
                }
            }
        });

        // Newly pressed keys → trigger voice
        for &midi in &current_held {
            if !self.piano_held_midi.contains(&midi) {
                self.voice_on(midi);
            }
        }
        // Released keys → release voice
        let released: Vec<u8> = self.piano_held_midi.iter()
            .filter(|&&m| !current_held.contains(&m))
            .copied().collect();
        for midi in released { self.voice_off(midi); }

        self.piano_held_midi = current_held;

        // Draw the piano keyboard
        self.draw_piano(ui);

        ui.add_space(8.0);

        // ADSR knobs (as sliders — simpler than custom knob widgets)
        ui.label("Envelope (ADSR):");
        ui.horizontal(|ui| {
            let labels = ["Attack", "Decay", "Sustain", "Release"];
            let ranges = [
                0.001_f32..=2.0,
                0.001..=2.0,
                0.0..=1.0,
                0.001..=4.0,
            ];
            for (i, (label, range)) in labels.iter().zip(ranges).enumerate() {
                ui.vertical(|ui| {
                    ui.set_width(90.0);
                    ui.label(*label);
                    let log = i != 2; // sustain is linear
                    if ui.add(
                        egui::Slider::new(&mut self.adsr[i], range)
                            .vertical()
                            .logarithmic(log)
                    ).changed() {
                        match i {
                            0 => self.state.adsr_attack.set(self.adsr[0]),
                            1 => self.state.adsr_decay.set(self.adsr[1]),
                            2 => self.state.adsr_sustain.set(self.adsr[2]),
                            _ => self.state.adsr_release.set(self.adsr[3]),
                        }
                    }
                });
            }
        });

        ui.add_space(8.0);
        ui.label(egui::RichText::new(
            "💡 Hold multiple keys at once for chords! a–l = white keys, w e t y u = sharps. \
             Mouse click/drag also works. Up to 6 voices play simultaneously.",
        ).weak());
    }

    fn draw_piano(&mut self, ui: &mut egui::Ui) {
        let white_w = 36.0_f32;
        let white_h = 120.0_f32;
        let black_w = 22.0_f32;
        let black_h = 74.0_f32;
        let num_white = 14; // 2 octaves
        let total_width = white_w * num_white as f32;

        let (resp, painter) = ui.allocate_painter(
            Vec2::new(total_width, white_h + 4.0),
            Sense::click_and_drag(),
        );
        let origin = resp.rect.left_top();

        let pointer_pos = resp.interact_pointer_pos();

        let mut clicked_midi: Option<u8> = None;

        // Draw white keys
        for oct in 0..2_i32 {
            for (wi, &semi) in WHITE_SEMITONES.iter().enumerate() {
                let x = (oct * 7 + wi as i32) as f32 * white_w;
                let rect = Rect::from_min_size(
                    origin + Vec2::new(x + 1.0, 1.0),
                    Vec2::new(white_w - 2.0, white_h - 2.0),
                );
                let midi = ((self.piano_octave + oct) * 12 + semi) as u8;
                let is_pressed = self.piano_held_midi.contains(&midi)
                    || self.piano_mouse_midi == Some(midi);

                let fill = if is_pressed {
                    Color32::from_rgb(100, 180, 255)
                } else {
                    Color32::WHITE
                };
                painter.rect_filled(rect, Rounding::same(3.0), fill);
                painter.rect_stroke(rect, Rounding::same(3.0), Stroke::new(1.0, Color32::DARK_GRAY));

                if let Some(pos) = pointer_pos {
                    if rect.contains(pos) { clicked_midi = Some(midi); }
                }
            }
        }

        // Draw black keys (on top)
        for oct in 0..2_i32 {
            for (bi, semi_opt) in BLACK_SEMITONES.iter().enumerate() {
                let Some(semi) = semi_opt else { continue };
                let x = (oct * 7 + bi as i32) as f32 * white_w + white_w * 0.6;
                let rect = Rect::from_min_size(
                    origin + Vec2::new(x, 1.0),
                    Vec2::new(black_w, black_h),
                );
                let midi = ((self.piano_octave + oct) * 12 + semi) as u8;
                let is_pressed = self.piano_held_midi.contains(&midi)
                    || self.piano_mouse_midi == Some(midi);

                let fill = if is_pressed {
                    Color32::from_rgb(60, 120, 200)
                } else {
                    Color32::BLACK
                };
                painter.rect_filled(rect, Rounding::same(2.0), fill);

                if let Some(pos) = pointer_pos {
                    if rect.contains(pos) { clicked_midi = Some(midi); }
                }
            }
        }

        // Handle mouse press/release — supports sliding between keys
        if resp.is_pointer_button_down_on() {
            if let Some(midi) = clicked_midi {
                if self.piano_mouse_midi != Some(midi) {
                    if let Some(old) = self.piano_mouse_midi { self.voice_off(old); }
                    self.piano_mouse_midi = Some(midi);
                    self.voice_on(midi);
                }
            }
        } else if let Some(midi) = self.piano_mouse_midi.take() {
            self.voice_off(midi);
        }
    }
}

// ---------------------------------------------------------------------------
// Tab 3 — Step sequencer
// ---------------------------------------------------------------------------

const SEQ_SCALE: &[u8] = &[60, 62, 64, 67, 69, 72, 74, 76]; // C pentatonic

impl SynthApp {
    fn tick_sequencer(&mut self, ctx: &egui::Context) {
        if !self.seq_playing { return; }

        let step_dur = std::time::Duration::from_millis(60_000 / self.seq_bpm as u64 / 2);
        if self.seq_last_tick.elapsed() < step_dur { return; }
        self.seq_last_tick = std::time::Instant::now();

        self.seq_current_step = (self.seq_current_step + 1) % 8;
        if self.seq_steps[self.seq_current_step] {
            let midi = self.seq_notes[self.seq_current_step];
            self.state.frequency.set(midi_hz(midi as f64) as f32);
            self.state.gate.set(1.0);
        } else {
            self.state.gate.set(0.0);
        }

        ctx.request_repaint_after(step_dur);
    }

    fn ui_sequencer(&mut self, ui: &mut egui::Ui) {
        // Transport controls
        ui.horizontal(|ui| {
            let btn = if self.seq_playing { "⏹ Stop" } else { "▶ Play" };
            if ui.button(btn).clicked() {
                self.seq_playing = !self.seq_playing;
                if !self.seq_playing { self.state.gate.set(0.0); }
            }
            ui.separator();
            ui.label("BPM:");
            ui.add(egui::Slider::new(&mut self.seq_bpm, 40..=200));
            ui.separator();
            if ui.button("🎲 Randomize").clicked() {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut h = DefaultHasher::new();
                std::time::SystemTime::now().hash(&mut h);
                let seed = h.finish();
                for i in 0..8 {
                    self.seq_steps[i] = (seed >> i) & 1 == 1;
                    self.seq_notes[i] = SEQ_SCALE[((seed >> (i * 3)) & 7) as usize % SEQ_SCALE.len()];
                }
            }
        });

        ui.add_space(12.0);
        ui.label("Steps (click to toggle on/off, ▲▼ to change pitch):");
        ui.add_space(6.0);

        // Step grid
        ui.horizontal(|ui| {
            for i in 0..8 {
                ui.vertical(|ui| {
                    ui.set_width(80.0);

                    // Pitch up/down — find current position in SEQ_SCALE, then step
                    if ui.small_button("▲").clicked() {
                        let pos = SEQ_SCALE.iter().position(|&n| n == self.seq_notes[i]).unwrap_or(0);
                        self.seq_notes[i] = SEQ_SCALE[(pos + 1).min(SEQ_SCALE.len() - 1)];
                    }

                    // Note name
                    let note_name = midi_note_name(self.seq_notes[i]);
                    ui.label(egui::RichText::new(note_name).monospace());

                    // Step toggle button — highlighted when active step
                    let is_current = self.seq_playing && self.seq_current_step == i;
                    let is_on = self.seq_steps[i];
                    let (fill, stroke) = if is_current {
                        (Color32::from_rgb(255, 200, 50), Stroke::new(2.0, Color32::WHITE))
                    } else if is_on {
                        (Color32::from_rgb(0, 180, 120), Stroke::new(1.0, Color32::WHITE))
                    } else {
                        (Color32::from_rgb(40, 40, 50), Stroke::new(1.0, Color32::GRAY))
                    };

                    let btn_size = Vec2::splat(50.0);
                    let (r, painter) = ui.allocate_painter(btn_size, Sense::click());
                    painter.rect_filled(r.rect, Rounding::same(6.0), fill);
                    painter.rect_stroke(r.rect, Rounding::same(6.0), stroke);
                    if r.clicked() { self.seq_steps[i] = !self.seq_steps[i]; }

                    if ui.small_button("▼").clicked() {
                        let pos = SEQ_SCALE.iter().position(|&n| n == self.seq_notes[i]).unwrap_or(0);
                        self.seq_notes[i] = SEQ_SCALE[pos.saturating_sub(1)];
                    }
                });
            }
        });

        ui.add_space(8.0);
        ui.label(egui::RichText::new(
            "💡 A step sequencer triggers notes at a fixed tempo. \
             Each lit step plays its note; dark steps are silent. \
             This is how drum machines and bassline synths work.",
        ).weak());
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn midi_note_name(midi: u8) -> &'static str {
    match midi % 12 {
        0  => "C",
        1  => "C#",
        2  => "D",
        3  => "D#",
        4  => "E",
        5  => "F",
        6  => "F#",
        7  => "G",
        8  => "G#",
        9  => "A",
        10 => "A#",
        11 => "B",
        _  => "?",
    }
}
