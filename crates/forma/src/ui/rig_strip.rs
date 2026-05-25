use crate::audio::TRACK_COUNT;
use crate::ui::layout::AppMode;
use crate::SynthApp;
use eframe::egui;
use egui::{Color32, Rect, Sense, Stroke, Vec2};

impl SynthApp {
    /// Rig strip panel — rendered as a TopBottomPanel above the synth editor in LIVE mode.
    pub fn ui_rig_strip(&mut self, ui: &mut egui::Ui) {
        let accent = self.theme.c(&self.theme.accent);
        let text_sec = self.theme.c(&self.theme.text_secondary);
        let text_dis = self.theme.c(&self.theme.text_disabled);
        let bg_surface = self.theme.c(&self.theme.bg_surface);
        let border = self.theme.c(&self.theme.border);

        ui.horizontal(|ui| {
            // ── Synth track cells ────────────────────────────────────────────
            for t in 0..TRACK_COUNT {
                let focused = self.focused_track == t;
                let muted = self.track_mixer[t].muted();
                let solo = self.track_mixer[t].solo();
                let peak = self.track_mixer[t].peak();

                // Cell frame
                let cell_fill = if focused {
                    bg_surface
                } else {
                    Color32::TRANSPARENT
                };
                let cell_stroke = if focused {
                    Stroke::new(1.0, accent)
                } else {
                    Stroke::new(0.5, border)
                };

                egui::Frame::new()
                    .fill(cell_fill)
                    .stroke(cell_stroke)
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        ui.set_min_width(90.0);

                        // Track name + playing indicator
                        ui.horizontal(|ui| {
                            let dot_col = if !muted {
                                accent
                            } else {
                                Color32::from_gray(50)
                            };
                            ui.label(egui::RichText::new("●").size(8.0).color(dot_col));
                            let name_col = if focused { accent } else { text_sec };
                            let label =
                                egui::RichText::new(format!("T{}  {}", t + 1, self.track_names[t]))
                                    .size(10.0)
                                    .color(name_col);
                            if ui
                                .add(egui::Label::new(label).sense(Sense::click()))
                                .clicked()
                            {
                                self.switch_focused_track(t);
                                // Switch to LIVE mode if in STUDIO
                                if self.app_mode == AppMode::Studio {
                                    self.app_mode = AppMode::Live;
                                }
                            }
                        });

                        // VU meter bar
                        let meter_width = 78.0;
                        let meter_height = 4.0;
                        let (meter_rect, _) = ui.allocate_exact_size(
                            Vec2::new(meter_width, meter_height),
                            Sense::hover(),
                        );
                        let painter = ui.painter_at(meter_rect);
                        painter.rect_filled(meter_rect, 1.0, Color32::from_gray(22));
                        let fill_w = (peak.min(1.0) * meter_width).max(0.0);
                        if fill_w > 0.5 {
                            let fill_rect = Rect::from_min_size(
                                meter_rect.min,
                                Vec2::new(fill_w, meter_height),
                            );
                            let bar_col = if peak > 0.9 {
                                Color32::from_rgb(220, 80, 60)
                            } else if peak > 0.6 {
                                Color32::from_rgb(200, 180, 40)
                            } else {
                                Color32::from_rgb(
                                    (accent.r() as f32 * 0.7) as u8,
                                    (accent.g() as f32 * 0.7) as u8,
                                    (accent.b() as f32 * 0.7) as u8,
                                )
                            };
                            painter.rect_filled(fill_rect, 1.0, bar_col);
                        }

                        // Patch name
                        ui.label(
                            egui::RichText::new(&self.track_patches[t].name)
                                .size(9.0)
                                .color(text_dis),
                        );

                        // M / S buttons
                        ui.horizontal(|ui| {
                            let m_col = if muted {
                                Color32::from_rgb(220, 80, 60)
                            } else {
                                text_dis
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("M").size(9.0).color(m_col),
                                    )
                                    .frame(muted)
                                    .min_size(Vec2::new(18.0, 14.0)),
                                )
                                .on_hover_text("Mute this track")
                                .clicked()
                            {
                                self.track_mixer[t].set_muted(!muted);
                            }

                            let s_col = if solo { accent } else { text_dis };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("S").size(9.0).color(s_col),
                                    )
                                    .frame(solo)
                                    .min_size(Vec2::new(18.0, 14.0)),
                                )
                                .on_hover_text("Solo this track")
                                .clicked()
                            {
                                self.track_mixer[t].set_solo(!solo);
                            }
                        });
                    });

                ui.add_space(2.0);
            }

            // ── Drums cell ────────────────────────────────────────────────────
            let drums_focused = self.app_mode == AppMode::DrumMachine;
            let drums_cell_stroke = if drums_focused {
                Stroke::new(1.0, accent)
            } else {
                Stroke::new(0.5, border)
            };
            egui::Frame::new()
                .fill(if drums_focused {
                    bg_surface
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(drums_cell_stroke)
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(6, 4))
                .show(ui, |ui| {
                    ui.set_min_width(64.0);
                    ui.horizontal(|ui| {
                        let dot_col = if self.drums.enabled {
                            accent
                        } else {
                            Color32::from_gray(50)
                        };
                        ui.label(egui::RichText::new("●").size(8.0).color(dot_col));
                        let col = if drums_focused { accent } else { text_sec };
                        if ui
                            .add(
                                egui::Label::new(
                                    egui::RichText::new("DRUMS").size(10.0).color(col),
                                )
                                .sense(Sense::click()),
                            )
                            .clicked()
                        {
                            self.app_mode = AppMode::DrumMachine;
                        }
                    });
                    // Placeholder VU (drum engine not yet wired)
                    let (meter_rect, _) =
                        ui.allocate_exact_size(Vec2::new(52.0, 4.0), Sense::hover());
                    ui.painter_at(meter_rect)
                        .rect_filled(meter_rect, 1.0, Color32::from_gray(22));
                    ui.label(egui::RichText::new("Phase 5").size(9.0).color(text_dis));
                    let drum_muted = !self.drums.enabled;
                    let m_col = if drum_muted {
                        Color32::from_rgb(220, 80, 60)
                    } else {
                        text_dis
                    };
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new("M").size(9.0).color(m_col))
                                .frame(drum_muted)
                                .min_size(Vec2::new(18.0, 14.0)),
                        )
                        .on_hover_text("Mute drums")
                        .clicked()
                    {
                        self.drums.enabled = !self.drums.enabled;
                    }
                });

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            // ── Transport + MIX▸ ─────────────────────────────────────────────
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(format!("♩ {} BPM", self.global_bpm))
                        .size(10.0)
                        .color(accent),
                );
                let mix_col = if self.show_mixer { accent } else { text_dis };
                if ui
                    .add(
                        egui::Label::new(egui::RichText::new("MIX▸").size(10.0).color(mix_col))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Toggle mixer panel")
                    .clicked()
                {
                    self.show_mixer = !self.show_mixer;
                }
            });
        });
    }
}
