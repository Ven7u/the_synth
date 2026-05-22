use crate::audio::TRACK_COUNT;
use crate::ui::frame::SynthFrame;
use crate::ui::layout::AppMode;
use crate::SynthApp;
use eframe::egui;
use egui::{Color32, Rect, Sense, Vec2};
use std::sync::atomic::Ordering;
use std::sync::Arc;

// Width of each track card in the selector strip.
const CARD_W: f32 = 130.0;
const CARD_H: f32 = 62.0;
const DRUMS_CARD_W: f32 = 100.0;

// Per-track colors for the VU / indicator tints.
const TRACK_TINTS: [Color32; TRACK_COUNT] = [
    Color32::from_rgb(80, 180, 230),
    Color32::from_rgb(160, 220, 80),
    Color32::from_rgb(230, 150, 60),
    Color32::from_rgb(200, 80, 200),
];

impl SynthApp {
    // ── Shared dock renderer ─────────────────────────────────────────────────
    //
    // Used by Studio mode and the LIVE expanded-track view.

    pub fn ui_synth_dock(&mut self, ui: &mut egui::Ui) {
        if self.reset_layout_pending {
            self.dock_state = crate::ui::dock::default_dock_state();
            self.reset_layout_pending = false;
        }
        let mut dock_state =
            std::mem::replace(&mut self.dock_state, egui_dock::DockState::new(vec![]));
        let mut s = egui_dock::Style::from_egui(ui.style());
        s.separator.width = 6.0;
        s.separator.color_idle = Color32::TRANSPARENT;
        s.separator.color_hovered = Color32::from_black_alpha(60);
        s.separator.color_dragged = Color32::from_black_alpha(100);
        s.dock_area_padding = Some(egui::Margin::same(6i8));
        let rm = self.theme.rounding_md as u8;
        let r_top = egui::CornerRadius {
            nw: rm,
            ne: rm,
            sw: 0,
            se: 0,
        };
        let r_body = egui::CornerRadius {
            nw: 0,
            ne: rm,
            sw: rm,
            se: rm,
        };
        let bg_surface = self.theme.c(&self.theme.bg_surface);
        let border = self.theme.c(&self.theme.border);
        let text_pri = self.theme.c(&self.theme.text_primary);
        let text_sec = self.theme.c(&self.theme.text_secondary);
        let accent = self.theme.c(&self.theme.accent);
        s.tab.tab_body.corner_radius = r_body;
        s.tab.tab_body.stroke = egui::Stroke::new(self.theme.stroke_ui, border);
        s.tab_bar.bg_fill = Color32::TRANSPARENT;
        s.tab_bar.hline_color = Color32::TRANSPARENT;
        s.tab_bar.corner_radius = r_body;
        s.tab_bar.height = 28.0;
        s.tab.active = egui_dock::TabInteractionStyle {
            corner_radius: r_top,
            bg_fill: bg_surface,
            text_color: text_pri,
            outline_color: accent,
        };
        s.tab.focused = egui_dock::TabInteractionStyle {
            corner_radius: r_top,
            bg_fill: bg_surface,
            text_color: accent,
            outline_color: accent,
        };
        s.tab.inactive = egui_dock::TabInteractionStyle {
            corner_radius: r_top,
            bg_fill: Color32::TRANSPARENT,
            text_color: text_sec,
            outline_color: Color32::TRANSPARENT,
        };
        s.tab.hovered = egui_dock::TabInteractionStyle {
            corner_radius: r_top,
            bg_fill: bg_surface,
            text_color: text_pri,
            outline_color: border,
        };
        egui_dock::DockArea::new(&mut dock_state)
            .style(s)
            .show_inside(ui, &mut crate::ui::dock::SynthTabViewer { app: self });
        self.dock_state = dock_state;
    }

    // ── LIVE view ────────────────────────────────────────────────────────────
    //
    // Track selector strip at the top, full studio dock below.

    pub fn ui_live_view(&mut self, ui: &mut egui::Ui) {
        // Optional mixer side-panel.
        if self.show_mixer {
            egui::SidePanel::right("live_mixer_panel")
                .resizable(true)
                .default_width(300.0)
                .min_width(260.0)
                .frame(
                    egui::Frame::new()
                        .fill(self.theme.c(&self.theme.bg_surface))
                        .inner_margin(egui::Margin::same(8))
                        .stroke(egui::Stroke::new(1.0, self.theme.c(&self.theme.border))),
                )
                .show_inside(ui, |ui| {
                    self.ui_mix_board(ui);
                });
        }

        // Track selector strip — pinned to the top of the live area.
        egui::TopBottomPanel::top("live_track_strip")
            .exact_height(CARD_H + 12.0)
            .frame(
                egui::Frame::new()
                    .fill(self.theme.c(&self.theme.bg_surface))
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .stroke(egui::Stroke::new(0.5, self.theme.c(&self.theme.border))),
            )
            .show_inside(ui, |ui| {
                self.ui_live_track_strip(ui);
            });

        // Full studio dock for the focused track.
        egui::CentralPanel::default()
            .frame(SynthFrame::app_bg(&self.theme))
            .show_inside(ui, |ui| {
                self.ui_synth_dock(ui);
            });
    }

    // ── Track selector strip ─────────────────────────────────────────────────

    fn ui_live_track_strip(&mut self, ui: &mut egui::Ui) {
        let accent = self.theme.c(&self.theme.accent);
        let text_sec = self.theme.c(&self.theme.text_secondary);
        let text_dis = self.theme.c(&self.theme.text_disabled);
        let border = self.theme.c(&self.theme.border);

        ui.horizontal(|ui| {
            // ── Synth tracks ────────────────────────────────────────────────
            for t in 0..TRACK_COUNT {
                let focused = self.focused_track == t && self.app_mode == AppMode::Live;
                let tint = TRACK_TINTS[t % TRACK_TINTS.len()];
                let mixer = Arc::clone(&self.track_mixer[t]);
                let peak = mixer.peak();
                let muted = mixer.muted();
                let solo = mixer.solo();
                let seq_playing = self.track_seq[t].playing.load(Ordering::Relaxed);
                let patch_name = self.track_patches[t].name.clone();
                let track_name = self.track_names[t].clone();

                let card_fill = if focused {
                    // Slightly lighten bg_surface to distinguish focused card.
                    let b = self.theme.c(&self.theme.bg_surface);
                    Color32::from_rgb(
                        (b.r() as u16 + 18).min(255) as u8,
                        (b.g() as u16 + 18).min(255) as u8,
                        (b.b() as u16 + 18).min(255) as u8,
                    )
                } else {
                    Color32::TRANSPARENT
                };

                let card_stroke = if focused {
                    egui::Stroke::new(1.5, tint)
                } else {
                    egui::Stroke::new(0.5, border)
                };

                let card_resp = egui::Frame::new()
                    .fill(card_fill)
                    .stroke(card_stroke)
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .corner_radius(egui::CornerRadius::same(4))
                    .show(ui, |ui| {
                        ui.set_min_width(CARD_W);
                        ui.set_max_width(CARD_W);

                        // Row 1: dot + track label + seq indicator.
                        ui.horizontal(|ui| {
                            let dot_col = if muted { Color32::from_gray(40) } else { tint };
                            ui.label(egui::RichText::new("●").size(7.0).color(dot_col));

                            let label_col = if focused { tint } else { text_sec };
                            ui.label(
                                egui::RichText::new(format!("T{}  {}", t + 1, track_name))
                                    .size(10.0)
                                    .color(label_col),
                            );

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if seq_playing {
                                        ui.label(egui::RichText::new("▶").size(8.0).color(tint));
                                    }
                                },
                            );
                        });

                        // Row 2: patch name.
                        ui.label(egui::RichText::new(&patch_name).size(8.0).color(text_dis));

                        ui.add_space(3.0);

                        // Row 3: VU bar + M/S buttons.
                        ui.horizontal(|ui| {
                            // VU bar
                            let vu_w = CARD_W - 52.0;
                            let (vu_rect, _) =
                                ui.allocate_exact_size(Vec2::new(vu_w, 6.0), Sense::hover());
                            let painter = ui.painter_at(vu_rect);
                            painter.rect_filled(vu_rect, 1.0, Color32::from_gray(20));
                            let fill_w = (peak.min(1.0) * vu_w).max(0.0);
                            if fill_w > 0.5 {
                                let fill = Rect::from_min_size(vu_rect.min, Vec2::new(fill_w, 6.0));
                                let bar_col = if muted {
                                    Color32::from_gray(40)
                                } else if peak > 0.9 {
                                    Color32::from_rgb(220, 80, 60)
                                } else if peak > 0.6 {
                                    Color32::from_rgb(200, 180, 40)
                                } else {
                                    tint
                                };
                                painter.rect_filled(fill, 1.0, bar_col);
                            }

                            ui.add_space(4.0);

                            // M button.
                            let m_col = if muted {
                                Color32::from_rgb(220, 80, 60)
                            } else {
                                text_dis
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("M").size(8.0).color(m_col),
                                    )
                                    .frame(muted)
                                    .min_size(Vec2::new(16.0, 12.0)),
                                )
                                .on_hover_text("Mute")
                                .clicked()
                            {
                                mixer.set_muted(!muted);
                            }

                            // S button.
                            let s_col = if solo { accent } else { text_dis };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("S").size(8.0).color(s_col),
                                    )
                                    .frame(solo)
                                    .min_size(Vec2::new(16.0, 12.0)),
                                )
                                .on_hover_text("Solo")
                                .clicked()
                            {
                                mixer.set_solo(!solo);
                            }
                        });
                    })
                    .response;

                // Click anywhere on the card to focus this track.
                if card_resp.interact(Sense::click()).clicked() {
                    self.switch_focused_track(t);
                    self.app_mode = AppMode::Live;
                }

                ui.add_space(4.0);
            }

            // Divider before drums.
            ui.separator();
            ui.add_space(4.0);

            // ── Drums card ──────────────────────────────────────────────────
            let drums_active = self.app_mode == AppMode::DrumMachine;
            let drum_muted = self.drum_engine.muted.load(Ordering::Relaxed);
            let dpeak = self.drum_engine.peak();
            let drum_seq_enabled = self.drums.enabled;

            let card_fill = if drums_active {
                let b = self.theme.c(&self.theme.bg_surface);
                Color32::from_rgb(
                    (b.r() as u16 + 18).min(255) as u8,
                    (b.g() as u16 + 18).min(255) as u8,
                    (b.b() as u16 + 18).min(255) as u8,
                )
            } else {
                Color32::TRANSPARENT
            };
            let drum_tint = Color32::from_rgb(200, 160, 60);
            let card_stroke = if drums_active {
                egui::Stroke::new(1.5, drum_tint)
            } else {
                egui::Stroke::new(0.5, border)
            };

            let drums_resp = egui::Frame::new()
                .fill(card_fill)
                .stroke(card_stroke)
                .inner_margin(egui::Margin::symmetric(6, 4))
                .corner_radius(egui::CornerRadius::same(4))
                .show(ui, |ui| {
                    ui.set_min_width(DRUMS_CARD_W);
                    ui.set_max_width(DRUMS_CARD_W);

                    ui.horizontal(|ui| {
                        let dot_col = if drum_seq_enabled && !drum_muted {
                            drum_tint
                        } else {
                            Color32::from_gray(40)
                        };
                        ui.label(egui::RichText::new("●").size(7.0).color(dot_col));
                        let label_col = if drums_active { drum_tint } else { text_sec };
                        ui.label(egui::RichText::new("DRUMS").size(10.0).color(label_col));
                    });

                    ui.label(egui::RichText::new("Step seq").size(8.0).color(text_dis));
                    ui.add_space(3.0);

                    ui.horizontal(|ui| {
                        let vu_w = DRUMS_CARD_W - 30.0;
                        let (vu_rect, _) =
                            ui.allocate_exact_size(Vec2::new(vu_w, 6.0), Sense::hover());
                        let painter = ui.painter_at(vu_rect);
                        painter.rect_filled(vu_rect, 1.0, Color32::from_gray(20));
                        let fill_w = (dpeak.min(1.0) * vu_w).max(0.0);
                        if fill_w > 0.5 {
                            let fill = Rect::from_min_size(vu_rect.min, Vec2::new(fill_w, 6.0));
                            let bar_col = if drum_muted {
                                Color32::from_gray(40)
                            } else if dpeak > 0.9 {
                                Color32::from_rgb(220, 80, 60)
                            } else {
                                drum_tint
                            };
                            painter.rect_filled(fill, 1.0, bar_col);
                        }

                        ui.add_space(4.0);

                        let m_col = if drum_muted {
                            Color32::from_rgb(220, 80, 60)
                        } else {
                            text_dis
                        };
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("M").size(8.0).color(m_col))
                                    .frame(drum_muted)
                                    .min_size(Vec2::new(16.0, 12.0)),
                            )
                            .on_hover_text("Mute drums")
                            .clicked()
                        {
                            self.drum_engine.muted.store(!drum_muted, Ordering::Relaxed);
                        }
                    });
                })
                .response;

            if drums_resp.interact(Sense::click()).clicked() {
                self.app_mode = AppMode::DrumMachine;
            }

            // ── Right-side controls: MIX▸ + keyboard split ──────────────────
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mix_col = if self.show_mixer { accent } else { text_dis };
                if ui
                    .add(
                        egui::Label::new(egui::RichText::new("MIX▸").size(10.0).color(mix_col))
                            .sense(Sense::click()),
                    )
                    .on_hover_text("Toggle mixer panel")
                    .clicked()
                {
                    self.show_mixer = !self.show_mixer;
                }

                ui.add_space(12.0);

                // SPLIT mini-editor for the focused track.
                if self.app_mode == AppMode::Live {
                    let t = self.focused_track;
                    let tint = TRACK_TINTS[t % TRACK_TINTS.len()];
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("SPLIT").size(8.0).color(text_dis));
                            ui.add_space(4.0);
                            self.ui_live_split_bar(ui, t, tint, text_dis);
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new("CH").size(8.0).color(text_dis));
                            let mut ch = self.track_midi_ch[t] as i32;
                            if ui
                                .add(
                                    egui::DragValue::new(&mut ch)
                                        .range(0..=16)
                                        .speed(1.0)
                                        .custom_formatter(|v, _| {
                                            if v == 0.0 {
                                                "All".to_string()
                                            } else {
                                                format!("{}", v as u8)
                                            }
                                        }),
                                )
                                .on_hover_text("MIDI channel (0 = all channels)")
                                .changed()
                            {
                                self.track_midi_ch[t] = ch as u8;
                            }
                        });
                    });
                }
            });
        });
    }

    // ── Keyboard split bar ───────────────────────────────────────────────────
    //
    // Draws a 128-note range bar showing all tracks' zones; drag handles on
    // the focused track's lo/hi boundaries.

    fn ui_live_split_bar(
        &mut self,
        ui: &mut egui::Ui,
        focused: usize,
        accent: Color32,
        text_dis: Color32,
    ) {
        const BAR_W: f32 = 200.0;
        const BAR_H: f32 = 10.0;
        const HANDLE_W: f32 = 5.0;

        let (bar_rect, _) = ui.allocate_exact_size(Vec2::new(BAR_W, BAR_H), Sense::hover());
        let painter = ui.painter_at(bar_rect);
        painter.rect_filled(bar_rect, 2.0, Color32::from_gray(22));

        // All tracks: semi-transparent zone.
        for t in 0..crate::audio::TRACK_COUNT {
            let lo = self.track_key_lo[t] as f32 / 127.0;
            let hi = self.track_key_hi[t] as f32 / 127.0;
            let x0 = bar_rect.min.x + lo * BAR_W;
            let x1 = bar_rect.min.x + hi * BAR_W;
            let c = TRACK_TINTS[t % TRACK_TINTS.len()];
            let col = if t == focused {
                c
            } else {
                Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 55)
            };
            painter.rect_filled(
                Rect::from_min_max(
                    egui::pos2(x0, bar_rect.min.y),
                    egui::pos2(x1, bar_rect.max.y),
                ),
                1.0,
                col,
            );
        }

        // Drag handles for focused track.
        for field_is_hi in [false, true] {
            let frac = if field_is_hi {
                self.track_key_hi[focused] as f32 / 127.0
            } else {
                self.track_key_lo[focused] as f32 / 127.0
            };
            let hx = bar_rect.min.x + frac * BAR_W;
            let handle_rect = Rect::from_min_size(
                egui::pos2(hx - HANDLE_W * 0.5, bar_rect.min.y - 2.0),
                Vec2::new(HANDLE_W, BAR_H + 4.0),
            );
            let resp = ui.interact(
                handle_rect,
                ui.id().with(("split_handle", focused, field_is_hi)),
                Sense::drag(),
            );
            if resp.dragged() {
                let delta = (resp.drag_delta().x / BAR_W * 127.0).round() as i32;
                if field_is_hi {
                    self.track_key_hi[focused] = (self.track_key_hi[focused] as i32 + delta)
                        .clamp(self.track_key_lo[focused] as i32, 127)
                        as u8;
                } else {
                    self.track_key_lo[focused] = (self.track_key_lo[focused] as i32 + delta)
                        .clamp(0, self.track_key_hi[focused] as i32)
                        as u8;
                }
            }
            painter.rect_filled(handle_rect, 1.0, accent);
        }

        // Note labels.
        ui.add_space(4.0);
        let lo = crate::ui::midi_note_full(self.track_key_lo[focused]);
        let hi = crate::ui::midi_note_full(self.track_key_hi[focused]);
        ui.label(
            egui::RichText::new(format!("{} – {}", lo, hi))
                .size(8.0)
                .color(text_dis),
        );
    }
}
