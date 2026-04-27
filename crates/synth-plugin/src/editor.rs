use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, resizable_window::ResizableWindow, EguiState};
use std::sync::Arc;
use synth_engine::Patch;
use synth_ui::{
    midnight,
    panels::{
        draw_adsr_visualizer, ui_adsr_panel, ui_filter_panel, ui_fx_chain_vertical, ui_lfo2_panel,
        ui_lfo_panel, ui_mixer_panel, ui_osc_panel,
    },
    ParamWriter, SynthUiState,
};

use crate::params::TheSynthParams;
use crate::plugin_param_writer::PluginParamWriter;

pub(crate) fn default_state() -> Arc<EguiState> {
    EguiState::from_size(1300, 720)
}

// ---------------------------------------------------------------------------
// Patch loading
// ---------------------------------------------------------------------------

fn collect_patch_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if p.is_dir() {
            collect_patch_files(&p, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("json"))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
}

fn load_patches() -> Vec<Patch> {
    // compile-time dev path: always works when built from the workspace
    const DEV_ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/patches");

    let candidates = [
        std::path::PathBuf::from(DEV_ASSETS),
        std::path::PathBuf::from("assets/patches"),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("assets/patches")))
            .unwrap_or_default(),
    ];
    for dir in &candidates {
        if dir.is_dir() {
            let mut files = Vec::new();
            collect_patch_files(dir, &mut files);
            files.sort();
            let patches: Vec<Patch> = files
                .iter()
                .filter_map(|p| std::fs::read_to_string(p).ok())
                .filter_map(|s| serde_json::from_str(&s).ok())
                .collect();
            if !patches.is_empty() {
                return patches;
            }
        }
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// Param → state sync (runs each frame to pick up host automation)
// ---------------------------------------------------------------------------

fn sync_state_from_params(s: &mut SynthUiState, p: &TheSynthParams) {
    macro_rules! sync_osc {
        ($op:expr, $i:expr) => {
            s.osc_wave[$i] = $op.wave.value() as usize;
            s.osc_vol[$i] = $op.vol.value();
            s.osc_pulse_width[$i] = $op.pulse_width.value();
        };
    }
    sync_osc!(p.osc1, 0);
    sync_osc!(p.osc2, 1);
    sync_osc!(p.osc3, 2);

    s.hard_sync = p.hard_sync.value();
    s.fm_depth = p.fm_depth.value();
    s.ring_depth = p.ring_depth.value();
    s.noise_vol = p.noise_vol.value();

    s.filter_cutoff = p.filter.cutoff.value();
    s.filter_q = p.filter.resonance.value();
    s.filter_drive = p.filter.drive.value();
    s.filter_key_track = p.filter.key_track.value();
    s.filter_env_amount = p.filter.env_amount.value();
    s.fenv_attack = p.filter.attack.value();
    s.fenv_decay = p.filter.decay.value();
    s.fenv_sustain = p.filter.sustain.value();
    s.fenv_release = p.filter.release.value();

    s.lfo_rate = p.lfo1.rate.value();
    s.lfo_depth = p.lfo1.depth.value();
    s.lfo_shape = p.lfo1.shape.value() as usize;
    s.lfo_dest = p.lfo1.dest.value() as usize;

    s.lfo2_rate = p.lfo2.rate.value();
    s.lfo2_depth = p.lfo2.depth.value();
    s.lfo2_shape = p.lfo2.shape.value() as usize;
    s.lfo2_dest = p.lfo2.dest.value() as usize;

    s.amp_attack = p.amp.attack.value();
    s.amp_decay = p.amp.decay.value();
    s.amp_sustain = p.amp.sustain.value();
    s.amp_release = p.amp.release.value();
    s.glide_time = p.amp.glide_time.value();

    s.master_vol = p.master.master_vol.value();
    s.global_vol = p.master.global_vol.value();
    s.limiter_enabled = p.master.limiter_enabled.value();
    s.limiter_threshold = p.master.limiter_threshold.value();

    s.fx_overdrive_drive = p.fx.overdrive_drive.value();
    s.fx_overdrive_mix = p.fx.overdrive_mix.value();
    s.fx_overdrive_tone = p.fx.overdrive_tone.value();
    s.fx_overdrive_asym = p.fx.overdrive_asym.value();
    s.fx_distortion_drive = p.fx.distortion_drive.value();
    s.fx_distortion_mix = p.fx.distortion_mix.value();
    s.fx_distortion_tone = p.fx.distortion_tone.value();
    s.fx_distortion_pre = p.fx.distortion_pre.value();
    s.fx_chorus_rate = p.fx.chorus_rate.value();
    s.fx_chorus_depth = p.fx.chorus_depth.value();
    s.fx_chorus_mix = p.fx.chorus_mix.value();
    s.fx_delay_time = p.fx.delay_time.value();
    s.fx_delay_feedback = p.fx.delay_feedback.value();
    s.fx_delay_mix = p.fx.delay_mix.value();
    s.fx_reverb_size = p.fx.reverb_size.value();
    s.fx_reverb_damp = p.fx.reverb_damp.value();
    s.fx_reverb_mix = p.fx.reverb_mix.value();
    s.fx_reverb_predelay = p.fx.reverb_predelay.value();
    s.fx_reverb_type = p.fx.reverb_type.value() as u8;
    s.stereo_spread = p.fx.stereo_spread.value();
    s.stereo_width = p.fx.stereo_width.value();
    s.fx_shimmer_size = p.fx.shimmer_size.value();
    s.fx_shimmer_damp = p.fx.shimmer_damp.value();
    s.fx_shimmer_mix = p.fx.shimmer_mix.value();
    s.fx_shimmer_amt = p.fx.shimmer_amount.value();
    s.fx_shimmer_width = p.fx.shimmer_width.value();
    s.fx_shimmer_spread = p.fx.shimmer_spread.value();
    s.fx_crystal_grain_ms = p.fx.crystal_grain.value();
    s.fx_crystal_scatter = p.fx.crystal_scatter.value();
    s.fx_crystal_feedback = p.fx.crystal_feedback.value();
    s.fx_crystal_delay_ms = p.fx.crystal_delay.value();
    s.fx_crystal_mix = p.fx.crystal_mix.value();
}

// ---------------------------------------------------------------------------
// Patch apply
// ---------------------------------------------------------------------------

fn apply_patch(patch: &Patch, state: &mut SynthUiState, pw: &mut impl ParamWriter) {
    state.current_patch_name = patch.name.clone();

    macro_rules! apply_osc {
        ($op:expr, $i:expr) => {
            state.osc_wave[$i] = $op[$i];
            state.osc_vol[$i] = patch.osc_vol[$i];
            state.osc_pulse_width[$i] = patch.osc_pulse_width[$i];
            state.osc_octave[$i] = patch.osc_octave[$i];
            state.osc_detune[$i] = patch.osc_detune[$i];
            state.osc_enabled[$i] = patch.osc_enabled[$i];
            state.osc_unison_enabled[$i] = patch.osc_unison_enabled[$i];
            state.osc_unison_count[$i] = patch.osc_unison_count[$i];
            state.osc_unison_spread[$i] = patch.osc_unison_spread[$i];
            pw.set_osc_wave($i as u8, state.osc_wave[$i] as u8);
            pw.set_osc_vol($i as u8, state.osc_vol[$i]);
            pw.set_osc_pulse_width($i as u8, state.osc_pulse_width[$i]);
            pw.set_osc_freq_mult($i as u8, 1.0); // octave/detune applied via freq_mult
        };
    }
    apply_osc!(patch.osc_wave, 0);
    apply_osc!(patch.osc_wave, 1);
    apply_osc!(patch.osc_wave, 2);

    state.hard_sync = patch.hard_sync;
    state.fm_enabled = patch.fm_enabled;
    state.fm_depth = patch.fm_depth;
    state.ring_enabled = patch.ring_enabled;
    state.ring_depth = patch.ring_depth;
    state.noise_vol = patch.noise_vol;
    pw.set_hard_sync_enabled(state.hard_sync);
    pw.set_fm_depth(state.fm_depth);
    pw.set_ring_depth(state.ring_depth);
    pw.set_noise_vol(state.noise_vol);

    state.filter_enabled = patch.filter_enabled;
    state.filter_cutoff = patch.filter_cutoff;
    state.filter_q = patch.filter_q;
    state.filter_drive = patch.filter_drive;
    state.filter_key_track = patch.filter_key_track;
    state.filter_env_amount = patch.filter_env_amount;
    state.fenv_attack = patch.fenv_adsr[0];
    state.fenv_decay = patch.fenv_adsr[1];
    state.fenv_sustain = patch.fenv_adsr[2];
    state.fenv_release = patch.fenv_adsr[3];
    pw.set_filter_cutoff(state.filter_cutoff);
    pw.set_filter_resonance(state.filter_q);
    pw.set_filter_drive(state.filter_drive);
    pw.set_filter_key_track(state.filter_key_track);
    pw.set_filter_env_amount(state.filter_env_amount);
    pw.set_fenv_attack(state.fenv_attack);
    pw.set_fenv_decay(state.fenv_decay);
    pw.set_fenv_sustain(state.fenv_sustain);
    pw.set_fenv_release(state.fenv_release);

    state.lfo_enabled = patch.lfo_enabled;
    state.lfo_rate = patch.lfo_rate;
    state.lfo_depth = patch.lfo_depth;
    state.lfo_shape = patch.lfo_shape;
    state.lfo_dest = patch.lfo_dest;
    state.lfo2_enabled = patch.lfo2_enabled;
    state.lfo2_rate = patch.lfo2_rate;
    state.lfo2_depth = patch.lfo2_depth;
    state.lfo2_shape = patch.lfo2_shape;
    state.lfo2_dest = patch.lfo2_dest;
    pw.set_lfo_rate(state.lfo_rate);
    pw.set_lfo_depth(state.lfo_depth);
    pw.set_lfo_shape(state.lfo_shape as u8);
    pw.set_lfo_dest(state.lfo_dest as u8);
    pw.set_lfo2_rate(state.lfo2_rate);
    pw.set_lfo2_depth(state.lfo2_depth);
    pw.set_lfo2_shape(state.lfo2_shape as u8);
    pw.set_lfo2_dest(state.lfo2_dest as u8);

    state.amp_attack = patch.amp_adsr[0];
    state.amp_decay = patch.amp_adsr[1];
    state.amp_sustain = patch.amp_adsr[2];
    state.amp_release = patch.amp_adsr[3];
    state.glide_time = patch.glide_time;
    state.master_vol = patch.master_vol;
    state.global_vol = patch.global_vol;
    state.limiter_enabled = patch.limiter_enabled;
    state.limiter_threshold = patch.limiter_threshold;
    pw.set_amp_attack(state.amp_attack);
    pw.set_amp_decay(state.amp_decay);
    pw.set_amp_sustain(state.amp_sustain);
    pw.set_amp_release(state.amp_release);
    pw.set_glide_time(state.glide_time);
    pw.set_master_volume(state.master_vol);
    pw.set_global_volume(state.global_vol);
    pw.set_limiter_enabled(state.limiter_enabled);
    pw.set_limiter_threshold(state.limiter_threshold);
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn draw_header(
    ui: &mut egui::Ui,
    state: &mut SynthUiState,
    patches: &[Patch],
    pw: &mut impl ParamWriter,
    theme: &synth_ui::SynthTheme,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("TheSynth")
                .strong()
                .color(theme.c(&theme.accent)),
        );

        ui.separator();

        // Prev / current name / Next
        let can_prev = patches
            .iter()
            .position(|p| p.name == state.current_patch_name)
            .map(|i| i > 0)
            .unwrap_or(false);
        let can_next = patches
            .iter()
            .position(|p| p.name == state.current_patch_name)
            .map(|i| i + 1 < patches.len())
            .unwrap_or(!patches.is_empty());

        if ui.add_enabled(can_prev, egui::Button::new("◀")).clicked() {
            if let Some(i) = patches
                .iter()
                .position(|p| p.name == state.current_patch_name)
            {
                if i > 0 {
                    apply_patch(&patches[i - 1], state, pw);
                }
            }
        }
        ui.label(
            egui::RichText::new(&state.current_patch_name)
                .strong()
                .monospace(),
        );
        if ui.add_enabled(can_next, egui::Button::new("▶")).clicked() {
            let next = patches
                .iter()
                .position(|p| p.name == state.current_patch_name)
                .map(|i| i + 1)
                .unwrap_or(0);
            if next < patches.len() {
                apply_patch(&patches[next], state, pw);
            }
        }

        ui.separator();

        let browse_label = if state.browser_open {
            "▲ Close"
        } else {
            "⊞ Browse"
        };
        if ui.button(browse_label).clicked() {
            state.browser_open = !state.browser_open;
        }

        // Right-aligned master vol + limiter
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Slider::new(&mut state.master_vol, 0.0_f32..=1.0)
                    .text("Vol")
                    .clamp_to_range(true),
            );
            if ui.selectable_label(state.limiter_enabled, "Lim").clicked() {
                state.limiter_enabled = !state.limiter_enabled;
                pw.set_limiter_enabled(state.limiter_enabled);
            }
        });
    });
}

// ---------------------------------------------------------------------------
// Patch browser
// ---------------------------------------------------------------------------

fn draw_patch_browser(
    ui: &mut egui::Ui,
    state: &mut SynthUiState,
    patches: &[Patch],
    pw: &mut impl ParamWriter,
    theme: &synth_ui::SynthTheme,
) {
    // Category pills
    let mut categories: Vec<&str> = std::iter::once("All")
        .chain(
            patches
                .iter()
                .map(|p| p.category.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter(),
        )
        .collect();

    ui.horizontal_wrapped(|ui| {
        for (i, cat) in categories.iter().enumerate() {
            if ui
                .selectable_label(state.browser_category == i, *cat)
                .clicked()
            {
                state.browser_category = i;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.browser_search)
                    .hint_text("🔍 search")
                    .desired_width(140.0),
            );
        });
    });

    ui.add_space(4.0);

    let search = state.browser_search.to_lowercase();
    let selected_cat = if state.browser_category == 0 {
        None
    } else {
        categories.get(state.browser_category).copied()
    };

    let visible: Vec<&Patch> = patches
        .iter()
        .filter(|p| {
            selected_cat.map(|c| p.category == c).unwrap_or(true)
                && (search.is_empty()
                    || p.name.to_lowercase().contains(&search)
                    || p.category.to_lowercase().contains(&search))
        })
        .collect();

    let row_height = 22.0;
    let browser_height = 180.0;

    egui::ScrollArea::vertical()
        .max_height(browser_height)
        .show(ui, |ui| {
            egui::Grid::new("patch_grid")
                .num_columns(2)
                .min_col_width(ui.available_width() / 2.0 - 8.0)
                .min_row_height(row_height)
                .show(ui, |ui| {
                    for (idx, patch) in visible.iter().enumerate() {
                        let is_current = patch.name == state.current_patch_name;
                        let label = egui::RichText::new(format!(
                            "{}  {}",
                            if is_current { "●" } else { " " },
                            patch.name
                        ))
                        .color(if is_current {
                            theme.c(&theme.accent)
                        } else {
                            theme.c(&theme.text_primary)
                        });

                        if ui.button(label).clicked() {
                            apply_patch(patch, state, pw);
                            state.browser_open = false;
                        }

                        ui.label(
                            egui::RichText::new(&patch.category)
                                .small()
                                .color(theme.c(&theme.text_secondary)),
                        );

                        if idx % 2 == 1 {
                            ui.end_row();
                        }
                    }
                    // end_row for odd count
                    if visible.len() % 2 == 1 {
                        ui.end_row();
                    }
                });
        });
}

// ---------------------------------------------------------------------------
// Editor entry point
// ---------------------------------------------------------------------------

pub(crate) fn create_editor(
    params: Arc<TheSynthParams>,
    editor_state: Arc<EguiState>,
) -> Option<Box<dyn Editor>> {
    let patches = Arc::new(load_patches());
    let theme = midnight();
    let state_for_resize = Arc::clone(&editor_state);

    create_egui_editor(
        editor_state,
        SynthUiState::default(),
        |ctx, _| {
            midnight().apply_to_egui(ctx);
        },
        move |ctx, setter, state| {
            sync_state_from_params(state, &params);

            let mut pw = PluginParamWriter {
                params: &params,
                setter,
            };

            ResizableWindow::new("synth_resize")
                .min_size([900.0, 500.0])
                .show(ctx, &state_for_resize, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        // ── Header ────────────────────────────────────────────────
                        draw_header(ui, state, &patches, &mut pw, &theme);
                        ui.separator();

                        // ── Inline patch browser ──────────────────────────────────
                        if state.browser_open {
                            draw_patch_browser(ui, state, &patches, &mut pw, &theme);
                            ui.separator();
                        }

                        // ── ROW 1: OSC bank ──────────────────────────────────────
                        // Each OSC panel is given exactly 1/3 of available width so
                        // that chip_w calculations inside the panel use the right value.
                        ui.horizontal(|ui| {
                            let osc_w = (ui.available_width() / 3.0).floor();
                            for i in 0..3 {
                                ui.vertical(|ui| {
                                    ui.set_max_width(osc_w);
                                    ui_osc_panel(ui, state, &mut pw, &theme, i, false);
                                });
                            }
                        });
                        ui.horizontal(|ui| {
                            ui_mixer_panel(ui, state, &mut pw, &theme);
                        });
                        ui.separator();

                        // ── ROW 2: Filter+Env+LFO (left) | FX chain (right) ──────
                        ui.horizontal(|ui| {
                            let total_w = ui.available_width();
                            let fx_w = 420.0_f32.min(total_w * 0.35);
                            let left_w = total_w - fx_w - theme.sp_sm;

                            // Left column: filter, envelopes, modulation
                            ui.vertical(|ui| {
                                ui.set_max_width(left_w);
                                let col_w = (left_w - theme.sp_sm * 2.0) / 3.0;
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.set_max_width(col_w);
                                        ui_filter_panel(ui, state, &mut pw, &theme);
                                    });
                                    ui.vertical(|ui| {
                                        ui.set_max_width(col_w);
                                        ui_adsr_panel(
                                            ui,
                                            state,
                                            &mut pw,
                                            &theme,
                                            "FILTER ENV",
                                            true,
                                            &[],
                                        );
                                        ui.add_space(theme.sp_xs);
                                        ui_adsr_panel(
                                            ui,
                                            state,
                                            &mut pw,
                                            &theme,
                                            "AMP",
                                            false,
                                            &[],
                                        );
                                    });
                                    ui.vertical(|ui| {
                                        ui.set_max_width(col_w);
                                        ui_lfo_panel(ui, state, &mut pw, &theme);
                                        ui.add_space(theme.sp_xs);
                                        ui_lfo2_panel(ui, state, &mut pw, &theme);
                                    });
                                });
                            });

                            ui.separator();

                            // Right column: vertical FX strip
                            ui.vertical(|ui| {
                                ui.set_max_width(fx_w);
                                ui_fx_chain_vertical(ui, state, &mut pw, &theme);
                            });
                        });
                    });
                });
        },
    )
}
