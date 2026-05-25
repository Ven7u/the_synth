use crate::SynthApp;
use eframe::egui;
use egui::{Color32, CornerRadius, Sense, Stroke, Vec2};
use serde::{Deserialize, Serialize};

// ── Constants ────────────────────────────────────────────────────────────────

pub const STEP_COUNT: usize = 16;
pub const CHANNEL_COUNT: usize = 8;

pub const CHANNEL_NAMES: [&str; CHANNEL_COUNT] = [
    "KICK", "SNARE", "HAT", "CLAP", "TOM1", "TOM2", "PERC", "NOISE",
];

// ── Drum machine state (UI-owned) ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrumMachineState {
    pub enabled: bool,
    pub steps: [[bool; STEP_COUNT]; CHANNEL_COUNT],
    pub muted: [bool; CHANNEL_COUNT],
    pub channel_volume: [f32; CHANNEL_COUNT],
    /// Which channel's voice editor is currently expanded (None = collapsed all).
    #[serde(skip)]
    pub expanded_channel: Option<usize>,
    #[serde(skip)]
    pub current_step: usize, // playhead, driven by clock in future phases
    pub swing: f32, // 0.0–0.75
}

impl Default for DrumMachineState {
    fn default() -> Self {
        let mut steps = [[false; STEP_COUNT]; CHANNEL_COUNT];
        // Seed a basic four-on-floor pattern so the grid isn't empty on first open.
        steps[0][0] = true;
        steps[0][4] = true;
        steps[0][8] = true;
        steps[0][12] = true;
        steps[1][4] = true;
        steps[1][12] = true;
        for i in 0..STEP_COUNT {
            if i % 2 == 0 {
                steps[2][i] = true;
            }
        }
        Self {
            enabled: false,
            steps,
            muted: [false; CHANNEL_COUNT],
            channel_volume: [0.8; CHANNEL_COUNT],
            expanded_channel: None,
            current_step: 0,
            swing: 0.0,
        }
    }
}

// ── UI ───────────────────────────────────────────────────────────────────────

impl SynthApp {
    /// Renders the drum machine view inside an already-open central panel ui.
    pub fn ui_drum_machine(&mut self, ui: &mut egui::Ui) {
        let accent = self.theme.c(&self.theme.accent);
        let text_sec = self.theme.c(&self.theme.text_secondary);
        let text_dis = self.theme.c(&self.theme.text_disabled);
        let bg_surface = self.theme.c(&self.theme.bg_surface);
        let border = self.theme.c(&self.theme.border);

        {
            ui.add_space(8.0);

            // ── Header row ───────────────────────────────────────────────
            ui.horizontal(|ui| {
                let on = self.drums.enabled;
                let on_col = if on { accent } else { text_sec };
                let on_label = if on { "● ON" } else { "○ OFF" };
                if ui
                    .button(egui::RichText::new(on_label).color(on_col))
                    .clicked()
                {
                    self.drums.enabled = !on;
                }

                ui.separator();
                ui.label(egui::RichText::new("Pattern").small().color(text_sec));
                ui.label(egui::RichText::new("Four-on-Floor").small().color(accent));

                ui.separator();
                ui.label(egui::RichText::new("Div: 1/16").small().color(text_sec));

                ui.separator();
                ui.label(egui::RichText::new("Swing").small().color(text_sec));
                ui.add(
                    egui::DragValue::new(&mut self.drums.swing)
                        .range(0.0..=0.75)
                        .speed(0.005)
                        .fixed_decimals(2),
                );

                ui.separator();
                ui.label(egui::RichText::new("▶ RST").small().color(text_dis))
                    .on_hover_text("Playback and reset — coming in Phase 5");
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            // ── Step numbers header ───────────────────────────────────────
            ui.horizontal(|ui| {
                ui.add_space(70.0); // channel name column width
                for step in 0..STEP_COUNT {
                    let beat_marker = step % 4 == 0;
                    let col = if beat_marker { accent } else { text_dis };
                    ui.add_sized(
                        [28.0, 14.0],
                        egui::Label::new(
                            egui::RichText::new(format!("{}", step + 1))
                                .size(9.0)
                                .color(col),
                        ),
                    );
                }
            });

            ui.add_space(2.0);

            // ── Channel rows ─────────────────────────────────────────────
            let playhead = self.drums.current_step;

            for ch in 0..CHANNEL_COUNT {
                let ch_name = CHANNEL_NAMES[ch];
                let muted = self.drums.muted[ch];
                let expanded = self.drums.expanded_channel == Some(ch);

                ui.horizontal(|ui| {
                    // Channel name — click to expand voice editor
                    let name_col = if muted {
                        text_dis
                    } else if expanded {
                        accent
                    } else {
                        text_sec
                    };
                    if ui
                        .add_sized(
                            [64.0, 26.0],
                            egui::Button::new(
                                egui::RichText::new(ch_name).size(10.0).color(name_col),
                            )
                            .frame(expanded),
                        )
                        .on_hover_text("Click to open voice editor")
                        .clicked()
                    {
                        self.drums.expanded_channel = if expanded { None } else { Some(ch) };
                    }

                    // Step buttons
                    for step in 0..STEP_COUNT {
                        let active = self.drums.steps[ch][step];
                        let is_playhead = step == playhead && self.drums.enabled;

                        let beat_group = step % 4 == 0;
                        let fill = if is_playhead && active {
                            Color32::WHITE
                        } else if is_playhead {
                            Color32::from_gray(80)
                        } else if active {
                            if muted {
                                Color32::from_gray(60)
                            } else if beat_group {
                                accent
                            } else {
                                Color32::from_rgb(
                                    (accent.r() as u16 * 2 / 3) as u8,
                                    (accent.g() as u16 * 2 / 3) as u8,
                                    (accent.b() as u16 * 2 / 3) as u8,
                                )
                            }
                        } else {
                            Color32::from_gray(28)
                        };

                        let stroke_col = if beat_group {
                            border
                        } else {
                            Color32::TRANSPARENT
                        };

                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(26.0, 24.0), Sense::click());
                        let painter = ui.painter_at(rect);
                        painter.rect_filled(rect.shrink(2.0), CornerRadius::same(3), fill);
                        if beat_group {
                            painter.rect_stroke(
                                rect.shrink(2.0),
                                CornerRadius::same(3),
                                Stroke::new(0.5, stroke_col),
                                egui::StrokeKind::Outside,
                            );
                        }
                        if resp.clicked() {
                            self.drums.steps[ch][step] = !active;
                        }
                    }

                    ui.add_space(4.0);

                    // Mute button
                    let m_col = if muted {
                        Color32::from_rgb(220, 80, 80)
                    } else {
                        text_dis
                    };
                    if ui
                        .button(egui::RichText::new("M").size(9.0).color(m_col))
                        .clicked()
                    {
                        self.drums.muted[ch] = !muted;
                    }
                });

                // ── Voice editor (inline, expands below channel row) ─────
                if expanded {
                    egui::Frame::new()
                        .fill(bg_surface)
                        .corner_radius(egui::CornerRadius::same(4))
                        .inner_margin(egui::Margin::same(8))
                        .outer_margin(egui::Margin {
                            left: 70,
                            right: 0,
                            top: 2,
                            bottom: 4,
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{} — Voice Editor", ch_name))
                                        .size(10.0)
                                        .color(accent),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button(egui::RichText::new("✕").color(text_dis))
                                            .clicked()
                                        {
                                            self.drums.expanded_channel = None;
                                        }
                                    },
                                );
                            });
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(
                                    "Synthesis parameters — implemented in Phase 5",
                                )
                                .size(10.0)
                                .color(text_dis),
                            );
                            // Placeholder rows for voice params
                            ui.horizontal(|ui| {
                                for label in ["Freq", "Sweep", "Noise", "Attack", "Decay", "Filter"]
                                {
                                    ui.group(|ui| {
                                        ui.set_min_width(60.0);
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new(label)
                                                    .size(9.0)
                                                    .color(text_sec),
                                            );
                                            ui.add(
                                                egui::DragValue::new(
                                                    &mut self.drums.channel_volume[ch],
                                                )
                                                .range(0.0..=1.0)
                                                .speed(0.01)
                                                .fixed_decimals(2),
                                            );
                                        });
                                    });
                                }
                            });
                        });
                }
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            // ── Footer ───────────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "♩ {} BPM  ·  16 steps  ·  Phase 5: engine + per-voice synthesis",
                        self.global_bpm
                    ))
                    .size(10.0)
                    .color(text_dis),
                );
            });

            let _ = (accent, text_sec, text_dis, bg_surface, border);
        }
    }
}
