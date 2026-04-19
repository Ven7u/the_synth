use crate::SynthApp;
use eframe::egui;
use egui::{Color32, Pos2, Stroke};
use std::sync::atomic::Ordering;

/// (label, beats_per_cycle) — beats relative to a quarter note.
/// rate_hz = bpm / 60.0 / beats_per_cycle
pub const LFO_SYNC_DIVISIONS: &[(&str, f32)] = &[
    ("4",    16.0),  // 4 bars
    ("2",     8.0),  // 2 bars
    ("1",     4.0),  // 1 bar
    ("1/2",   2.0),
    ("1/4",   1.0),
    ("1/8",   0.5),
    ("1/16",  0.25),
    ("1/4T",  2.0 / 3.0),  // quarter triplet
    ("1/8T",  1.0 / 3.0),  // eighth triplet
];

pub fn lfo_synced_rate(bpm: f32, division: usize) -> f32 {
    let beats = LFO_SYNC_DIVISIONS[division.min(LFO_SYNC_DIVISIONS.len() - 1)].1;
    (bpm / 60.0 / beats).clamp(0.01, 20.0)
}

impl SynthApp {
    pub fn ui_lfo_panel(&mut self, ui: &mut egui::Ui) {
        // Header toggle
        ui.horizontal(|ui| {
            let on = self.lfo_enabled;
            let label = egui::RichText::new("LFO").strong()
                .color(if on { self.theme.c(&self.theme.accent) } else { Color32::GRAY });
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
                // Rate knob — hidden when synced, shown when free
                let sync_on = self.lfo_sync_active();
                if !sync_on {
                    if super::widgets::knob(ui, &mut self.lfo_rate, 0.1..=20.0, "Rate", &self.theme, false)
                        .on_hover_text("LFO speed in Hz. 0.1 = very slow. 5 = fast vibrato. 20 = audio range.")
                        .changed()
                    {
                        self.state.lfo_rate.set(self.lfo_rate);
                    }
                }
                if super::widgets::knob(ui, &mut self.lfo_depth, 0.0..=1.0, "Depth", &self.theme, false)
                    .on_hover_text("How strongly the LFO modulates its destination. 0 = off, 1 = full.")
                    .changed()
                {
                    self.state.lfo_depth.set(self.lfo_depth);
                }

                // Sync toggle button
                ui.add_enabled_ui(!self.global_sync, |ui| {
                    let sync_label = egui::RichText::new("Sync")
                        .color(if sync_on { self.theme.c(&self.theme.accent) } else { Color32::GRAY });
                    if ui.button(sync_label)
                        .on_hover_text("Lock LFO rate to a note division of the Global BPM.")
                        .clicked()
                    {
                        self.lfo_sync = !self.lfo_sync;
                        if self.lfo_sync_active() {
                            let rate = lfo_synced_rate(self.global_bpm as f32, self.lfo_division);
                            self.lfo_rate = rate;
                            self.state.lfo_rate.set(rate);
                        }
                    }
                });
            });

            // Division selector — shown only when synced
            if self.lfo_sync_active() {
                ui.horizontal_wrapped(|ui| {
                    for (i, &(label, _)) in LFO_SYNC_DIVISIONS.iter().enumerate() {
                        let active = self.lfo_division == i;
                        let btn_label = egui::RichText::new(label).small()
                            .color(if active { self.theme.c(&self.theme.accent) } else { Color32::GRAY });
                        let rate = lfo_synced_rate(self.global_bpm as f32, i);
                        if ui.button(btn_label)
                            .on_hover_text(format!("{} → {:.3} Hz @ {} BPM", label, rate, self.global_bpm))
                            .clicked()
                        {
                            self.lfo_division = i;
                            self.lfo_rate = rate;
                            self.state.lfo_rate.set(rate);
                        }
                    }
                });
            }
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

    pub fn ui_filter_panel(&mut self, ui: &mut egui::Ui) {
        // Header toggle
        ui.horizontal(|ui| {
            let on = self.filter_enabled;
            let label = egui::RichText::new("FILTER").strong()
                .color(if on { self.theme.c(&self.theme.accent) } else { Color32::GRAY });
            if ui.button(label)
                .on_hover_text("Moog-style 4-pole lowpass filter. Removes high frequencies, shaping the brightness and timbre of the sound. The classic 'sweep' sound of a synthesizer.")
                .clicked()
            {
                self.filter_enabled = !on;
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
                if super::widgets::knob(ui, &mut self.filter_cutoff, 80.0..=18000.0, "Cut", &self.theme, true)
                    .on_hover_text("Cutoff frequency. 80 Hz = dark, 18000 Hz = fully open.")
                    .changed()
                {
                    self.state.cutoff.set(self.filter_cutoff);
                }
                if super::widgets::knob(ui, &mut self.filter_q, 0.0..=0.95, "Res", &self.theme, false)
                    .on_hover_text("Resonance. 0 = none. 0.9+ = self-oscillation.")
                    .changed()
                {
                    self.state.resonance.set(self.filter_q);
                }
                if super::widgets::knob(ui, &mut self.filter_env_amount, 0.0..=1.0, "Env", &self.theme, false)
                    .on_hover_text("Filter envelope amount. 0 = no effect, 1 = full sweep.")
                    .changed()
                {
                    self.state.filter_env_amount.set(self.filter_env_amount);
                }
            });

            // XY pad — X: cutoff, Y: resonance (drag up = more resonance)
            let pad_size = egui::Vec2::new(ui.available_width(), 120.0);
            let (rect, response) = ui.allocate_exact_size(pad_size, egui::Sense::click_and_drag());

            if response.dragged() {
                let delta = response.drag_delta();
                let dx = delta.x / rect.width();
                let dy = -delta.y / rect.height(); // invert: up = increase
                let log_min = 80.0_f32.ln();
                let log_max = 18000.0_f32.ln();
                let log_cur = self.filter_cutoff.ln();
                self.filter_cutoff = ((log_cur + dx * (log_max - log_min)).clamp(log_min, log_max)).exp();
                self.filter_q = (self.filter_q + dy * 0.95).clamp(0.0, 0.95);
                self.state.cutoff.set(self.filter_cutoff);
                self.state.resonance.set(self.filter_q);
            }
            if response.double_clicked() {
                self.filter_cutoff = 18000.0;
                self.filter_q = 0.0;
                self.state.cutoff.set(self.filter_cutoff);
                self.state.resonance.set(self.filter_q);
            }

            if ui.is_rect_visible(rect) {
                let painter = ui.painter_at(rect);
                let accent = self.theme.c(&self.theme.accent);
                let bg = Color32::from_rgba_premultiplied(
                    accent.r() / 5, accent.g() / 5, accent.b() / 5, 120,
                );
                painter.rect_filled(rect, egui::Rounding::same(6.0), bg);
                painter.rect_stroke(rect, egui::Rounding::same(6.0),
                    egui::Stroke::new(1.0, if response.hovered() || response.dragged() {
                        accent
                    } else {
                        Color32::from_gray(60)
                    }));

                let tx = (self.filter_cutoff.ln() - 80.0_f32.ln()) / (18000.0_f32.ln() - 80.0_f32.ln());
                let ty = 1.0 - self.filter_q / 0.95; // invert for screen coords
                let px = rect.left() + tx * rect.width();
                let py = rect.top() + ty * rect.height();

                // Crosshair lines
                let line_color = Color32::from_rgba_premultiplied(
                    accent.r(), accent.g(), accent.b(), 40,
                );
                painter.line_segment(
                    [egui::pos2(px, rect.top()), egui::pos2(px, rect.bottom())],
                    egui::Stroke::new(1.0, line_color),
                );
                painter.line_segment(
                    [egui::pos2(rect.left(), py), egui::pos2(rect.right(), py)],
                    egui::Stroke::new(1.0, line_color),
                );

                // Handle dot
                painter.circle_filled(egui::pos2(px, py), 6.0, accent);
                painter.circle_stroke(egui::pos2(px, py), 6.0,
                    egui::Stroke::new(1.0, Color32::WHITE));

                // Axis labels
                let label_color = Color32::from_gray(110);
                let font = egui::FontId::proportional(9.0);
                painter.text(egui::pos2(rect.left() + 4.0, rect.bottom() - 3.0),
                    egui::Align2::LEFT_BOTTOM, "cut →", font.clone(), label_color);
                painter.text(egui::pos2(rect.left() + 4.0, rect.top() + 3.0),
                    egui::Align2::LEFT_TOP, "res ↑", font, label_color);
            }
        });
    }

    pub fn ui_adsr_panel(
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

        let cursors: Vec<f32> = if is_filter {
            self.state.fenv_cursors.iter().map(|s| s.value()).collect()
        } else {
            self.state.amp_cursors.iter().map(|s| s.value()).collect()
        };
        draw_adsr_visualizer(ui, adsr, &cursors, &self.theme);
    }
}

pub fn draw_adsr_visualizer(ui: &mut egui::Ui, adsr: &[f32; 4], cursors: &[f32], theme: &super::theme::SynthTheme) {
    let height = 48.0;
    let (resp, painter) =
        ui.allocate_painter(egui::Vec2::new(ui.available_width(), height), egui::Sense::hover());
    let rect = resp.rect;

    painter.rect_filled(rect, egui::Rounding::same(3.0), theme.c(&theme.bg_adsr));

    let a = adsr[0];
    let d = adsr[1];
    let s = adsr[2];
    let r = adsr[3];

    let total = a + d + r;
    let s_vis = total * 0.35;
    let span  = a + d + s_vis + r;

    let w = rect.width();
    let h = rect.height();
    let pad_y = 4.0;
    let usable_h = h - pad_y * 2.0;

    let tx = |t: f32| rect.left() + (t / span) * w;
    let ly = |level: f32| rect.bottom() - pad_y - level * usable_h;

    let p0 = Pos2::new(rect.left(),    ly(0.0));
    let p1 = Pos2::new(tx(a),          ly(1.0));
    let p2 = Pos2::new(tx(a + d),      ly(s));
    let p3 = Pos2::new(tx(a + d + s_vis), ly(s));
    let p4 = Pos2::new(rect.right(),   ly(0.0));

    let fill_pts = vec![
        p0, p1, p2, p3, p4,
        Pos2::new(rect.right(), rect.bottom() - pad_y),
        Pos2::new(rect.left(),  rect.bottom() - pad_y),
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
        if cursor < 0.5 { continue; }

        let phase    = cursor as u8;
        let progress = cursor.fract();

        let pos = match phase {
            1 => Pos2::new(tx(a * progress),                              ly(progress)),
            2 => Pos2::new(tx(a + d * progress),                          ly(1.0 - (1.0 - s) * progress)),
            3 => Pos2::new(tx(a + d + s_vis * 0.5),                       ly(s)),
            4 => Pos2::new(tx(a + d + s_vis + r * progress),              ly(s * (1.0 - progress))),
            _ => continue,
        };

        let cursor_c = theme.c(&theme.adsr_cursor);
        painter.circle_filled(pos, 5.0, Color32::from_rgba_premultiplied(cursor_c.r(), cursor_c.g(), cursor_c.b(), 40));
        painter.circle_filled(pos, 2.5, cursor_c);
    }
}
