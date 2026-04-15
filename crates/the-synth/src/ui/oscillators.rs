use crate::SynthApp;
use eframe::egui;
use egui::Color32;
use std::sync::atomic::Ordering;

pub const WAVE_LABELS: &[&str] = &["Sin", "Saw", "Sqr", "Tri"];

impl SynthApp {
    pub fn ui_osc_panel(&mut self, ui: &mut egui::Ui, i: usize) {
        // Header: label + on/off toggle
        ui.horizontal(|ui| {
            let label = egui::RichText::new(format!("OSC {}", i + 1)).strong();
            let on = self.osc_enabled[i];
            let text = if on {
                label.color(self.theme.c(&self.theme.accent))
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
                        self.theme.c(&self.theme.accent)
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
                    self.theme.c(&self.theme.accent)
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
                        .color(if on { self.theme.c(&self.theme.accent_hard_sync) } else { Color32::GRAY });
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
                        .color(if on { self.theme.c(&self.theme.accent_fm) } else { Color32::GRAY });
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
                        .color(if on { self.theme.c(&self.theme.accent_ring) } else { Color32::GRAY });
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

    pub fn update_freq_mult(&self, i: usize) {
        let oct = self.osc_octave[i] as f32;
        let cents = self.osc_detune[i];
        let mult = 2_f32.powf(oct + cents / 1200.0);
        self.state.osc_freq_mult[i].set(mult);
    }

    pub fn update_unison(&self, i: usize) {
        let count = self.osc_unison_count[i];
        let spread = self.osc_unison_spread[i];

        if !self.osc_unison_enabled[i] || count <= 1 {
            for c in 0..5 {
                self.state.osc_unison_detune[i][c].set(1.0);
                self.state.osc_unison_vol[i][c].set(if c == 0 { 1.0 } else { 0.0 });
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
                self.state.osc_unison_detune[i][c].set(detune);
                self.state.osc_unison_vol[i][c].set(vol);
            } else {
                self.state.osc_unison_detune[i][c].set(1.0);
                self.state.osc_unison_vol[i][c].set(0.0);
            }
        }
    }

    pub fn ui_mixer_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("MIXER").strong());
        ui.horizontal(|ui| {
            for i in 0..3 {
                let label = format!("O{}", i + 1);
                if super::widgets::knob(ui, &mut self.osc_vol[i], 0.0..=1.0, &label, &self.theme)
                    .on_hover_text(format!("OSC {} volume in the mix.", i + 1))
                    .changed()
                {
                    if self.osc_enabled[i] {
                        self.state.osc_vol[i].set(self.osc_vol[i]);
                    }
                }
            }
            if super::widgets::knob(ui, &mut self.noise_vol, 0.0..=1.0, "Noise", &self.theme)
                .on_hover_text("White noise volume. Adds breathiness, air, or full noise textures.")
                .changed()
            {
                self.state.noise_vol.set(self.noise_vol);
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if super::widgets::knob(ui, &mut self.master_vol, 0.0..=1.0, "Vol", &self.theme)
                .on_hover_text("Master output volume.")
                .changed()
            {
                self.state.master_vol.set(self.master_vol);
            }
            if super::widgets::knob(ui, &mut self.glide_time, 0.0..=0.5, "Glide", &self.theme)
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
                egui::RichText::new("LIM").color(self.theme.c(&self.theme.accent_limiter))
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
        self.peak_display = (self.peak_display * 0.85 + peak_raw * 0.15).max(peak_raw * 0.3);

        let dt = 1.0 / 60.0_f32;
        if peak_raw > self.peak_hold {
            self.peak_hold = peak_raw;
            self.peak_hold_timer = 0.0;
        } else {
            self.peak_hold_timer += dt;
            if self.peak_hold_timer > 1.0 {
                self.peak_hold *= 0.95;
            }
        }

        super::scope::draw_peak_meter(ui, self.peak_display, self.peak_hold, &self.theme);
    }
}
