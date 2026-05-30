use crate::history;
use crate::SynthApp;
use eframe::egui;
use std::time::{SystemTime, UNIX_EPOCH};

impl SynthApp {
    pub fn ui_history_window(&mut self, ctx: &egui::Context) {
        if !self.history_open {
            return;
        }

        let accent = self.theme.c(&self.theme.accent);
        let text_sec = self.theme.c(&self.theme.text_secondary);
        let text_dis = self.theme.c(&self.theme.text_disabled);
        let bg = self.theme.c(&self.theme.bg_surface);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut open = self.history_open;
        egui::Window::new("Patch History")
            .open(&mut open)
            .resizable(true)
            .default_size([340.0, 480.0])
            .show(ctx, |ui| {
                // ── Pin current state ─────────────────────────────────────
                ui.label(
                    egui::RichText::new("Pin current state")
                        .small()
                        .strong()
                        .color(text_sec),
                );
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.history_pin_name)
                            .desired_width(180.0)
                            .hint_text("Label (optional)"),
                    );
                    if ui
                        .button(egui::RichText::new("● Pin").color(accent))
                        .on_hover_text("Save a named snapshot of the current sound")
                        .clicked()
                    {
                        let label = self.history_pin_name.trim().to_string();
                        self.pin_history(label);
                        self.history_pin_name.clear();
                    }
                });

                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{} snapshots  ·  {} manual pins  ·  auto-saves every 3 s of silence",
                        self.patch_history.entries.len(),
                        self.patch_history
                            .entries
                            .iter()
                            .filter(|e| e.is_manual())
                            .count(),
                    ))
                    .small()
                    .color(text_dis),
                );
                ui.separator();

                // ── Timeline list ─────────────────────────────────────────
                let avail = ui.available_height();
                let mut to_restore: Option<usize> = None;
                let mut to_delete: Option<usize> = None;
                let mut rename: Option<(usize, String)> = None;

                egui::ScrollArea::vertical()
                    .max_height(avail)
                    .show(ui, |ui| {
                        if self.patch_history.entries.is_empty() {
                            ui.label(
                                egui::RichText::new("No history yet — start tweaking!")
                                    .small()
                                    .color(text_dis),
                            );
                            return;
                        }

                        for (idx, entry) in self.patch_history.entries.iter().enumerate() {
                            let is_manual = entry.is_manual();
                            let dot = if is_manual { "●" } else { "○" };
                            let dot_col = if is_manual { accent } else { text_dis };
                            let age = entry.age_str(now);
                            let name = entry.label.as_deref().unwrap_or(&entry.patch.name);

                            egui::Frame::new()
                                .fill(bg)
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::same(6))
                                .outer_margin(egui::Margin::symmetric(0, 2))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(dot).color(dot_col).size(10.0),
                                        );
                                        let patch_btn = ui.add(
                                            egui::Button::new(
                                                egui::RichText::new(name)
                                                    .color(if is_manual {
                                                        accent
                                                    } else {
                                                        text_sec
                                                    })
                                                    .size(11.0),
                                            )
                                            .fill(egui::Color32::TRANSPARENT),
                                        );
                                        if patch_btn
                                            .on_hover_text(format!(
                                                "{} — {}",
                                                entry.patch.name, age
                                            ))
                                            .clicked()
                                        {
                                            to_restore = Some(idx);
                                        }

                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .small_button(
                                                        egui::RichText::new("✕").color(text_dis),
                                                    )
                                                    .on_hover_text("Delete this snapshot")
                                                    .clicked()
                                                {
                                                    to_delete = Some(idx);
                                                }
                                                if is_manual {
                                                    // Inline rename: click the age to start editing
                                                    ui.label(
                                                        egui::RichText::new(&age)
                                                            .small()
                                                            .color(text_dis),
                                                    );
                                                    if ui
                                                        .small_button(
                                                            egui::RichText::new("✎")
                                                                .color(text_dis),
                                                        )
                                                        .on_hover_text("Rename this pin")
                                                        .clicked()
                                                    {
                                                        rename = Some((
                                                            idx,
                                                            entry.label.clone().unwrap_or_default(),
                                                        ));
                                                    }
                                                } else {
                                                    ui.label(
                                                        egui::RichText::new(&age)
                                                            .small()
                                                            .color(text_dis),
                                                    );
                                                    if ui
                                                        .small_button(
                                                            egui::RichText::new("📌")
                                                                .color(text_dis),
                                                        )
                                                        .on_hover_text("Promote to manual pin")
                                                        .clicked()
                                                    {
                                                        rename =
                                                            Some((idx, entry.patch.name.clone()));
                                                    }
                                                }
                                            },
                                        );
                                    });
                                });
                        }
                    });

                // Apply deferred actions outside the borrow.
                if let Some(idx) = to_restore {
                    let p = self.patch_history.entries[idx].patch.clone();
                    self.apply_patch(p);
                }
                if let Some(idx) = to_delete {
                    self.patch_history.remove(idx);
                    history::save_history(&self.patch_history);
                }
                if let Some((idx, label)) = rename {
                    self.patch_history.rename(idx, label);
                    history::save_history(&self.patch_history);
                }
            });
        self.history_open = open;
    }
}
