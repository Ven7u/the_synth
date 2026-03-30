//! The Synth — unified MiniMoog-style synthesizer
//! Run with: cargo run

#![allow(clippy::precedence)]

mod audio;
mod osc;

use audio::{AudioEngine, AudioState, VOICE_COUNT};
use eframe::egui;
use egui::{Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use fundsp::prelude::midi_hz;
use std::sync::Arc;
use std::sync::atomic::Ordering;

fn main() -> eframe::Result {
    let engine = AudioEngine::new().expect("Failed to start audio");
    let state  = Arc::clone(&engine.state);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 740.0])
            .with_title("The Synth"),
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
    _audio: AudioEngine,  // keeps cpal stream alive
    state: Arc<AudioState>,

    // OSC bank
    osc_wave:    [usize; 3],   // 0=sine 1=saw 2=square 3=triangle
    osc_octave:  [i32; 3],     // -2..+2
    osc_detune:  [f32; 3],     // -100..+100 cents
    osc_vol:     [f32; 3],
    osc_enabled: [bool; 3],

    // Noise
    noise_vol: f32,

    // LFO
    lfo_rate:  f32,
    lfo_depth: f32,
    lfo_dest:  usize,         // 0=pitch 1=filter 2=amp

    // Filter
    filter_cutoff:     f32,
    filter_q:          f32,
    filter_env_amount: f32,
    fenv_adsr: [f32; 4],

    // Amp ADSR
    amp_adsr: [f32; 4],

    // Glide + master
    glide_time: f32,
    master_vol: f32,

    // Keyboard
    piano_octave:      i32,
    piano_held_midi:   std::collections::HashSet<u8>,
    piano_voice_notes: [Option<u8>; VOICE_COUNT],
    piano_steal_idx:   usize,
    piano_mouse_midi:  Option<u8>,

    // Sequencer
    seq_playing:      bool,
    seq_bpm:          u32,
    seq_steps:        [bool; 8],
    seq_notes:        [u8; 8],
    seq_current_step: usize,
    seq_last_tick:    std::time::Instant,
    seq_prev_midi:    Option<u8>,
}

impl SynthApp {
    fn new(state: Arc<AudioState>, audio: AudioEngine) -> Self {
        Self {
            _audio: audio,
            state,
            osc_wave:    [0, 0, 0],  // sine x3 — default until filter is in place
            osc_octave:  [0, 0, 0],
            osc_detune:  [0.0, 0.0, 0.0],
            osc_vol:     [0.5, 0.5, 0.5],
            osc_enabled: [true, true, false],
            noise_vol:  0.0,
            lfo_rate:   2.0,
            lfo_depth:  0.0,
            lfo_dest:   1,
            filter_cutoff:     3000.0,
            filter_q:          1.0,
            filter_env_amount: 0.3,
            fenv_adsr: [0.01, 0.3, 0.0, 0.2],
            amp_adsr:  [0.01, 0.15, 0.7, 0.4],
            glide_time: 0.0,
            master_vol: 0.5,
            piano_octave:      4,
            piano_held_midi:   std::collections::HashSet::new(),
            piano_voice_notes: [None; VOICE_COUNT],
            piano_steal_idx:   0,
            piano_mouse_midi:  None,
            seq_playing:      false,
            seq_bpm:          120,
            seq_steps:        [true, false, true, false, true, true, false, true],
            seq_notes:        [60, 62, 64, 67, 69, 72, 67, 64],
            seq_current_step: 0,
            seq_last_tick:    std::time::Instant::now(),
            seq_prev_midi:    None,
        }
    }
}

// ---------------------------------------------------------------------------
// Voice management
// ---------------------------------------------------------------------------

impl SynthApp {
    fn voice_on(&mut self, midi: u8) {
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

    fn voice_off(&mut self, midi: u8) {
        for (slot, note) in self.piano_voice_notes.iter_mut().enumerate() {
            if *note == Some(midi) {
                *note = None;
                self.state.voice_gates[slot].set(0.0);
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sequencer tick
// ---------------------------------------------------------------------------

impl SynthApp {
    fn tick_sequencer(&mut self, ctx: &egui::Context) {
        if !self.seq_playing { return; }
        let step_dur = std::time::Duration::from_millis(60_000 / self.seq_bpm as u64 / 2);
        if self.seq_last_tick.elapsed() < step_dur { return; }
        self.seq_last_tick = std::time::Instant::now();

        // Release previous step note
        if let Some(m) = self.seq_prev_midi.take() { self.voice_off(m); }

        self.seq_current_step = (self.seq_current_step + 1) % 8;
        if self.seq_steps[self.seq_current_step] {
            let midi = self.seq_notes[self.seq_current_step];
            self.voice_on(midi);
            self.seq_prev_midi = Some(midi);
        }
        ctx.request_repaint_after(step_dur);
    }
}

// ---------------------------------------------------------------------------
// Main update
// ---------------------------------------------------------------------------

impl eframe::App for SynthApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick_sequencer(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
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

            // Row 3: Keyboard + Sequencer
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("KEYBOARD & SEQUENCER").strong().small());
            });
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.columns(2, |cols| {
                    self.ui_keyboard_panel(&mut cols[0]);
                    self.ui_sequencer_panel(&mut cols[1]);
                });
            });

            ui.add_space(4.0);

            // Oscilloscope footer
            let buf = self.state.osc_buffer.lock().unwrap().clone();
            draw_oscilloscope(ui, &buf, 70.0);
        });

        ctx.request_repaint();
    }
}

// ---------------------------------------------------------------------------
// OSC panel
// ---------------------------------------------------------------------------

const WAVE_LABELS: &[&str] = &["Sin", "Saw", "Sqr", "Tri"];

impl SynthApp {
    fn ui_osc_panel(&mut self, ui: &mut egui::Ui, i: usize) {
        // Header: label + on/off toggle
        ui.horizontal(|ui| {
            let label = egui::RichText::new(format!("OSC {}", i + 1)).strong();
            let on = self.osc_enabled[i];
            let text = if on { label.color(Color32::from_rgb(0, 220, 160)) } else { label.color(Color32::GRAY) };
            if ui.button(text).clicked() {
                self.osc_enabled[i] = !on;
                let vol = if self.osc_enabled[i] { self.osc_vol[i] } else { 0.0 };
                self.state.osc_vol[i].set(vol);
            }
        });

        // Controls greyed out when disabled
        ui.add_enabled_ui(self.osc_enabled[i], |ui| {
            // Waveform selector
            ui.horizontal(|ui| {
                for (w, &label) in WAVE_LABELS.iter().enumerate() {
                    if ui.selectable_label(self.osc_wave[i] == w, label).clicked() {
                        self.osc_wave[i] = w;
                        self.state.osc_wave[i].store(w as u8, Ordering::Relaxed);
                    }
                }
            });

            // Octave
            ui.horizontal(|ui| {
                ui.label("Oct:");
                if ui.small_button("−").clicked() && self.osc_octave[i] > -2 {
                    self.osc_octave[i] -= 1;
                    self.update_freq_mult(i);
                }
                ui.label(format!("{:+}", self.osc_octave[i]));
                if ui.small_button("+").clicked() && self.osc_octave[i] < 2 {
                    self.osc_octave[i] += 1;
                    self.update_freq_mult(i);
                }
            });

            // Detune
            ui.horizontal(|ui| {
                ui.label("Det:");
                if ui.add(egui::Slider::new(&mut self.osc_detune[i], -100.0..=100.0)
                    .text("¢").fixed_decimals(0))
                    .changed()
                {
                    self.update_freq_mult(i);
                }
            });
        });
    }

    fn update_freq_mult(&self, i: usize) {
        let oct   = self.osc_octave[i] as f32;
        let cents = self.osc_detune[i];
        let mult  = 2_f32.powf(oct + cents / 1200.0);
        self.state.osc_freq_mult[i].set(mult);
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
                    if ui.add(egui::Slider::new(&mut self.osc_vol[i], 0.0..=1.0)
                        .vertical()
                        .text(format!("{}", i + 1)))
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
                if ui.add(egui::Slider::new(&mut self.noise_vol, 0.0..=1.0)
                    .vertical()
                    .text("N"))
                    .changed()
                {
                    self.state.noise_vol.set(self.noise_vol);
                }
            });
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Vol:");
            if ui.add(egui::Slider::new(&mut self.master_vol, 0.0..=1.0)).changed() {
                self.state.master_vol.set(self.master_vol);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Glide:");
            if ui.add(egui::Slider::new(&mut self.glide_time, 0.0..=0.5).text("s")).changed() {
                self.state.glide_time.set(self.glide_time);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// LFO panel
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_lfo_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("LFO").strong());
        ui.horizontal(|ui| {
            ui.label("Rate:");
            if ui.add(egui::Slider::new(&mut self.lfo_rate, 0.1..=20.0)
                .text("Hz").logarithmic(true))
                .changed()
            {
                self.state.lfo_rate.set(self.lfo_rate);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Depth:");
            if ui.add(egui::Slider::new(&mut self.lfo_depth, 0.0..=1.0)).changed() {
                self.state.lfo_depth.set(self.lfo_depth);
            }
        });
        ui.horizontal(|ui| {
            ui.label("→");
            for (d, label) in [(0usize, "Pitch"), (1, "Filter"), (2, "Amp")] {
                if ui.selectable_label(self.lfo_dest == d, label).clicked() {
                    self.lfo_dest = d;
                    self.state.lfo_dest.store(d as u8, Ordering::Relaxed);
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Filter panel
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_filter_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("FILTER").strong());
        ui.horizontal(|ui| {
            ui.label("Cut:");
            if ui.add(egui::Slider::new(&mut self.filter_cutoff, 80.0..=18000.0)
                .text("Hz").logarithmic(true))
                .changed()
            {
                self.state.cutoff.set(self.filter_cutoff);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Res:");
            if ui.add(egui::Slider::new(&mut self.filter_q, 0.5..=20.0)
                .text("Q").logarithmic(true))
                .changed()
            {
                self.state.resonance.set(self.filter_q);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Env:");
            if ui.add(egui::Slider::new(&mut self.filter_env_amount, 0.0..=1.0)).changed() {
                self.state.filter_env_amount.set(self.filter_env_amount);
            }
        });
    }
}

// ---------------------------------------------------------------------------
// ADSR panel (shared for filter env and amp env)
// ---------------------------------------------------------------------------

impl SynthApp {
    fn ui_adsr_panel(&mut self, ui: &mut egui::Ui, title: &str, _slots: &mut [usize; 4], is_filter: bool) {
        ui.label(egui::RichText::new(title).strong());

        let adsr = if is_filter { &mut self.fenv_adsr } else { &mut self.amp_adsr };
        let labels = ["A", "D", "S", "R"];
        let ranges: [std::ops::RangeInclusive<f32>; 4] = [
            0.001..=2.0,
            0.001..=2.0,
            0.0..=1.0,
            0.001..=4.0,
        ];

        ui.horizontal(|ui| {
            for i in 0..4 {
                ui.vertical(|ui| {
                    ui.set_width(28.0);
                    let log = i != 2;
                    let changed = ui.add(
                        egui::Slider::new(&mut adsr[i], ranges[i].clone())
                            .vertical()
                            .logarithmic(log)
                            .text(labels[i])
                    ).changed();
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
    }
}

// ---------------------------------------------------------------------------
// Keyboard panel
// ---------------------------------------------------------------------------

const WHITE_SEMITONES: &[i32]         = &[0, 2, 4, 5, 7, 9, 11];
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
            ui.label("Octave:");
            if ui.button("−").clicked() && self.piano_octave > 1 { self.piano_octave -= 1; }
            ui.label(format!("{}", self.piano_octave));
            if ui.button("+").clicked() && self.piano_octave < 7 { self.piano_octave += 1; }
            ui.label(egui::RichText::new("  a–l = white keys, w e t y u = sharps").weak().small());
        });

        // Keyboard input
        let mut current_held = std::collections::HashSet::<u8>::new();
        ui.input(|inp| {
            for &(key, semitone) in KEY_MAP {
                if inp.key_down(key) {
                    current_held.insert((self.piano_octave * 12 + semitone) as u8);
                }
            }
        });
        for &midi in &current_held {
            if !self.piano_held_midi.contains(&midi) { self.voice_on(midi); }
        }
        let released: Vec<u8> = self.piano_held_midi.iter()
            .filter(|&&m| !current_held.contains(&m)).copied().collect();
        for midi in released { self.voice_off(midi); }
        self.piano_held_midi = current_held;

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
                let pressed = self.piano_held_midi.contains(&midi) || self.piano_mouse_midi == Some(midi);
                let fill = if pressed { Color32::from_rgb(100, 180, 255) } else { Color32::WHITE };
                painter.rect_filled(rect, Rounding::same(3.0), fill);
                painter.rect_stroke(rect, Rounding::same(3.0), Stroke::new(1.0, Color32::DARK_GRAY));
                if let Some(pos) = pointer_pos { if rect.contains(pos) { clicked_midi = Some(midi); } }
            }
        }
        for oct in 0..2_i32 {
            for (bi, semi_opt) in BLACK_SEMITONES.iter().enumerate() {
                let Some(semi) = semi_opt else { continue };
                let x = (oct * 7 + bi as i32) as f32 * white_w + white_w * 0.6;
                let rect = Rect::from_min_size(
                    origin + Vec2::new(x, 1.0),
                    Vec2::new(black_w, black_h),
                );
                let midi = ((self.piano_octave + oct) * 12 + semi) as u8;
                let pressed = self.piano_held_midi.contains(&midi) || self.piano_mouse_midi == Some(midi);
                let fill = if pressed { Color32::from_rgb(60, 120, 200) } else { Color32::BLACK };
                painter.rect_filled(rect, Rounding::same(2.0), fill);
                if let Some(pos) = pointer_pos { if rect.contains(pos) { clicked_midi = Some(midi); } }
            }
        }

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
// Sequencer panel
// ---------------------------------------------------------------------------

const SEQ_SCALE: &[u8] = &[60, 62, 64, 67, 69, 72, 74, 76];

impl SynthApp {
    fn ui_sequencer_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let btn = if self.seq_playing { "⏹ Stop" } else { "▶ Play" };
            if ui.button(btn).clicked() {
                self.seq_playing = !self.seq_playing;
                if !self.seq_playing {
                    if let Some(m) = self.seq_prev_midi.take() { self.voice_off(m); }
                }
            }
            ui.label("BPM:");
            ui.add(egui::Slider::new(&mut self.seq_bpm, 40..=200));
            if ui.button("🎲").clicked() {
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

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for i in 0..8 {
                ui.vertical(|ui| {
                    ui.set_width(52.0);
                    if ui.small_button("▲").clicked() {
                        let pos = SEQ_SCALE.iter().position(|&n| n == self.seq_notes[i]).unwrap_or(0);
                        self.seq_notes[i] = SEQ_SCALE[(pos + 1).min(SEQ_SCALE.len() - 1)];
                    }
                    ui.label(egui::RichText::new(midi_note_name(self.seq_notes[i])).monospace().small());

                    let is_current = self.seq_playing && self.seq_current_step == i;
                    let is_on = self.seq_steps[i];
                    let fill = if is_current {
                        Color32::from_rgb(255, 200, 50)
                    } else if is_on {
                        Color32::from_rgb(0, 180, 120)
                    } else {
                        Color32::from_rgb(40, 40, 55)
                    };
                    let (r, painter) = ui.allocate_painter(Vec2::splat(40.0), Sense::click());
                    painter.rect_filled(r.rect, Rounding::same(5.0), fill);
                    painter.rect_stroke(r.rect, Rounding::same(5.0),
                        Stroke::new(1.0, if is_current { Color32::WHITE } else { Color32::GRAY }));
                    if r.clicked() { self.seq_steps[i] = !self.seq_steps[i]; }

                    if ui.small_button("▼").clicked() {
                        let pos = SEQ_SCALE.iter().position(|&n| n == self.seq_notes[i]).unwrap_or(0);
                        self.seq_notes[i] = SEQ_SCALE[pos.saturating_sub(1)];
                    }
                });
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Oscilloscope
// ---------------------------------------------------------------------------

fn draw_oscilloscope(ui: &mut egui::Ui, buffer: &[f32], height: f32) {
    let (resp, painter) = ui.allocate_painter(
        Vec2::new(ui.available_width(), height),
        Sense::hover(),
    );
    let rect = resp.rect;
    painter.rect_filled(rect, Rounding::same(4.0), Color32::from_rgb(10, 15, 20));
    if buffer.is_empty() { return; }

    let mid_y  = rect.center().y;
    let half_h = rect.height() * 0.45;
    let step   = rect.width() / buffer.len() as f32;

    painter.line_segment(
        [Pos2::new(rect.left(), mid_y), Pos2::new(rect.right(), mid_y)],
        Stroke::new(1.0, Color32::from_rgb(30, 40, 50)),
    );
    let points: Vec<Pos2> = buffer.iter().enumerate()
        .map(|(i, &s)| Pos2::new(rect.left() + i as f32 * step, mid_y - s * half_h))
        .collect();
    for w in points.windows(2) {
        painter.line_segment([w[0], w[1]], Stroke::new(1.5, Color32::from_rgb(0, 220, 160)));
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn midi_note_name(midi: u8) -> &'static str {
    match midi % 12 {
        0  => "C",  1  => "C#", 2  => "D",  3  => "D#",
        4  => "E",  5  => "F",  6  => "F#", 7  => "G",
        8  => "G#", 9  => "A",  10 => "A#", 11 => "B",
        _  => "?",
    }
}
