use crate::SynthApp;
use eframe::egui;

impl SynthApp {
    pub fn ui_scene_browser(&mut self, ctx: &egui::Context) {
        if !self.scene_browser_open {
            return;
        }

        let accent = self.theme.c(&self.theme.accent);
        let text_pri = self.theme.c(&self.theme.text_primary);
        let text_sec = self.theme.c(&self.theme.text_secondary);
        let text_dis = self.theme.c(&self.theme.text_disabled);
        let bg_surface = self.theme.c(&self.theme.bg_surface);
        let border = self.theme.c(&self.theme.border);

        let mut open = self.scene_browser_open;
        egui::Window::new("Scenes")
            .open(&mut open)
            .resizable(true)
            .default_width(320.0)
            .default_height(400.0)
            .frame(
                egui::Frame::new()
                    .fill(bg_surface)
                    .stroke(egui::Stroke::new(1.0, border))
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                // ── Save section ──────────────────────────────────────────────
                ui.label(egui::RichText::new("SAVE SCENE").size(10.0).color(accent));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.scene_name)
                            .desired_width(180.0)
                            .hint_text("Scene name…"),
                    );
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new("SAVE").size(11.0).color(accent))
                                .min_size(egui::Vec2::new(50.0, 0.0)),
                        )
                        .on_hover_text("Save current rig state as a scene")
                        .clicked()
                    {
                        let s = self.capture_scene();
                        // Replace if name matches, otherwise append.
                        if let Some(idx) = self.scene_library.iter().position(|x| x.name == s.name)
                        {
                            self.scene_library[idx] = s;
                        } else {
                            self.scene_library.push(s);
                        }
                        crate::scene::save_scenes(&self.scene_library);
                    }
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                // ── Scene list ────────────────────────────────────────────────
                if self.scene_library.is_empty() {
                    ui.label(
                        egui::RichText::new("No scenes saved yet.")
                            .size(11.0)
                            .color(text_dis),
                    );
                } else {
                    ui.label(egui::RichText::new("SCENES").size(10.0).color(text_sec));
                    ui.add_space(4.0);

                    egui::ScrollArea::vertical()
                        .id_salt("scene_list")
                        .max_height(180.0)
                        .show(ui, |ui| {
                            let mut to_delete: Option<usize> = None;
                            let mut to_load: Option<usize> = None;
                            let mut to_chain: Option<usize> = None;

                            for (i, scene) in self.scene_library.iter().enumerate() {
                                let in_chain = self.scene_chain.contains(&i);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(&scene.name).size(11.0).color(text_pri),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        egui::RichText::new("✕")
                                                            .size(10.0)
                                                            .color(text_dis),
                                                    )
                                                    .frame(false),
                                                )
                                                .on_hover_text("Delete scene")
                                                .clicked()
                                            {
                                                to_delete = Some(i);
                                            }
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        egui::RichText::new("LOAD")
                                                            .size(10.0)
                                                            .color(accent),
                                                    )
                                                    .min_size(egui::Vec2::new(40.0, 0.0)),
                                                )
                                                .on_hover_text("Load this scene")
                                                .clicked()
                                            {
                                                to_load = Some(i);
                                            }
                                            let chain_col =
                                                if in_chain { accent } else { text_dis };
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        egui::RichText::new("+CHAIN")
                                                            .size(9.0)
                                                            .color(chain_col),
                                                    )
                                                    .frame(false),
                                                )
                                                .on_hover_text("Add to scene chain")
                                                .clicked()
                                            {
                                                to_chain = Some(i);
                                            }
                                        },
                                    );
                                });
                                ui.add_space(2.0);
                            }

                            if let Some(i) = to_delete {
                                // Remove from chain too if present.
                                self.scene_chain.retain(|&x| x != i);
                                self.scene_chain.iter_mut().for_each(|x| {
                                    if *x > i {
                                        *x -= 1;
                                    }
                                });
                                self.scene_library.remove(i);
                                crate::scene::save_scenes(&self.scene_library);
                            }
                            if let Some(i) = to_load {
                                let s = self.scene_library[i].clone();
                                self.load_scene(s);
                            }
                            if let Some(i) = to_chain {
                                self.scene_chain.push(i);
                            }
                        });
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                // ── Scene chain ───────────────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("CHAIN").size(10.0).color(accent));
                    ui.add_space(8.0);

                    // Bars-per-step control.
                    ui.label(egui::RichText::new("Bars/step:").size(9.0).color(text_dis));
                    let mut bars = self.scene_chain_bars as i32;
                    if ui
                        .add(egui::DragValue::new(&mut bars).range(1..=64).speed(1.0))
                        .changed()
                    {
                        self.scene_chain_bars = bars as u32;
                    }

                    ui.add_space(8.0);

                    // Play / Stop.
                    let (play_label, play_col) = if self.scene_chain_active {
                        ("■ STOP", egui::Color32::from_rgb(220, 80, 60))
                    } else {
                        ("▶ PLAY", accent)
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(play_label).size(10.0).color(play_col),
                            )
                            .min_size(egui::Vec2::new(60.0, 0.0)),
                        )
                        .clicked()
                    {
                        self.scene_chain_active = !self.scene_chain_active;
                        if self.scene_chain_active {
                            self.scene_chain_pos = 0;
                            self.scene_chain_elapsed_s = 0.0;
                            // Load first scene immediately.
                            if let Some(&idx) = self.scene_chain.first() {
                                if idx < self.scene_library.len() {
                                    let s = self.scene_library[idx].clone();
                                    self.load_scene(s);
                                }
                            }
                        }
                    }
                });

                if self.scene_chain.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Add scenes above with +CHAIN.")
                            .size(10.0)
                            .color(text_dis),
                    );
                } else {
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .id_salt("chain_list")
                        .max_height(120.0)
                        .show(ui, |ui| {
                            let mut to_remove: Option<usize> = None;
                            let mut swap_up: Option<usize> = None;

                            for (step, &scene_idx) in self.scene_chain.iter().enumerate() {
                                let is_current =
                                    self.scene_chain_active && step == self.scene_chain_pos;
                                let name = self
                                    .scene_library
                                    .get(scene_idx)
                                    .map(|s| s.name.as_str())
                                    .unwrap_or("(deleted)");
                                let row_col = if is_current { accent } else { text_sec };

                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{}  {}. {}",
                                            if is_current { "▶" } else { "  " },
                                            step + 1,
                                            name,
                                        ))
                                        .size(10.0)
                                        .color(row_col),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        egui::RichText::new("✕")
                                                            .size(9.0)
                                                            .color(text_dis),
                                                    )
                                                    .frame(false),
                                                )
                                                .clicked()
                                            {
                                                to_remove = Some(step);
                                            }
                                            if step > 0 {
                                                if ui
                                                    .add(
                                                        egui::Button::new(
                                                            egui::RichText::new("↑")
                                                                .size(9.0)
                                                                .color(text_dis),
                                                        )
                                                        .frame(false),
                                                    )
                                                    .clicked()
                                                {
                                                    swap_up = Some(step);
                                                }
                                            }
                                        },
                                    );
                                });
                            }

                            if let Some(i) = to_remove {
                                self.scene_chain.remove(i);
                                if self.scene_chain_pos >= self.scene_chain.len() {
                                    self.scene_chain_pos = self.scene_chain.len().saturating_sub(1);
                                }
                            }
                            if let Some(i) = swap_up {
                                self.scene_chain.swap(i - 1, i);
                            }
                        });
                }
            });

        self.scene_browser_open = open;
    }
}
