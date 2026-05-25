use crate::patch::Patch;
use crate::SynthApp;
use eframe::egui;
use egui::Color32;

// ---------------------------------------------------------------------------
// Tag groups shown in the browser filter panel
// ---------------------------------------------------------------------------

const INSPIRED_BY_TAGS: &[&str] = &["eno", "pink-floyd", "frahm", "zimmer", "glass", "wagner"];

const CHARACTER_TAGS: &[&str] = &[
    "warm",
    "dark",
    "bright",
    "cold",
    "lush",
    "raw",
    "soft",
    "aggressive",
    "evolving",
    "drone",
    "pulsing",
    "rhythmic",
    "glitchy",
];

const TIMBRE_TAGS: &[&str] = &[
    "analog",
    "digital",
    "fm",
    "bell",
    "choir",
    "strings",
    "noise",
    "plucked",
    "long-release",
    "short-attack",
];

const GENRE_TAGS: &[&str] = &["electronic", "synthwave", "rock", "cinematic"];

// ---------------------------------------------------------------------------

impl SynthApp {
    pub fn ui_patch_bar(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("PATCH").strong().small());

        ui.add(egui::TextEdit::singleline(&mut self.patch_name).desired_width(120.0))
            .on_hover_text("Patch name. Used as filename when saving.");

        if ui
            .button("SAVE")
            .on_hover_text("Save current patch to a JSON file.")
            .clicked()
        {
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

        if ui
            .button("LOAD")
            .on_hover_text("Load a patch from a JSON file.")
            .clicked()
        {
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

        let browser_label = egui::RichText::new("LIBRARY").color(if self.patch_browser_open {
            self.theme.c(&self.theme.accent)
        } else {
            Color32::WHITE
        });
        if ui
            .button(browser_label)
            .on_hover_text("Browse factory patches by category, tags, and favourites.")
            .clicked()
        {
            self.patch_browser_open = !self.patch_browser_open;
        }
    }

    pub fn ui_patch_browser(&mut self, ctx: &egui::Context) {
        if !self.patch_browser_open {
            return;
        }

        let mut open = self.patch_browser_open;
        egui::Window::new("Patch Library")
            .open(&mut open)
            .resizable(true)
            .default_size([520.0, 640.0])
            .show(ctx, |ui| {
                self.patch_browser_inner(ui);
            });
        self.patch_browser_open = open;
    }

    fn patch_browser_inner(&mut self, ui: &mut egui::Ui) {
        let accent = self.theme.c(&self.theme.accent);
        let text_dim = self.theme.c(&self.theme.text_secondary);

        // ── Top bar: search + FX toggle ───────────────────────────────────
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🔍").size(13.0));
            ui.add(
                egui::TextEdit::singleline(&mut self.patch_search)
                    .hint_text("Search patches…")
                    .desired_width(200.0),
            );
            if !self.patch_search.is_empty() && ui.small_button("✕").clicked() {
                self.patch_search.clear();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let fx_label =
                    egui::RichText::new("Load FX")
                        .small()
                        .color(if self.patch_load_fx {
                            accent
                        } else {
                            Color32::GRAY
                        });
                ui.checkbox(&mut self.patch_load_fx, fx_label)
                    .on_hover_text("When on, loading a patch also restores its FX chain.");
                if !self.patch_active_tags.is_empty() {
                    if ui
                        .small_button(
                            egui::RichText::new("Clear filters")
                                .color(Color32::from_rgb(200, 100, 70)),
                        )
                        .clicked()
                    {
                        self.patch_active_tags.clear();
                        self.patch_browser_category = "All".into();
                    }
                }
            });
        });

        ui.separator();

        // ── Category chips ────────────────────────────────────────────────
        let categories: Vec<String> = {
            let mut cats = vec!["All".to_string()];
            let mut seen = std::collections::HashSet::new();
            for p in &self.patch_library {
                if seen.insert(p.category.clone()) {
                    cats.push(p.category.clone());
                }
            }
            cats.sort_by(|a, b| {
                if a == "All" {
                    std::cmp::Ordering::Less
                } else if b == "All" {
                    std::cmp::Ordering::Greater
                } else {
                    a.cmp(b)
                }
            });
            cats
        };

        ui.horizontal_wrapped(|ui| {
            for cat in &categories {
                let active = &self.patch_browser_category == cat;
                let col = if active { accent } else { text_dim };
                let label = egui::RichText::new(cat).small().color(col);
                if ui.selectable_label(active, label).clicked() {
                    self.patch_browser_category = cat.clone();
                }
            }
        });

        ui.add_space(4.0);

        // ── Tag filter groups ─────────────────────────────────────────────
        let all_used_tags: std::collections::HashSet<String> = self
            .patch_library
            .iter()
            .flat_map(|p| p.tags.iter().cloned())
            .collect();

        let mut tag_group = |ui: &mut egui::Ui, label: &str, tags: &[&str]| {
            let relevant: Vec<&str> = tags
                .iter()
                .copied()
                .filter(|t| all_used_tags.contains(*t))
                .collect();
            if relevant.is_empty() {
                return;
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(label).weak().small().monospace());
                for &tag in &relevant {
                    let active = self.patch_active_tags.contains(tag);
                    let col = if active { accent } else { text_dim };
                    let label_text = egui::RichText::new(tag).size(12.0).color(col);
                    if ui.selectable_label(active, label_text).clicked() {
                        if active {
                            self.patch_active_tags.remove(tag);
                        } else {
                            self.patch_active_tags.insert(tag.to_string());
                        }
                    }
                }
            });
        };

        tag_group(ui, "Inspired By │", INSPIRED_BY_TAGS);
        tag_group(ui, "Character   │", CHARACTER_TAGS);
        tag_group(ui, "Timbre      │", TIMBRE_TAGS);
        tag_group(ui, "Genre       │", GENRE_TAGS);

        ui.separator();

        // ── Build filtered list ───────────────────────────────────────────
        let search_lc = self.patch_search.to_lowercase();
        let cat_filter = &self.patch_browser_category;
        let tag_filter = &self.patch_active_tags;

        let filtered: Vec<usize> = self
            .patch_library
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                let cat_ok = cat_filter == "All" || &p.category == cat_filter;
                let search_ok = search_lc.is_empty()
                    || p.name.to_lowercase().contains(&search_lc)
                    || p.category.to_lowercase().contains(&search_lc)
                    || p.tags.iter().any(|t| t.contains(&search_lc));
                let tag_ok = tag_filter.is_empty() || tag_filter.iter().all(|t| p.tags.contains(t));
                cat_ok && search_ok && tag_ok
            })
            .map(|(i, _)| i)
            .collect();

        // Count label
        let total = self.patch_library.len();
        let showing = filtered.len();
        ui.label(
            egui::RichText::new(format!("{showing} / {total} patches"))
                .weak()
                .small(),
        );

        ui.separator();

        // ── Patch list ────────────────────────────────────────────────────
        let mut load_idx: Option<usize> = None;
        let mut toggle_fav: Option<String> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            // ── Favourites section ────────────────────────────────────────
            let fav_indices: Vec<usize> = filtered
                .iter()
                .copied()
                .filter(|&i| self.patch_favorites.contains(&self.patch_library[i].name))
                .collect();

            if !fav_indices.is_empty() {
                ui.label(
                    egui::RichText::new("★  Favourites")
                        .small()
                        .color(Color32::from_rgb(220, 185, 60)),
                );
                for i in &fav_indices {
                    if let Some(action) =
                        Self::patch_row(ui, &self.patch_library[*i], true, accent, text_dim)
                    {
                        match action {
                            PatchAction::Load => load_idx = Some(*i),
                            PatchAction::ToggleFav(name) => toggle_fav = Some(name),
                        }
                    }
                }
                ui.separator();
            }

            // ── Recent section ────────────────────────────────────────────
            let recent_indices: Vec<usize> = self
                .patch_recent
                .iter()
                .filter_map(|name| {
                    filtered
                        .iter()
                        .copied()
                        .find(|&i| &self.patch_library[i].name == name)
                })
                .take(6)
                .collect();

            if !recent_indices.is_empty() {
                ui.label(egui::RichText::new("⏱  Recent").small().color(text_dim));
                for i in &recent_indices {
                    let is_fav = self.patch_favorites.contains(&self.patch_library[*i].name);
                    if let Some(action) =
                        Self::patch_row(ui, &self.patch_library[*i], is_fav, accent, text_dim)
                    {
                        match action {
                            PatchAction::Load => load_idx = Some(*i),
                            PatchAction::ToggleFav(name) => toggle_fav = Some(name),
                        }
                    }
                }
                ui.separator();
            }

            // ── All filtered ──────────────────────────────────────────────
            for i in &filtered {
                let is_fav = self.patch_favorites.contains(&self.patch_library[*i].name);
                if let Some(action) =
                    Self::patch_row(ui, &self.patch_library[*i], is_fav, accent, text_dim)
                {
                    match action {
                        PatchAction::Load => load_idx = Some(*i),
                        PatchAction::ToggleFav(name) => toggle_fav = Some(name),
                    }
                }
            }
        });

        // Apply deferred actions
        if let Some(name) = toggle_fav {
            if self.patch_favorites.contains(&name) {
                self.patch_favorites.remove(&name);
            } else {
                self.patch_favorites.insert(name);
            }
        }
        if let Some(idx) = load_idx {
            let p = self.patch_library[idx].clone();
            self.apply_patch(p);
            self.patch_browser_open = false;
        }
    }

    /// Render one patch row; returns Some(action) if something was clicked.
    fn patch_row(
        ui: &mut egui::Ui,
        patch: &Patch,
        is_fav: bool,
        accent: Color32,
        text_dim: Color32,
    ) -> Option<PatchAction> {
        let mut action = None;

        ui.horizontal(|ui| {
            // Favourite star
            let star = if is_fav { "★" } else { "☆" };
            let star_col = if is_fav {
                Color32::from_rgb(220, 185, 60)
            } else {
                Color32::from_gray(70)
            };
            if ui
                .add(
                    egui::Button::new(egui::RichText::new(star).color(star_col).size(11.0))
                        .frame(false),
                )
                .clicked()
            {
                action = Some(PatchAction::ToggleFav(patch.name.clone()));
            }

            // Category chip
            ui.label(
                egui::RichText::new(format!("[{}]", patch.category))
                    .weak()
                    .small()
                    .monospace()
                    .color(text_dim),
            );

            // Name (clickable → load)
            if ui
                .selectable_label(false, egui::RichText::new(&patch.name).small())
                .clicked()
            {
                action = Some(PatchAction::Load);
            }

            // Tags (truncated)
            if !patch.tags.is_empty() {
                let tag_str = patch.tags.join(" · ");
                ui.label(
                    egui::RichText::new(tag_str)
                        .weak()
                        .size(12.0)
                        .color(Color32::from_gray(110))
                        .monospace(),
                );
            }
        });

        action
    }
}

enum PatchAction {
    Load,
    ToggleFav(String),
}
