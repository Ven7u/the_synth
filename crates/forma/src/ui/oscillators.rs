use crate::ui::frame::SynthFrame;
use crate::SynthApp;
use eframe::egui;
use egui::{Color32, Pos2, RichText, Stroke, Vec2};

pub const WAVE_LABELS: &[&str] = &["Sin", "Saw", "Sqr", "Tri"];

impl SynthApp {
    pub fn ui_osc_panel(&mut self, ui: &mut egui::Ui, i: usize) {
        let sp_xs = self.theme.sp_xs;
        let sp_sm = self.theme.sp_sm;
        let is_osc1 = i == 0;
        let flip = is_osc1 && self.osc1_mod_view;

        // Back face gets a slightly different tint via a modified frame.
        let frame = if flip {
            SynthFrame::section(&self.theme).fill(self.theme.c(&self.theme.bg_sunken))
        } else {
            SynthFrame::section(&self.theme)
        };

        frame.show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            // ── Header ────────────────────────────────────────────────────
            let on = self.osc_enabled[i];
            ui.horizontal(|ui| {
                // Title
                let title = if flip {
                    format!("OSC {} · MOD", i + 1)
                } else {
                    format!("OSC {}", i + 1)
                };
                let title_col = if on {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.text_disabled)
                };
                if ui
                    .add(egui::SelectableLabel::new(
                        on,
                        RichText::new(title).size(11.0).italics().color(title_col),
                    ))
                    .on_hover_text("Toggle oscillator on/off")
                    .clicked()
                {
                    self.osc_enabled[i] = !on;
                    let vol = if self.osc_enabled[i] {
                        self.osc_vol[i]
                    } else {
                        0.0
                    };
                    self.engine.set_osc_vol(i as u8, vol);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // MOD / back flip button (OSC 1 only)
                    if is_osc1 {
                        let flip_label = if flip { "‹ back" } else { "mod ›" };
                        let flip_col = self.theme.c(&self.theme.text_secondary);
                        if ui
                            .add(
                                egui::Label::new(
                                    RichText::new(flip_label).size(10.0).color(flip_col),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_text(if flip {
                                "Back to main controls"
                            } else {
                                "Sync / FM / Ring mod"
                            })
                            .clicked()
                        {
                            self.osc1_mod_view = !self.osc1_mod_view;
                        }
                    }
                });
            });

            ui.add_space(sp_xs);

            if flip {
                self.ui_osc1_mod_back(ui);
            } else {
                self.ui_osc_front(ui, i);
            }
        });
    }

    // ── Front face (identical for all 3 OSCs) ────────────────────────────────

    fn ui_osc_front(&mut self, ui: &mut egui::Ui, i: usize) {
        let sp_xs = self.theme.sp_xs;
        let sp_sm = self.theme.sp_sm;
        let on = self.osc_enabled[i];

        ui.add_enabled_ui(on, |ui| {
            // ── Waveform chips ────────────────────────────────────────────
            let chip_w = (ui.available_width() - sp_xs * 3.0) / 4.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = sp_xs;
                for (w, &label) in WAVE_LABELS.iter().enumerate() {
                    let active = self.osc_wave[i] == w;
                    if ui
                        .add_sized(
                            [chip_w, 22.0],
                            egui::SelectableLabel::new(active, RichText::new(label).size(10.0)),
                        )
                        .clicked()
                    {
                        self.osc_wave[i] = w;
                        self.engine.set_osc_wave(i as u8, w as u8);
                    }
                }
            });

            ui.add_space(sp_sm);

            // ── Knob row 1: OCT · DET · PW ───────────────────────────────
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = sp_xs;

                // OCT — integer DragValue styled like a knob column
                ui.vertical(|ui| {
                    ui.set_width(44.0);
                    ui.add_space(4.0);
                    if ui
                        .add_sized(
                            [44.0, 32.0],
                            egui::DragValue::new(&mut self.osc_octave[i])
                                .range(-2..=2)
                                .prefix("Oct "),
                        )
                        .on_hover_text("Octave shift (−2 … +2)")
                        .changed()
                    {
                        self.update_freq_mult(i);
                    }
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("OCT")
                            .size(9.0)
                            .color(self.theme.c(&self.theme.text_secondary)),
                    );
                });

                // DET knob (±100 ¢)
                if super::widgets::knob(
                    ui,
                    &mut self.osc_detune[i],
                    -100.0..=100.0,
                    "DET",
                    &self.theme,
                    false,
                )
                .on_hover_text("Detune ±100 ¢. Shift+drag for fine control.")
                .changed()
                {
                    self.update_freq_mult(i);
                }

                // PW knob (only meaningful for square, but always shown for layout consistency)
                let pw_enabled = self.osc_wave[i] == 2;
                ui.add_enabled_ui(pw_enabled, |ui| {
                    if super::widgets::knob(
                        ui,
                        &mut self.osc_pulse_width[i],
                        0.01..=0.99,
                        "PW",
                        &self.theme,
                        false,
                    )
                    .on_hover_text("Pulse Width — duty cycle of the square wave.\n0.5 = symmetric square (hollow/woody).\nLower or higher = thin, nasal tone.\nModulate with LFO for classic PWM sweep.\nOnly active on Sqr waveform.")
                    .changed()
                    {
                        self.engine.set_osc_pulse_width(i as u8, self.osc_pulse_width[i]);
                    }
                });
            });

            ui.add_space(sp_xs);

            // ── Unison row ────────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = sp_xs;
                let uni_on = self.osc_unison_enabled[i];
                let uni_col = if uni_on {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.text_disabled)
                };
                if ui
                    .add_sized(
                        [36.0, 22.0],
                        egui::SelectableLabel::new(
                            uni_on,
                            RichText::new("UNI").size(10.0).color(uni_col),
                        ),
                    )
                    .on_hover_text("Stack detuned voices for a thick, wide sound")
                    .clicked()
                {
                    self.osc_unison_enabled[i] = !uni_on;
                    self.update_unison(i);
                }

                if uni_on {
                    let mut changed = false;
                    changed |= ui
                        .add_sized(
                            [36.0, 22.0],
                            egui::DragValue::new(&mut self.osc_unison_count[i])
                                .range(2..=5)
                                .prefix("×"),
                        )
                        .on_hover_text("Number of unison voices (2–5)")
                        .changed();
                    changed |= super::widgets::knob(
                        ui,
                        &mut self.osc_unison_spread[i],
                        0.0..=50.0,
                        "SPRD",
                        &self.theme,
                        false,
                    )
                    .on_hover_text("Total pitch spread across unison voices (cents)")
                    .changed();
                    if changed {
                        self.update_unison(i);
                    }
                }
            });

            ui.add_space(sp_sm);

            // ── Mini waveform preview ─────────────────────────────────────
            let notes_held = !self.piano_held_midi.is_empty()
                || self.seq.playing.load(std::sync::atomic::Ordering::Relaxed);
            let active = on && notes_held;

            let preview_h = 36.0;
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), preview_h),
                egui::Sense::hover(),
            );
            if ui.is_rect_visible(rect) {
                // Oscilloscope style: waveform is always stationary (triggered at zero crossing).
                // Only brightness changes to signal active/idle state.
                let line_color = if active {
                    self.theme.c(&self.theme.accent)
                } else {
                    self.theme.c(&self.theme.accent).linear_multiply(0.3)
                };
                draw_wave_preview(
                    ui.painter(),
                    rect,
                    self.osc_wave[i],
                    self.osc_pulse_width[i],
                    self.theme.c(&self.theme.scope_bg),
                    line_color,
                    self.theme.rounding_sm,
                );
            }
        });
    }

    // ── Back face: OSC 1 mod controls ────────────────────────────────────────

    fn ui_osc1_mod_back(&mut self, ui: &mut egui::Ui) {
        let sp_xs = self.theme.sp_xs;
        let sp_sm = self.theme.sp_sm;

        // SYNC
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = sp_xs;
            let on = self.hard_sync;
            let col = self
                .theme
                .active_with(on, &self.theme.accent_hard_sync.clone());
            if ui
                .add_sized(
                    [44.0, 22.0],
                    egui::SelectableLabel::new(on, RichText::new("SYNC").size(10.0).color(col)),
                )
                .on_hover_text("Hard Sync — OSC 1 resets OSC 2 phase each cycle")
                .clicked()
            {
                self.hard_sync = !on;
                self.engine.set_hard_sync_enabled(self.hard_sync);
            }
            ui.label(
                RichText::new("→ OSC 2")
                    .size(10.0)
                    .color(self.theme.c(&self.theme.text_disabled)),
            );
        });

        ui.add_space(sp_sm);

        // FM chip + depth slider
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = sp_xs;
            let on = self.fm_enabled;
            let col = self.theme.active_with(on, &self.theme.accent_fm.clone());
            if ui
                .add_sized(
                    [44.0, 22.0],
                    egui::SelectableLabel::new(on, RichText::new("FM").size(10.0).color(col)),
                )
                .on_hover_text("Frequency Modulation — OSC 2 modulates OSC 1 pitch at audio rate")
                .clicked()
            {
                self.fm_enabled = !on;
                self.engine
                    .set_fm_depth(if self.fm_enabled { self.fm_depth } else { 0.0 });
            }
            ui.add_enabled_ui(self.fm_enabled, |ui| {
                if ui
                    .add_sized(
                        [ui.available_width(), 22.0],
                        egui::Slider::new(&mut self.fm_depth, 0.0..=10.0).fixed_decimals(1),
                    )
                    .on_hover_text("FM depth — ~1 subtle, 3–5 bells, 8+ chaotic sidebands")
                    .changed()
                {
                    self.engine.set_fm_depth(self.fm_depth);
                }
            });
        });

        ui.add_space(sp_xs);

        // RING chip + depth slider
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = sp_xs;
            let on = self.ring_enabled;
            let col = self.theme.active_with(on, &self.theme.accent_ring.clone());
            if ui
                .add_sized(
                    [44.0, 22.0],
                    egui::SelectableLabel::new(on, RichText::new("RING").size(10.0).color(col)),
                )
                .on_hover_text("Ring Mod — OSC 1 × OSC 2: metallic, bell-like textures")
                .clicked()
            {
                self.ring_enabled = !on;
                self.engine.set_ring_depth(if self.ring_enabled {
                    self.ring_depth
                } else {
                    0.0
                });
            }
            ui.add_enabled_ui(self.ring_enabled, |ui| {
                if ui
                    .add_sized(
                        [ui.available_width(), 22.0],
                        egui::Slider::new(&mut self.ring_depth, 0.0..=2.0).fixed_decimals(2),
                    )
                    .on_hover_text("Ring mod depth — mute OSC 1 and 2 in mixer for pure ring mod")
                    .changed()
                {
                    self.engine.set_ring_depth(self.ring_depth);
                }
            });
        });
    }

    // ── Audio helpers ─────────────────────────────────────────────────────────

    pub fn update_freq_mult(&self, i: usize) {
        let oct = self.osc_octave[i] as f32;
        let cents = self.osc_detune[i];
        let mult = 2_f32.powf(oct + cents / 1200.0);
        self.engine.set_osc_freq_mult(i as u8, mult);
    }

    pub fn update_unison(&self, i: usize) {
        let count = self.osc_unison_count[i];
        let spread = self.osc_unison_spread[i];
        let osc = i as u8;

        if !self.osc_unison_enabled[i] || count <= 1 {
            for c in 0..5 {
                self.engine.set_osc_unison_detune(osc, c as u8, 1.0);
                self.engine
                    .set_osc_unison_vol(osc, c as u8, if c == 0 { 1.0 } else { 0.0 });
            }
            return;
        }

        let vol = 1.0 / count as f32;
        for c in 0..5 {
            if c < count {
                let t = if count > 1 {
                    c as f32 / (count - 1) as f32
                } else {
                    0.5
                };
                let cents = -spread * 0.5 + t * spread;
                let detune = 2_f32.powf(cents / 1200.0);
                self.engine.set_osc_unison_detune(osc, c as u8, detune);
                self.engine.set_osc_unison_vol(osc, c as u8, vol);
            } else {
                self.engine.set_osc_unison_detune(osc, c as u8, 1.0);
                self.engine.set_osc_unison_vol(osc, c as u8, 0.0);
            }
        }
    }

    pub fn ui_mixer_panel(&mut self, ui: &mut egui::Ui) {
        let sp_xs = self.theme.sp_xs;

        SynthFrame::section(&self.theme).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new("MIX")
                    .size(11.0)
                    .italics()
                    .color(self.theme.c(&self.theme.text_primary)),
            );
            ui.add_space(sp_xs);

            // Vertical faders for OSC 1/2/3 + Noise
            ui.horizontal(|ui| {
                for i in 0..3 {
                    ui.vertical(|ui| {
                        ui.set_width(36.0);
                        ui.label(RichText::new(format!("O{}", i + 1)).size(10.0).color(
                            if self.osc_enabled[i] {
                                self.theme.c(&self.theme.text_primary)
                            } else {
                                self.theme.c(&self.theme.text_disabled)
                            },
                        ));
                        if ui
                            .add_sized(
                                [20.0, 90.0],
                                egui::Slider::new(&mut self.osc_vol[i], 0.0..=1.0)
                                    .vertical()
                                    .fixed_decimals(2),
                            )
                            .on_hover_text(format!("OSC {} volume in the mix", i + 1))
                            .changed()
                            && self.osc_enabled[i]
                        {
                            self.engine.set_osc_vol(i as u8, self.osc_vol[i]);
                        }
                    });
                }

                ui.vertical(|ui| {
                    ui.set_width(36.0);
                    ui.label(
                        RichText::new("N")
                            .size(10.0)
                            .color(self.theme.c(&self.theme.text_secondary)),
                    );
                    let mut noise_vol = self.engine.noise_vol();
                    if ui
                        .add_sized(
                            [20.0, 90.0],
                            egui::Slider::new(&mut noise_vol, 0.0..=1.0)
                                .vertical()
                                .fixed_decimals(2),
                        )
                        .on_hover_text("White noise volume")
                        .changed()
                    {
                        self.engine.set_noise_vol(noise_vol);
                    }
                });
            });

            ui.add_space(sp_xs);
            ui.separator();
            ui.add_space(sp_xs);

            ui.horizontal(|ui| {
                let mut master = self.engine.master_volume();
                if super::widgets::knob(ui, &mut master, 0.0..=1.0, "MAST", &self.theme, false)
                    .on_hover_text("Master output volume — applied after all FX")
                    .changed()
                {
                    self.engine.set_master_volume(master);
                }
                let mut glide = self.engine.glide_time();
                if super::widgets::knob(ui, &mut glide, 0.0..=0.5, "GLIDE", &self.theme, false)
                    .on_hover_text("Pitch slide time between notes (seconds)")
                    .changed()
                {
                    self.engine.set_glide_time(glide);
                }
            });

            ui.add_space(sp_xs);

            ui.horizontal(|ui| {
                let lim_on = self.limiter_enabled;
                let lim_col = if lim_on {
                    self.theme.c(&self.theme.accent_limiter)
                } else {
                    self.theme.c(&self.theme.text_disabled)
                };
                if ui
                    .add_sized(
                        [36.0, 22.0],
                        egui::SelectableLabel::new(
                            lim_on,
                            RichText::new("LIM").size(10.0).color(lim_col),
                        ),
                    )
                    .on_hover_text("Limiter — prevents output clipping")
                    .clicked()
                {
                    self.limiter_enabled = !lim_on;
                    self.engine.set_limiter_enabled(self.limiter_enabled);
                }
                ui.add_enabled_ui(lim_on, |ui| {
                    let mut thr = self.engine.limiter_threshold();
                    if ui
                        .add(
                            egui::DragValue::new(&mut thr)
                                .range(0.5..=1.0)
                                .speed(0.005)
                                .fixed_decimals(2),
                        )
                        .on_hover_text("Threshold — lower = more compression")
                        .changed()
                        && lim_on
                    {
                        self.engine.set_limiter_threshold(thr);
                    }
                });
            });
        });
    }
}

// ── Waveform preview painter ──────────────────────────────────────────────────

fn draw_wave_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    wave: usize,
    pulse_width: f32,
    bg: Color32,
    line_color: Color32,
    rounding: f32,
) {
    painter.rect_filled(rect, egui::CornerRadius::same(rounding as u8), bg);

    let w = rect.width();
    let h = rect.height();
    let cx = rect.left();
    let cy = rect.center().y;
    let amp = h * 0.38;
    let cycles = 2.0_f32;
    let steps = 80usize;

    let points: Vec<Pos2> = (0..=steps)
        .map(|s| {
            let t = s as f32 / steps as f32;
            let norm_phase = (t * cycles).fract();
            let phase_rad = t * cycles * std::f32::consts::TAU;

            let y = match wave {
                0 => phase_rad.sin(),
                1 => 1.0 - 2.0 * norm_phase,
                2 => {
                    if norm_phase < pulse_width {
                        1.0
                    } else {
                        -1.0
                    }
                }
                3 => {
                    if norm_phase < 0.5 {
                        4.0 * norm_phase - 1.0
                    } else {
                        3.0 - 4.0 * norm_phase
                    }
                }
                _ => 0.0,
            };

            Pos2::new(cx + t * w, cy - y * amp)
        })
        .collect();

    let clip = painter.clip_rect();
    let painter = painter.with_clip_rect(clip.intersect(rect));
    for pair in points.windows(2) {
        painter.line_segment([pair[0], pair[1]], Stroke::new(1.5, line_color));
    }
}
