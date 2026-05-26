use crate::audio::{DRUM_DEFAULT_NOISE_MIX, DRUM_STEP_COUNT};
use crate::SynthApp;
use eframe::egui;
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use serde_json;

// ── Constants ────────────────────────────────────────────────────────────────

pub const STEP_COUNT: usize = DRUM_STEP_COUNT;
pub const CHANNEL_COUNT: usize = 8;

pub const CHANNEL_NAMES: [&str; CHANNEL_COUNT] = [
    "KICK", "SNARE", "HAT", "CLAP", "TOM1", "TOM2", "PERC", "NOISE",
];

// ── Drum machine state (UI-owned) ────────────────────────────────────────────

pub const PATTERN_COUNT: usize = 4;
pub const PATTERN_NAMES: [&str; PATTERN_COUNT] = ["A", "B", "C", "D"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrumMachineState {
    pub enabled: bool,
    /// 4 independent step patterns; active one is `active_pattern`.
    pub patterns: [[[bool; STEP_COUNT]; CHANNEL_COUNT]; PATTERN_COUNT],
    /// Per-step velocity (0–127) mirroring `patterns` layout.
    pub step_vel: [[[u8; STEP_COUNT]; CHANNEL_COUNT]; PATTERN_COUNT],
    pub active_pattern: usize,
    #[serde(skip)]
    pub pattern_clipboard: Option<[[bool; STEP_COUNT]; CHANNEL_COUNT]>,
    pub muted: [bool; CHANNEL_COUNT],
    pub soloed: [bool; CHANNEL_COUNT],
    pub channel_volume: [f32; CHANNEL_COUNT],
    pub base_freq: [f32; CHANNEL_COUNT],   // Hz
    pub pitch_range: [f32; CHANNEL_COUNT], // Hz of sweep
    pub amp_decay: [f32; CHANNEL_COUNT],   // seconds
    pub noise_mix: [f32; CHANNEL_COUNT],   // 0.0–1.0
    // Euclidean generator per lane (applies to active pattern)
    pub euclid_on: [bool; CHANNEL_COUNT],
    pub euclid_hits: [u8; CHANNEL_COUNT],   // 1–16
    pub euclid_steps: [u8; CHANNEL_COUNT],  // 1–16
    pub euclid_offset: [u8; CHANNEL_COUNT], // 0–15
    /// Which channel's voice editor is currently expanded (None = collapsed all).
    #[serde(skip)]
    pub expanded_channel: Option<usize>,
    #[serde(skip)]
    pub current_step: usize, // playhead, driven by clock in future phases
    pub swing: f32, // 0.0–0.75
}

impl Default for DrumMachineState {
    fn default() -> Self {
        let mut patterns = [[[false; STEP_COUNT]; CHANNEL_COUNT]; PATTERN_COUNT];
        // Seed a basic four-on-floor in slot A so the grid isn't empty on first open.
        let a = &mut patterns[0];
        a[0][0] = true;
        a[0][4] = true;
        a[0][8] = true;
        a[0][12] = true;
        a[1][4] = true;
        a[1][12] = true;
        for i in 0..STEP_COUNT {
            if i % 2 == 0 {
                a[2][i] = true;
            }
        }
        Self {
            enabled: false,
            patterns,
            step_vel: [[[100u8; STEP_COUNT]; CHANNEL_COUNT]; PATTERN_COUNT],
            active_pattern: 0,
            pattern_clipboard: None,
            muted: [false; CHANNEL_COUNT],
            soloed: [false; CHANNEL_COUNT],
            channel_volume: [0.8; CHANNEL_COUNT],
            base_freq: [55.0, 180.0, 0.0, 0.0, 120.0, 75.0, 350.0, 0.0],
            pitch_range: [150.0, 0.0, 0.0, 0.0, 80.0, 60.0, 200.0, 0.0],
            amp_decay: [0.25, 0.12, 0.045, 0.09, 0.20, 0.26, 0.06, 0.35],
            noise_mix: DRUM_DEFAULT_NOISE_MIX,
            euclid_on: [false; CHANNEL_COUNT],
            euclid_hits: [4; CHANNEL_COUNT],
            euclid_steps: [16; CHANNEL_COUNT],
            euclid_offset: [0; CHANNEL_COUNT],
            expanded_channel: None,
            current_step: 0,
            swing: 0.0,
        }
    }
}

// ── Kit preset ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrumKit {
    pub name: String,
    pub channel_volume: [f32; CHANNEL_COUNT],
    pub base_freq: [f32; CHANNEL_COUNT],
    pub pitch_range: [f32; CHANNEL_COUNT],
    pub amp_decay: [f32; CHANNEL_COUNT],
    pub noise_mix: [f32; CHANNEL_COUNT],
    pub patterns: [[[bool; STEP_COUNT]; CHANNEL_COUNT]; PATTERN_COUNT],
    pub step_vel: [[[u8; STEP_COUNT]; CHANNEL_COUNT]; PATTERN_COUNT],
    pub swing: f32,
}

impl DrumKit {
    pub fn from_state(name: impl Into<String>, s: &DrumMachineState) -> Self {
        Self {
            name: name.into(),
            channel_volume: s.channel_volume,
            base_freq: s.base_freq,
            pitch_range: s.pitch_range,
            amp_decay: s.amp_decay,
            noise_mix: s.noise_mix,
            patterns: s.patterns,
            step_vel: s.step_vel,
            swing: s.swing,
        }
    }

    pub fn apply_to_state(&self, s: &mut DrumMachineState) {
        s.channel_volume = self.channel_volume;
        s.base_freq = self.base_freq;
        s.pitch_range = self.pitch_range;
        s.amp_decay = self.amp_decay;
        s.noise_mix = self.noise_mix;
        s.patterns = self.patterns;
        s.step_vel = self.step_vel;
        s.swing = self.swing;
        s.active_pattern = 0;
    }
}

/// A small set of factory kits bundled with the app.
pub fn factory_kits() -> Vec<DrumKit> {
    let empty = [[[false; STEP_COUNT]; CHANNEL_COUNT]; PATTERN_COUNT];
    let flat_vel = [[[100u8; STEP_COUNT]; CHANNEL_COUNT]; PATTERN_COUNT];

    // ── Four-on-Floor ──────────────────────────────────────────────────────────
    let mut fof_pats = empty;
    let a = &mut fof_pats[0];
    a[0][0] = true;
    a[0][4] = true;
    a[0][8] = true;
    a[0][12] = true;
    a[1][4] = true;
    a[1][12] = true;
    for i in 0..16 {
        if i % 2 == 0 {
            a[2][i] = true;
        }
    }
    let fof = DrumKit {
        name: "Four-on-Floor".into(),
        channel_volume: [0.9, 0.8, 0.6, 0.7, 0.75, 0.75, 0.7, 0.5],
        base_freq: [55.0, 180.0, 0.0, 0.0, 120.0, 75.0, 350.0, 0.0],
        pitch_range: [150.0, 0.0, 0.0, 0.0, 80.0, 60.0, 200.0, 0.0],
        amp_decay: [0.25, 0.12, 0.045, 0.09, 0.20, 0.26, 0.06, 0.35],
        noise_mix: DRUM_DEFAULT_NOISE_MIX,
        patterns: fof_pats,
        step_vel: flat_vel,
        swing: 0.0,
    };

    // ── Breakbeat ──────────────────────────────────────────────────────────────
    let mut bb_pats = empty;
    let a = &mut bb_pats[0];
    // kick
    a[0][0] = true;
    a[0][6] = true;
    a[0][9] = true;
    a[0][14] = true;
    // snare
    a[1][4] = true;
    a[1][12] = true;
    a[1][14] = true;
    // hat (all 16)
    for i in 0..16 {
        a[2][i] = true;
    }
    // clap on 4 & 12
    a[3][4] = true;
    a[3][12] = true;
    let bb = DrumKit {
        name: "Breakbeat".into(),
        channel_volume: [0.9, 0.8, 0.5, 0.7, 0.75, 0.75, 0.7, 0.5],
        base_freq: [55.0, 180.0, 0.0, 0.0, 120.0, 75.0, 350.0, 0.0],
        pitch_range: [150.0, 0.0, 0.0, 0.0, 80.0, 60.0, 200.0, 0.0],
        amp_decay: [0.22, 0.10, 0.035, 0.09, 0.20, 0.26, 0.06, 0.35],
        noise_mix: DRUM_DEFAULT_NOISE_MIX,
        patterns: bb_pats,
        step_vel: flat_vel,
        swing: 0.05,
    };

    // ── Minimal ────────────────────────────────────────────────────────────────
    let mut min_pats = empty;
    let a = &mut min_pats[0];
    a[0][0] = true;
    a[0][8] = true;
    a[1][4] = true;
    a[1][12] = true;
    a[2][2] = true;
    a[2][6] = true;
    a[2][10] = true;
    a[2][14] = true;
    let minimal = DrumKit {
        name: "Minimal".into(),
        channel_volume: [0.85, 0.75, 0.5, 0.6, 0.7, 0.7, 0.65, 0.4],
        base_freq: [50.0, 160.0, 0.0, 0.0, 110.0, 70.0, 320.0, 0.0],
        pitch_range: [130.0, 0.0, 0.0, 0.0, 70.0, 50.0, 180.0, 0.0],
        amp_decay: [0.30, 0.14, 0.05, 0.09, 0.22, 0.28, 0.07, 0.35],
        noise_mix: DRUM_DEFAULT_NOISE_MIX,
        patterns: min_pats,
        step_vel: flat_vel,
        swing: 0.0,
    };

    vec![fof, bb, minimal]
}

// ── Euclidean rhythm ─────────────────────────────────────────────────────────

/// Bresenham/Bjorklund: distribute `hits` evenly across `steps`, then rotate by `offset`.
fn euclidean_pattern(hits: usize, steps: usize, offset: usize) -> [bool; STEP_COUNT] {
    let mut out = [false; STEP_COUNT];
    if steps == 0 || hits == 0 {
        return out;
    }
    let hits = hits.min(steps).min(STEP_COUNT);
    let steps = steps.min(STEP_COUNT);
    for i in 0..steps {
        if (i * hits) % steps < hits {
            out[(i + offset) % steps] = true;
        }
    }
    out
}

// ── UI ───────────────────────────────────────────────────────────────────────

impl SynthApp {
    /// Renders the drum machine view inside an already-open central panel ui.
    pub fn ui_drum_machine(&mut self, ui: &mut egui::Ui) {
        // Apply euclidean generator for any lanes that have it enabled.
        let pat = self.drums.active_pattern;
        for ch in 0..CHANNEL_COUNT {
            if self.drums.euclid_on[ch] {
                let hits = self.drums.euclid_hits[ch] as usize;
                let steps = self.drums.euclid_steps[ch] as usize;
                let offset = self.drums.euclid_offset[ch] as usize;
                self.drums.patterns[pat][ch] = euclidean_pattern(hits, steps, offset);
            }
        }

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
                for (i, name) in PATTERN_NAMES.iter().enumerate() {
                    let is_active = self.drums.active_pattern == i;
                    let col = if is_active { accent } else { text_sec };
                    let btn = egui::Button::new(egui::RichText::new(*name).small().color(col))
                        .fill(if is_active {
                            accent.gamma_multiply(0.2)
                        } else {
                            egui::Color32::TRANSPARENT
                        });
                    if ui
                        .add(btn)
                        .on_hover_text(format!("Switch to pattern {name}"))
                        .clicked()
                    {
                        self.drums.active_pattern = i;
                    }
                }
                ui.separator();
                if ui
                    .small_button(egui::RichText::new("Copy").color(text_sec))
                    .on_hover_text("Copy active pattern to clipboard")
                    .clicked()
                {
                    self.drums.pattern_clipboard =
                        Some(self.drums.patterns[self.drums.active_pattern]);
                }
                if ui
                    .add_enabled(
                        self.drums.pattern_clipboard.is_some(),
                        egui::Button::new(egui::RichText::new("Paste").small().color(text_sec)),
                    )
                    .on_hover_text("Paste clipboard into active pattern")
                    .clicked()
                {
                    if let Some(clip) = self.drums.pattern_clipboard {
                        self.drums.patterns[self.drums.active_pattern] = clip;
                    }
                }
                if ui
                    .small_button(egui::RichText::new("Clear").color(text_dis))
                    .on_hover_text("Clear all steps in active pattern")
                    .clicked()
                {
                    self.drums.patterns[self.drums.active_pattern] =
                        [[false; STEP_COUNT]; CHANNEL_COUNT];
                }

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

                ui.separator();
                let kit_col = if self.show_kit_browser {
                    accent
                } else {
                    text_sec
                };
                let kits_btn = egui::Button::new(
                    egui::RichText::new("KITS").small().color(kit_col),
                )
                .fill(if self.show_kit_browser {
                    accent.gamma_multiply(0.2)
                } else {
                    egui::Color32::TRANSPARENT
                });
                if ui.add(kits_btn).on_hover_text("Open kit browser").clicked() {
                    self.show_kit_browser = !self.show_kit_browser;
                }
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
                let soloed_any = self.drums.soloed.iter().any(|&s| s);
                let effectively_muted = muted || (soloed_any && !self.drums.soloed[ch]);
                let expanded = self.drums.expanded_channel == Some(ch);

                ui.horizontal(|ui| {
                    // Channel name — click to expand voice editor
                    let name_col = if effectively_muted {
                        text_dis
                    } else if self.drums.soloed[ch] {
                        Color32::from_rgb(255, 200, 50)
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
                        let active = self.drums.patterns[self.drums.active_pattern][ch][step];
                        let is_playhead = step == playhead && self.drums.enabled;

                        let beat_group = step % 4 == 0;
                        let fill = if is_playhead && active {
                            Color32::WHITE
                        } else if is_playhead {
                            Color32::from_gray(80)
                        } else if active {
                            if effectively_muted {
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

                        let pat = self.drums.active_pattern;
                        let vel = self.drums.step_vel[pat][ch][step];

                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(26.0, 24.0), Sense::click_and_drag());
                        let painter = ui.painter_at(rect);
                        let inner = rect.shrink(2.0);
                        painter.rect_filled(inner, CornerRadius::same(3), fill);
                        if beat_group {
                            painter.rect_stroke(
                                inner,
                                CornerRadius::same(3),
                                Stroke::new(0.5, stroke_col),
                                egui::StrokeKind::Outside,
                            );
                        }
                        // Velocity bar — bottom portion of active step.
                        if active {
                            let bar_h = (vel as f32 / 127.0) * (inner.height() - 2.0);
                            let bar = Rect::from_min_max(
                                Pos2::new(inner.left() + 1.0, inner.bottom() - bar_h),
                                Pos2::new(inner.right() - 1.0, inner.bottom()),
                            );
                            let vel_col = Color32::from_rgba_premultiplied(255, 255, 255, 60);
                            painter.rect_filled(bar, CornerRadius::same(2), vel_col);
                        }
                        if resp.clicked() {
                            self.drums.patterns[pat][ch][step] = !active;
                        }
                        // Drag up/down on an active step adjusts its velocity.
                        if resp.dragged() && active {
                            let delta = resp.drag_delta().y;
                            let v = &mut self.drums.step_vel[pat][ch][step];
                            *v = (*v as f32 - delta * 2.0).clamp(1.0, 127.0) as u8;
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
                        .on_hover_text("Mute this lane")
                        .clicked()
                    {
                        self.drums.muted[ch] = !muted;
                    }

                    // Solo button
                    let soloed = self.drums.soloed[ch];
                    let s_col = if soloed {
                        Color32::from_rgb(255, 200, 50)
                    } else {
                        text_dis
                    };
                    let s_btn = egui::Button::new(egui::RichText::new("S").size(9.0).color(s_col))
                        .fill(if soloed {
                            Color32::from_rgba_premultiplied(255, 200, 50, 40)
                        } else {
                            egui::Color32::TRANSPARENT
                        });
                    if ui
                        .add(s_btn)
                        .on_hover_text("Solo — mutes all other lanes")
                        .clicked()
                    {
                        self.drums.soloed[ch] = !soloed;
                    }

                    // Reverse button
                    if ui
                        .button(egui::RichText::new("⇄").size(9.0).color(text_dis))
                        .on_hover_text("Reverse this lane's pattern")
                        .clicked()
                    {
                        let pat = self.drums.active_pattern;
                        self.drums.patterns[pat][ch].reverse();
                        self.drums.step_vel[pat][ch].reverse();
                    }

                    // Randomize button
                    if ui
                        .button(egui::RichText::new("?").size(9.0).color(text_dis))
                        .on_hover_text("Randomize this lane's steps (50% density)")
                        .clicked()
                    {
                        let pat = self.drums.active_pattern;
                        // Simple LCG seeded from current time for cheap randomness without pulling rand crate.
                        let seed = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.subsec_nanos())
                            .unwrap_or(12345)
                            .wrapping_add(ch as u32 * 2891336453u32);
                        let mut rng = seed;
                        for step in 0..STEP_COUNT {
                            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            self.drums.patterns[pat][ch][step] = (rng >> 31) != 0;
                        }
                    }

                    // Euclidean toggle button
                    let euclid_on = self.drums.euclid_on[ch];
                    let e_col = if euclid_on { accent } else { text_dis };
                    let e_btn = egui::Button::new(egui::RichText::new("E").size(9.0).color(e_col))
                        .fill(if euclid_on {
                            accent.gamma_multiply(0.2)
                        } else {
                            egui::Color32::TRANSPARENT
                        });
                    if ui
                        .add(e_btn)
                        .on_hover_text("Toggle euclidean rhythm generator for this lane")
                        .clicked()
                    {
                        self.drums.euclid_on[ch] = !euclid_on;
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
                            ui.horizontal(|ui| {
                                // Freq
                                ui.group(|ui| {
                                    ui.set_min_width(70.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new("Freq (Hz)")
                                                .size(9.0)
                                                .color(text_sec),
                                        );
                                        ui.add(
                                            egui::DragValue::new(&mut self.drums.base_freq[ch])
                                                .range(0.0..=800.0)
                                                .speed(1.0)
                                                .fixed_decimals(0),
                                        );
                                    });
                                });
                                // Sweep
                                ui.group(|ui| {
                                    ui.set_min_width(70.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new("Sweep (Hz)")
                                                .size(9.0)
                                                .color(text_sec),
                                        );
                                        ui.add(
                                            egui::DragValue::new(&mut self.drums.pitch_range[ch])
                                                .range(0.0..=500.0)
                                                .speed(1.0)
                                                .fixed_decimals(0),
                                        );
                                    });
                                });
                                // Decay
                                ui.group(|ui| {
                                    ui.set_min_width(70.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new("Decay (s)")
                                                .size(9.0)
                                                .color(text_sec),
                                        );
                                        ui.add(
                                            egui::DragValue::new(&mut self.drums.amp_decay[ch])
                                                .range(0.01..=2.0)
                                                .speed(0.005)
                                                .fixed_decimals(3),
                                        );
                                    });
                                });
                                // Noise mix
                                ui.group(|ui| {
                                    ui.set_min_width(70.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new("Noise").size(9.0).color(text_sec),
                                        );
                                        ui.add(
                                            egui::DragValue::new(&mut self.drums.noise_mix[ch])
                                                .range(0.0..=1.0)
                                                .speed(0.01)
                                                .fixed_decimals(2),
                                        );
                                    });
                                });
                                // Volume
                                ui.group(|ui| {
                                    ui.set_min_width(70.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new("Volume").size(9.0).color(text_sec),
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
                            });
                            // Euclidean controls
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                let euclid_on = self.drums.euclid_on[ch];
                                let e_label = if euclid_on {
                                    "Euclidean ON"
                                } else {
                                    "Euclidean OFF"
                                };
                                let e_col = if euclid_on { accent } else { text_dis };
                                let e_btn = egui::Button::new(
                                    egui::RichText::new(e_label).size(9.0).color(e_col),
                                )
                                .fill(if euclid_on {
                                    accent.gamma_multiply(0.2)
                                } else {
                                    egui::Color32::TRANSPARENT
                                });
                                if ui
                                    .add(e_btn)
                                    .on_hover_text("Toggle euclidean rhythm generator")
                                    .clicked()
                                {
                                    self.drums.euclid_on[ch] = !euclid_on;
                                }
                                ui.add_enabled_ui(self.drums.euclid_on[ch], |ui| {
                                    ui.group(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new("Hits")
                                                    .size(9.0)
                                                    .color(text_sec),
                                            );
                                            let mut hits = self.drums.euclid_hits[ch] as u32;
                                            let steps = self.drums.euclid_steps[ch] as u32;
                                            if ui
                                                .add(
                                                    egui::DragValue::new(&mut hits)
                                                        .range(1..=steps)
                                                        .speed(0.1),
                                                )
                                                .changed()
                                            {
                                                self.drums.euclid_hits[ch] = hits as u8;
                                            }
                                        });
                                    });
                                    ui.group(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new("Steps")
                                                    .size(9.0)
                                                    .color(text_sec),
                                            );
                                            let mut steps = self.drums.euclid_steps[ch] as u32;
                                            if ui
                                                .add(
                                                    egui::DragValue::new(&mut steps)
                                                        .range(1..=16u32)
                                                        .speed(0.1),
                                                )
                                                .changed()
                                            {
                                                self.drums.euclid_steps[ch] = steps as u8;
                                                // Clamp hits to new steps count.
                                                self.drums.euclid_hits[ch] =
                                                    self.drums.euclid_hits[ch].min(steps as u8);
                                            }
                                        });
                                    });
                                    ui.group(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new("Offset")
                                                    .size(9.0)
                                                    .color(text_sec),
                                            );
                                            let mut offset = self.drums.euclid_offset[ch] as u32;
                                            let steps = self.drums.euclid_steps[ch] as u32;
                                            if ui
                                                .add(
                                                    egui::DragValue::new(&mut offset)
                                                        .range(0..=steps.saturating_sub(1))
                                                        .speed(0.1),
                                                )
                                                .changed()
                                            {
                                                self.drums.euclid_offset[ch] = offset as u8;
                                            }
                                        });
                                    });
                                });
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
                        "♩ {} BPM  ·  16 steps  ·  Pattern {}",
                        self.global_bpm, PATTERN_NAMES[self.drums.active_pattern]
                    ))
                    .size(10.0)
                    .color(text_dis),
                );
            });

            let _ = (accent, text_sec, text_dis, bg_surface, border);
        }
    }

    /// Floating kit browser window — call every frame from `ui_drum_machine`.
    pub fn ui_kit_browser(&mut self, ctx: &egui::Context) {
        if !self.show_kit_browser {
            return;
        }
        let accent = self.theme.c(&self.theme.accent);
        let text_sec = self.theme.c(&self.theme.text_secondary);
        let text_dis = self.theme.c(&self.theme.text_disabled);
        let bg = self.theme.c(&self.theme.bg_surface);

        let mut open = self.show_kit_browser;
        egui::Window::new("Kit Library")
            .open(&mut open)
            .resizable(true)
            .default_size([280.0, 400.0])
            .show(ctx, |ui| {
                // ── Save current kit ──────────────────────────────────────
                ui.label(
                    egui::RichText::new("Save current kit")
                        .small()
                        .color(text_sec),
                );
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.drum_kit_name)
                            .desired_width(140.0)
                            .hint_text("Kit name"),
                    );
                    if ui.button("Save").on_hover_text("Add to library").clicked() {
                        let kit = DrumKit::from_state(&self.drum_kit_name, &self.drums);
                        self.drum_kit_library.push(kit);
                    }
                    if ui
                        .button("Export…")
                        .on_hover_text("Save kit to a JSON file")
                        .clicked()
                    {
                        let kit = DrumKit::from_state(&self.drum_kit_name, &self.drums);
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name(&format!("{}.drumkit.json", self.drum_kit_name))
                            .add_filter("Drum Kit", &["json"])
                            .save_file()
                        {
                            if let Ok(json) = serde_json::to_string_pretty(&kit) {
                                let _ = std::fs::write(path, json);
                            }
                        }
                    }
                });

                ui.separator();

                // ── Import from file ──────────────────────────────────────
                if ui
                    .button("Import kit from file…")
                    .on_hover_text("Load a .drumkit.json file")
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Drum Kit", &["json"])
                        .pick_file()
                    {
                        if let Ok(data) = std::fs::read_to_string(&path) {
                            if let Ok(kit) = serde_json::from_str::<DrumKit>(&data) {
                                self.drum_kit_library.push(kit);
                            }
                        }
                    }
                }

                ui.separator();
                ui.label(egui::RichText::new("Library").small().color(text_sec));
                ui.add_space(2.0);

                // ── Kit list ──────────────────────────────────────────────
                let row_h = 22.0;
                let avail = ui.available_height();
                egui::ScrollArea::vertical()
                    .max_height(avail)
                    .show(ui, |ui| {
                        let mut to_delete: Option<usize> = None;
                        for (idx, kit) in self.drum_kit_library.iter().enumerate() {
                            let name = kit.name.clone();
                            ui.horizontal(|ui| {
                                let resp = ui.add_sized(
                                    [180.0, row_h],
                                    egui::Button::new(egui::RichText::new(&name).color(accent))
                                        .fill(bg),
                                );
                                if resp.on_hover_text("Click to load this kit").clicked() {
                                    kit.apply_to_state(&mut self.drums);
                                    self.drum_kit_name = name.clone();
                                }
                                if ui
                                    .small_button(egui::RichText::new("✕").color(text_dis))
                                    .on_hover_text("Remove from library")
                                    .clicked()
                                {
                                    to_delete = Some(idx);
                                }
                            });
                        }
                        if let Some(i) = to_delete {
                            self.drum_kit_library.remove(i);
                        }
                    });
            });
        self.show_kit_browser = open;
    }
}
