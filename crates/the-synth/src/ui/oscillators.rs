use crate::SynthApp;
use crate::ui::frame::SynthFrame;
use eframe::egui;
use egui::{Color32, RichText};
use std::sync::atomic::Ordering;

pub const WAVE_LABELS: &[&str] = &["Sin", "Saw", "Sqr", "Tri"];

impl SynthApp {
    pub fn ui_osc_panel(&mut self, ui: &mut egui::Ui, i: usize) {
        let sp_xs = self.theme.sp_xs;
        let sp_sm = self.theme.sp_sm;

        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let on = self.osc_enabled[i];

            // ── Header ────────────────────────────────────────────────────
            ui.horizontal(|ui| {
                let col = if on {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.text_disabled)
                };
                let text = RichText::new(format!("OSC {}", i + 1))
                    .size(12.0)
                    .strong()
                    .color(col);
                if ui
                    .add(egui::SelectableLabel::new(on, text))
                    .on_hover_text("Toggle oscillator on/off")
                    .clicked()
                {
                    self.osc_enabled[i] = !on;
                    let vol = if self.osc_enabled[i] { self.osc_vol[i] } else { 0.0 };
                    self.state.osc_vol[i].set(vol);
                }
            });

            ui.add_space(sp_xs);

            ui.add_enabled_ui(on, |ui| {
                // ── Waveform selector ─────────────────────────────────────
                let tips = [
                    "Sine — pure tone, no harmonics.",
                    "Sawtooth — bright buzz, all harmonics.",
                    "Square — hollow/woody, odd harmonics only.",
                    "Triangle — odd harmonics, softer than square.",
                ];
                ui.horizontal_wrapped(|ui| {
                    for (w, &label) in WAVE_LABELS.iter().enumerate() {
                        if ui
                            .selectable_label(self.osc_wave[i] == w, label)
                            .on_hover_text(tips[w])
                            .clicked()
                        {
                            self.osc_wave[i] = w;
                            self.state.osc_wave[i].store(w as u8, Ordering::Relaxed);
                        }
                    }
                });

                ui.add_space(sp_xs);

                // ── Octave + Detune ───────────────────────────────────────
                ui.horizontal(|ui| {
                    // Octave stepper
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("OCT")
                                .size(10.0)
                                .color(self.theme.c(&self.theme.text_secondary)),
                        );
                        ui.horizontal(|ui| {
                            if ui
                                .small_button("−")
                                .on_hover_text("One octave down")
                                .clicked()
                                && self.osc_octave[i] > -2
                            {
                                self.osc_octave[i] -= 1;
                                self.update_freq_mult(i);
                            }
                            ui.label(
                                RichText::new(format!("{:+}", self.osc_octave[i]))
                                    .size(12.0)
                                    .monospace(),
                            );
                            if ui
                                .small_button("+")
                                .on_hover_text("One octave up")
                                .clicked()
                                && self.osc_octave[i] < 2
                            {
                                self.osc_octave[i] += 1;
                                self.update_freq_mult(i);
                            }
                        });
                    });

                    ui.add_space(sp_xs);

                    // Detune knob (±100 ¢)
                    if super::widgets::knob(
                        ui,
                        &mut self.osc_detune[i],
                        -100.0..=100.0,
                        "DET",
                        &self.theme,
                        false,
                    )
                    .on_hover_text("Fine-tune in cents (±100 ¢ = ±1 semitone). Shift+drag for precision.")
                    .changed()
                    {
                        self.update_freq_mult(i);
                    }
                });

                // ── Pulse width (Square only) ─────────────────────────────
                if self.osc_wave[i] == 2 {
                    ui.add_space(sp_xs);
                    ui.horizontal(|ui| {
                        let pw_on = self.osc_pw_enabled[i];
                        let pw_col = if pw_on {
                            self.theme.c(&self.theme.accent)
                        } else {
                            self.theme.c(&self.theme.text_disabled)
                        };
                        if ui
                            .add(egui::SelectableLabel::new(
                                pw_on,
                                RichText::new("PW").size(10.0).color(pw_col),
                            ))
                            .on_hover_text("Pulse Width — vary square wave duty cycle")
                            .clicked()
                        {
                            self.osc_pw_enabled[i] = !pw_on;
                            if !self.osc_pw_enabled[i] {
                                self.osc_pulse_width[i] = 0.5;
                                self.state.osc_pulse_width[i].set(0.5);
                            }
                        }
                        ui.add_enabled_ui(pw_on, |ui| {
                            if ui
                                .add(
                                    egui::DragValue::new(&mut self.osc_pulse_width[i])
                                        .range(0.01..=0.99)
                                        .speed(0.005)
                                        .fixed_decimals(2),
                                )
                                .on_hover_text("Duty cycle: 0.5 = square, narrow = thin/nasal")
                                .changed()
                            {
                                self.state.osc_pulse_width[i].set(self.osc_pulse_width[i]);
                            }
                        });
                    });
                }

                // ── Unison ────────────────────────────────────────────────
                ui.add_space(sp_xs);
                ui.horizontal(|ui| {
                    let uni_on = self.osc_unison_enabled[i];
                    let uni_col = if uni_on {
                        self.theme.c(&self.theme.accent)
                    } else {
                        self.theme.c(&self.theme.text_disabled)
                    };
                    if ui
                        .add(egui::SelectableLabel::new(
                            uni_on,
                            RichText::new("UNI").size(10.0).color(uni_col),
                        ))
                        .on_hover_text("Stack detuned copies for a thick, wide sound")
                        .clicked()
                    {
                        self.osc_unison_enabled[i] = !uni_on;
                        self.update_unison(i);
                    }

                    if uni_on {
                        let mut changed = false;
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.osc_unison_count[i])
                                    .range(2..=5)
                                    .prefix("×"),
                            )
                            .on_hover_text("Number of unison voices (2–5)")
                            .changed();
                        changed |= super::widgets::knob(
                            ui,
                            &mut self.osc_unison_spread[i],
                            0.0..=50.0,
                            "SPRD",
                            &self.theme,
                            false,
                        )
                        .on_hover_text("Total pitch spread across unison voices (cents)")
                        .changed();
                        if changed {
                            self.update_unison(i);
                        }
                    }
                });

                // ── OSC 1 extras: Sync / FM / Ring ───────────────────────
                if i == 0 {
                    ui.add_space(sp_xs);
                    ui.separator();
                    ui.add_space(sp_xs);

                    // Hard sync
                    ui.horizontal(|ui| {
                        let on = self.hard_sync;
                        let col = if on {
                            self.theme.c(&self.theme.accent_hard_sync)
                        } else {
                            self.theme.c(&self.theme.text_disabled)
                        };
                        if ui
                            .add(egui::SelectableLabel::new(
                                on,
                                RichText::new("SYNC").size(10.0).color(col),
                            ))
                            .on_hover_text("Hard Sync — OSC 1 resets OSC 2's phase on every cycle. Sweep OSC 2's pitch for classic sync sweep.")
                            .clicked()
                        {
                            self.hard_sync = !on;
                            self.state
                                .hard_sync_enabled
                                .store(self.hard_sync, std::sync::atomic::Ordering::Relaxed);
                        }
                        ui.label(
                            RichText::new("→ OSC 2")
                                .size(10.0)
                                .color(self.theme.c(&self.theme.text_disabled)),
                        );
                    });

                    // FM
                    ui.add_space(sp_xs);
                    ui.horizontal(|ui| {
                        let on = self.fm_enabled;
                        let col = if on {
                            self.theme.c(&self.theme.accent_fm)
                        } else {
                            self.theme.c(&self.theme.text_disabled)
                        };
                        if ui
                            .add(egui::SelectableLabel::new(
                                on,
                                RichText::new("FM").size(10.0).color(col),
                            ))
                            .on_hover_text("Frequency Modulation — OSC 2 modulates OSC 1's pitch at audio rate")
                            .clicked()
                        {
                            self.fm_enabled = !on;
                            self.state
                                .fm_depth
                                .set(if self.fm_enabled { self.fm_depth } else { 0.0 });
                        }
                        ui.add_enabled_ui(self.fm_enabled, |ui| {
                            if ui
                                .add_sized(
                                    [ui.available_width(), 16.0],
                                    egui::Slider::new(&mut self.fm_depth, 0.0..=10.0)
                                        .fixed_decimals(1),
                                )
                                .on_hover_text("FM depth — ~1 subtle, 3–5 bells, 8+ chaotic sidebands")
                                .changed()
                            {
                                self.state.fm_depth.set(self.fm_depth);
                            }
                        });
                    });

                    // Ring mod
                    ui.add_space(sp_xs);
                    ui.horizontal(|ui| {
                        let on = self.ring_enabled;
                        let col = if on {
                            self.theme.c(&self.theme.accent_ring)
                        } else {
                            self.theme.c(&self.theme.text_disabled)
                        };
                        if ui
                            .add(egui::SelectableLabel::new(
                                on,
                                RichText::new("RING").size(10.0).color(col),
                            ))
                            .on_hover_text("Ring Mod — OSC 1 × OSC 2: metallic, bell-like textures")
                            .clicked()
                        {
                            self.ring_enabled = !on;
                            self.state
                                .ring_depth
                                .set(if self.ring_enabled { self.ring_depth } else { 0.0 });
                        }
                        ui.add_enabled_ui(self.ring_enabled, |ui| {
                            if ui
                                .add_sized(
                                    [ui.available_width(), 16.0],
                                    egui::Slider::new(&mut self.ring_depth, 0.0..=2.0)
                                        .fixed_decimals(2),
                                )
                                .on_hover_text("Ring mod depth — mute OSC 1 and 2 in mixer for pure ring mod")
                                .changed()
                            {
                                self.state.ring_depth.set(self.ring_depth);
                            }
                        });
                    });
                }
            }); // add_enabled_ui
        }); // section
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
        let sp_xs = self.theme.sp_xs;

        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            // Header
            ui.label(
                RichText::new("MIX")
                    .size(12.0)
                    .strong()
                    .color(self.theme.c(&self.theme.text_primary)),
            );
            ui.add_space(sp_xs);

            // Vertical faders for OSC 1/2/3 + Noise
            ui.horizontal(|ui| {
                for i in 0..3 {
                    ui.vertical(|ui| {
                        ui.set_width(36.0);
                        ui.label(
                            RichText::new(format!("O{}", i + 1))
                                .size(10.0)
                                .color(if self.osc_enabled[i] {
                                    self.theme.c(&self.theme.text_primary)
                                } else {
                                    self.theme.c(&self.theme.text_disabled)
                                }),
                        );
                        if ui
                            .add_sized(
                                [20.0, 90.0],
                                egui::Slider::new(&mut self.osc_vol[i], 0.0..=1.0)
                                    .vertical()
                                    .fixed_decimals(2),
                            )
                            .on_hover_text(format!("OSC {} volume in the mix", i + 1))
                            .changed()
                            && self.osc_enabled[i]
                        {
                            self.state.osc_vol[i].set(self.osc_vol[i]);
                        }
                    });
                }

                // Noise fader
                ui.vertical(|ui| {
                    ui.set_width(36.0);
                    ui.label(
                        RichText::new("N")
                            .size(10.0)
                            .color(self.theme.c(&self.theme.text_secondary)),
                    );
                    if ui
                        .add_sized(
                            [20.0, 90.0],
                            egui::Slider::new(&mut self.noise_vol, 0.0..=1.0)
                                .vertical()
                                .fixed_decimals(2),
                        )
                        .on_hover_text("White noise volume — adds breathiness or full noise textures")
                        .changed()
                    {
                        self.state.noise_vol.set(self.noise_vol);
                    }
                });
            });

            ui.add_space(sp_xs);
            ui.separator();
            ui.add_space(sp_xs);

            // Master + Glide knobs
            ui.horizontal(|ui| {
                if super::widgets::knob(
                    ui,
                    &mut self.master_vol,
                    0.0..=1.0,
                    "MAST",
                    &self.theme,
                    false,
                )
                .on_hover_text("Master output volume — applied after all FX")
                .changed()
                {
                    self.state.master_vol.set(self.master_vol);
                }

                if super::widgets::knob(
                    ui,
                    &mut self.glide_time,
                    0.0..=0.5,
                    "GLIDE",
                    &self.theme,
                    false,
                )
                .on_hover_text("Pitch slide time between notes (seconds)")
                .changed()
                {
                    self.state.glide_time.set(self.glide_time);
                }
            });

            ui.add_space(sp_xs);

            // Limiter
            ui.horizontal(|ui| {
                let lim_on = self.limiter_enabled;
                let lim_col = if lim_on {
                    self.theme.c(&self.theme.accent_limiter)
                } else {
                    self.theme.c(&self.theme.text_disabled)
                };
                if ui
                    .add(egui::SelectableLabel::new(
                        lim_on,
                        RichText::new("LIM").size(10.0).color(lim_col),
                    ))
                    .on_hover_text("Limiter — prevents output clipping")
                    .clicked()
                {
                    self.limiter_enabled = !lim_on;
                    self.state
                        .limiter_enabled
                        .store(self.limiter_enabled, Ordering::Relaxed);
                }
                ui.add_enabled_ui(lim_on, |ui| {
                    if ui
                        .add(
                            egui::DragValue::new(&mut self.limiter_threshold)
                                .range(0.5..=1.0)
                                .speed(0.005)
                                .fixed_decimals(2),
                        )
                        .on_hover_text("Threshold — lower = more compression")
                        .changed()
                        && lim_on
                    {
                        self.state.limiter_threshold.set(self.limiter_threshold);
                    }
                });
            });
        });
    }
}
