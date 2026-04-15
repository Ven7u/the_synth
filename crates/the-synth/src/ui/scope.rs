use crate::SynthApp;
use crate::audio::AudioState;
use eframe::egui;
use egui::{Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};

impl SynthApp {
    pub fn ui_oscilloscope(&mut self, ui: &mut egui::Ui) {
        // Controls row
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("SCOPE")
                    .small()
                    .color(self.theme.c(&self.theme.scope_label)),
            );
            ui.add_space(8.0);
            let scope_ctrl = self.theme.c(&self.theme.scope_label);
            ui.label(egui::RichText::new("X").small().color(scope_ctrl))
                .on_hover_text("Horizontal zoom — drag to stretch or compress the waveform in time.");
            ui.add(
                egui::DragValue::new(&mut self.scope_x_scale)
                    .speed(0.02)
                    .range(0.25_f32..=8.0)
                    .suffix("×"),
            ).on_hover_text("Horizontal zoom (0.25–8×). Drag left/right to adjust.");
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Y").small().color(scope_ctrl))
                .on_hover_text("Vertical zoom — drag to scale the waveform amplitude.");
            ui.add(
                egui::DragValue::new(&mut self.scope_y_scale)
                    .speed(0.02)
                    .range(0.25_f32..=8.0)
                    .suffix("×"),
            ).on_hover_text("Vertical zoom (0.25–8×). Drag left/right to adjust.");
        });

        let buf = self.state.osc_buffer.lock().unwrap().clone();
        let width = ui.available_width();

        // Main scope area
        let (resp, painter) =
            ui.allocate_painter(Vec2::new(width, self.scope_height), Sense::hover());
        let rect = resp.rect;

        // CRT background
        painter.rect_filled(rect, Rounding::same(4.0), self.theme.c(&self.theme.scope_bg));

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
                Stroke::new(1.0, self.theme.c(&self.theme.scope_zero)),
            );

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
                (5.0_f32, self.theme.ca(&self.theme.scope_glow_outer)),
                (3.0_f32, self.theme.ca(&self.theme.scope_glow_mid)),
                (1.2_f32, self.theme.ca(&self.theme.scope_glow_core)),
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
        if handle_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        let ac = self.theme.c(&self.theme.accent);
        let grip_color = if handle_resp.hovered() || handle_resp.dragged() {
            Color32::from_rgba_premultiplied(ac.r(), ac.g(), ac.b(), 140)
        } else {
            Color32::from_rgba_premultiplied(ac.r() / 3, ac.g() / 3, ac.b() / 3, 100)
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

pub fn draw_latency_bar(ui: &mut egui::Ui, state: &AudioState, attack_s: f32, theme: &super::theme::SynthTheme) {
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

pub fn draw_peak_meter(ui: &mut egui::Ui, level: f32, peak_hold: f32, theme: &super::theme::SynthTheme) {
    let (resp, painter) =
        ui.allocate_painter(Vec2::new(ui.available_width(), 14.0), Sense::hover());
    let rect = resp.rect;
    painter.rect_filled(rect, Rounding::same(2.0), theme.c(&theme.meter_bg));

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
        painter.rect_filled(bar_rect, Rounding::same(2.0), color);
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
