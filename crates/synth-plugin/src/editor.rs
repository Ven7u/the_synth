use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::Arc;
use synth_ui::{
    midnight,
    panels::{
        draw_adsr_visualizer, ui_adsr_panel, ui_filter_panel, ui_fx_chain, ui_lfo2_panel,
        ui_lfo_panel, ui_mixer_panel, ui_osc_panel,
    },
    SynthUiState,
};

use crate::params::TheSynthParams;
use crate::plugin_param_writer::PluginParamWriter;

pub(crate) fn default_state() -> Arc<EguiState> {
    EguiState::from_size(1300, 680)
}

/// Populate the shared UI state from current nih-plug param values each frame.
/// Fields that are UI-local decompositions (osc_octave, osc_detune, unison UI,
/// enable toggles, BPM sync) are left as editor-session state.
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

pub(crate) fn create_editor(
    params: Arc<TheSynthParams>,
    editor_state: Arc<EguiState>,
) -> Option<Box<dyn Editor>> {
    // theme is const for the session; apply once on open, capture by frame closure for spacing/colors
    let theme = midnight();
    create_egui_editor(
        editor_state,
        SynthUiState::default(),
        |ctx, _| {
            midnight().apply_to_egui(ctx);
        },
        move |ctx, setter, state| {
            let _ = ctx; // egui style already applied in init callback
            sync_state_from_params(state, &params);

            let mut pw = PluginParamWriter {
                params: &params,
                setter,
            };

            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            for i in 0..3 {
                                ui_osc_panel(ui, state, &mut pw, &theme, i, false);
                            }
                        });

                        ui.separator();

                        ui.vertical(|ui| {
                            ui_mixer_panel(ui, state, &mut pw, &theme);
                        });

                        ui.separator();

                        ui.vertical(|ui| {
                            ui_filter_panel(ui, state, &mut pw, &theme);
                            ui.add_space(theme.sp_xs);
                            ui_adsr_panel(ui, state, &mut pw, &theme, "FILTER ENV", true, &[]);
                        });

                        ui.separator();

                        ui.vertical(|ui| {
                            ui_lfo_panel(ui, state, &mut pw, &theme);
                            ui.add_space(theme.sp_xs);
                            ui_lfo2_panel(ui, state, &mut pw, &theme);
                        });

                        ui.separator();

                        ui.vertical(|ui| {
                            ui_adsr_panel(ui, state, &mut pw, &theme, "AMP", false, &[]);
                            ui.add_space(theme.sp_xs);
                            // show filter env shape as a visual reference alongside amp env
                            let fenv = [
                                state.fenv_attack,
                                state.fenv_decay,
                                state.fenv_sustain,
                                state.fenv_release,
                            ];
                            draw_adsr_visualizer(ui, &fenv, &[], &theme);
                        });

                        ui.separator();

                        ui.vertical(|ui| {
                            ui_fx_chain(ui, state, &mut pw, &theme);
                        });
                    });
                });
            });
        },
    )
}
