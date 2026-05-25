use crate::ui::frame::SynthFrame;
use crate::SynthApp;
use eframe::egui;
use egui::{Color32, Pos2, RichText, Stroke};

/// Identifies which LFO panel hosts the gate-lane sub-row. Used by
/// `ui_lfo_gate_row` to dispatch reads/writes to the right SynthApp fields.
#[derive(Clone, Copy)]
enum LfoGate {
    L1,
    L2,
}

/// (label, beats_per_cycle) — beats relative to a quarter note.
/// rate_hz = bpm / 60.0 / beats_per_cycle
pub const LFO_SYNC_DIVISIONS: &[(&str, f32)] = &[
    ("4", 16.0), // 4 bars
    ("2", 8.0),  // 2 bars
    ("1", 4.0),  // 1 bar
    ("1/2", 2.0),
    ("1/4", 1.0),
    ("1/8", 0.5),
    ("1/16", 0.25),
    ("1/4T", 2.0 / 3.0), // quarter triplet
    ("1/8T", 1.0 / 3.0), // eighth triplet
];

pub fn lfo_synced_rate(bpm: f32, division: usize) -> f32 {
    let beats = LFO_SYNC_DIVISIONS[division.min(LFO_SYNC_DIVISIONS.len() - 1)].1;
    (bpm / 60.0 / beats).clamp(0.01, 20.0)
}

impl SynthApp {
    pub fn ui_lfo_panel(&mut self, ui: &mut egui::Ui) {
        let sp_xs = self.theme.sp_xs;

        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            // Header
            ui.horizontal(|ui| {
                let on = self.lfo_enabled;
                let col = if on {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.text_disabled)
                };
                if ui
                    .add(egui::SelectableLabel::new(
                        on,
                        RichText::new("LFO 1").size(12.0).strong().color(col),
                    ))
                    .on_hover_text(
                        "Low Frequency Oscillator — modulates pitch, filter cutoff, or amplitude",
                    )
                    .clicked()
                {
                    self.lfo_enabled = !on;
                    self.engine.set_lfo_depth(if self.lfo_enabled {
                        self.lfo_depth
                    } else {
                        0.0
                    });
                }
            });

            ui.add_space(sp_xs);

            ui.add_enabled_ui(self.lfo_enabled, |ui| {
                // Rate + Depth knobs
                ui.horizontal(|ui| {
                    let sync_on = self.lfo_sync_active();
                    if !sync_on {
                        if super::widgets::knob(
                            ui,
                            &mut self.lfo_rate,
                            0.1..=20.0,
                            "RATE",
                            &self.theme,
                            false,
                        )
                        .on_hover_text(
                            "LFO speed in Hz. 0.1 = very slow, 5 = fast vibrato, 20 = audio range.",
                        )
                        .changed()
                        {
                            self.engine.set_lfo_rate(self.lfo_rate);
                        }
                    }
                    if super::widgets::knob(
                        ui,
                        &mut self.lfo_depth,
                        0.0..=1.0,
                        "DEPTH",
                        &self.theme,
                        false,
                    )
                    .on_hover_text("Modulation depth. 0 = off, 1 = full.")
                    .changed()
                    {
                        self.engine.set_lfo_depth(self.lfo_depth);
                    }

                    // Sync toggle
                    ui.add_enabled_ui(!self.global_sync, |ui| {
                        let sync_on = self.lfo_sync_active();
                        let sync_col = if sync_on {
                            self.theme.c(&self.theme.accent)
                        } else {
                            self.theme.c(&self.theme.text_disabled)
                        };
                        if ui
                            .add(egui::SelectableLabel::new(
                                sync_on,
                                RichText::new("SYNC").size(10.0).color(sync_col),
                            ))
                            .on_hover_text("Lock LFO rate to a note division of the Global BPM")
                            .clicked()
                        {
                            self.lfo_sync = !self.lfo_sync;
                            if self.lfo_sync_active() {
                                let rate =
                                    lfo_synced_rate(self.global_bpm as f32, self.lfo_division);
                                self.lfo_rate = rate;
                                self.engine.set_lfo_rate(rate);
                            }
                        }
                    });
                });

                // Division selector (when synced)
                if self.lfo_sync_active() {
                    ui.add_space(sp_xs);
                    ui.horizontal_wrapped(|ui| {
                        for (i, &(label, _)) in LFO_SYNC_DIVISIONS.iter().enumerate() {
                            let active = self.lfo_division == i;
                            let rate = lfo_synced_rate(self.global_bpm as f32, i);
                            if ui
                                .selectable_label(
                                    active,
                                    RichText::new(label).small().color(if active {
                                        self.theme.c(&self.theme.accent)
                                    } else {
                                        self.theme.c(&self.theme.text_secondary)
                                    }),
                                )
                                .on_hover_text(format!(
                                    "{} → {:.3} Hz @ {} BPM",
                                    label, rate, self.global_bpm
                                ))
                                .clicked()
                            {
                                self.lfo_division = i;
                                self.lfo_rate = rate;
                                self.engine.set_lfo_rate(rate);
                            }
                        }
                    });
                }

                ui.add_space(sp_xs);

                // Shape
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("SHAPE")
                            .size(10.0)
                            .color(self.theme.c(&self.theme.text_secondary)),
                    );
                    let shape_tips = [
                        "Sine — smooth, natural modulation.",
                        "Triangle — linear ramp up and down.",
                        "Saw — ramps up then resets.",
                    ];
                    for (s, label) in [(0usize, "Sin"), (1, "Tri"), (2, "Saw")] {
                        if ui
                            .selectable_label(self.lfo_shape == s, label)
                            .on_hover_text(shape_tips[s])
                            .clicked()
                        {
                            self.lfo_shape = s;
                            self.engine.set_lfo_shape(s as u8);
                        }
                    }
                });

                // Destination
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("→")
                            .size(10.0)
                            .color(self.theme.c(&self.theme.text_secondary)),
                    );
                    let dest_tips = [
                        "Pitch — vibrato.",
                        "Filter — wobble / wah.",
                        "Amp — tremolo.",
                    ];
                    for (d, label) in [(0usize, "Pitch"), (1, "Filter"), (2, "Amp")] {
                        if ui
                            .selectable_label(self.lfo_dest == d, label)
                            .on_hover_text(dest_tips[d])
                            .clicked()
                        {
                            self.lfo_dest = d;
                            self.engine.set_lfo_dest(d as u8);
                        }
                    }
                });

                ui.add_space(sp_xs);
                ui.separator();
                self.ui_lfo_gate_row(ui, LfoGate::L1);
            });
        });
    }

    pub fn ui_lfo2_panel(&mut self, ui: &mut egui::Ui) {
        let sp_xs = self.theme.sp_xs;

        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            // Header
            ui.horizontal(|ui| {
                let on = self.lfo2_enabled;
                let col = if on {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.text_disabled)
                };
                if ui
                    .add(egui::SelectableLabel::new(
                        on,
                        RichText::new("LFO 2").size(12.0).strong().color(col),
                    ))
                    .on_hover_text("Second LFO — runs independently of LFO 1")
                    .clicked()
                {
                    self.lfo2_enabled = !on;
                    self.engine.set_lfo2_depth(if self.lfo2_enabled {
                        self.lfo2_depth
                    } else {
                        0.0
                    });
                }
            });

            ui.add_space(sp_xs);

            ui.add_enabled_ui(self.lfo2_enabled, |ui| {
                ui.horizontal(|ui| {
                    if super::widgets::knob(
                        ui,
                        &mut self.lfo2_rate,
                        0.01..=20.0,
                        "RATE",
                        &self.theme,
                        true,
                    )
                    .on_hover_text("LFO 2 rate in Hz — as slow as 0.01 Hz for breathing swells")
                    .changed()
                    {
                        self.engine.set_lfo2_rate(self.lfo2_rate);
                    }
                    if super::widgets::knob(
                        ui,
                        &mut self.lfo2_depth,
                        0.0..=1.0,
                        "DEPTH",
                        &self.theme,
                        false,
                    )
                    .on_hover_text("LFO 2 modulation depth")
                    .changed()
                    {
                        self.engine.set_lfo2_depth(self.lfo2_depth);
                    }
                });

                ui.add_space(sp_xs);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("SHAPE")
                            .size(10.0)
                            .color(self.theme.c(&self.theme.text_secondary)),
                    );
                    for (s, label) in [(0usize, "Sin"), (1, "Tri"), (2, "Saw")] {
                        if ui.selectable_label(self.lfo2_shape == s, label).clicked() {
                            self.lfo2_shape = s;
                            self.engine.set_lfo2_shape(s as u8);
                        }
                    }
                });

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("→")
                            .size(10.0)
                            .color(self.theme.c(&self.theme.text_secondary)),
                    );
                    for (d, label) in [(0usize, "Pitch"), (1, "Filter"), (2, "Amp")] {
                        if ui.selectable_label(self.lfo2_dest == d, label).clicked() {
                            self.lfo2_dest = d;
                            self.engine.set_lfo2_dest(d as u8);
                        }
                    }
                });

                ui.add_space(sp_xs);
                ui.separator();
                self.ui_lfo_gate_row(ui, LfoGate::L2);
            });
        });
    }

    /// "PULSE" — gate-lane sequencer that ducks the master output on each "on" step,
    /// tempo-synced to the global BPM. Visual style mirrors `ui_lfo_panel` /
    /// `ui_lfo2_panel`: a `SynthFrame::section` with header, knob row, division row,
    /// and a 16-cell step row below.
    pub fn ui_pulse_panel(&mut self, ui: &mut egui::Ui) {
        let sp_xs = self.theme.sp_xs;

        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            // Header
            ui.horizontal(|ui| {
                let on = self.pulse_enabled;
                let col = if on {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.text_disabled)
                };
                if ui
                    .add(egui::SelectableLabel::new(
                        on,
                        RichText::new("PULSE").size(12.0).strong().color(col),
                    ))
                    .on_hover_text(
                        "Tempo-synced sidechain ducker — every \"on\" step dips the master output",
                    )
                    .clicked()
                {
                    self.pulse_enabled = !on;
                    self.engine.set_gate_aenv_enabled(self.pulse_enabled);
                }
            });

            ui.add_space(sp_xs);

            ui.add_enabled_ui(self.pulse_enabled, |ui| {
                // Depth knob + length stepper
                ui.horizontal(|ui| {
                    if super::widgets::knob(
                        ui,
                        &mut self.pulse_depth,
                        0.0..=1.0,
                        "DEPTH",
                        &self.theme,
                        false,
                    )
                    .on_hover_text("How hard each step ducks the master output")
                    .changed()
                    {
                        self.engine.set_gate_aenv_depth(self.pulse_depth);
                    }

                    ui.label(
                        RichText::new("LEN")
                            .size(10.0)
                            .color(self.theme.c(&self.theme.text_secondary)),
                    );
                    let mut len = self.pulse_length as i32;
                    if ui
                        .add(egui::DragValue::new(&mut len).range(1..=16))
                        .on_hover_text("Number of active steps before the pattern repeats")
                        .changed()
                    {
                        self.pulse_length = len.clamp(1, 16) as u8;
                        self.engine.set_gate_aenv_length(self.pulse_length);
                    }
                });

                ui.add_space(sp_xs);

                // Division selector — same idiom as LFO sync. Always tempo-synced.
                ui.horizontal_wrapped(|ui| {
                    for div in forma_common::ClockDivision::ALL {
                        let div_u8 = div.to_u8();
                        let active = self.pulse_division as u8 == div_u8;
                        let label = div.label();
                        let rate = div.hz(self.global_bpm as f32);
                        if ui
                            .selectable_label(
                                active,
                                RichText::new(label).small().color(if active {
                                    self.theme.c(&self.theme.accent)
                                } else {
                                    self.theme.c(&self.theme.text_secondary)
                                }),
                            )
                            .on_hover_text(format!(
                                "{} → {:.3} Hz @ {} BPM",
                                label, rate, self.global_bpm
                            ))
                            .clicked()
                        {
                            self.pulse_division = div_u8 as usize;
                            self.engine.set_gate_aenv_division(div_u8);
                            self.engine.set_gate_aenv_rate(rate);
                        }
                    }
                });

                ui.add_space(sp_xs);

                // 16-cell step row.
                ui.label(
                    RichText::new("STEPS")
                        .size(10.0)
                        .color(self.theme.c(&self.theme.text_secondary)),
                );
                ui.horizontal(|ui| {
                    let total_w = ui.available_width();
                    let spacing = ui.spacing().item_spacing.x;
                    let step_w = ((total_w - spacing * 15.0) / 16.0).max(14.0);
                    let cell_h = 26.0;
                    let active_col = self.theme.c(&self.theme.accent);
                    let inactive_col = self.theme.c(&self.theme.bg_sunken);
                    let edge = self.theme.c(&self.theme.text_disabled);
                    for i in 0..16u8 {
                        let on_step = (self.pulse_pattern >> i) & 1 != 0;
                        let in_active_len = (i as u8) < self.pulse_length;
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::Vec2::new(step_w, cell_h),
                            egui::Sense::click(),
                        );
                        let painter = ui.painter_at(rect);
                        let fill = if on_step { active_col } else { inactive_col };
                        let alpha = if in_active_len { 255 } else { 90 };
                        let fill = egui::Color32::from_rgba_unmultiplied(
                            fill.r(),
                            fill.g(),
                            fill.b(),
                            alpha,
                        );
                        painter.rect_filled(rect, egui::CornerRadius::same(3), fill);
                        painter.rect_stroke(
                            rect,
                            egui::CornerRadius::same(3),
                            Stroke::new(1.0, edge),
                            egui::StrokeKind::Middle,
                        );
                        if resp.clicked() {
                            self.pulse_pattern ^= 1u16 << i;
                            self.engine.set_gate_aenv_pattern(self.pulse_pattern);
                        }
                    }
                });
            });
        });
    }

    /// Compact RETRIG sub-row appended inside `ui_lfo_panel` / `ui_lfo2_panel`.
    /// Renders a small enable toggle, a division combobox, and a 16-cell step row.
    /// Routes reads/writes through `which` so the same code drives both LFOs.
    fn ui_lfo_gate_row(&mut self, ui: &mut egui::Ui, which: LfoGate) {
        // Snapshot current state for the requested lane (avoids &mut self field aliasing).
        let mut enabled = match which {
            LfoGate::L1 => self.lfo1_gate_enabled,
            LfoGate::L2 => self.lfo2_gate_enabled,
        };
        let mut pattern = match which {
            LfoGate::L1 => self.lfo1_gate_pattern,
            LfoGate::L2 => self.lfo2_gate_pattern,
        };
        let length = match which {
            LfoGate::L1 => self.lfo1_gate_length,
            LfoGate::L2 => self.lfo2_gate_length,
        };
        let mut division = match which {
            LfoGate::L1 => self.lfo1_gate_division,
            LfoGate::L2 => self.lfo2_gate_division,
        };

        // Header row: RETRIG enable + division dropdown
        ui.horizontal(|ui| {
            let col = if enabled {
                self.theme.c(&self.theme.accent)
            } else {
                self.theme.c(&self.theme.text_disabled)
            };
            if ui
                .add(egui::SelectableLabel::new(
                    enabled,
                    RichText::new("RETRIG").size(10.0).strong().color(col),
                ))
                .on_hover_text(
                    "Resets the LFO's phase to 0 on each \"on\" step (tempo-synced).\n\
                     \n\
                     The LFO keeps running at its own RATE — retrigger only restarts the cycle.\n\
                     Set the LFO rate SLOWER than the retrigger rate so each gated step plays\n\
                     just the start of the LFO shape, like a tempo-locked envelope:\n\
                       • LFO 0.3–1 Hz + RETRIG 1/4 → rhythmic filter/amp sweeps.\n\
                       • LFO at the same rate as RETRIG → no audible effect (already aligned).\n\
                       • LFO faster than RETRIG → barely audible (only tiny phase glitches).",
                )
                .clicked()
            {
                enabled = !enabled;
            }

            let div = forma_common::ClockDivision::from_u8(division as u8);
            egui::ComboBox::from_id_salt(("lfo_gate_div", which as u8))
                .selected_text(div.label())
                .show_ui(ui, |ui| {
                    for d in forma_common::ClockDivision::ALL {
                        if ui
                            .selectable_label(div == *d, d.label())
                            .on_hover_text(format!(
                                "{} → {:.2} Hz @ {} BPM",
                                d.label(),
                                d.hz(self.global_bpm as f32),
                                self.global_bpm
                            ))
                            .clicked()
                        {
                            division = d.to_u8() as usize;
                        }
                    }
                });
        });

        // 16-cell step row.
        let mut pattern_changed = false;
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                let total_w = ui.available_width();
                let spacing = ui.spacing().item_spacing.x;
                let step_w = ((total_w - spacing * 15.0) / 16.0).max(10.0);
                let cell_h = 18.0;
                let active_col = self.theme.c(&self.theme.accent);
                let inactive_col = self.theme.c(&self.theme.bg_sunken);
                let edge = self.theme.c(&self.theme.text_disabled);
                for i in 0..16u8 {
                    let on_step = (pattern >> i) & 1 != 0;
                    let in_active_len = i < length;
                    let (rect, resp) = ui
                        .allocate_exact_size(egui::Vec2::new(step_w, cell_h), egui::Sense::click());
                    let painter = ui.painter_at(rect);
                    let fill = if on_step { active_col } else { inactive_col };
                    let alpha = if in_active_len { 255 } else { 90 };
                    let fill =
                        egui::Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), alpha);
                    painter.rect_filled(rect, egui::CornerRadius::same(2), fill);
                    painter.rect_stroke(
                        rect,
                        egui::CornerRadius::same(2),
                        Stroke::new(1.0, edge),
                        egui::StrokeKind::Middle,
                    );
                    if resp.clicked() {
                        pattern ^= 1u16 << i;
                        pattern_changed = true;
                    }
                }
            });
        });

        // Push changes back to SynthApp + engine.
        match which {
            LfoGate::L1 => {
                if self.lfo1_gate_enabled != enabled {
                    self.lfo1_gate_enabled = enabled;
                    self.engine.set_gate_lfo1_enabled(enabled);
                }
                if self.lfo1_gate_division != division {
                    self.lfo1_gate_division = division;
                    self.engine.set_gate_lfo1_division(division as u8);
                    let rate = forma_common::ClockDivision::from_u8(division as u8)
                        .hz(self.global_bpm as f32);
                    self.engine.set_gate_lfo1_rate(rate);
                }
                if pattern_changed {
                    self.lfo1_gate_pattern = pattern;
                    self.engine.set_gate_lfo1_pattern(pattern);
                }
            }
            LfoGate::L2 => {
                if self.lfo2_gate_enabled != enabled {
                    self.lfo2_gate_enabled = enabled;
                    self.engine.set_gate_lfo2_enabled(enabled);
                }
                if self.lfo2_gate_division != division {
                    self.lfo2_gate_division = division;
                    self.engine.set_gate_lfo2_division(division as u8);
                    let rate = forma_common::ClockDivision::from_u8(division as u8)
                        .hz(self.global_bpm as f32);
                    self.engine.set_gate_lfo2_rate(rate);
                }
                if pattern_changed {
                    self.lfo2_gate_pattern = pattern;
                    self.engine.set_gate_lfo2_pattern(pattern);
                }
            }
        }
    }

    pub fn ui_filter_panel(&mut self, ui: &mut egui::Ui) {
        let sp_xs = self.theme.sp_xs;

        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            // ── Header ───────────────────────────────────────────────────────
            ui.horizontal(|ui| {
                let on = self.filter_enabled;
                let col = if on {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.text_disabled)
                };
                if ui
                    .add(egui::SelectableLabel::new(
                        on,
                        RichText::new("FILTER").size(12.0).strong().color(col),
                    ))
                    .on_hover_text("Moog-style 4-pole lowpass filter")
                    .clicked()
                {
                    self.filter_enabled = !on;
                    if self.filter_enabled {
                        self.engine.set_filter_cutoff(self.filter_cutoff);
                        self.engine.set_filter_resonance(self.filter_q);
                    } else {
                        self.engine.set_filter_cutoff(18000.0);
                        self.engine.set_filter_resonance(0.0);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new("LP24")
                            .size(10.0)
                            .color(self.theme.c(&self.theme.text_secondary)),
                    );
                });
            });

            ui.add_space(sp_xs);

            // ── Mode buttons ─────────────────────────────────────────────────
            // Only LP is active; BP/HP/NOTCH are shown but disabled.
            ui.horizontal(|ui| {
                let accent = self.theme.c(&self.theme.accent);
                let disabled = self.theme.c(&self.theme.text_disabled);
                // LP — always selected
                ui.add(egui::SelectableLabel::new(
                    true,
                    RichText::new("LP").size(10.0).strong().color(accent),
                ));
                for label in ["BP", "HP", "NOTCH"] {
                    ui.add_enabled(
                        false,
                        egui::SelectableLabel::new(
                            false,
                            RichText::new(label).size(10.0).color(disabled),
                        ),
                    );
                }
            });

            ui.add_space(sp_xs);

            ui.add_enabled_ui(self.filter_enabled, |ui| {
                // ── Response curve ────────────────────────────────────────────
                let curve_h = ui.available_width().min(100.0);
                let curve_size = egui::Vec2::new(ui.available_width(), curve_h);
                let (rect, response) =
                    ui.allocate_exact_size(curve_size, egui::Sense::click_and_drag());

                // Absolute-position interaction: clicking anywhere in the rect
                // jumps the node there. Shift slows delta movement for fine tuning.
                if response.dragged() {
                    let fine = ui.input(|i| i.modifiers.shift);
                    if fine {
                        let delta = response.drag_delta();
                        let log_min = 80.0_f32.ln();
                        let log_max = 18000.0_f32.ln();
                        self.filter_cutoff = ((self.filter_cutoff.ln()
                            + delta.x / rect.width() * (log_max - log_min) * 0.15)
                            .clamp(log_min, log_max))
                        .exp();
                        self.filter_q = (self.filter_q - delta.y / rect.height() * 0.95 * 0.15)
                            .clamp(0.0, 0.95);
                    } else if let Some(pos) = response.interact_pointer_pos() {
                        let x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                        let y = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                        let log_min = 80.0_f32.ln();
                        let log_max = 18000.0_f32.ln();
                        self.filter_cutoff = (log_min + x * (log_max - log_min)).exp();
                        self.filter_q = (1.0 - y) * 0.95;
                    }
                    self.engine.set_filter_cutoff(self.filter_cutoff);
                    self.engine.set_filter_resonance(self.filter_q);
                }
                if response.double_clicked() {
                    self.filter_cutoff = 3000.0;
                    self.filter_q = 0.0;
                    self.engine.set_filter_cutoff(self.filter_cutoff);
                    self.engine.set_filter_resonance(self.filter_q);
                }

                // Show mod wheel filter contribution on the curve only when dest==Filter.
                let mod_offset = if self.mod_wheel_dest == 1 {
                    self.piano_mod_wheel as f32 / 5.0 * self.mod_wheel_depth * self.filter_cutoff * 0.5
                } else {
                    0.0
                };
                let effective_cutoff = (self.filter_cutoff + mod_offset).clamp(80.0, 18000.0);

                if ui.is_rect_visible(rect) {
                    draw_lp_response_curve(
                        ui.painter(),
                        rect,
                        effective_cutoff,
                        self.filter_q,
                        response.hovered() || response.dragged(),
                        &self.theme,
                    );
                }

                ui.add_space(sp_xs);

                // ── Knobs ─────────────────────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        let mut display_cutoff = effective_cutoff;
                        let knob_resp = super::widgets::knob(
                            ui,
                            &mut display_cutoff,
                            80.0..=18000.0,
                            "CUTOFF",
                            &self.theme,
                            true,
                        )
                        .on_hover_text("Cutoff frequency. 80 Hz = dark, 18 kHz = fully open.");
                        if knob_resp.changed() {
                            // Knob moved: keep mod offset, back-solve base cutoff.
                            // Approximate back-solve: base ≈ effective / (1 + depth*0.5) when offset
                            // is additive-relative; for simplicity just subtract the pre-computed offset.
                            self.filter_cutoff = (display_cutoff - mod_offset).clamp(80.0, 18000.0);
                            self.engine.set_filter_cutoff(self.filter_cutoff);
                        }
                        if mod_offset > 0.0 {
                            let hz_str = if effective_cutoff >= 1000.0 {
                                format!("{:.1}k", effective_cutoff / 1000.0)
                            } else {
                                format!("{:.0}", effective_cutoff)
                            };
                            ui.label(
                                RichText::new(format!("▲ {hz_str}"))
                                    .size(9.0)
                                    .color(Color32::from_rgb(180, 120, 255)),
                            )
                            .on_hover_text("Effective cutoff with mod wheel offset");
                        }
                    });

                    if super::widgets::knob(
                        ui,
                        &mut self.filter_q,
                        0.0..=0.95,
                        "RES",
                        &self.theme,
                        false,
                        )
                    .on_hover_text("Resonance — 0 = flat, 0.9+ = self-oscillation.")
                    .changed()
                    {
                        self.engine.set_filter_resonance(self.filter_q);
                    }

                    let mut drive = self.engine.filter_drive();
                    if super::widgets::knob(
                        ui,
                        &mut drive,
                        1.0..=10.0,
                        "DRIVE",
                        &self.theme,
                        false,
                        )
                    .on_hover_text(
                        "Input drive — saturates the signal before the filter. 1 = clean, 10 = heavy.",
                    )
                    .changed()
                    {
                        self.engine.set_filter_drive(drive);
                    }

                    let mut key_track = self.engine.filter_key_track();
                    if super::widgets::knob(
                        ui,
                        &mut key_track,
                        0.0..=1.0,
                        "KEY",
                        &self.theme,
                        false,
                        )
                    .on_hover_text(
                        "Keyboard tracking — cutoff follows pitch. 0 = off, 1 = full (one octave up doubles the cutoff).",
                    )
                    .changed()
                    {
                        self.engine.set_filter_key_track(key_track);
                    }

                    let mut vel_amp = self.engine.vel_amp();
                    if super::widgets::knob(
                        ui,
                        &mut vel_amp,
                        0.0..=1.0,
                        "VEL>AMP",
                        &self.theme,
                        false,
                    )
                    .on_hover_text(
                        "Velocity → amplitude. 0 = velocity ignored (always full volume), 1 = full sensitivity.",
                    )
                    .changed()
                    {
                        self.engine.set_vel_amp(vel_amp);
                    }

                    let mut vel_filter = self.engine.vel_filter();
                    if super::widgets::knob(
                        ui,
                        &mut vel_filter,
                        0.0..=1.0,
                        "VEL>FLT",
                        &self.theme,
                        false,
                    )
                    .on_hover_text(
                        "Velocity → filter cutoff. Hard hits open the filter. 0 = off, 1 = adds up to 8 kHz.",
                    )
                    .changed()
                    {
                        self.engine.set_vel_filter(vel_filter);
                    }
                });
            });
        });
    }

    pub fn ui_mod_matrix_panel(&mut self, ui: &mut egui::Ui) {
        use crate::ui::frame::SynthFrame;
        const SOURCES: &[&str] = &["Off", "LFO 1", "LFO 2", "Mod Wheel", "Aftertouch"];
        const DESTS: &[&str] = &["Off", "Filter", "Amp", "Pitch"];

        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new("MOD MATRIX")
                    .size(12.0)
                    .strong()
                    .color(self.theme.c(&self.theme.accent)),
            );
            ui.add_space(self.theme.sp_xs);

            egui::Grid::new("mod_matrix_grid")
                .num_columns(4)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    // Header
                    for label in ["#", "SOURCE", "→ DEST", "DEPTH"] {
                        ui.label(
                            RichText::new(label)
                                .size(9.0)
                                .color(self.theme.c(&self.theme.text_secondary)),
                        );
                    }
                    ui.end_row();

                    for slot in 0..4 {
                        // Row number
                        ui.label(
                            RichText::new(format!("{}", slot + 1))
                                .size(10.0)
                                .color(self.theme.c(&self.theme.text_disabled)),
                        );

                        // Source combo
                        egui::ComboBox::from_id_salt(format!("mat_src_{slot}"))
                            .selected_text(SOURCES[self.mat_src[slot].min(4)])
                            .width(90.0)
                            .show_ui(ui, |ui| {
                                for (i, label) in SOURCES.iter().enumerate() {
                                    if ui
                                        .selectable_label(self.mat_src[slot] == i, *label)
                                        .clicked()
                                    {
                                        self.mat_src[slot] = i;
                                        self.engine.set_mat_src(slot, i as u8);
                                    }
                                }
                            });

                        // Dest combo
                        egui::ComboBox::from_id_salt(format!("mat_dst_{slot}"))
                            .selected_text(DESTS[self.mat_dst[slot].min(3)])
                            .width(70.0)
                            .show_ui(ui, |ui| {
                                for (i, label) in DESTS.iter().enumerate() {
                                    if ui
                                        .selectable_label(self.mat_dst[slot] == i, *label)
                                        .clicked()
                                    {
                                        self.mat_dst[slot] = i;
                                        self.engine.set_mat_dst(slot, i as u8);
                                    }
                                }
                            });

                        // Depth drag
                        let mut d = self.mat_depth[slot];
                        let active = self.mat_src[slot] != 0 && self.mat_dst[slot] != 0;
                        let resp = ui.add_enabled(
                            active,
                            egui::DragValue::new(&mut d)
                                .range(-1.0_f32..=1.0)
                                .speed(0.01)
                                .fixed_decimals(2),
                        );
                        if resp.changed() {
                            self.mat_depth[slot] = d;
                            self.engine.set_mat_depth(slot, d);
                        }

                        ui.end_row();
                    }
                });
        });
    }

    pub fn ui_mod_wheel_panel(&mut self, ui: &mut egui::Ui) {
        use crate::ui::frame::SynthFrame;
        let sp_xs = self.theme.sp_xs;
        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("MOD WHEEL")
                        .size(12.0)
                        .strong()
                        .color(self.theme.c(&self.theme.accent)),
                );
            });
            ui.add_space(sp_xs);
            ui.horizontal(|ui| {
                let mut depth = self.mod_wheel_depth;
                if super::widgets::knob(ui, &mut depth, 0.0..=1.0, "DEPTH", &self.theme, false)
                    .on_hover_text("How much the mod wheel affects the selected destination.")
                    .changed()
                {
                    self.mod_wheel_depth = depth;
                    self.engine.set_mod_wheel_depth(depth);
                }
                ui.add_space(sp_xs);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("→")
                            .size(10.0)
                            .color(self.theme.c(&self.theme.text_secondary)),
                    );
                    for (d, label, tip) in [
                        (0usize, "Off", "Mod wheel has no effect."),
                        (1, "Filter", "Opens the filter as you push the wheel."),
                        (
                            2,
                            "LFO Depth",
                            "Scales LFO 1 depth — classic vibrato/wah control.",
                        ),
                        (3, "Amp", "Reduces amplitude — expression/swell."),
                    ] {
                        if ui
                            .selectable_label(self.mod_wheel_dest == d, label)
                            .on_hover_text(tip)
                            .clicked()
                        {
                            self.mod_wheel_dest = d;
                            self.engine.set_mod_wheel_dest(d as u8);
                        }
                    }
                });
            });
        });
    }

    pub fn ui_aftertouch_panel(&mut self, ui: &mut egui::Ui) {
        use crate::ui::frame::SynthFrame;
        let sp_xs = self.theme.sp_xs;
        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("AFTERTOUCH")
                        .size(12.0)
                        .strong()
                        .color(self.theme.c(&self.theme.accent)),
                );
            });
            ui.add_space(sp_xs);
            ui.horizontal(|ui| {
                let mut depth = self.aftertouch_depth;
                if super::widgets::knob(ui, &mut depth, 0.0..=1.0, "DEPTH", &self.theme, false)
                    .on_hover_text(
                        "How much channel pressure (aftertouch) affects the selected destination.",
                    )
                    .changed()
                {
                    self.aftertouch_depth = depth;
                    self.engine.set_aftertouch_depth(depth);
                }
                ui.add_space(sp_xs);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("→")
                            .size(10.0)
                            .color(self.theme.c(&self.theme.text_secondary)),
                    );
                    for (d, label, tip) in [
                        (0usize, "Off", "Aftertouch has no effect."),
                        (1, "Filter", "Pressing harder opens the filter."),
                        (2, "LFO Depth", "Pressing harder increases LFO 1 depth."),
                        (3, "Amp", "Pressing harder reduces volume — swell effect."),
                    ] {
                        if ui
                            .selectable_label(self.aftertouch_dest == d, label)
                            .on_hover_text(tip)
                            .clicked()
                        {
                            self.aftertouch_dest = d;
                            self.engine.set_aftertouch_dest(d as u8);
                        }
                    }
                });
            });
        });
    }

    pub fn ui_adsr_panel(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        _slots: &mut [usize; 4],
        is_filter: bool,
    ) {
        let sp_xs = self.theme.sp_xs;

        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            // Header
            ui.label(
                RichText::new(title)
                    .size(12.0)
                    .strong()
                    .color(self.theme.c(&self.theme.text_primary)),
            );
            ui.add_space(sp_xs);

            // Snapshot current ADSR from the engine into a local buffer.
            // Sliders mutate the local; on .changed() we write back the
            // specific stage to the engine. No UI-side mirror.
            let mut adsr: [f32; 4] = if is_filter {
                [
                    self.engine.fenv_attack(),
                    self.engine.fenv_decay(),
                    self.engine.fenv_sustain(),
                    self.engine.fenv_release(),
                ]
            } else {
                [
                    self.engine.amp_attack(),
                    self.engine.amp_decay(),
                    self.engine.amp_sustain(),
                    self.engine.amp_release(),
                ]
            };
            let labels = ["A", "D", "S", "R"];
            let tips = [
                "Attack — time to reach full level after a note is pressed.",
                "Decay — time to fall from peak to sustain level.",
                "Sustain — level held while key is held (0 = silent, 1 = full).",
                "Release — time to fade out after key is released.",
            ];
            let ranges: [std::ops::RangeInclusive<f32>; 4] =
                [0.001..=10.0, 0.001..=5.0, 0.0..=1.0, 0.001..=15.0];

            // Vertical sliders with labels above
            ui.horizontal(|ui| {
                for i in 0..4 {
                    ui.vertical(|ui| {
                        // Label above slider
                        ui.label(
                            RichText::new(labels[i])
                                .size(10.0)
                                .color(self.theme.c(&self.theme.text_secondary)),
                        );
                        let log = i != 2;
                        let changed = ui
                            .add_sized(
                                [28.0, 80.0],
                                egui::Slider::new(&mut adsr[i], ranges[i].clone())
                                    .vertical()
                                    .logarithmic(log),
                            )
                            .on_hover_text(tips[i])
                            .changed();
                        if changed {
                            let v = adsr[i];
                            if is_filter {
                                match i {
                                    0 => self.engine.set_fenv_attack(v),
                                    1 => self.engine.set_fenv_decay(v),
                                    2 => self.engine.set_fenv_sustain(v),
                                    _ => self.engine.set_fenv_release(v),
                                }
                            } else {
                                match i {
                                    0 => self.engine.set_amp_attack(v),
                                    1 => self.engine.set_amp_decay(v),
                                    2 => self.engine.set_amp_sustain(v),
                                    _ => self.engine.set_amp_release(v),
                                }
                            }
                        }
                    });
                }
            });

            ui.add_space(sp_xs);

            let cursors: Vec<f32> = if is_filter {
                self.engine.fenv_cursors()
            } else {
                self.engine.amp_cursors()
            };
            draw_adsr_visualizer(ui, &adsr, &cursors, &self.theme);
        });
    }
}

/// Draws the LP24 filter visualiser: grid, response curve (read-only), and a
/// free-floating control node whose position encodes (cutoff, resonance).
///
/// The node lives in a 2D parameter space independent of the curve:
///   X → log-mapped cutoff (80 Hz left … 18 kHz right)
///   Y → linear resonance  (0.95 top … 0.0 bottom)
/// The curve is a pure visual projection of those values using a biquad LP4
/// transfer function; it is never the interaction surface.
fn draw_lp_response_curve(
    painter: &egui::Painter,
    rect: egui::Rect,
    cutoff: f32,
    q_engine: f32, // 0.0 … 0.95 (engine range)
    active: bool,
    theme: &super::theme::SynthTheme,
) {
    const F_MIN: f32 = 80.0;
    const F_MAX: f32 = 18_000.0;
    // Range chosen so the full curve is always visible:
    //  - at max Q (≈10), peak height is 20·log10(Q) ≈ 20 dB
    //  - rolloff reaches −60 dB within the plotted frequency range
    const DB_MIN: f32 = -60.0;
    const DB_MAX: f32 = 36.0;

    // Map engine resonance (0..0.95) to display Q (0.5..10) for the curve math.
    let q_display = 0.5 + (q_engine / 0.95) * 9.5;

    let accent = theme.c(&theme.accent);
    let border_col = if active {
        accent
    } else {
        Color32::from_gray(55)
    };

    // ── Coordinate helpers ────────────────────────────────────────────────
    let log_range = (F_MAX / F_MIN).ln();
    let freq_to_t = |f: f32| ((f / F_MIN).ln() / log_range).clamp(0.0, 1.0);
    let sx = |t: f32| rect.left() + t * rect.width();
    let sy = |db: f32| {
        let t = ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0);
        rect.bottom() - t * rect.height()
    };

    // ── Background ────────────────────────────────────────────────────────
    let bg = Color32::from_rgba_premultiplied(accent.r() / 6, accent.g() / 6, accent.b() / 6, 140);
    painter.rect_filled(rect, egui::CornerRadius::same(5), bg);

    // ── Grid — log-spaced vertical frequency lines ────────────────────────
    let grid_col = Color32::from_gray(42);
    let label_col = Color32::from_gray(72);
    let small = egui::FontId::proportional(8.0);
    for (f, label) in [
        (100.0_f32, "100"),
        (200.0, "200"),
        (500.0, "500"),
        (1_000.0, "1k"),
        (2_000.0, "2k"),
        (5_000.0, "5k"),
        (10_000.0, "10k"),
    ] {
        let x = sx(freq_to_t(f));
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, grid_col),
        );
        painter.text(
            egui::pos2(x, rect.bottom() - 2.0),
            egui::Align2::CENTER_BOTTOM,
            label,
            small.clone(),
            label_col,
        );
    }

    // ── Grid — horizontal dB lines ────────────────────────────────────────
    for db in [-48.0_f32, -24.0, -12.0, 0.0, 18.0] {
        let y = sy(db);
        let w = if db == 0.0 { 1.0 } else { 0.5 };
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(w, grid_col),
        );
    }

    // ── Response curve — LP4 = two cascaded identical LP2 sections ────────
    // |H_LP4(w)| = 1 / ((1 − w²)² + (w/Q)²)   where w = f / fc
    const N: usize = 200;
    let db_of = |f: f32| -> f32 {
        let w = f / cutoff;
        let denom = (1.0 - w * w).powi(2) + (w / q_display).powi(2);
        (20.0 * (1.0 / denom).log10()).clamp(DB_MIN, DB_MAX)
    };

    let mut pts: Vec<egui::Pos2> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let t = i as f32 / N as f32;
        let f = F_MIN * (F_MAX / F_MIN).powf(t);
        pts.push(egui::pos2(sx(t), sy(db_of(f))));
    }

    // Filled area — one trapezoid per curve segment. Always convex, so
    // egui's fan triangulation handles each strip correctly even when the
    // overall curve shape (with resonance peak) is non-convex.
    let fill_col =
        Color32::from_rgba_premultiplied(accent.r() / 3, accent.g() / 3, accent.b() / 3, 110);
    let baseline = rect.bottom();
    for w in pts.windows(2) {
        let quad = vec![
            w[0],
            w[1],
            egui::pos2(w[1].x, baseline),
            egui::pos2(w[0].x, baseline),
        ];
        painter.add(egui::Shape::convex_polygon(
            quad,
            fill_col,
            egui::Stroke::NONE,
        ));
    }

    // Curve line
    let line_col = Color32::from_rgba_premultiplied(accent.r(), accent.g(), accent.b(), 210);
    for w in pts.windows(2) {
        painter.line_segment([w[0], w[1]], egui::Stroke::new(1.5, line_col));
    }

    // ── Control node — free in (cutoff × Q) space, not on the curve ───────
    // X: log-mapped within F_MIN..F_MAX
    // Y: linear in q_engine (0=bottom, 0.95=top)
    let node_x = sx(freq_to_t(cutoff));
    let node_y = rect.bottom() - (q_engine / 0.95) * rect.height();

    // Subtle crosshair
    let cross = Color32::from_rgba_premultiplied(accent.r(), accent.g(), accent.b(), 45);
    painter.line_segment(
        [
            egui::pos2(node_x, rect.top()),
            egui::pos2(node_x, rect.bottom()),
        ],
        egui::Stroke::new(1.0, cross),
    );
    painter.line_segment(
        [
            egui::pos2(rect.left(), node_y),
            egui::pos2(rect.right(), node_y),
        ],
        egui::Stroke::new(1.0, cross),
    );

    // Node dot
    painter.circle_filled(egui::pos2(node_x, node_y), 5.0, accent);
    painter.circle_stroke(
        egui::pos2(node_x, node_y),
        5.0,
        egui::Stroke::new(1.5, Color32::WHITE),
    );

    // Border (drawn last so it's on top of the curve)
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(5),
        egui::Stroke::new(1.0, border_col),
        egui::StrokeKind::Middle,
    );
}

pub fn draw_adsr_visualizer(
    ui: &mut egui::Ui,
    adsr: &[f32; 4],
    cursors: &[f32],
    theme: &super::theme::SynthTheme,
) {
    let height = 48.0;
    let (resp, painter) = ui.allocate_painter(
        egui::Vec2::new(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let rect = resp.rect;

    painter.rect_filled(rect, egui::CornerRadius::same(3), theme.c(&theme.bg_adsr));

    let a = adsr[0];
    let d = adsr[1];
    let s = adsr[2];
    let r = adsr[3];

    let total = a + d + r;
    let s_vis = total * 0.35;
    let span = a + d + s_vis + r;

    let w = rect.width();
    let h = rect.height();
    let pad_y = 4.0;
    let usable_h = h - pad_y * 2.0;

    let tx = |t: f32| rect.left() + (t / span) * w;
    let ly = |level: f32| rect.bottom() - pad_y - level * usable_h;

    let p0 = Pos2::new(rect.left(), ly(0.0));
    let p1 = Pos2::new(tx(a), ly(1.0));
    let p2 = Pos2::new(tx(a + d), ly(s));
    let p3 = Pos2::new(tx(a + d + s_vis), ly(s));
    let p4 = Pos2::new(rect.right(), ly(0.0));

    let fill_pts = vec![
        p0,
        p1,
        p2,
        p3,
        p4,
        Pos2::new(rect.right(), rect.bottom() - pad_y),
        Pos2::new(rect.left(), rect.bottom() - pad_y),
    ];
    painter.add(egui::Shape::convex_polygon(
        fill_pts,
        theme.ca(&theme.adsr_fill),
        Stroke::NONE,
    ));

    let pts = vec![p0, p1, p2, p3, p4];
    let stroke = Stroke::new(1.5, theme.c(&theme.adsr_outline));
    for w in pts.windows(2) {
        painter.line_segment([w[0], w[1]], stroke);
    }

    let label_color = theme.ca(&theme.adsr_label);
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

    for &cursor in cursors {
        if cursor < 0.5 {
            continue;
        }

        let phase = cursor as u8;
        let progress = cursor.fract();

        let pos = match phase {
            1 => Pos2::new(tx(a * progress), ly(progress)),
            2 => Pos2::new(tx(a + d * progress), ly(1.0 - (1.0 - s) * progress)),
            3 => Pos2::new(tx(a + d + s_vis * 0.5), ly(s)),
            4 => Pos2::new(tx(a + d + s_vis + r * progress), ly(s * (1.0 - progress))),
            _ => continue,
        };

        let cursor_c = theme.c(&theme.adsr_cursor);
        painter.circle_filled(
            pos,
            5.0,
            Color32::from_rgba_premultiplied(cursor_c.r(), cursor_c.g(), cursor_c.b(), 40),
        );
        painter.circle_filled(pos, 2.5, cursor_c);
    }
}
