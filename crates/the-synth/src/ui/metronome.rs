use crate::SynthApp;
use eframe::egui;
use egui::{Color32, Pos2, Rect, Rounding, Sense, Vec2};

/// Advance the metronome phase by wall-clock delta each frame.
/// Must be called once per frame while the metronome is enabled.
impl SynthApp {
    pub fn tick_metronome(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.metro_enabled {
            let dt = (now - self.metro_last_time).max(0.0).min(0.5);
            // beats per second depends on BPM and beat unit.
            // 4/4 at 120 BPM: 2 beats/s. 6/8 at 120 BPM: 4 eighth-beats/s.
            let bps = self.global_bpm as f64 / 60.0 * (4.0 / self.metro_denom as f64);
            self.metro_phase = (self.metro_phase + dt * bps) % self.metro_beats as f64;
            ctx.request_repaint();
        }
        self.metro_last_time = now;
    }

    /// Reset metronome phase to 0 — call alongside sync_transport_now().
    pub fn metro_reset(&mut self) {
        self.metro_phase = 0.0;
    }

    pub fn ui_metronome_window(&mut self, ctx: &egui::Context) {
        if !self.show_metronome {
            return;
        }

        let accent = self.theme.c(&self.theme.accent);
        let text_sec = self.theme.c(&self.theme.text_secondary);
        let bg = self.theme.c(&self.theme.bg_surface);

        let mut open = self.show_metronome;
        egui::Window::new("Metronome")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_size([320.0, 120.0])
            .show(ctx, |ui| {
                // ── Controls row ─────────────────────────────────────────
                ui.horizontal(|ui| {
                    // Play/stop toggle
                    let play_label = if self.metro_enabled {
                        egui::RichText::new("■ STOP").color(Color32::from_rgb(220, 80, 80))
                    } else {
                        egui::RichText::new("▶ START").color(accent)
                    };
                    if ui.button(play_label).clicked() {
                        self.metro_enabled = !self.metro_enabled;
                        if self.metro_enabled {
                            self.metro_phase = 0.0;
                        }
                    }

                    ui.separator();

                    // Time signature
                    ui.label(egui::RichText::new("Time sig").small().color(text_sec));

                    // Numerator
                    let mut beats = self.metro_beats as u32;
                    if ui
                        .add(egui::DragValue::new(&mut beats).range(2..=8).speed(0.05))
                        .on_hover_text("Beats per bar (2–8)")
                        .changed()
                    {
                        self.metro_beats = beats as u8;
                        self.metro_phase = 0.0;
                    }

                    ui.label(egui::RichText::new("/").color(text_sec));

                    // Denominator — only musical values
                    let denoms = [2u8, 4, 8, 16];
                    let cur = self.metro_denom;
                    egui::ComboBox::from_id_salt("metro_denom")
                        .width(42.0)
                        .selected_text(format!("{cur}"))
                        .show_ui(ui, |ui| {
                            for &d in &denoms {
                                if ui.selectable_label(cur == d, format!("{d}")).clicked() {
                                    self.metro_denom = d;
                                    self.metro_phase = 0.0;
                                }
                            }
                        });

                    ui.separator();

                    // Quick presets
                    for (label, num, den) in [
                        ("2/4", 2u8, 4u8),
                        ("3/4", 3, 4),
                        ("4/4", 4, 4),
                        ("5/4", 5, 4),
                        ("6/8", 6, 8),
                        ("7/8", 7, 8),
                    ] {
                        let active = self.metro_beats == num && self.metro_denom == den;
                        let col = if active { accent } else { text_sec };
                        if ui
                            .button(egui::RichText::new(label).small().color(col))
                            .clicked()
                        {
                            self.metro_beats = num;
                            self.metro_denom = den;
                            self.metro_phase = 0.0;
                        }
                    }
                });

                ui.add_space(6.0);

                // ── Beat circles ─────────────────────────────────────────
                let beats = self.metro_beats as usize;
                let current_beat = self.metro_phase as usize; // integer part = current beat index
                let beat_frac = self.metro_phase.fract() as f32;

                // Allocate space for the circles
                const CIRCLE_D: f32 = 28.0;
                const GAP: f32 = 8.0;
                let total_w = beats as f32 * CIRCLE_D + (beats - 1) as f32 * GAP;
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(total_w.max(ui.available_width()), CIRCLE_D + 8.0),
                    Sense::hover(),
                );
                let painter = ui.painter_at(rect);

                // Center circles horizontally
                let x_start = rect.center().x - total_w * 0.5 + CIRCLE_D * 0.5;
                let cy = rect.center().y;

                for i in 0..beats {
                    let cx = x_start + i as f32 * (CIRCLE_D + GAP);
                    let center = Pos2::new(cx, cy);
                    let radius = CIRCLE_D * 0.5;

                    let is_current = i == current_beat && self.metro_enabled;
                    let is_downbeat = i == 0;

                    // Background circle
                    let bg_color = if is_current {
                        if is_downbeat {
                            // Downbeat: bright accent, pulse on beat attack
                            let pulse = (1.0 - beat_frac).powf(2.5);
                            let r = (accent.r() as f32 * (0.6 + 0.4 * pulse)) as u8;
                            let g = (accent.g() as f32 * (0.6 + 0.4 * pulse)) as u8;
                            let b = (accent.b() as f32 * (0.6 + 0.4 * pulse)) as u8;
                            Color32::from_rgb(r, g, b)
                        } else {
                            // Other beats: dimmer, different hue
                            let pulse = (1.0 - beat_frac).powf(2.5);
                            let base = Color32::from_rgb(80, 160, 200);
                            let r = (base.r() as f32 * (0.4 + 0.6 * pulse)) as u8;
                            let g = (base.g() as f32 * (0.4 + 0.6 * pulse)) as u8;
                            let b = (base.b() as f32 * (0.4 + 0.6 * pulse)) as u8;
                            Color32::from_rgb(r, g, b)
                        }
                    } else if is_downbeat {
                        // Downbeat idle: slightly brighter than rest
                        Color32::from_rgb(
                            (accent.r() / 5).max(30),
                            (accent.g() / 5).max(30),
                            (accent.b() / 5).max(30),
                        )
                    } else {
                        Color32::from_gray(40)
                    };

                    painter.circle_filled(center, radius, bg_color);

                    // Beat number label
                    let label_col = if is_current {
                        Color32::WHITE
                    } else {
                        Color32::from_gray(110)
                    };
                    painter.text(
                        center,
                        egui::Align2::CENTER_CENTER,
                        format!("{}", i + 1),
                        egui::FontId::proportional(11.0),
                        label_col,
                    );

                }

                // Small info line
                ui.add_space(4.0);
                let bps = self.global_bpm as f64 / 60.0 * (4.0 / self.metro_denom as f64);
                let bar_dur_s = self.metro_beats as f64 / bps;
                ui.label(
                    egui::RichText::new(format!(
                        "{} BPM  ·  {}/{} = {:.2}s / bar",
                        self.global_bpm, self.metro_beats, self.metro_denom, bar_dur_s
                    ))
                    .small()
                    .color(text_sec),
                );

                let _ = bg; // suppress unused warning
            });
        self.show_metronome = open;
    }
}
