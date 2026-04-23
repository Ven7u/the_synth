use egui::{Color32, Pos2, Response, Sense, Stroke, Ui, Vec2};
use std::f32::consts::PI;

use crate::ui::theme::SynthTheme;

/// A circular arc knob widget for synth parameters.
///
/// - Displays a 270-degree arc with a fill indicator
/// - Drag up/down to adjust value
/// - Shift+drag for fine control (10x slower)
/// - Shows label below, value text centered
///
/// Returns the egui Response (for hover text, etc).
pub fn knob(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    label: &str,
    theme: &SynthTheme,
    logarithmic: bool,
) -> Response {
    let desired_size = Vec2::new(44.0, 64.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());

    let knob_radius = 16.0;
    let center = Pos2::new(rect.center().x, rect.top() + knob_radius + 2.0);

    let old_value = *value;

    let log_t = |v: f32| -> f32 {
        if logarithmic && *range.start() > 0.0 {
            (v / *range.start()).ln() / (*range.end() / *range.start()).ln()
        } else {
            (v - *range.start()) / (*range.end() - *range.start())
        }
    };
    let log_v = |t: f32| -> f32 {
        if logarithmic && *range.start() > 0.0 {
            *range.start() * (*range.end() / *range.start()).powf(t)
        } else {
            *range.start() + t * (*range.end() - *range.start())
        }
    };

    // Handle drag input.
    if response.dragged() {
        let delta = -response.drag_delta().y; // up = increase
        let speed = if ui.input(|i| i.modifiers.shift) {
            0.001
        } else {
            0.005
        };
        let t = (log_t(*value) + delta * speed).clamp(0.0, 1.0);
        *value = log_v(t).clamp(*range.start(), *range.end());
    }

    // Double-click to reset to midpoint.
    if response.double_clicked() {
        *value = log_v(0.5);
    }

    if (*value - old_value).abs() > f32::EPSILON {
        response.mark_changed();
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);

        // Normalize value to 0..1.
        let range_span = *range.end() - *range.start();
        let t = if range_span > 0.0 { log_t(*value) } else { 0.0 };

        // Arc geometry: 270 degrees, starting from bottom-left going clockwise.
        let start_angle = PI * 0.75; // 135 degrees (bottom-left)
        let sweep = PI * 1.5; // 270 degrees total

        let accent = theme.c(&theme.accent);

        // Track arc (dim background).
        let track_color =
            Color32::from_rgba_premultiplied(accent.r() / 4, accent.g() / 4, accent.b() / 4, 80);
        draw_arc(
            &painter,
            center,
            knob_radius,
            start_angle,
            sweep,
            2.5,
            track_color,
        );

        // Fill arc (colored portion).
        if t > 0.01 {
            draw_arc(
                &painter,
                center,
                knob_radius,
                start_angle,
                sweep * t,
                3.0,
                accent,
            );
        }

        // Indicator dot at current position.
        let indicator_angle = start_angle + sweep * t;
        let dot_pos = Pos2::new(
            center.x + indicator_angle.cos() * knob_radius,
            center.y + indicator_angle.sin() * knob_radius,
        );
        painter.circle_filled(dot_pos, 3.5, accent);

        // Center dot.
        let center_color = if response.hovered() || response.dragged() {
            Color32::from_rgb(200, 200, 200)
        } else {
            Color32::from_rgb(80, 80, 80)
        };
        painter.circle_filled(center, 4.0, center_color);

        // Value text — compact format.
        let value_text = if range_span >= 100.0 {
            format!("{:.0}", *value)
        } else if range_span >= 1.0 {
            format!("{:.1}", *value)
        } else {
            format!("{:.2}", *value)
        };
        painter.text(
            Pos2::new(center.x, center.y + knob_radius + 5.0),
            egui::Align2::CENTER_TOP,
            &value_text,
            egui::FontId::proportional(9.0),
            Color32::from_rgb(180, 180, 180),
        );

        // Label below value.
        painter.text(
            Pos2::new(center.x, rect.bottom() - 1.0),
            egui::Align2::CENTER_BOTTOM,
            label,
            egui::FontId::proportional(9.0),
            Color32::from_rgb(140, 140, 140),
        );
    }

    response
}

/// Draw an arc using line segments.
fn draw_arc(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    start_angle: f32,
    sweep: f32,
    width: f32,
    color: Color32,
) {
    let segments = (sweep.abs() * 20.0).ceil() as usize;
    if segments < 2 {
        return;
    }
    let step = sweep / segments as f32;
    for i in 0..segments {
        let a0 = start_angle + step * i as f32;
        let a1 = start_angle + step * (i + 1) as f32;
        let p0 = Pos2::new(center.x + a0.cos() * radius, center.y + a0.sin() * radius);
        let p1 = Pos2::new(center.x + a1.cos() * radius, center.y + a1.sin() * radius);
        painter.line_segment([p0, p1], Stroke::new(width, color));
    }
}
