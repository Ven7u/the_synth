//! nih-plug-egui editor for the plugin.
//!
//! Intentionally minimal: synthesis controls only (OSC, Filter, LFO, Amp, FX).
//! The standalone app's keyboard, sequencer, and oscilloscope are omitted —
//! those are app-level features, not plugin-level.

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, widgets, EguiState};
use std::sync::Arc;

use crate::params::TheSynthParams;

pub(crate) fn default_state() -> Arc<EguiState> {
    EguiState::from_size(1100, 600)
}

pub(crate) fn create_editor(
    params: Arc<TheSynthParams>,
    editor_state: Arc<EguiState>,
) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        editor_state,
        (),
        |_, _| {},
        move |ctx, setter, _state| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("The Synth");
                ui.separator();

                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // --- OSC 1 ---
                        ui.vertical(|ui| {
                            ui.label("OSC 1");
                            ui.add(widgets::ParamSlider::for_param(&params.osc1.wave, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.osc1.vol, setter));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.osc1.freq_mult,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.osc1.pulse_width,
                                setter,
                            ));
                        });

                        ui.separator();

                        // --- OSC 2 ---
                        ui.vertical(|ui| {
                            ui.label("OSC 2");
                            ui.add(widgets::ParamSlider::for_param(&params.osc2.wave, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.osc2.vol, setter));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.osc2.freq_mult,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.osc2.pulse_width,
                                setter,
                            ));
                        });

                        ui.separator();

                        // --- OSC 3 ---
                        ui.vertical(|ui| {
                            ui.label("OSC 3");
                            ui.add(widgets::ParamSlider::for_param(&params.osc3.wave, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.osc3.vol, setter));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.osc3.freq_mult,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.osc3.pulse_width,
                                setter,
                            ));
                        });

                        ui.separator();

                        // --- Mod sources ---
                        ui.vertical(|ui| {
                            ui.label("Mod");
                            ui.add(widgets::ParamSlider::for_param(&params.noise_vol, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.fm_depth, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.ring_depth, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.hard_sync, setter));
                        });

                        ui.separator();

                        // --- Filter ---
                        ui.vertical(|ui| {
                            ui.label("Filter");
                            ui.add(widgets::ParamSlider::for_param(
                                &params.filter.cutoff,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.filter.resonance,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.filter.drive,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.filter.key_track,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.filter.env_amount,
                                setter,
                            ));
                            ui.label("Filter Env");
                            ui.add(widgets::ParamSlider::for_param(
                                &params.filter.attack,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.filter.decay,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.filter.sustain,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.filter.release,
                                setter,
                            ));
                        });

                        ui.separator();

                        // --- LFO 1 ---
                        ui.vertical(|ui| {
                            ui.label("LFO 1");
                            ui.add(widgets::ParamSlider::for_param(&params.lfo1.rate, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.lfo1.depth, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.lfo1.shape, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.lfo1.dest, setter));
                        });

                        ui.separator();

                        // --- LFO 2 ---
                        ui.vertical(|ui| {
                            ui.label("LFO 2");
                            ui.add(widgets::ParamSlider::for_param(&params.lfo2.rate, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.lfo2.depth, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.lfo2.shape, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.lfo2.dest, setter));
                        });

                        ui.separator();

                        // --- Amp ---
                        ui.vertical(|ui| {
                            ui.label("Amp");
                            ui.add(widgets::ParamSlider::for_param(&params.amp.attack, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.amp.decay, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.amp.sustain, setter));
                            ui.add(widgets::ParamSlider::for_param(&params.amp.release, setter));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.amp.glide_time,
                                setter,
                            ));
                        });

                        ui.separator();

                        // --- FX ---
                        ui.vertical(|ui| {
                            ui.label("FX");
                            ui.label("Chorus");
                            ui.add(widgets::ParamSlider::for_param(
                                &params.fx.chorus_rate,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.fx.chorus_depth,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.fx.chorus_mix,
                                setter,
                            ));
                            ui.label("Delay");
                            ui.add(widgets::ParamSlider::for_param(
                                &params.fx.delay_time,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.fx.delay_feedback,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.fx.delay_mix,
                                setter,
                            ));
                            ui.label("Reverb");
                            ui.add(widgets::ParamSlider::for_param(
                                &params.fx.reverb_size,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.fx.reverb_damp,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.fx.reverb_mix,
                                setter,
                            ));
                        });

                        ui.separator();

                        // --- Master ---
                        ui.vertical(|ui| {
                            ui.label("Master");
                            ui.add(widgets::ParamSlider::for_param(
                                &params.master.master_vol,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.master.global_vol,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.master.limiter_enabled,
                                setter,
                            ));
                            ui.add(widgets::ParamSlider::for_param(
                                &params.master.limiter_threshold,
                                setter,
                            ));
                        });
                    });
                });
            });
        },
    )
}
