use egui::{Color32, RichText};

use crate::param_writer::ParamWriter;
use crate::state::SynthUiState;
use crate::theme::SynthTheme;

pub const DELAY_DIVISIONS: &[(&str, f32)] = &[
    ("1/1", 4.0),
    ("1/2", 2.0),
    ("1/4", 1.0),
    ("1/8", 0.5),
    ("1/16", 0.25),
    ("3/8", 1.5),
    ("3/16", 0.75),
];

pub fn ui_fx_chain(
    ui: &mut egui::Ui,
    s: &mut SynthUiState,
    pw: &mut impl ParamWriter,
    theme: &SynthTheme,
) {
    let col_od = theme.c(&theme.fx_overdrive);
    let col_dist = theme.c(&theme.fx_distortion);
    let col_cho = theme.c(&theme.fx_chorus);
    let col_dly = theme.c(&theme.fx_delay);
    let col_rev = theme.c(&theme.fx_reverb);
    let col_crys = theme.c(&theme.fx_crystallizer);

    ui.horizontal(|ui| {
        // ── Overdrive ──────────────────────────────────────────────────────
        ui.group(|ui| {
            ui.set_min_width(110.0);
            ui.vertical(|ui| {
                let label = RichText::new("OVERDRIVE").small().strong()
                    .color(if s.fx_overdrive_on { col_od } else { Color32::GRAY });
                if ui.button(label).on_hover_text("Toggle overdrive (soft-clip / tanh saturation).").clicked() {
                    s.fx_overdrive_on = !s.fx_overdrive_on;
                    pw.set_fx_overdrive_mix(if s.fx_overdrive_on { s.fx_overdrive_mix } else { 0.0 });
                }
                ui.add(egui::Slider::new(&mut s.fx_overdrive_drive, 1.0_f32..=10.0).text("Drive").clamp_to_range(true))
                    .on_hover_text("Drive — how hard the signal is pushed into tanh saturation.");
                ui.add(egui::Slider::new(&mut s.fx_overdrive_tone, 0.0_f32..=1.0).text("Tone").clamp_to_range(true))
                    .on_hover_text("Tone — post-clipper low-pass.");
                ui.add(egui::Slider::new(&mut s.fx_overdrive_asym, 0.0_f32..=1.0).text("Asym").clamp_to_range(true))
                    .on_hover_text("Asymmetry — DC bias before clipping adds even harmonics.");
                ui.add(egui::Slider::new(&mut s.fx_overdrive_mix, 0.0_f32..=1.0).text("Mix").clamp_to_range(true))
                    .on_hover_text("Wet/dry mix.");
                pw.set_fx_overdrive_drive(s.fx_overdrive_drive);
                pw.set_fx_overdrive_tone(s.fx_overdrive_tone);
                pw.set_fx_overdrive_asym(s.fx_overdrive_asym);
                if s.fx_overdrive_on { pw.set_fx_overdrive_mix(s.fx_overdrive_mix); }
            });
        });

        // ── Distortion ─────────────────────────────────────────────────────
        ui.group(|ui| {
            ui.set_min_width(110.0);
            ui.vertical(|ui| {
                let label = RichText::new("DISTORTION").small().strong()
                    .color(if s.fx_distortion_on { col_dist } else { Color32::GRAY });
                if ui.button(label).on_hover_text("Toggle distortion (hard clipping).").clicked() {
                    s.fx_distortion_on = !s.fx_distortion_on;
                    pw.set_fx_distortion_mix(if s.fx_distortion_on { s.fx_distortion_mix } else { 0.0 });
                }
                ui.add(egui::Slider::new(&mut s.fx_distortion_drive, 1.0_f32..=20.0).text("Drive").clamp_to_range(true))
                    .on_hover_text("Drive — pre-gain before hard clipping.");
                ui.add(egui::Slider::new(&mut s.fx_distortion_pre, 0.0_f32..=1.0).text("Pre").clamp_to_range(true))
                    .on_hover_text("Pre — high-pass before clipper.");
                ui.add(egui::Slider::new(&mut s.fx_distortion_tone, 0.0_f32..=1.0).text("Tone").clamp_to_range(true))
                    .on_hover_text("Tone — post-clipper low-pass.");
                ui.add(egui::Slider::new(&mut s.fx_distortion_mix, 0.0_f32..=1.0).text("Mix").clamp_to_range(true))
                    .on_hover_text("Wet/dry mix.");
                pw.set_fx_distortion_drive(s.fx_distortion_drive);
                pw.set_fx_distortion_pre(s.fx_distortion_pre);
                pw.set_fx_distortion_tone(s.fx_distortion_tone);
                if s.fx_distortion_on { pw.set_fx_distortion_mix(s.fx_distortion_mix); }
            });
        });

        // ── Chorus ─────────────────────────────────────────────────────────
        ui.group(|ui| {
            ui.set_min_width(130.0);
            ui.vertical(|ui| {
                let label = RichText::new("CHORUS").small().strong()
                    .color(if s.fx_chorus_on { col_cho } else { Color32::GRAY });
                if ui.button(label).on_hover_text("Toggle chorus.").clicked() {
                    s.fx_chorus_on = !s.fx_chorus_on;
                    pw.set_fx_chorus_mix(if s.fx_chorus_on { s.fx_chorus_mix } else { 0.0 });
                }
                ui.add(egui::Slider::new(&mut s.fx_chorus_rate, 0.1_f32..=5.0).text("Rate").suffix(" Hz").clamp_to_range(true))
                    .on_hover_text("LFO rate in Hz.");
                ui.add(egui::Slider::new(&mut s.fx_chorus_depth, 0.0_f32..=0.02).text("Depth").clamp_to_range(true))
                    .on_hover_text("Depth of LFO modulation in seconds (0–20 ms).");
                ui.add(egui::Slider::new(&mut s.fx_chorus_mix, 0.0_f32..=1.0).text("Mix").clamp_to_range(true))
                    .on_hover_text("Wet/dry mix.");
                pw.set_fx_chorus_rate(s.fx_chorus_rate);
                pw.set_fx_chorus_depth(s.fx_chorus_depth);
                if s.fx_chorus_on { pw.set_fx_chorus_mix(s.fx_chorus_mix); }
            });
        });

        // ── Delay ──────────────────────────────────────────────────────────
        ui.group(|ui| {
            ui.set_min_width(160.0);
            ui.vertical(|ui| {
                let label = RichText::new("DELAY").small().strong()
                    .color(if s.fx_delay_on { col_dly } else { Color32::GRAY });
                if ui.button(label).on_hover_text("Toggle delay.").clicked() {
                    s.fx_delay_on = !s.fx_delay_on;
                    pw.set_fx_delay_mix(if s.fx_delay_on { s.fx_delay_mix } else { 0.0 });
                }

                ui.add_enabled_ui(!s.global_sync, |ui| {
                    let delay_sync_on = s.delay_sync_active();
                    let sync_label = RichText::new("BPM Sync")
                        .color(if delay_sync_on { col_dly } else { Color32::GRAY });
                    if ui.button(sync_label).on_hover_text("Sync delay time to the Global BPM.").clicked() {
                        s.fx_delay_sync = !s.fx_delay_sync;
                    }
                });

                if s.delay_sync_active() {
                    let bpm = s.global_bpm as f32;
                    let beat_sec = 60.0 / bpm;
                    ui.horizontal_wrapped(|ui| {
                        for (i, (name, _)) in DELAY_DIVISIONS.iter().enumerate() {
                            let active = s.fx_delay_division == i;
                            let btn_label = RichText::new(*name).small()
                                .color(if active { col_dly } else { Color32::GRAY });
                            if ui.button(btn_label).on_hover_text(format!("Set delay to {} note", name)).clicked() {
                                s.fx_delay_division = i;
                            }
                        }
                    });
                    let synced_time = (beat_sec * DELAY_DIVISIONS[s.fx_delay_division].1).clamp(0.01, 1.0);
                    s.fx_delay_time = synced_time;
                    ui.label(RichText::new(format!("{:.3} s  @{}BPM", synced_time, s.global_bpm)).small().color(Color32::DARK_GRAY));
                } else {
                    ui.add(egui::Slider::new(&mut s.fx_delay_time, 0.01_f32..=1.0).text("Time").suffix(" s").clamp_to_range(true))
                        .on_hover_text("Delay time in seconds.");
                }

                ui.add(egui::Slider::new(&mut s.fx_delay_feedback, 0.0_f32..=0.95).text("Feedback").clamp_to_range(true))
                    .on_hover_text("Feedback amount.");
                ui.add(egui::Slider::new(&mut s.fx_delay_mix, 0.0_f32..=1.0).text("Mix").clamp_to_range(true))
                    .on_hover_text("Wet/dry mix.");
                pw.set_fx_delay_time(s.fx_delay_time);
                pw.set_fx_delay_feedback(s.fx_delay_feedback);
                if s.fx_delay_on { pw.set_fx_delay_mix(s.fx_delay_mix); }
            });
        });

        // ── Reverb ─────────────────────────────────────────────────────────
        ui.group(|ui| {
            ui.set_min_width(130.0);
            ui.vertical(|ui| {
                let label = RichText::new("REVERB").small().strong()
                    .color(if s.fx_reverb_on { col_rev } else { Color32::GRAY });
                if ui.button(label).on_hover_text("Toggle reverb.").clicked() {
                    s.fx_reverb_on = !s.fx_reverb_on;
                    pw.set_fx_reverb_mix(if s.fx_reverb_on { s.fx_reverb_mix } else { 0.0 });
                }
                ui.horizontal(|ui| {
                    for (i, name) in ["Free", "Plate", "Hall"].iter().enumerate() {
                        let selected = s.fx_reverb_type == i as u8;
                        let label = RichText::new(*name).small()
                            .color(if selected { col_rev } else { Color32::GRAY });
                        if ui.selectable_label(selected, label).clicked() {
                            s.fx_reverb_type = i as u8;
                            pw.set_fx_reverb_type(i as u8);
                        }
                    }
                });
                ui.add(egui::Slider::new(&mut s.fx_reverb_predelay, 0.0_f32..=0.1)
                    .text("Pre").suffix(" s").clamp_to_range(true)
                    .custom_formatter(|v, _| format!("{:.0} ms", v * 1000.0)))
                    .on_hover_text("Pre-delay.");
                ui.add(egui::Slider::new(&mut s.fx_reverb_size, 0.0_f32..=1.0).text("Size").clamp_to_range(true))
                    .on_hover_text("Room size.");
                ui.add(egui::Slider::new(&mut s.fx_reverb_damp, 0.0_f32..=1.0).text("Damp").clamp_to_range(true))
                    .on_hover_text("High-frequency damping.");
                ui.add(egui::Slider::new(&mut s.fx_reverb_mix, 0.0_f32..=1.0).text("Mix").clamp_to_range(true))
                    .on_hover_text("Wet/dry mix.");
                pw.set_fx_reverb_predelay(s.fx_reverb_predelay);
                pw.set_fx_reverb_size(s.fx_reverb_size);
                pw.set_fx_reverb_damp(s.fx_reverb_damp);
                if s.fx_reverb_on { pw.set_fx_reverb_mix(s.fx_reverb_mix); }
            });
        });

        // ── Shimmer ────────────────────────────────────────────────────────
        let col_shim = theme.c(&theme.fx_shimmer);
        ui.group(|ui| {
            ui.set_min_width(110.0);
            ui.vertical(|ui| {
                let label = RichText::new("SHIMMER").small().strong()
                    .color(if s.fx_shimmer_on { col_shim } else { Color32::GRAY });
                if ui.button(label).on_hover_text("Shimmer reverb.").clicked() {
                    s.fx_shimmer_on = !s.fx_shimmer_on;
                    pw.set_shimmer_mix(if s.fx_shimmer_on { s.fx_shimmer_mix } else { 0.0 });
                }
                ui.add(egui::Slider::new(&mut s.fx_shimmer_size, 0.0_f32..=1.0).text("Size").clamp_to_range(true));
                ui.add(egui::Slider::new(&mut s.fx_shimmer_damp, 0.0_f32..=1.0).text("Damp").clamp_to_range(true));
                ui.add(egui::Slider::new(&mut s.fx_shimmer_amt, 0.0_f32..=1.0).text("Shimmer").clamp_to_range(true));
                ui.add(egui::Slider::new(&mut s.fx_shimmer_width, 0.5_f32..=2.0).text("Width").clamp_to_range(true));
                ui.add(egui::Slider::new(&mut s.fx_shimmer_spread, 0.0_f32..=0.3).text("Spread").clamp_to_range(true));
                ui.add(egui::Slider::new(&mut s.fx_shimmer_mix, 0.0_f32..=1.0).text("Mix").clamp_to_range(true));
                ui.horizontal(|ui| {
                    ui.label("Pitch:");
                    for (i, lbl) in ["0", "+12", "+24"].iter().enumerate() {
                        if ui.selectable_label(s.fx_shimmer_pitch == i as u8, *lbl).clicked() {
                            s.fx_shimmer_pitch = i as u8;
                            pw.set_shimmer_pitch(i as u8);
                        }
                    }
                });
                pw.set_shimmer_size(s.fx_shimmer_size);
                pw.set_shimmer_damp(s.fx_shimmer_damp);
                pw.set_shimmer_amount(if s.fx_shimmer_on { s.fx_shimmer_amt } else { 0.0 });
                pw.set_shimmer_width(s.fx_shimmer_width);
                pw.set_shimmer_spread(s.fx_shimmer_spread);
                pw.set_shimmer_mix(if s.fx_shimmer_on { s.fx_shimmer_mix } else { 0.0 });
            });
        });

        // ── Crystallizer ───────────────────────────────────────────────────
        ui.group(|ui| {
            ui.set_min_width(140.0);
            ui.vertical(|ui| {
                let label = RichText::new("CRYSTAL").small().strong()
                    .color(if s.fx_crystal_on { col_crys } else { Color32::GRAY });
                if ui.button(label).on_hover_text("Crystallizer — granular pitch-shift delay.").clicked() {
                    s.fx_crystal_on = !s.fx_crystal_on;
                    pw.set_crystal_mix(if s.fx_crystal_on { s.fx_crystal_mix } else { 0.0 });
                }
                ui.add(egui::Slider::new(&mut s.fx_crystal_grain_ms, 10.0_f32..=400.0).text("Grain").suffix(" ms").clamp_to_range(true));
                ui.add(egui::Slider::new(&mut s.fx_crystal_scatter, 0.0_f32..=1.0).text("Scatter").clamp_to_range(true));
                ui.add(egui::Slider::new(&mut s.fx_crystal_delay_ms, 20.0_f32..=1200.0).text("Delay").suffix(" ms").clamp_to_range(true));
                ui.add(egui::Slider::new(&mut s.fx_crystal_feedback, 0.0_f32..=0.95).text("Feedback").clamp_to_range(true));
                ui.add(egui::Slider::new(&mut s.fx_crystal_mix, 0.0_f32..=1.0).text("Mix").clamp_to_range(true));
                ui.horizontal(|ui| {
                    ui.label("Pitch:");
                    for (i, lbl) in ["0.5x", "1x", "2x", "4x"].iter().enumerate() {
                        if ui.selectable_label(s.fx_crystal_pitch == i as u8, *lbl).clicked() {
                            s.fx_crystal_pitch = i as u8;
                            pw.set_crystal_pitch(i as u8);
                        }
                    }
                });
                pw.set_crystal_grain(s.fx_crystal_grain_ms);
                pw.set_crystal_scatter(s.fx_crystal_scatter);
                pw.set_crystal_delay(s.fx_crystal_delay_ms);
                pw.set_crystal_feedback(s.fx_crystal_feedback);
                pw.set_crystal_mix(if s.fx_crystal_on { s.fx_crystal_mix } else { 0.0 });
            });
        });

        // ── Stereo Width ───────────────────────────────────────────────────
        ui.group(|ui| {
            ui.set_min_width(120.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("STEREO").small().strong().color(theme.c(&theme.accent)));
                ui.add(egui::Slider::new(&mut s.stereo_spread, 0.0_f32..=0.012)
                    .text("Spread")
                    .clamp_to_range(true)
                    .custom_formatter(|v, _| format!("{:.1} ms", v * 1000.0)))
                    .on_hover_text("Haas spread: delays R channel.");
                ui.add(egui::Slider::new(&mut s.stereo_width, 0.0_f32..=2.0)
                    .text("Width")
                    .clamp_to_range(true))
                    .on_hover_text("M/S width on the final output.");
                pw.set_stereo_spread(s.stereo_spread);
                pw.set_stereo_width(s.stereo_width);
            });
        });
    });
}
