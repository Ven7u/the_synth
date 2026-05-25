use crate::audio::TRACK_COUNT;
use crate::SynthApp;
use eframe::egui;
use egui::{Color32, Rect, Sense, Vec2};
use std::sync::Arc;

const FADER_HEIGHT: f32 = 120.0;
const CHANNEL_WIDTH: f32 = 56.0;

impl SynthApp {
    pub fn ui_mix_board(&mut self, ui: &mut egui::Ui) {
        let accent = self.theme.c(&self.theme.accent);
        let text_sec = self.theme.c(&self.theme.text_secondary);
        let text_dis = self.theme.c(&self.theme.text_disabled);
        let border = self.theme.c(&self.theme.border);

        // Header
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("MIXER").size(11.0).color(accent));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close_col = text_dis;
                if ui
                    .add(
                        egui::Label::new(egui::RichText::new("✕").size(11.0).color(close_col))
                            .sense(Sense::click()),
                    )
                    .clicked()
                {
                    self.show_mixer = false;
                }
            });
        });

        ui.add_space(4.0);

        // Check if any track is soloed
        let any_solo = (0..TRACK_COUNT).any(|t| self.track_mixer[t].solo());

        ui.horizontal(|ui| {
            // ── Synth track channels ──────────────────────────────────────────
            for t in 0..TRACK_COUNT {
                let focused = self.focused_track == t;
                let mixer = Arc::clone(&self.track_mixer[t]);

                let mut vol = mixer.volume();
                let mut pan = mixer.pan();
                let muted = mixer.muted();
                let solo = mixer.solo();
                let peak = mixer.peak();

                // A track is effectively silent if muted or if another track is soloed
                let silenced = muted || (any_solo && !solo);

                ui.vertical(|ui| {
                    ui.set_min_width(CHANNEL_WIDTH);

                    // Track name
                    let name_col = if focused { accent } else { text_sec };
                    ui.label(
                        egui::RichText::new(format!("T{}", t + 1))
                            .size(9.0)
                            .color(name_col),
                    );
                    ui.label(
                        egui::RichText::new(&self.track_names[t])
                            .size(8.0)
                            .color(text_dis),
                    );

                    ui.add_space(4.0);

                    // VU + fader together in a horizontal strip
                    ui.horizontal(|ui| {
                        // VU meter (vertical bar)
                        let vu_w = 6.0;
                        let (vu_rect, _) =
                            ui.allocate_exact_size(Vec2::new(vu_w, FADER_HEIGHT), Sense::hover());
                        let painter = ui.painter_at(vu_rect);
                        painter.rect_filled(vu_rect, 1.0, Color32::from_gray(20));
                        let fill_h = (peak.min(1.0) * FADER_HEIGHT).max(0.0);
                        if fill_h > 0.5 {
                            let fill_rect = Rect::from_min_size(
                                egui::pos2(vu_rect.min.x, vu_rect.max.y - fill_h),
                                Vec2::new(vu_w, fill_h),
                            );
                            let bar_col = if silenced {
                                Color32::from_gray(40)
                            } else if peak > 0.9 {
                                Color32::from_rgb(220, 80, 60)
                            } else if peak > 0.6 {
                                Color32::from_rgb(200, 180, 40)
                            } else {
                                Color32::from_rgb(
                                    (accent.r() as f32 * 0.65) as u8,
                                    (accent.g() as f32 * 0.65) as u8,
                                    (accent.b() as f32 * 0.65) as u8,
                                )
                            };
                            painter.rect_filled(fill_rect, 1.0, bar_col);
                        }

                        ui.add_space(2.0);

                        // Volume fader (vertical slider)
                        let fader_col = if silenced { text_dis } else { accent };
                        let fader_resp = ui.add(
                            egui::Slider::new(&mut vol, 0.0..=1.0)
                                .vertical()
                                .show_value(false)
                                .text("")
                                .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 2.0 }),
                        );
                        // Tint the fader track to match the theme
                        let _ = fader_col;
                        if fader_resp.changed() {
                            mixer.set_volume(vol);
                        }
                    });

                    ui.add_space(2.0);

                    // Volume readout
                    ui.label(
                        egui::RichText::new(format!("{:.0}%", vol * 100.0))
                            .size(8.0)
                            .color(text_dis),
                    );

                    ui.add_space(2.0);

                    // Pan slider
                    let pan_resp = ui.add(
                        egui::Slider::new(&mut pan, -1.0..=1.0)
                            .show_value(false)
                            .text("")
                            .fixed_decimals(2),
                    );
                    if pan_resp.changed() {
                        mixer.set_pan(pan);
                    }
                    // Pan readout
                    let pan_label = if pan.abs() < 0.02 {
                        "C".to_string()
                    } else if pan < 0.0 {
                        format!("L{:.0}", pan.abs() * 100.0)
                    } else {
                        format!("R{:.0}", pan * 100.0)
                    };
                    ui.label(egui::RichText::new(pan_label).size(8.0).color(text_dis));

                    ui.add_space(4.0);

                    // M / S buttons
                    ui.horizontal(|ui| {
                        let m_col = if muted {
                            Color32::from_rgb(220, 80, 60)
                        } else {
                            text_dis
                        };
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("M").size(9.0).color(m_col))
                                    .frame(muted)
                                    .min_size(Vec2::new(20.0, 14.0)),
                            )
                            .on_hover_text("Mute")
                            .clicked()
                        {
                            mixer.set_muted(!muted);
                        }
                        let s_col = if solo { accent } else { text_dis };
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("S").size(9.0).color(s_col))
                                    .frame(solo)
                                    .min_size(Vec2::new(20.0, 14.0)),
                            )
                            .on_hover_text("Solo")
                            .clicked()
                        {
                            mixer.set_solo(!solo);
                        }
                    });

                    // Focused indicator
                    if focused {
                        let (r, _) = ui.allocate_exact_size(
                            Vec2::new(CHANNEL_WIDTH - 4.0, 2.0),
                            Sense::hover(),
                        );
                        ui.painter_at(r).rect_filled(r, 0.0, accent);
                    }
                });

                // Divider between channels
                if t < TRACK_COUNT - 1 {
                    ui.add_space(2.0);
                    let (line_rect, _) =
                        ui.allocate_exact_size(Vec2::new(1.0, FADER_HEIGHT + 80.0), Sense::hover());
                    ui.painter_at(line_rect).rect_filled(line_rect, 0.0, border);
                    ui.add_space(2.0);
                }
            }

            // ── Drum bus channel ─────────────────────────────────────────────
            ui.add_space(2.0);
            let (line_rect, _) =
                ui.allocate_exact_size(Vec2::new(1.0, FADER_HEIGHT + 80.0), Sense::hover());
            ui.painter_at(line_rect).rect_filled(line_rect, 0.0, border);
            ui.add_space(2.0);

            ui.vertical(|ui| {
                ui.set_min_width(CHANNEL_WIDTH);

                let drums_col = if self.app_mode == crate::ui::layout::AppMode::DrumMachine {
                    accent
                } else {
                    text_sec
                };
                ui.label(egui::RichText::new("DRUMS").size(9.0).color(drums_col));
                ui.label(egui::RichText::new("step seq").size(8.0).color(text_dis));

                ui.add_space(4.0);

                let drum_engine = std::sync::Arc::clone(&self.drum_engine);
                let mut dvol = drum_engine.volume();
                let mut dpan = drum_engine.pan();
                let drum_muted = drum_engine.muted.load(std::sync::atomic::Ordering::Relaxed);
                let dpeak = drum_engine.peak();

                // VU + volume fader
                ui.horizontal(|ui| {
                    let vu_w = 6.0;
                    let (vu_rect, _) =
                        ui.allocate_exact_size(Vec2::new(vu_w, FADER_HEIGHT), Sense::hover());
                    let painter = ui.painter_at(vu_rect);
                    painter.rect_filled(vu_rect, 1.0, Color32::from_gray(20));
                    let fill_h = (dpeak.min(1.0) * FADER_HEIGHT).max(0.0);
                    if fill_h > 0.5 {
                        let fill_rect = Rect::from_min_size(
                            egui::pos2(vu_rect.min.x, vu_rect.max.y - fill_h),
                            Vec2::new(vu_w, fill_h),
                        );
                        let bar_col = if drum_muted {
                            Color32::from_gray(40)
                        } else if dpeak > 0.9 {
                            Color32::from_rgb(220, 80, 60)
                        } else if dpeak > 0.6 {
                            Color32::from_rgb(200, 180, 40)
                        } else {
                            Color32::from_rgb(
                                (accent.r() as f32 * 0.65) as u8,
                                (accent.g() as f32 * 0.65) as u8,
                                (accent.b() as f32 * 0.65) as u8,
                            )
                        };
                        painter.rect_filled(fill_rect, 1.0, bar_col);
                    }
                    ui.add_space(2.0);
                    if ui
                        .add(
                            egui::Slider::new(&mut dvol, 0.0..=1.0)
                                .vertical()
                                .show_value(false)
                                .text("")
                                .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 2.0 }),
                        )
                        .changed()
                    {
                        drum_engine.set_volume(dvol);
                    }
                });

                ui.label(
                    egui::RichText::new(format!("{:.0}%", dvol * 100.0))
                        .size(8.0)
                        .color(text_dis),
                );
                ui.add_space(2.0);

                let pan_resp = ui.add(
                    egui::Slider::new(&mut dpan, -1.0..=1.0)
                        .show_value(false)
                        .text(""),
                );
                if pan_resp.changed() {
                    drum_engine.set_pan(dpan);
                }
                let pan_label = if dpan.abs() < 0.02 {
                    "C".to_string()
                } else if dpan < 0.0 {
                    format!("L{:.0}", dpan.abs() * 100.0)
                } else {
                    format!("R{:.0}", dpan * 100.0)
                };
                ui.label(egui::RichText::new(pan_label).size(8.0).color(text_dis));
                ui.add_space(4.0);

                let m_col = if drum_muted {
                    Color32::from_rgb(220, 80, 60)
                } else {
                    text_dis
                };
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("M").size(9.0).color(m_col))
                            .frame(drum_muted)
                            .min_size(Vec2::new(20.0, 14.0)),
                    )
                    .on_hover_text("Mute drum bus")
                    .clicked()
                {
                    drum_engine
                        .muted
                        .store(!drum_muted, std::sync::atomic::Ordering::Relaxed);
                }
            });
        });
    }
}
