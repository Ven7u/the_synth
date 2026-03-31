//! The Synth — unified MiniMoog-style synthesizer
//! Run with: cargo run

#![allow(clippy::precedence)]

mod audio;
mod osc;

use audio::{AudioEngine, AudioState, VOICE_COUNT};
use eframe::egui;
use egui::{Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};
use fundsp::prelude::midi_hz;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn main() -> eframe::Result {
    let engine = AudioEngine::new().expect("Failed to start audio");
    let state = Arc::clone(&engine.state);

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
    _audio: AudioEngine, // keeps cpal stream alive
    state: Arc<AudioState>,

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
    lfo_rate: f32,
    lfo_depth: f32,
    lfo_dest: usize, // 0=pitch 1=filter 2=amp

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
    piano_voice_notes: [Option<u8>; VOICE_COUNT],
    piano_steal_idx: usize,
    piano_mouse_midi: Option<u8>,

    // Peak meter
    peak_display: f32,
    peak_hold: f32,
    peak_hold_timer: f32,

    // Limiter
    limiter_enabled: bool,
    limiter_threshold: f32,

    // Sequencer
    seq_playing: bool,
    seq_bpm: u32,
    seq_steps: [bool; 8],
    seq_notes: [u8; 8],
    seq_current_step: usize,
    seq_last_tick: std::time::Instant,
    seq_prev_midi: Option<u8>,

    // Oscilloscope
    scope_height: f32,
    scope_x_scale: f32,
    scope_y_scale: f32,
}

impl SynthApp {
    fn new(state: Arc<AudioState>, audio: AudioEngine) -> Self {
        Self {
            _audio: audio,
            state,
            osc_wave: [0, 0, 0], // sine x3 — default until filter is in place
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
            lfo_rate: 2.0,
            lfo_depth: 0.0,
            lfo_dest: 1,
            filter_cutoff: 3000.0,
            filter_q: 1.0,
            filter_env_amount: 0.3,
            fenv_adsr: [0.01, 0.3, 0.0, 0.2],
            amp_adsr: [0.01, 0.15, 0.7, 0.4],
            glide_time: 0.0,
            master_vol: 0.5,
            piano_octave: 4,
            piano_held_midi: std::collections::HashSet::new(),
            piano_voice_notes: [None; VOICE_COUNT],
            piano_steal_idx: 0,
            piano_mouse_midi: None,
            peak_display: 0.0,
            peak_hold: 0.0,
            peak_hold_timer: 0.0,
            limiter_enabled: true,
            limiter_threshold: 0.95,
            seq_playing: false,
            seq_bpm: 120,
            seq_steps: [true, false, true, false, true, true, false, true],
            seq_notes: [60, 62, 64, 67, 69, 72, 67, 64],
            seq_current_step: 0,
            seq_last_tick: std::time::Instant::now(),
            seq_prev_midi: None,
            scope_height: 90.0,
            scope_x_scale: 1.0,
            scope_y_scale: 1.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Voice management
// ---------------------------------------------------------------------------

impl SynthApp {
    fn voice_on(&mut self, midi: u8) {
        if self.piano_voice_notes.iter().any(|&n| n == Some(midi)) {
            return;
        }
        let slot = self
            .piano_voice_notes
            .iter()
            .position(|n| n.is_none())
            .unwrap_or_else(|| {
                let s = self.piano_steal_idx % VOICE_COUNT;
                self.piano_steal_idx += 1;
                s
            });
        self.piano_voice_notes[slot] = Some(midi);
        self.state.voice_freqs[slot].set(midi_hz(midi as f64) as f32);
        // Stamp the time just before setting the gate — the audio callback will
        // measure how long it takes to reach this note.
        if let Ok(mut t) = self.state.note_on_time.lock() {
            *t = Some(std::time::Instant::now());
        }
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
        if !self.seq_playing {
            return;
        }
        let step_dur = std::time::Duration::from_millis(60_000 / self.seq_bpm as u64 / 2);
        if self.seq_last_tick.elapsed() < step_dur {
            return;
        }
        self.seq_last_tick = std::time::Instant::now();

        // Release previous step note
        if let Some(m) = self.seq_prev_midi.take() {
            self.voice_off(m);
        }

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

            // Latency indicator
            draw_latency_bar(ui, &self.state, self.amp_adsr[0]);

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
            if ui.button(text).clicked() {
                self.osc_enabled[i] = !on;
                let vol = if self.osc_enabled[i] {
                    self.osc_vol[i]
                } else {
                    0.0
                };
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
                if ui
                    .add(
                        egui::Slider::new(&mut self.osc_detune[i], -100.0..=100.0)
                            .text("¢")
                            .fixed_decimals(0),
                    )
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
                    if ui.button(label).clicked() {
                        self.osc_pw_enabled[i] = !pw_on;
                        if !self.osc_pw_enabled[i] {
                            // Reset to 50% when disabled
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
                if ui.button(label).clicked() {
                    self.osc_unison_enabled[i] = !uni_on;
                    self.update_unison(i);
                }
                ui.add_enabled_ui(self.osc_unison_enabled[i], |ui| {
                    let mut changed = false;
                    changed |= ui
                        .add(egui::Slider::new(&mut self.osc_unison_count[i], 2..=5).text("v"))
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut self.osc_unison_spread[i], 0.0..=50.0)
                                .text("¢")
                                .fixed_decimals(0),
                        )
                        .changed();
                    if changed {
                        self.update_unison(i);
                    }
                });
            });

            // Hard sync toggle — only shown on OSC 1 (the master)
            if i == 0 {
                ui.horizontal(|ui| {
                    let on = self.hard_sync;
                    let label = egui::RichText::new("Sync→2").small()
                        .color(if on { Color32::from_rgb(255, 180, 0) } else { Color32::GRAY });
                    if ui.button(label).clicked() {
                        self.hard_sync = !on;
                        self.state.hard_sync_enabled.store(self.hard_sync, std::sync::atomic::Ordering::Relaxed);
                    }
                    ui.label(egui::RichText::new("OSC1 → OSC2").weak().small());
                });

                // FM: OSC 2 → OSC 1 pitch (audio-rate FM)
                ui.horizontal(|ui| {
                    let on = self.fm_enabled;
                    let label = egui::RichText::new("FM").small()
                        .color(if on { Color32::from_rgb(120, 180, 255) } else { Color32::GRAY });
                    if ui.button(label).clicked() {
                        self.fm_enabled = !on;
                        let depth = if self.fm_enabled { self.fm_depth } else { 0.0 };
                        self.state.fm_depth.set(depth);
                    }
                    ui.add_enabled_ui(self.fm_enabled, |ui| {
                        if ui.add(egui::Slider::new(&mut self.fm_depth, 0.0..=10.0)
                            .text("depth").fixed_decimals(1)).changed()
                        {
                            self.state.fm_depth.set(self.fm_depth);
                        }
                    });
                });

                // Ring mod: OSC 1 × OSC 2 added to mix
                ui.horizontal(|ui| {
                    let on = self.ring_enabled;
                    let label = egui::RichText::new("Ring").small()
                        .color(if on { Color32::from_rgb(255, 130, 200) } else { Color32::GRAY });
                    if ui.button(label).clicked() {
                        self.ring_enabled = !on;
                        let depth = if self.ring_enabled { self.ring_depth } else { 0.0 };
                        self.state.ring_depth.set(depth);
                    }
                    ui.add_enabled_ui(self.ring_enabled, |ui| {
                        if ui.add(egui::Slider::new(&mut self.ring_depth, 0.0..=2.0)
                            .text("depth").fixed_decimals(2)).changed()
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
                    .changed()
                {
                    self.state.noise_vol.set(self.noise_vol);
                }
            });
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Vol:");
            if ui
                .add(egui::Slider::new(&mut self.master_vol, 0.0..=1.0))
                .changed()
            {
                self.state.master_vol.set(self.master_vol);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Glide:");
            if ui
                .add(egui::Slider::new(&mut self.glide_time, 0.0..=0.5).text("s"))
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
            if ui.button(label).clicked() {
                self.limiter_enabled = !self.limiter_enabled;
                self.state
                    .limiter_enabled
                    .store(self.limiter_enabled, Ordering::Relaxed);
            }
            ui.add_enabled(
                self.limiter_enabled,
                egui::Slider::new(&mut self.limiter_threshold, 0.5..=1.0).text("Thr"),
            );
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
        ui.label(egui::RichText::new("LFO").strong());
        ui.horizontal(|ui| {
            ui.label("Rate:");
            if ui
                .add(
                    egui::Slider::new(&mut self.lfo_rate, 0.1..=20.0)
                        .text("Hz")
                        .logarithmic(true),
                )
                .changed()
            {
                self.state.lfo_rate.set(self.lfo_rate);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Depth:");
            if ui
                .add(egui::Slider::new(&mut self.lfo_depth, 0.0..=1.0))
                .changed()
            {
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
            if ui
                .add(
                    egui::Slider::new(&mut self.filter_cutoff, 80.0..=18000.0)
                        .text("Hz")
                        .logarithmic(true),
                )
                .changed()
            {
                self.state.cutoff.set(self.filter_cutoff);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Res:");
            if ui
                .add(
                    egui::Slider::new(&mut self.filter_q, 0.5..=20.0)
                        .text("Q")
                        .logarithmic(true),
                )
                .changed()
            {
                self.state.resonance.set(self.filter_q);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Env:");
            if ui
                .add(egui::Slider::new(&mut self.filter_env_amount, 0.0..=1.0))
                .changed()
            {
                self.state.filter_env_amount.set(self.filter_env_amount);
            }
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
            ui.label("Octave:");
            if ui.button("−").clicked() && self.piano_octave > 1 {
                self.piano_octave -= 1;
            }
            ui.label(format!("{}", self.piano_octave));
            if ui.button("+").clicked() && self.piano_octave < 7 {
                self.piano_octave += 1;
            }
            ui.label(
                egui::RichText::new("  a–l = white keys, w e t y u = sharps")
                    .weak()
                    .small(),
            );
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
            if !self.piano_held_midi.contains(&midi) {
                self.voice_on(midi);
            }
        }
        let released: Vec<u8> = self
            .piano_held_midi
            .iter()
            .filter(|&&m| !current_held.contains(&m))
            .copied()
            .collect();
        for midi in released {
            self.voice_off(midi);
        }
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
                        self.voice_off(old);
                    }
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
            let btn = if self.seq_playing {
                "⏹ Stop"
            } else {
                "▶ Play"
            };
            if ui.button(btn).clicked() {
                self.seq_playing = !self.seq_playing;
                if !self.seq_playing {
                    if let Some(m) = self.seq_prev_midi.take() {
                        self.voice_off(m);
                    }
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
                    self.seq_notes[i] =
                        SEQ_SCALE[((seed >> (i * 3)) & 7) as usize % SEQ_SCALE.len()];
                }
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for i in 0..8 {
                ui.vertical(|ui| {
                    ui.set_width(52.0);
                    if ui.small_button("▲").clicked() {
                        let pos = SEQ_SCALE
                            .iter()
                            .position(|&n| n == self.seq_notes[i])
                            .unwrap_or(0);
                        self.seq_notes[i] = SEQ_SCALE[(pos + 1).min(SEQ_SCALE.len() - 1)];
                    }
                    ui.label(
                        egui::RichText::new(midi_note_name(self.seq_notes[i]))
                            .monospace()
                            .small(),
                    );

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
                    painter.rect_stroke(
                        r.rect,
                        Rounding::same(5.0),
                        Stroke::new(
                            1.0,
                            if is_current {
                                Color32::WHITE
                            } else {
                                Color32::GRAY
                            },
                        ),
                    );
                    if r.clicked() {
                        self.seq_steps[i] = !self.seq_steps[i];
                    }

                    if ui.small_button("▼").clicked() {
                        let pos = SEQ_SCALE
                            .iter()
                            .position(|&n| n == self.seq_notes[i])
                            .unwrap_or(0);
                        self.seq_notes[i] = SEQ_SCALE[pos.saturating_sub(1)];
                    }
                });
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
            ui.label(egui::RichText::new("X").small().color(Color32::from_rgb(100, 180, 140)));
            ui.add(
                egui::DragValue::new(&mut self.scope_x_scale)
                    .speed(0.02)
                    .range(0.25_f32..=8.0)
                    .suffix("×"),
            );
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Y").small().color(Color32::from_rgb(100, 180, 140)));
            ui.add(
                egui::DragValue::new(&mut self.scope_y_scale)
                    .speed(0.02)
                    .range(0.25_f32..=8.0)
                    .suffix("×"),
            );
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
