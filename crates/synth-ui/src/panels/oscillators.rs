use egui::{Color32, Pos2, RichText, Stroke, Vec2};

use crate::frame::SynthFrame;
use crate::param_writer::ParamWriter;
use crate::state::SynthUiState;
use crate::theme::SynthTheme;

pub const WAVE_LABELS: &[&str] = &["Sin", "Saw", "Sqr", "Tri"];

/// Main OSC card (front + back flip for OSC 1 mod section).
/// `notes_active` — true when at least one voice is sounding (lights waveform preview).
pub fn ui_osc_panel(
    ui: &mut egui::Ui,
    s: &mut SynthUiState,
    pw: &mut impl ParamWriter,
    theme: &SynthTheme,
    i: usize,
    notes_active: bool,
) {
    let sp_xs = theme.sp_xs;
    let is_osc1 = i == 0;
    let flip = is_osc1 && s.osc1_mod_view;

    let frame = if flip {
        SynthFrame::section(theme).fill(theme.c(&theme.bg_sunken))
    } else {
        SynthFrame::section(theme)
    };

    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());

        // Header
        let on = s.osc_enabled[i];
        ui.horizontal(|ui| {
            let title = if flip {
                format!("OSC {} · MOD", i + 1)
            } else {
                format!("OSC {}", i + 1)
            };
            let title_col = if on {
                theme.c(&theme.accent)
            } else {
                theme.c(&theme.text_disabled)
            };
            if ui
                .add(egui::SelectableLabel::new(
                    on,
                    RichText::new(title).size(11.0).italics().color(title_col),
                ))
                .on_hover_text("Toggle oscillator on/off")
                .clicked()
            {
                s.osc_enabled[i] = !on;
                let vol = if s.osc_enabled[i] { s.osc_vol[i] } else { 0.0 };
                pw.set_osc_vol(i as u8, vol);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if is_osc1 {
                    let flip_label = if flip { "‹ back" } else { "mod ›" };
                    let flip_col = theme.c(&theme.text_secondary);
                    if ui
                        .add(
                            egui::Label::new(RichText::new(flip_label).size(10.0).color(flip_col))
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text(if flip {
                            "Back to main controls"
                        } else {
                            "Sync / FM / Ring mod"
                        })
                        .clicked()
                    {
                        s.osc1_mod_view = !s.osc1_mod_view;
                    }
                }
            });
        });

        ui.add_space(sp_xs);

        if flip {
            ui_osc1_mod_back(ui, s, pw, theme);
        } else {
            ui_osc_front(ui, s, pw, theme, i, notes_active);
        }
    });
}

fn ui_osc_front(
    ui: &mut egui::Ui,
    s: &mut SynthUiState,
    pw: &mut impl ParamWriter,
    theme: &SynthTheme,
    i: usize,
    notes_active: bool,
) {
    let sp_xs = theme.sp_xs;
    let sp_sm = theme.sp_sm;
    let on = s.osc_enabled[i];

    ui.add_enabled_ui(on, |ui| {
        // Waveform chips
        let chip_w = (ui.available_width() - sp_xs * 3.0) / 4.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = sp_xs;
            for (w, &label) in WAVE_LABELS.iter().enumerate() {
                let active = s.osc_wave[i] == w;
                if ui
                    .add_sized(
                        [chip_w, 22.0],
                        egui::SelectableLabel::new(active, RichText::new(label).size(10.0)),
                    )
                    .clicked()
                {
                    s.osc_wave[i] = w;
                    pw.set_osc_wave(i as u8, w as u8);
                }
            }
        });

        ui.add_space(sp_sm);

        // Knob row: OCT · DET · PW
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = sp_xs;

            ui.vertical(|ui| {
                ui.set_width(44.0);
                ui.add_space(4.0);
                if ui
                    .add_sized(
                        [44.0, 32.0],
                        egui::DragValue::new(&mut s.osc_octave[i])
                            .range(-2..=2)
                            .prefix("Oct "),
                    )
                    .on_hover_text("Octave shift (−2 … +2)")
                    .changed()
                {
                    update_freq_mult(s, pw, i);
                }
                ui.add_space(2.0);
                ui.label(
                    RichText::new("OCT")
                        .size(9.0)
                        .color(theme.c(&theme.text_secondary)),
                );
            });

            if crate::widgets::knob(
                ui,
                &mut s.osc_detune[i],
                -100.0..=100.0,
                "DET",
                theme,
                false,
            )
            .on_hover_text("Detune ±100 ¢. Shift+drag for fine control.")
            .changed()
            {
                update_freq_mult(s, pw, i);
            }

            let pw_enabled = s.osc_wave[i] == 2;
            ui.add_enabled_ui(pw_enabled, |ui| {
                if crate::widgets::knob(
                    ui,
                    &mut s.osc_pulse_width[i],
                    0.01..=0.99,
                    "PW",
                    theme,
                    false,
                )
                .on_hover_text("Pulse Width — duty cycle of the square wave.")
                .changed()
                {
                    pw.set_osc_pulse_width(i as u8, s.osc_pulse_width[i]);
                }
            });
        });

        ui.add_space(sp_xs);

        // Unison row
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = sp_xs;
            let uni_on = s.osc_unison_enabled[i];
            let uni_col = if uni_on {
                theme.c(&theme.accent)
            } else {
                theme.c(&theme.text_disabled)
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
                s.osc_unison_enabled[i] = !uni_on;
                update_unison(s, pw, i);
            }

            if uni_on {
                let mut changed = false;
                changed |= ui
                    .add_sized(
                        [36.0, 22.0],
                        egui::DragValue::new(&mut s.osc_unison_count[i])
                            .range(2..=5)
                            .prefix("×"),
                    )
                    .on_hover_text("Number of unison voices (2–5)")
                    .changed();
                changed |= crate::widgets::knob(
                    ui,
                    &mut s.osc_unison_spread[i],
                    0.0..=50.0,
                    "SPRD",
                    theme,
                    false,
                )
                .on_hover_text("Total pitch spread across unison voices (cents)")
                .changed();
                if changed {
                    update_unison(s, pw, i);
                }
            }
        });

        ui.add_space(sp_sm);

        // Mini waveform preview
        let active = on && notes_active;
        let preview_h = 36.0;
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), preview_h),
            egui::Sense::hover(),
        );
        if ui.is_rect_visible(rect) {
            let line_color = if active {
                theme.c(&theme.accent)
            } else {
                theme.c(&theme.accent).linear_multiply(0.3)
            };
            draw_wave_preview(
                ui.painter(),
                rect,
                s.osc_wave[i],
                s.osc_pulse_width[i],
                theme.c(&theme.scope_bg),
                line_color,
                theme.rounding_sm,
            );
        }
    });
}

fn ui_osc1_mod_back(
    ui: &mut egui::Ui,
    s: &mut SynthUiState,
    pw: &mut impl ParamWriter,
    theme: &SynthTheme,
) {
    let sp_xs = theme.sp_xs;
    let sp_sm = theme.sp_sm;

    // SYNC
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = sp_xs;
        let on = s.hard_sync;
        let col = theme.active_with(on, &theme.accent_hard_sync.clone());
        if ui
            .add_sized(
                [44.0, 22.0],
                egui::SelectableLabel::new(on, RichText::new("SYNC").size(10.0).color(col)),
            )
            .on_hover_text("Hard Sync — OSC 1 resets OSC 2 phase each cycle")
            .clicked()
        {
            s.hard_sync = !on;
            pw.set_hard_sync_enabled(s.hard_sync);
        }
        ui.label(
            RichText::new("→ OSC 2")
                .size(10.0)
                .color(theme.c(&theme.text_disabled)),
        );
    });

    ui.add_space(sp_sm);

    // FM chip + depth slider
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = sp_xs;
        let on = s.fm_enabled;
        let col = theme.active_with(on, &theme.accent_fm.clone());
        if ui
            .add_sized(
                [44.0, 22.0],
                egui::SelectableLabel::new(on, RichText::new("FM").size(10.0).color(col)),
            )
            .on_hover_text("Frequency Modulation — OSC 2 modulates OSC 1 pitch at audio rate")
            .clicked()
        {
            s.fm_enabled = !on;
            pw.set_fm_depth(if s.fm_enabled { s.fm_depth } else { 0.0 });
        }
        ui.add_enabled_ui(s.fm_enabled, |ui| {
            if ui
                .add_sized(
                    [ui.available_width(), 22.0],
                    egui::Slider::new(&mut s.fm_depth, 0.0..=10.0).fixed_decimals(1),
                )
                .on_hover_text("FM depth — ~1 subtle, 3–5 bells, 8+ chaotic sidebands")
                .changed()
            {
                pw.set_fm_depth(s.fm_depth);
            }
        });
    });

    ui.add_space(sp_xs);

    // RING chip + depth slider
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = sp_xs;
        let on = s.ring_enabled;
        let col = theme.active_with(on, &theme.accent_ring.clone());
        if ui
            .add_sized(
                [44.0, 22.0],
                egui::SelectableLabel::new(on, RichText::new("RING").size(10.0).color(col)),
            )
            .on_hover_text("Ring Mod — OSC 1 × OSC 2: metallic, bell-like textures")
            .clicked()
        {
            s.ring_enabled = !on;
            pw.set_ring_depth(if s.ring_enabled { s.ring_depth } else { 0.0 });
        }
        ui.add_enabled_ui(s.ring_enabled, |ui| {
            if ui
                .add_sized(
                    [ui.available_width(), 22.0],
                    egui::Slider::new(&mut s.ring_depth, 0.0..=2.0).fixed_decimals(2),
                )
                .on_hover_text("Ring mod depth — mute OSC 1 and 2 in mixer for pure ring mod")
                .changed()
            {
                pw.set_ring_depth(s.ring_depth);
            }
        });
    });
}

/// Mixer panel — OSC volumes, noise, master vol, glide, limiter.
pub fn ui_mixer_panel(
    ui: &mut egui::Ui,
    s: &mut SynthUiState,
    pw: &mut impl ParamWriter,
    theme: &SynthTheme,
) {
    let sp_xs = theme.sp_xs;

    SynthFrame::section(theme).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.label(
            RichText::new("MIX")
                .size(11.0)
                .italics()
                .color(theme.c(&theme.text_primary)),
        );
        ui.add_space(sp_xs);

        ui.horizontal(|ui| {
            for i in 0..3 {
                ui.vertical(|ui| {
                    ui.set_width(36.0);
                    ui.label(RichText::new(format!("O{}", i + 1)).size(10.0).color(
                        if s.osc_enabled[i] {
                            theme.c(&theme.text_primary)
                        } else {
                            theme.c(&theme.text_disabled)
                        },
                    ));
                    if ui
                        .add_sized(
                            [20.0, 90.0],
                            egui::Slider::new(&mut s.osc_vol[i], 0.0..=1.0)
                                .vertical()
                                .fixed_decimals(2),
                        )
                        .on_hover_text(format!("OSC {} volume in the mix", i + 1))
                        .changed()
                        && s.osc_enabled[i]
                    {
                        pw.set_osc_vol(i as u8, s.osc_vol[i]);
                    }
                });
            }

            ui.vertical(|ui| {
                ui.set_width(36.0);
                ui.label(
                    RichText::new("N")
                        .size(10.0)
                        .color(theme.c(&theme.text_secondary)),
                );
                if ui
                    .add_sized(
                        [20.0, 90.0],
                        egui::Slider::new(&mut s.noise_vol, 0.0..=1.0)
                            .vertical()
                            .fixed_decimals(2),
                    )
                    .on_hover_text("White noise volume")
                    .changed()
                {
                    pw.set_noise_vol(s.noise_vol);
                }
            });
        });

        ui.add_space(sp_xs);
        ui.separator();
        ui.add_space(sp_xs);

        ui.horizontal(|ui| {
            if crate::widgets::knob(ui, &mut s.master_vol, 0.0..=1.0, "MAST", theme, false)
                .on_hover_text("Master output volume — applied after all FX")
                .changed()
            {
                pw.set_master_volume(s.master_vol);
            }
            if crate::widgets::knob(ui, &mut s.glide_time, 0.0..=0.5, "GLIDE", theme, false)
                .on_hover_text("Pitch slide time between notes (seconds)")
                .changed()
            {
                pw.set_glide_time(s.glide_time);
            }
        });

        ui.add_space(sp_xs);

        ui.horizontal(|ui| {
            let lim_on = s.limiter_enabled;
            let lim_col = if lim_on {
                theme.c(&theme.accent_limiter)
            } else {
                theme.c(&theme.text_disabled)
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
                s.limiter_enabled = !lim_on;
                pw.set_limiter_enabled(s.limiter_enabled);
            }
            ui.add_enabled_ui(lim_on, |ui| {
                if ui
                    .add(
                        egui::DragValue::new(&mut s.limiter_threshold)
                            .range(0.5..=1.0)
                            .speed(0.005)
                            .fixed_decimals(2),
                    )
                    .on_hover_text("Threshold — lower = more compression")
                    .changed()
                    && lim_on
                {
                    pw.set_limiter_threshold(s.limiter_threshold);
                }
            });
        });
    });
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn update_freq_mult(s: &SynthUiState, pw: &mut impl ParamWriter, i: usize) {
    let oct = s.osc_octave[i] as f32;
    let cents = s.osc_detune[i];
    let mult = 2_f32.powf(oct + cents / 1200.0);
    pw.set_osc_freq_mult(i as u8, mult);
}

pub fn update_unison(s: &SynthUiState, pw: &mut impl ParamWriter, i: usize) {
    let count = s.osc_unison_count[i];
    let spread = s.osc_unison_spread[i];
    let osc = i as u8;

    if !s.osc_unison_enabled[i] || count <= 1 {
        for c in 0..5 {
            pw.set_osc_unison_detune(osc, c as u8, 1.0);
            pw.set_osc_unison_vol(osc, c as u8, if c == 0 { 1.0 } else { 0.0 });
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
            pw.set_osc_unison_detune(osc, c as u8, detune);
            pw.set_osc_unison_vol(osc, c as u8, vol);
        } else {
            pw.set_osc_unison_detune(osc, c as u8, 1.0);
            pw.set_osc_unison_vol(osc, c as u8, 0.0);
        }
    }
}

fn draw_wave_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    wave: usize,
    pulse_width: f32,
    bg: Color32,
    line_color: Color32,
    rounding: f32,
) {
    painter.rect_filled(rect, rounding, bg);

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
