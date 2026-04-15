use crate::SynthApp;
use eframe::egui;
use egui::{Color32, Pos2, Stroke};
use std::sync::atomic::Ordering;

impl SynthApp {
    pub fn ui_lfo_panel(&mut self, ui: &mut egui::Ui) {
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

    pub fn ui_filter_panel(&mut self, ui: &mut egui::Ui) {
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
        draw_adsr_visualizer(ui, adsr, &cursors);
    }
}

pub fn draw_adsr_visualizer(ui: &mut egui::Ui, adsr: &[f32; 4], cursors: &[f32]) {
    let height = 48.0;
    let (resp, painter) =
        ui.allocate_painter(egui::Vec2::new(ui.available_width(), height), egui::Sense::hover());
    let rect = resp.rect;

    painter.rect_filled(rect, egui::Rounding::same(3.0), Color32::from_rgb(8, 14, 10));

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
        Color32::from_rgba_premultiplied(0, 160, 100, 30),
        Stroke::NONE,
    ));

    let pts = vec![p0, p1, p2, p3, p4];
    let stroke = Stroke::new(1.5, Color32::from_rgb(0, 200, 130));
    for w in pts.windows(2) {
        painter.line_segment([w[0], w[1]], stroke);
    }

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

        painter.circle_filled(pos, 5.0, Color32::from_rgba_premultiplied(0, 255, 160, 40));
        painter.circle_filled(pos, 2.5, Color32::from_rgb(0, 255, 160));
    }
}
