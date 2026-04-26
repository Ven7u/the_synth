use egui::{Color32, Pos2, RichText, Stroke};

use crate::frame::SynthFrame;
use crate::param_writer::ParamWriter;
use crate::state::SynthUiState;
use crate::theme::SynthTheme;

pub const LFO_SYNC_DIVISIONS: &[(&str, f32)] = &[
    ("4", 16.0),
    ("2", 8.0),
    ("1", 4.0),
    ("1/2", 2.0),
    ("1/4", 1.0),
    ("1/8", 0.5),
    ("1/16", 0.25),
    ("1/4T", 2.0 / 3.0),
    ("1/8T", 1.0 / 3.0),
];

pub fn lfo_synced_rate(bpm: f32, division: usize) -> f32 {
    let beats = LFO_SYNC_DIVISIONS[division.min(LFO_SYNC_DIVISIONS.len() - 1)].1;
    (bpm / 60.0 / beats).clamp(0.01, 20.0)
}

// ── LFO 1 ────────────────────────────────────────────────────────────────────

pub fn ui_lfo_panel(
    ui: &mut egui::Ui,
    s: &mut SynthUiState,
    pw: &mut impl ParamWriter,
    theme: &SynthTheme,
) {
    let sp_xs = theme.sp_xs;

    SynthFrame::section(theme).show(ui, |ui| {
        ui.set_min_width(ui.available_width());

        ui.horizontal(|ui| {
            let on = s.lfo_enabled;
            let col = if on { theme.c(&theme.accent) } else { theme.c(&theme.text_disabled) };
            if ui
                .add(egui::SelectableLabel::new(
                    on,
                    RichText::new("LFO 1").size(12.0).strong().color(col),
                ))
                .on_hover_text("Low Frequency Oscillator — modulates pitch, filter cutoff, or amplitude")
                .clicked()
            {
                s.lfo_enabled = !on;
                pw.set_lfo_depth(if s.lfo_enabled { s.lfo_depth } else { 0.0 });
            }
        });

        ui.add_space(sp_xs);

        ui.add_enabled_ui(s.lfo_enabled, |ui| {
            ui.horizontal(|ui| {
                let sync_on = s.lfo_sync_active();
                if !sync_on {
                    if crate::widgets::knob(ui, &mut s.lfo_rate, 0.1..=20.0, "RATE", theme, false)
                        .on_hover_text("LFO speed in Hz.")
                        .changed()
                    {
                        pw.set_lfo_rate(s.lfo_rate);
                    }
                }
                if crate::widgets::knob(ui, &mut s.lfo_depth, 0.0..=1.0, "DEPTH", theme, false)
                    .on_hover_text("Modulation depth. 0 = off, 1 = full.")
                    .changed()
                {
                    pw.set_lfo_depth(s.lfo_depth);
                }

                ui.add_enabled_ui(!s.global_sync, |ui| {
                    let sync_on = s.lfo_sync_active();
                    let sync_col = if sync_on { theme.c(&theme.accent) } else { theme.c(&theme.text_disabled) };
                    if ui
                        .add(egui::SelectableLabel::new(
                            sync_on,
                            RichText::new("SYNC").size(10.0).color(sync_col),
                        ))
                        .on_hover_text("Lock LFO rate to a note division of the Global BPM")
                        .clicked()
                    {
                        s.lfo_sync = !s.lfo_sync;
                        if s.lfo_sync_active() {
                            let rate = lfo_synced_rate(s.global_bpm as f32, s.lfo_division);
                            s.lfo_rate = rate;
                            pw.set_lfo_rate(rate);
                        }
                    }
                });
            });

            if s.lfo_sync_active() {
                ui.add_space(sp_xs);
                ui.horizontal_wrapped(|ui| {
                    for (i, &(label, _)) in LFO_SYNC_DIVISIONS.iter().enumerate() {
                        let active = s.lfo_division == i;
                        let rate = lfo_synced_rate(s.global_bpm as f32, i);
                        if ui
                            .selectable_label(
                                active,
                                RichText::new(label).small().color(if active {
                                    theme.c(&theme.accent)
                                } else {
                                    theme.c(&theme.text_secondary)
                                }),
                            )
                            .on_hover_text(format!("{} → {:.3} Hz @ {} BPM", label, rate, s.global_bpm))
                            .clicked()
                        {
                            s.lfo_division = i;
                            s.lfo_rate = rate;
                            pw.set_lfo_rate(rate);
                        }
                    }
                });
            }

            ui.add_space(sp_xs);

            ui.horizontal(|ui| {
                ui.label(RichText::new("SHAPE").size(10.0).color(theme.c(&theme.text_secondary)));
                let shape_tips = ["Sine — smooth.", "Triangle — linear.", "Saw — ramps up."];
                for (sh, label) in [(0usize, "Sin"), (1, "Tri"), (2, "Saw")] {
                    if ui
                        .selectable_label(s.lfo_shape == sh, label)
                        .on_hover_text(shape_tips[sh])
                        .clicked()
                    {
                        s.lfo_shape = sh;
                        pw.set_lfo_shape(sh as u8);
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label(RichText::new("→").size(10.0).color(theme.c(&theme.text_secondary)));
                let dest_tips = ["Pitch — vibrato.", "Filter — wobble.", "Amp — tremolo."];
                for (d, label) in [(0usize, "Pitch"), (1, "Filter"), (2, "Amp")] {
                    if ui
                        .selectable_label(s.lfo_dest == d, label)
                        .on_hover_text(dest_tips[d])
                        .clicked()
                    {
                        s.lfo_dest = d;
                        pw.set_lfo_dest(d as u8);
                    }
                }
            });
        });
    });
}

// ── LFO 2 ────────────────────────────────────────────────────────────────────

pub fn ui_lfo2_panel(
    ui: &mut egui::Ui,
    s: &mut SynthUiState,
    pw: &mut impl ParamWriter,
    theme: &SynthTheme,
) {
    let sp_xs = theme.sp_xs;

    SynthFrame::section(theme).show(ui, |ui| {
        ui.set_min_width(ui.available_width());

        ui.horizontal(|ui| {
            let on = s.lfo2_enabled;
            let col = if on { theme.c(&theme.accent) } else { theme.c(&theme.text_disabled) };
            if ui
                .add(egui::SelectableLabel::new(
                    on,
                    RichText::new("LFO 2").size(12.0).strong().color(col),
                ))
                .on_hover_text("Second LFO — runs independently of LFO 1")
                .clicked()
            {
                s.lfo2_enabled = !on;
                pw.set_lfo2_depth(if s.lfo2_enabled { s.lfo2_depth } else { 0.0 });
            }
        });

        ui.add_space(sp_xs);

        ui.add_enabled_ui(s.lfo2_enabled, |ui| {
            ui.horizontal(|ui| {
                if crate::widgets::knob(ui, &mut s.lfo2_rate, 0.01..=20.0, "RATE", theme, true)
                    .on_hover_text("LFO 2 rate in Hz")
                    .changed()
                {
                    pw.set_lfo2_rate(s.lfo2_rate);
                }
                if crate::widgets::knob(ui, &mut s.lfo2_depth, 0.0..=1.0, "DEPTH", theme, false)
                    .on_hover_text("LFO 2 modulation depth")
                    .changed()
                {
                    pw.set_lfo2_depth(s.lfo2_depth);
                }
            });

            ui.add_space(sp_xs);

            ui.horizontal(|ui| {
                ui.label(RichText::new("SHAPE").size(10.0).color(theme.c(&theme.text_secondary)));
                for (sh, label) in [(0usize, "Sin"), (1, "Tri"), (2, "Saw")] {
                    if ui.selectable_label(s.lfo2_shape == sh, label).clicked() {
                        s.lfo2_shape = sh;
                        pw.set_lfo2_shape(sh as u8);
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label(RichText::new("→").size(10.0).color(theme.c(&theme.text_secondary)));
                for (d, label) in [(0usize, "Pitch"), (1, "Filter"), (2, "Amp")] {
                    if ui.selectable_label(s.lfo2_dest == d, label).clicked() {
                        s.lfo2_dest = d;
                        pw.set_lfo2_dest(d as u8);
                    }
                }
            });
        });
    });
}

// ── Filter ───────────────────────────────────────────────────────────────────

pub fn ui_filter_panel(
    ui: &mut egui::Ui,
    s: &mut SynthUiState,
    pw: &mut impl ParamWriter,
    theme: &SynthTheme,
) {
    let sp_xs = theme.sp_xs;

    SynthFrame::section(theme).show(ui, |ui| {
        ui.set_min_width(ui.available_width());

        ui.horizontal(|ui| {
            let on = s.filter_enabled;
            let col = if on { theme.c(&theme.accent) } else { theme.c(&theme.text_disabled) };
            if ui
                .add(egui::SelectableLabel::new(
                    on,
                    RichText::new("FILTER").size(12.0).strong().color(col),
                ))
                .on_hover_text("Moog-style 4-pole lowpass filter")
                .clicked()
            {
                s.filter_enabled = !on;
                if s.filter_enabled {
                    pw.set_filter_cutoff(s.filter_cutoff);
                    pw.set_filter_resonance(s.filter_q);
                } else {
                    pw.set_filter_cutoff(18000.0);
                    pw.set_filter_resonance(0.0);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new("LP24")
                        .size(10.0)
                        .color(theme.c(&theme.text_secondary)),
                );
            });
        });

        ui.add_space(sp_xs);

        ui.horizontal(|ui| {
            let accent = theme.c(&theme.accent);
            let disabled = theme.c(&theme.text_disabled);
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

        ui.add_enabled_ui(s.filter_enabled, |ui| {
            let curve_h = ui.available_width().min(100.0);
            let curve_size = egui::Vec2::new(ui.available_width(), curve_h);
            let (rect, response) =
                ui.allocate_exact_size(curve_size, egui::Sense::click_and_drag());

            if response.dragged() {
                let fine = ui.input(|i| i.modifiers.shift);
                if fine {
                    let delta = response.drag_delta();
                    let log_min = 80.0_f32.ln();
                    let log_max = 18000.0_f32.ln();
                    s.filter_cutoff = ((s.filter_cutoff.ln()
                        + delta.x / rect.width() * (log_max - log_min) * 0.15)
                        .clamp(log_min, log_max))
                    .exp();
                    s.filter_q =
                        (s.filter_q - delta.y / rect.height() * 0.95 * 0.15).clamp(0.0, 0.95);
                } else if let Some(pos) = response.interact_pointer_pos() {
                    let x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                    let y = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                    let log_min = 80.0_f32.ln();
                    let log_max = 18000.0_f32.ln();
                    s.filter_cutoff = (log_min + x * (log_max - log_min)).exp();
                    s.filter_q = (1.0 - y) * 0.95;
                }
                pw.set_filter_cutoff(s.filter_cutoff);
                pw.set_filter_resonance(s.filter_q);
            }
            if response.double_clicked() {
                s.filter_cutoff = 3000.0;
                s.filter_q = 0.0;
                pw.set_filter_cutoff(s.filter_cutoff);
                pw.set_filter_resonance(s.filter_q);
            }

            if ui.is_rect_visible(rect) {
                draw_lp_response_curve(
                    ui.painter(),
                    rect,
                    s.filter_cutoff,
                    s.filter_q,
                    response.hovered() || response.dragged(),
                    theme,
                );
            }

            ui.add_space(sp_xs);

            ui.horizontal(|ui| {
                if crate::widgets::knob(
                    ui,
                    &mut s.filter_cutoff,
                    80.0..=18000.0,
                    "CUTOFF",
                    theme,
                    true,
                )
                .on_hover_text("Cutoff frequency. 80 Hz = dark, 18 kHz = fully open.")
                .changed()
                {
                    pw.set_filter_cutoff(s.filter_cutoff);
                }

                if crate::widgets::knob(ui, &mut s.filter_q, 0.0..=0.95, "RES", theme, false)
                    .on_hover_text("Resonance — 0 = flat, 0.9+ = self-oscillation.")
                    .changed()
                {
                    pw.set_filter_resonance(s.filter_q);
                }

                if crate::widgets::knob(
                    ui,
                    &mut s.filter_drive,
                    1.0..=10.0,
                    "DRIVE",
                    theme,
                    false,
                )
                .on_hover_text("Input drive — saturates the signal before the filter.")
                .changed()
                {
                    pw.set_filter_drive(s.filter_drive);
                }

                if crate::widgets::knob(
                    ui,
                    &mut s.filter_key_track,
                    0.0..=1.0,
                    "KEY",
                    theme,
                    false,
                )
                .on_hover_text("Keyboard tracking — cutoff follows pitch.")
                .changed()
                {
                    pw.set_filter_key_track(s.filter_key_track);
                }
            });
        });
    });
}

// ── ADSR ─────────────────────────────────────────────────────────────────────

/// `cursors` — per-voice envelope phase+progress values from the engine.
/// Pass an empty slice if cursor visualization is not available.
pub fn ui_adsr_panel(
    ui: &mut egui::Ui,
    s: &mut SynthUiState,
    pw: &mut impl ParamWriter,
    theme: &SynthTheme,
    title: &str,
    is_filter: bool,
    cursors: &[f32],
) {
    let sp_xs = theme.sp_xs;

    SynthFrame::section(theme).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.label(
            RichText::new(title)
                .size(12.0)
                .strong()
                .color(theme.c(&theme.text_primary)),
        );
        ui.add_space(sp_xs);

        let mut adsr = if is_filter {
            [s.fenv_attack, s.fenv_decay, s.fenv_sustain, s.fenv_release]
        } else {
            [s.amp_attack, s.amp_decay, s.amp_sustain, s.amp_release]
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

        ui.horizontal(|ui| {
            for i in 0..4 {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(labels[i])
                            .size(10.0)
                            .color(theme.c(&theme.text_secondary)),
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
                                0 => { s.fenv_attack = v; pw.set_fenv_attack(v); }
                                1 => { s.fenv_decay = v; pw.set_fenv_decay(v); }
                                2 => { s.fenv_sustain = v; pw.set_fenv_sustain(v); }
                                _ => { s.fenv_release = v; pw.set_fenv_release(v); }
                            }
                        } else {
                            match i {
                                0 => { s.amp_attack = v; pw.set_amp_attack(v); }
                                1 => { s.amp_decay = v; pw.set_amp_decay(v); }
                                2 => { s.amp_sustain = v; pw.set_amp_sustain(v); }
                                _ => { s.amp_release = v; pw.set_amp_release(v); }
                            }
                        }
                    }
                });
            }
        });

        ui.add_space(sp_xs);
        draw_adsr_visualizer(ui, &adsr, cursors, theme);
    });
}

// ── Visualizers ───────────────────────────────────────────────────────────────

pub fn draw_adsr_visualizer(
    ui: &mut egui::Ui,
    adsr: &[f32; 4],
    cursors: &[f32],
    theme: &SynthTheme,
) {
    let height = 48.0;
    let (resp, painter) = ui.allocate_painter(
        egui::Vec2::new(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let rect = resp.rect;

    painter.rect_filled(rect, 3.0, theme.c(&theme.bg_adsr));

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
    painter.add(egui::Shape::convex_polygon(fill_pts, theme.ca(&theme.adsr_fill), Stroke::NONE));

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

fn draw_lp_response_curve(
    painter: &egui::Painter,
    rect: egui::Rect,
    cutoff: f32,
    q_engine: f32,
    active: bool,
    theme: &SynthTheme,
) {
    const F_MIN: f32 = 80.0;
    const F_MAX: f32 = 18_000.0;
    const DB_MIN: f32 = -60.0;
    const DB_MAX: f32 = 36.0;

    let q_display = 0.5 + (q_engine / 0.95) * 9.5;

    let accent = theme.c(&theme.accent);
    let border_col = if active { accent } else { Color32::from_gray(55) };

    let log_range = (F_MAX / F_MIN).ln();
    let freq_to_t = |f: f32| ((f / F_MIN).ln() / log_range).clamp(0.0, 1.0);
    let sx = |t: f32| rect.left() + t * rect.width();
    let sy = |db: f32| {
        let t = ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0);
        rect.bottom() - t * rect.height()
    };

    let bg = Color32::from_rgba_premultiplied(accent.r() / 6, accent.g() / 6, accent.b() / 6, 140);
    painter.rect_filled(rect, 5.0, bg);

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
            egui::Stroke::new(0.5, grid_col),
        );
        painter.text(
            egui::pos2(x + 2.0, rect.bottom() - 2.0),
            egui::Align2::LEFT_BOTTOM,
            label,
            small.clone(),
            label_col,
        );
    }

    let omega_c = std::f32::consts::TAU * cutoff;
    let db_of = |f: f32| -> f32 {
        let w = std::f32::consts::TAU * f;
        let ratio = w / omega_c;
        let r2 = ratio * ratio;
        let denom_sq = (1.0 - r2).powi(2) + (r2 / (q_display * q_display));
        20.0 * (1.0 / denom_sq.sqrt().powi(2)).log10()
    };

    const N: usize = 120;
    let mut pts: Vec<egui::Pos2> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let t = i as f32 / N as f32;
        let f = F_MIN * (F_MAX / F_MIN).powf(t);
        pts.push(egui::pos2(sx(t), sy(db_of(f))));
    }

    let fill_col = Color32::from_rgba_premultiplied(accent.r() / 3, accent.g() / 3, accent.b() / 3, 110);
    let baseline = rect.bottom();
    for w in pts.windows(2) {
        let quad = vec![
            w[0],
            w[1],
            egui::pos2(w[1].x, baseline),
            egui::pos2(w[0].x, baseline),
        ];
        painter.add(egui::Shape::convex_polygon(quad, fill_col, egui::Stroke::NONE));
    }

    let line_col = Color32::from_rgba_premultiplied(accent.r(), accent.g(), accent.b(), 210);
    for w in pts.windows(2) {
        painter.line_segment([w[0], w[1]], egui::Stroke::new(1.5, line_col));
    }

    let node_x = sx(freq_to_t(cutoff));
    let node_y = rect.bottom() - (q_engine / 0.95) * rect.height();

    let cross = Color32::from_rgba_premultiplied(accent.r(), accent.g(), accent.b(), 45);
    painter.line_segment(
        [egui::pos2(node_x, rect.top()), egui::pos2(node_x, rect.bottom())],
        egui::Stroke::new(1.0, cross),
    );
    painter.line_segment(
        [egui::pos2(rect.left(), node_y), egui::pos2(rect.right(), node_y)],
        egui::Stroke::new(1.0, cross),
    );

    painter.circle_filled(egui::pos2(node_x, node_y), 5.0, accent);
    painter.circle_stroke(
        egui::pos2(node_x, node_y),
        5.0,
        egui::Stroke::new(1.5, Color32::WHITE),
    );

    painter.rect_stroke(rect, 5.0, egui::Stroke::new(1.0, border_col), egui::StrokeKind::Outside);
}
