use crate::SynthApp;
use crate::patch::Patch;
use eframe::egui;
use egui::Color32;

impl SynthApp {
    pub fn ui_patch_bar(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("PATCH").strong().small());

        // Patch name
        ui.add(egui::TextEdit::singleline(&mut self.patch_name).desired_width(120.0))
            .on_hover_text("Patch name. Used as filename when saving.");

        // Save to file
        if ui.button("💾 Save").on_hover_text("Save current patch to a JSON file.").clicked() {
            let p = self.capture_patch();
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(&format!("{}.json", p.name))
                .add_filter("Patch", &["json"])
                .save_file()
            {
                if let Ok(json) = serde_json::to_string_pretty(&p) {
                    let _ = std::fs::write(path, json);
                }
            }
        }

        // Load from file
        if ui.button("📂 Load").on_hover_text("Load a patch from a JSON file.").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Patch", &["json"])
                .pick_file()
            {
                if let Ok(json) = std::fs::read_to_string(path) {
                    if let Ok(p) = serde_json::from_str::<Patch>(&json) {
                        self.apply_patch(p);
                    }
                }
            }
        }

        // Library browser toggle
        let browser_label = egui::RichText::new("📚 Library")
            .color(if self.patch_browser_open { Color32::from_rgb(0, 220, 160) } else { Color32::WHITE });
        if ui.button(browser_label).on_hover_text("Browse and load factory patches organized by category and synth model.").clicked() {
            self.patch_browser_open = !self.patch_browser_open;
        }
    }

    pub fn ui_patch_browser(&mut self, ctx: &egui::Context) {
        if !self.patch_browser_open { return; }

        let mut open = self.patch_browser_open;
        egui::Window::new("Patch Library")
            .open(&mut open)
            .resizable(true)
            .default_size([440.0, 520.0])
            .show(ctx, |ui| {
                // --- Category filter ---
                let categories: Vec<String> = {
                    let mut cats = vec!["All".to_string()];
                    let mut seen = std::collections::HashSet::new();
                    for p in &self.patch_library {
                        if seen.insert(p.category.clone()) {
                            cats.push(p.category.clone());
                        }
                    }
                    cats
                };
                ui.label(egui::RichText::new("Category").weak().small());
                ui.horizontal_wrapped(|ui| {
                    for cat in &categories {
                        let active = &self.patch_browser_category == cat;
                        let label = egui::RichText::new(cat)
                            .color(if active { Color32::from_rgb(0, 220, 160) } else { Color32::GRAY });
                        if ui.button(label).clicked() {
                            self.patch_browser_category = cat.clone();
                        }
                    }
                });

                ui.add_space(4.0);

                // --- Synth model filter ---
                let models: Vec<String> = {
                    let cat_filter = &self.patch_browser_category;
                    let mut models = vec!["All".to_string()];
                    let mut seen = std::collections::HashSet::new();
                    for p in &self.patch_library {
                        if cat_filter != "All" && &p.category != cat_filter { continue; }
                        let m = if p.synth_model.is_empty() { "Original".to_string() } else { p.synth_model.clone() };
                        if seen.insert(m.clone()) {
                            models.push(m);
                        }
                    }
                    models
                };
                if !models.contains(&self.patch_browser_model) {
                    self.patch_browser_model = "All".into();
                }
                ui.label(egui::RichText::new("Synth").weak().small());
                ui.horizontal_wrapped(|ui| {
                    for m in &models {
                        let active = &self.patch_browser_model == m;
                        let label = egui::RichText::new(m)
                            .color(if active { Color32::from_rgb(100, 180, 255) } else { Color32::GRAY });
                        if ui.button(label).clicked() {
                            self.patch_browser_model = m.clone();
                        }
                    }
                });

                ui.separator();

                // --- Load FX toggle ---
                ui.horizontal(|ui| {
                    let label = egui::RichText::new("Load FX with patch")
                        .small()
                        .color(if self.patch_load_fx { Color32::from_rgb(255, 180, 60) } else { Color32::GRAY });
                    ui.checkbox(&mut self.patch_load_fx, label)
                        .on_hover_text("When enabled, loading a patch also restores its FX chain settings.\nWhen disabled, your current FX chain stays untouched.");
                });

                ui.separator();

                // --- Patch list ---
                let cat_filter  = self.patch_browser_category.clone();
                let model_filter = self.patch_browser_model.clone();
                let patches: Vec<(usize, String, String, String)> = self.patch_library.iter().enumerate()
                    .filter(|(_, p)| {
                        let cat_ok   = cat_filter == "All"   || p.category == cat_filter;
                        let model_display = if p.synth_model.is_empty() { "Original" } else { &p.synth_model };
                        let model_ok = model_filter == "All" || model_display == model_filter;
                        cat_ok && model_ok
                    })
                    .map(|(i, p)| {
                        let m = if p.synth_model.is_empty() { "Original".to_string() } else { p.synth_model.clone() };
                        (i, p.name.clone(), p.category.clone(), m)
                    })
                    .collect();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut load_idx: Option<usize> = None;
                    for (idx, name, cat, model) in &patches {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("[{cat}]")).weak().small().monospace());
                            ui.label(egui::RichText::new(format!("{model}")).color(Color32::from_rgb(100, 180, 255)).small().monospace());
                            if ui.selectable_label(false, name).clicked() {
                                load_idx = Some(*idx);
                            }
                        });
                    }
                    if let Some(idx) = load_idx {
                        let p = self.patch_library[idx].clone();
                        self.apply_patch(p);
                        self.patch_browser_open = false;
                    }
                });
            });
        self.patch_browser_open = open;
    }
}
