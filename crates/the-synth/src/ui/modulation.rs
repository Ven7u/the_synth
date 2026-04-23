use crate::ui::frame::SynthFrame;
use crate::SynthApp;
use eframe::egui;
use egui::{Color32, Pos2, RichText, Stroke};

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
            });
        });
    }

    pub fn ui_filter_panel(&mut self, ui: &mut egui::Ui) {
        let sp_xs = self.theme.sp_xs;

        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            // Header
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
            });

            ui.add_space(sp_xs);

            ui.add_enabled_ui(self.filter_enabled, |ui| {
                // Knobs: Cut · Res · Env
                ui.horizontal(|ui| {
                    if super::widgets::knob(
                        ui,
                        &mut self.filter_cutoff,
                        80.0..=18000.0,
                        "CUT",
                        &self.theme,
                        true,
                    )
                    .on_hover_text("Cutoff frequency. 80 Hz = dark, 18 kHz = fully open.")
                    .changed()
                    {
                        self.engine.set_filter_cutoff(self.filter_cutoff);
                    }
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
                    let mut env_amt = self.engine.filter_env_amount();
                    if super::widgets::knob(ui, &mut env_amt, 0.0..=1.0, "ENV", &self.theme, false)
                        .on_hover_text(
                            "Filter envelope amount — how much the filter env sweeps the cutoff.",
                        )
                        .changed()
                    {
                        self.engine.set_filter_env_amount(env_amt);
                    }
                });

                ui.add_space(sp_xs);

                // XY pad — constrained height, X: cutoff, Y: resonance
                let pad_h = ui.available_width().min(110.0);
                let pad_size = egui::Vec2::new(ui.available_width(), pad_h);
                let (rect, response) =
                    ui.allocate_exact_size(pad_size, egui::Sense::click_and_drag());

                if response.dragged() {
                    let delta = response.drag_delta();
                    let dx = delta.x / rect.width();
                    let dy = -delta.y / rect.height();
                    let log_min = 80.0_f32.ln();
                    let log_max = 18000.0_f32.ln();
                    self.filter_cutoff = ((self.filter_cutoff.ln() + dx * (log_max - log_min))
                        .clamp(log_min, log_max))
                    .exp();
                    self.filter_q = (self.filter_q + dy * 0.95).clamp(0.0, 0.95);
                    self.engine.set_filter_cutoff(self.filter_cutoff);
                    self.engine.set_filter_resonance(self.filter_q);
                }
                if response.double_clicked() {
                    self.filter_cutoff = 18000.0;
                    self.filter_q = 0.0;
                    self.engine.set_filter_cutoff(self.filter_cutoff);
                    self.engine.set_filter_resonance(self.filter_q);
                }

                if ui.is_rect_visible(rect) {
                    let painter = ui.painter_at(rect);
                    let accent = self.theme.c(&self.theme.accent);
                    let bg = Color32::from_rgba_premultiplied(
                        accent.r() / 5,
                        accent.g() / 5,
                        accent.b() / 5,
                        120,
                    );
                    painter.rect_filled(rect, egui::Rounding::same(6.0), bg);
                    painter.rect_stroke(
                        rect,
                        egui::Rounding::same(6.0),
                        egui::Stroke::new(
                            1.0,
                            if response.hovered() || response.dragged() {
                                accent
                            } else {
                                Color32::from_gray(60)
                            },
                        ),
                    );

                    let tx = (self.filter_cutoff.ln() - 80.0_f32.ln())
                        / (18000.0_f32.ln() - 80.0_f32.ln());
                    let ty = 1.0 - self.filter_q / 0.95;
                    let px = rect.left() + tx * rect.width();
                    let py = rect.top() + ty * rect.height();

                    let line_col =
                        Color32::from_rgba_premultiplied(accent.r(), accent.g(), accent.b(), 40);
                    painter.line_segment(
                        [egui::pos2(px, rect.top()), egui::pos2(px, rect.bottom())],
                        egui::Stroke::new(1.0, line_col),
                    );
                    painter.line_segment(
                        [egui::pos2(rect.left(), py), egui::pos2(rect.right(), py)],
                        egui::Stroke::new(1.0, line_col),
                    );
                    painter.circle_filled(egui::pos2(px, py), 6.0, accent);
                    painter.circle_stroke(
                        egui::pos2(px, py),
                        6.0,
                        egui::Stroke::new(1.0, Color32::WHITE),
                    );

                    let font = egui::FontId::proportional(9.0);
                    let label_col = Color32::from_gray(110);
                    painter.text(
                        egui::pos2(rect.left() + 4.0, rect.bottom() - 3.0),
                        egui::Align2::LEFT_BOTTOM,
                        "cut →",
                        font.clone(),
                        label_col,
                    );
                    painter.text(
                        egui::pos2(rect.left() + 4.0, rect.top() + 3.0),
                        egui::Align2::LEFT_TOP,
                        "res ↑",
                        font,
                        label_col,
                    );
                }
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

    painter.rect_filled(rect, egui::Rounding::same(3.0), theme.c(&theme.bg_adsr));

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
