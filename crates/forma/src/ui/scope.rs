use crate::SynthApp;
use eframe::egui;
use eframe::egui_wgpu;
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};

use super::scope_wgpu::ScopeCallback;

impl SynthApp {
    pub fn ui_oscilloscope(&mut self, ui: &mut egui::Ui) {
        self.draw_scope_panel(ui);
    }

    /// Fullscreen overlay — call once per frame from update() before panels.
    pub fn ui_scope_fullscreen(&mut self, ctx: &egui::Context) {
        if !self.scope_fullscreen {
            return;
        }
        let screen = ctx.screen_rect();
        egui::Window::new("scope_fs_window")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .fixed_pos(screen.min)
            .fixed_size(screen.size())
            .frame(egui::Frame::new().fill(self.theme.c(&self.theme.bg_app)))
            .show(ctx, |ui| {
                self.draw_scope_panel(ui);
            });
    }

    fn draw_scope_panel(&mut self, ui: &mut egui::Ui) {
        let accent = self.theme.c(&self.theme.accent);
        let scope_ctrl = self.theme.c(&self.theme.scope_label);
        let text_sec = self.theme.c(&self.theme.text_secondary);

        // ── Toolbar ────────────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("SCOPE").small().color(scope_ctrl));
            ui.add_space(4.0);

            ui.label(egui::RichText::new("X").small().color(scope_ctrl))
                .on_hover_text("Horizontal zoom — stretch or compress the waveform in time.");
            ui.add(
                egui::DragValue::new(&mut self.scope_x_scale)
                    .speed(0.02)
                    .range(0.25_f32..=8.0)
                    .suffix("×"),
            )
            .on_hover_text("Horizontal zoom (0.25–8×). Drag left/right to adjust.");

            ui.add_space(4.0);
            ui.label(egui::RichText::new("Y").small().color(scope_ctrl))
                .on_hover_text("Vertical zoom — scale the waveform amplitude.");
            ui.add(
                egui::DragValue::new(&mut self.scope_y_scale)
                    .speed(0.02)
                    .range(0.25_f32..=8.0)
                    .suffix("×"),
            )
            .on_hover_text("Vertical zoom (0.25–8×). Drag left/right to adjust.");

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let fs_col = if self.scope_fullscreen {
                    accent
                } else {
                    text_sec
                };
                if ui
                    .button(
                        egui::RichText::new(if self.scope_fullscreen {
                            "EXIT FULL"
                        } else {
                            "FULL"
                        })
                        .small()
                        .color(fs_col),
                    )
                    .on_hover_text("Toggle fullscreen scope view.")
                    .clicked()
                {
                    self.scope_fullscreen = !self.scope_fullscreen;
                }

                let v_col = if self.show_voice_debug {
                    accent
                } else {
                    Color32::from_gray(80)
                };
                if ui
                    .button(egui::RichText::new("VOICES").small().color(v_col))
                    .on_hover_text("Per-voice gate and envelope stage inspector.")
                    .clicked()
                {
                    self.show_voice_debug = !self.show_voice_debug;
                }
            });
        });

        // ── Voice inspector ────────────────────────────────────────────────────
        if self.show_voice_debug {
            let gates = self.engine.voice_gates();
            let cursors = self.engine.amp_cursors();
            ui.horizontal(|ui| {
                for vi in 0..gates.len() {
                    let gate = gates[vi];
                    let cursor = cursors[vi];
                    let stage = match cursor as u8 {
                        0 => "idle",
                        1 => "A",
                        2 => "D",
                        3 => "S",
                        4 => "R",
                        _ => "?",
                    };
                    let (dot_color, label_color) = if gate > 0.5 {
                        (
                            Color32::from_rgb(220, 60, 60),
                            Color32::from_rgb(255, 120, 120),
                        )
                    } else if cursor > 0.5 {
                        (
                            Color32::from_rgb(200, 140, 40),
                            Color32::from_rgb(220, 180, 80),
                        )
                    } else {
                        (Color32::from_gray(50), Color32::from_gray(100))
                    };
                    ui.vertical(|ui| {
                        ui.set_min_width(48.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, dot_color);
                            ui.label(
                                egui::RichText::new(format!("V{}", vi + 1))
                                    .small()
                                    .color(label_color),
                            );
                        });
                        ui.label(
                            egui::RichText::new(format!("g:{:.0} {}", gate, stage))
                                .monospace()
                                .size(9.0)
                                .color(label_color),
                        );
                    });
                }
            });
        }

        // ── Canvas area — waveform fills the full available space ────────────
        let avail = ui.available_size();
        let (row_resp, cpu_painter) = ui.allocate_painter(avail, Sense::hover());
        let row = row_resp.rect;

        let buf = self.engine.scope_buffer_snapshot();
        let ppp = ui.ctx().pixels_per_point();
        let vp_w = (row.width() * ppp).round() as u32;
        let vp_h = (row.height() * ppp).round() as u32;

        cpu_painter.add(egui_wgpu::Callback::new_paint_callback(
            row,
            ScopeCallback {
                samples: buf,
                x_scale: self.scope_x_scale,
                y_scale: self.scope_y_scale,
                viewport_size: (vp_w.max(1), vp_h.max(1)),
            },
        ));
    }
}

pub fn draw_latency_bar(
    ui: &mut egui::Ui,
    engine: &forma_engine::SynthEngineHandle,
    attack_s: f32,
    theme: &super::theme::SynthTheme,
) {
    let sr = engine.sample_rate();
    let frames = engine.buffer_frames();
    let measured_us = engine.last_latency_us();

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

        let est_color = if est_ms < 20.0 {
            theme.c(&theme.latency_ok)
        } else if est_ms < 40.0 {
            theme.c(&theme.latency_warn)
        } else {
            theme.c(&theme.latency_bad)
        };
        ui.label(
            egui::RichText::new(format!(
                "est ~{est_ms:.0}ms  (buf {buffer_ms:.1} + UI ~{ui_ms:.0} + atk {attack_ms:.0})"
            ))
            .small()
            .color(est_color),
        );

        if measured_us > 0 {
            let measured_ms = measured_us as f32 / 1000.0;
            let meas_color = if measured_ms < 20.0 {
                theme.c(&theme.accent)
            } else if measured_ms < 40.0 {
                theme.c(&theme.latency_warn)
            } else {
                theme.c(&theme.latency_bad)
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

pub fn draw_peak_meter(
    ui: &mut egui::Ui,
    level: f32,
    peak_hold: f32,
    theme: &super::theme::SynthTheme,
) {
    let (resp, painter) =
        ui.allocate_painter(Vec2::new(ui.available_width(), 14.0), Sense::hover());
    let rect = resp.rect;
    painter.rect_filled(rect, CornerRadius::same(2), theme.c(&theme.meter_bg));

    let max_display = 1.5_f32;
    let bar_frac = (level / max_display).clamp(0.0, 1.0);
    let bar_w = rect.width() * bar_frac;

    if bar_w > 0.5 {
        let color = if level < 0.7 {
            theme.c(&theme.meter_green)
        } else if level < 1.0 {
            let t = (level - 0.7) / 0.3;
            let g = theme.meter_green;
            let c = theme.meter_clip;
            Color32::from_rgb(
                (g[0] as f32 + (c[0] as f32 - g[0] as f32) * t) as u8,
                (g[1] as f32 + (c[1] as f32 - g[1] as f32) * t) as u8,
                (g[2] as f32 + (c[2] as f32 - g[2] as f32) * t) as u8,
            )
        } else {
            theme.c(&theme.meter_clip)
        };
        let bar_rect = Rect::from_min_size(rect.min, Vec2::new(bar_w, rect.height()));
        painter.rect_filled(bar_rect, CornerRadius::same(2), color);
    }

    let unity_x = rect.left() + rect.width() * (1.0 / max_display);
    painter.line_segment(
        [
            Pos2::new(unity_x, rect.top()),
            Pos2::new(unity_x, rect.bottom()),
        ],
        Stroke::new(1.0, Color32::from_rgba_premultiplied(255, 255, 255, 100)),
    );

    if peak_hold > 0.01 {
        let hold_frac = (peak_hold / max_display).clamp(0.0, 1.0);
        let hold_x = rect.left() + rect.width() * hold_frac;
        let hold_color = if peak_hold >= 1.0 {
            theme.c(&theme.meter_clip)
        } else {
            Color32::WHITE
        };
        painter.line_segment(
            [
                Pos2::new(hold_x, rect.top() + 1.0),
                Pos2::new(hold_x, rect.bottom() - 1.0),
            ],
            Stroke::new(2.0, hold_color),
        );
    }

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
