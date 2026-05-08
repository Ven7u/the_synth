use crate::SynthApp;
use eframe::egui;
use egui::Color32;

/// Delay note divisions: (label, beats relative to a quarter-note pulse).
pub const DELAY_DIVISIONS: &[(&str, f32)] = &[
    ("1/1", 4.0),
    ("1/2", 2.0),
    ("1/4", 1.0),
    ("1/8", 0.5),
    ("1/16", 0.25),
    ("3/8", 1.5),   // dotted quarter
    ("3/16", 0.75), // dotted eighth
];

impl SynthApp {
    pub fn ui_fx_chain(&mut self, ui: &mut egui::Ui) {
        let col_od = self.theme.c(&self.theme.fx_overdrive);
        let col_dist = self.theme.c(&self.theme.fx_distortion);
        let col_cho = self.theme.c(&self.theme.fx_chorus);
        let col_dly = self.theme.c(&self.theme.fx_delay);
        let col_rev = self.theme.c(&self.theme.fx_reverb);
        let col_crys = self.theme.c(&self.theme.fx_crystallizer);

        ui.horizontal(|ui| {
            // ---- Overdrive ----
            ui.group(|ui| {
                ui.set_min_width(110.0);
                ui.vertical(|ui| {
                    let on = &mut self.fx_overdrive_on;
                    let label = egui::RichText::new("OVERDRIVE").small().strong()
                        .color(if *on { col_od } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Toggle overdrive (soft-clip / tanh saturation).").clicked() {
                        *on = !*on;
                        self.engine.set_fx_overdrive_mix(if *on { self.fx_overdrive_mix } else { 0.0 });
                    }
                    ui.add(egui::Slider::new(&mut self.fx_overdrive_drive, 1.0_f32..=10.0)
                        .text("Drive").clamp_to_range(true))
                        .on_hover_text("Drive — how hard the signal is pushed into tanh saturation.");
                    ui.add(egui::Slider::new(&mut self.fx_overdrive_tone, 0.0_f32..=1.0)
                        .text("Tone").clamp_to_range(true))
                        .on_hover_text("Tone — post-clipper low-pass: 0 = dark (400 Hz), 1 = bright (18 kHz).");
                    ui.add(egui::Slider::new(&mut self.fx_overdrive_asym, 0.0_f32..=1.0)
                        .text("Asym").clamp_to_range(true))
                        .on_hover_text("Asymmetry — DC bias before clipping adds even harmonics for a warmer, tube-like character.");
                    ui.add(egui::Slider::new(&mut self.fx_overdrive_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Wet/dry mix: 0 = dry, 1 = fully overdriven.");
                    self.engine.set_fx_overdrive_drive(self.fx_overdrive_drive);
                    self.engine.set_fx_overdrive_tone(self.fx_overdrive_tone);
                    self.engine.set_fx_overdrive_asym(self.fx_overdrive_asym);
                    if self.fx_overdrive_on {
                        self.engine.set_fx_overdrive_mix(self.fx_overdrive_mix);
                    }
                });
            });

            // ---- Distortion ----
            ui.group(|ui| {
                ui.set_min_width(110.0);
                ui.vertical(|ui| {
                    let on = &mut self.fx_distortion_on;
                    let label = egui::RichText::new("DISTORTION").small().strong()
                        .color(if *on { col_dist } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Toggle distortion (hard clipping).").clicked() {
                        *on = !*on;
                        self.engine.set_fx_distortion_mix(if *on { self.fx_distortion_mix } else { 0.0 });
                    }
                    ui.add(egui::Slider::new(&mut self.fx_distortion_drive, 1.0_f32..=20.0)
                        .text("Drive").clamp_to_range(true))
                        .on_hover_text("Drive — pre-gain before hard clipping. Higher = more of the wave is squared off.");
                    ui.add(egui::Slider::new(&mut self.fx_distortion_pre, 0.0_f32..=1.0)
                        .text("Pre").clamp_to_range(true))
                        .on_hover_text("Pre — high-pass before clipper (0 = all bass in, 1 = 800 Hz cut). Removes mud from low-end distortion.");
                    ui.add(egui::Slider::new(&mut self.fx_distortion_tone, 0.0_f32..=1.0)
                        .text("Tone").clamp_to_range(true))
                        .on_hover_text("Tone — post-clipper low-pass: 0 = dark (400 Hz), 1 = bright (18 kHz). Rolls off harsh high harmonics.");
                    ui.add(egui::Slider::new(&mut self.fx_distortion_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Wet/dry mix: 0 = dry, 1 = fully distorted.");
                    self.engine.set_fx_distortion_drive(self.fx_distortion_drive);
                    self.engine.set_fx_distortion_pre(self.fx_distortion_pre);
                    self.engine.set_fx_distortion_tone(self.fx_distortion_tone);
                    if self.fx_distortion_on {
                        self.engine.set_fx_distortion_mix(self.fx_distortion_mix);
                    }
                });
            });

            // ---- Chorus ----
            ui.group(|ui| {
                ui.set_min_width(130.0);
                ui.vertical(|ui| {
                    let on = &mut self.fx_chorus_on;
                    let label = egui::RichText::new("CHORUS").small().strong()
                        .color(if *on { col_cho } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Toggle chorus (LFO-modulated delay for width/shimmer).").clicked() {
                        *on = !*on;
                        self.engine.set_fx_chorus_mix(if *on { self.fx_chorus_mix } else { 0.0 });
                    }
                    ui.add(egui::Slider::new(&mut self.fx_chorus_rate, 0.1_f32..=5.0)
                        .text("Rate").suffix(" Hz").clamp_to_range(true))
                        .on_hover_text("LFO rate in Hz — how fast the chorus modulates.");
                    ui.add(egui::Slider::new(&mut self.fx_chorus_depth, 0.0_f32..=0.02)
                        .text("Depth").clamp_to_range(true))
                        .on_hover_text("Depth of LFO modulation in seconds (0–20 ms).");
                    ui.add(egui::Slider::new(&mut self.fx_chorus_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Wet/dry mix.");
                    self.engine.set_fx_chorus_rate(self.fx_chorus_rate);
                    self.engine.set_fx_chorus_depth(self.fx_chorus_depth);
                    if self.fx_chorus_on {
                        self.engine.set_fx_chorus_mix(self.fx_chorus_mix);
                    }
                });
            });

            // ---- Delay ----
            ui.group(|ui| {
                ui.set_min_width(160.0);
                ui.vertical(|ui| {
                    let on = &mut self.fx_delay_on;
                    let label = egui::RichText::new("DELAY").small().strong()
                        .color(if *on { col_dly } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Toggle delay (echo effect with feedback).").clicked() {
                        *on = !*on;
                        if *on { self.engine.reset_fx_tails(); }
                        self.engine.set_fx_delay_mix(if *on { self.fx_delay_mix } else { 0.0 });
                    }

                    ui.add_enabled_ui(!self.global_sync, |ui| {
                        let delay_sync_on = self.delay_sync_active();
                        let sync_label = egui::RichText::new("BPM Sync")
                            .color(if delay_sync_on { col_dly } else { Color32::GRAY });
                        if ui.button(sync_label).on_hover_text("Sync delay time to the Global BPM.").clicked() {
                            self.fx_delay_sync = !self.fx_delay_sync;
                        }
                    });

                    if self.delay_sync_active() {
                        let bpm = self.global_bpm as f32;
                        let beat_sec = 60.0 / bpm;
                        ui.horizontal_wrapped(|ui| {
                            for (i, (name, _)) in DELAY_DIVISIONS.iter().enumerate() {
                                let active = self.fx_delay_division == i;
                                let btn_label = egui::RichText::new(*name).small()
                                    .color(if active { col_dly } else { Color32::GRAY });
                                if ui.button(btn_label).on_hover_text(format!("Set delay to {} note ({:.0} BPM → {:.3}s)", name, bpm, beat_sec * DELAY_DIVISIONS[i].1)).clicked() {
                                    self.fx_delay_division = i;
                                }
                            }
                        });
                        let synced_time = (beat_sec * DELAY_DIVISIONS[self.fx_delay_division].1).clamp(0.01, 1.0);
                        self.fx_delay_time = synced_time;
                        ui.label(egui::RichText::new(format!("{:.3} s  @{}BPM", synced_time, self.global_bpm)).small().color(Color32::DARK_GRAY))
                            .on_hover_text("Current delay time computed from Global BPM and selected note division.");
                    } else {
                        ui.add(egui::Slider::new(&mut self.fx_delay_time, 0.01_f32..=1.0)
                            .text("Time").suffix(" s").clamp_to_range(true))
                            .on_hover_text("Delay time in seconds (10 ms – 1 s).");
                    }

                    ui.add(egui::Slider::new(&mut self.fx_delay_feedback, 0.0_f32..=0.95)
                        .text("Feedback").clamp_to_range(true))
                        .on_hover_text("Feedback amount — how much of the delayed signal repeats.");
                    ui.add(egui::Slider::new(&mut self.fx_delay_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Wet/dry mix.");
                    self.engine.set_fx_delay_time(self.fx_delay_time);
                    self.engine.set_fx_delay_feedback(self.fx_delay_feedback);
                    if self.fx_delay_on {
                        self.engine.set_fx_delay_mix(self.fx_delay_mix);
                    }
                });
            });

            // ---- Reverb ----
            ui.group(|ui| {
                ui.set_min_width(130.0);
                ui.vertical(|ui| {
                    let on = &mut self.fx_reverb_on;
                    let label = egui::RichText::new("REVERB").small().strong()
                        .color(if *on { col_rev } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Toggle reverb.").clicked() {
                        *on = !*on;
                        if *on { self.engine.reset_fx_tails(); }
                        self.engine.set_fx_reverb_mix(if *on { self.fx_reverb_mix } else { 0.0 });
                    }
                    ui.horizontal(|ui| {
                        for (i, name) in ["Free", "Plate", "Hall"].iter().enumerate() {
                            let selected = self.fx_reverb_type == i as u8;
                            let label = egui::RichText::new(*name).small()
                                .color(if selected { col_rev } else { Color32::GRAY });
                            if ui.selectable_label(selected, label).clicked() {
                                self.fx_reverb_type = i as u8;
                                self.engine.set_fx_reverb_type(i as u8);
                            }
                        }
                    });
                    ui.add(egui::Slider::new(&mut self.fx_reverb_predelay, 0.0_f32..=0.1)
                        .text("Pre").suffix(" s").clamp_to_range(true)
                        .custom_formatter(|v, _| format!("{:.0} ms", v * 1000.0)))
                        .on_hover_text("Pre-delay: silence before the reverb tail starts. 20–80 ms separates the dry note from the wash, giving cinematic depth.");
                    ui.add(egui::Slider::new(&mut self.fx_reverb_size, 0.0_f32..=1.0)
                        .text("Size").clamp_to_range(true))
                        .on_hover_text("Room size — controls reverb decay time.");
                    ui.add(egui::Slider::new(&mut self.fx_reverb_damp, 0.0_f32..=1.0)
                        .text("Damp").clamp_to_range(true))
                        .on_hover_text("High-frequency damping — 0 = bright, 1 = dark/muffled.");
                    ui.add(egui::Slider::new(&mut self.fx_reverb_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Wet/dry mix.");
                    self.engine.set_fx_reverb_predelay(self.fx_reverb_predelay);
                    self.engine.set_fx_reverb_size(self.fx_reverb_size);
                    self.engine.set_fx_reverb_damp(self.fx_reverb_damp);
                    if self.fx_reverb_on {
                        self.engine.set_fx_reverb_mix(self.fx_reverb_mix);
                    }
                });
            });

            // ---- Shimmer ----
            let col_shim = self.theme.c(&self.theme.fx_shimmer);
            ui.group(|ui| {
                ui.set_min_width(110.0);
                ui.vertical(|ui| {
                    let on = &mut self.fx_shimmer_on;
                    let label = egui::RichText::new("SHIMMER").small().strong()
                        .color(if *on { col_shim } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Shimmer reverb — pitch-shifted feedback loop creates a rising harmonic halo.").clicked() {
                        *on = !*on;
                        if *on { self.engine.reset_fx_tails(); }
                        self.engine.set_shimmer_mix(if *on { self.fx_shimmer_mix } else { 0.0 });
                    }
                    ui.add(egui::Slider::new(&mut self.fx_shimmer_size, 0.0_f32..=1.0)
                        .text("Size").clamp_to_range(true))
                        .on_hover_text("Shimmer reverb room size.");
                    ui.add(egui::Slider::new(&mut self.fx_shimmer_damp, 0.0_f32..=1.0)
                        .text("Damp").clamp_to_range(true))
                        .on_hover_text("Shimmer high-frequency damping.");
                    ui.add(egui::Slider::new(&mut self.fx_shimmer_amt, 0.0_f32..=1.0)
                        .text("Shimmer").clamp_to_range(true))
                        .on_hover_text("Amount of pitch-shifted signal fed back into the reverb loop.");
                    ui.add(egui::Slider::new(&mut self.fx_shimmer_width, 0.5_f32..=2.0)
                        .text("Width").clamp_to_range(true))
                        .on_hover_text("Stereo width of the wet reverb/shimmer field. 1.0 = neutral.");
                    ui.add(egui::Slider::new(&mut self.fx_shimmer_spread, 0.0_f32..=0.3)
                        .text("Spread").clamp_to_range(true))
                        .on_hover_text("Left/right decorrelation depth for reverb and shimmer tails.");
                    ui.add(egui::Slider::new(&mut self.fx_shimmer_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Shimmer wet level.");
                    ui.horizontal(|ui| {
                        ui.label("Pitch:");
                        for (i, lbl) in ["0", "+12", "+24"].iter().enumerate() {
                            if ui.selectable_label(self.fx_shimmer_pitch == i as u8, *lbl).clicked() {
                                self.fx_shimmer_pitch = i as u8;
                                self.engine.set_shimmer_pitch(i as u8);
                            }
                        }
                    });
                    self.engine.set_shimmer_size(self.fx_shimmer_size);
                    self.engine.set_shimmer_damp(self.fx_shimmer_damp);
                    self.engine.set_shimmer_amount(
                        if self.fx_shimmer_on { self.fx_shimmer_amt } else { 0.0 });
                    self.engine.set_shimmer_width(self.fx_shimmer_width);
                    self.engine.set_shimmer_spread(self.fx_shimmer_spread);
                    self.engine.set_shimmer_mix(
                        if self.fx_shimmer_on { self.fx_shimmer_mix } else { 0.0 });
                });
            });

            // ---- Crystallizer ----
            ui.group(|ui| {
                ui.set_min_width(140.0);
                ui.vertical(|ui| {
                    let on = &mut self.fx_crystal_on;
                    let label = egui::RichText::new("CRYSTAL").small().strong()
                        .color(if *on { col_crys } else { Color32::GRAY });
                    if ui.button(label).on_hover_text("Crystallizer — granular pitch-shift delay with feedback.").clicked() {
                        *on = !*on;
                        self.engine.set_crystal_mix(if *on { self.fx_crystal_mix } else { 0.0 });
                    }
                    ui.add(egui::Slider::new(&mut self.fx_crystal_grain_ms, 10.0_f32..=400.0)
                        .text("Grain").suffix(" ms").clamp_to_range(true))
                        .on_hover_text("Grain size in milliseconds.");
                    ui.add(egui::Slider::new(&mut self.fx_crystal_scatter, 0.0_f32..=1.0)
                        .text("Scatter").clamp_to_range(true))
                        .on_hover_text("Random grain position offset.");
                    ui.add(egui::Slider::new(&mut self.fx_crystal_delay_ms, 20.0_f32..=1200.0)
                        .text("Delay").suffix(" ms").clamp_to_range(true))
                        .on_hover_text("Base delay time.");
                    ui.add(egui::Slider::new(&mut self.fx_crystal_feedback, 0.0_f32..=0.95)
                        .text("Feedback").clamp_to_range(true))
                        .on_hover_text("Feedback amount.");
                    ui.add(egui::Slider::new(&mut self.fx_crystal_mix, 0.0_f32..=1.0)
                        .text("Mix").clamp_to_range(true))
                        .on_hover_text("Crystallizer wet level.");
                    ui.horizontal(|ui| {
                        ui.label("Pitch:");
                        for (i, lbl) in ["0.5x", "1x", "2x", "4x"].iter().enumerate() {
                            if ui.selectable_label(self.fx_crystal_pitch == i as u8, *lbl).clicked() {
                                self.fx_crystal_pitch = i as u8;
                                self.engine.set_crystal_pitch(i as u8);
                            }
                        }
                    });
                    self.engine.set_crystal_grain(self.fx_crystal_grain_ms);
                    self.engine.set_crystal_scatter(self.fx_crystal_scatter);
                    self.engine.set_crystal_delay(self.fx_crystal_delay_ms);
                    self.engine.set_crystal_feedback(self.fx_crystal_feedback);
                    self.engine.set_crystal_mix(
                        if self.fx_crystal_on { self.fx_crystal_mix } else { 0.0 });
                });
            });
            // ---- Stereo Width ----
            ui.group(|ui| {
                ui.set_min_width(120.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("STEREO").small().strong()
                        .color(self.theme.c(&self.theme.accent)));
                    ui.add(egui::Slider::new(&mut self.stereo_spread, 0.0_f32..=0.012)
                        .text("Spread")
                        .clamp_to_range(true)
                        .custom_formatter(|v, _| format!("{:.1} ms", v * 1000.0)))
                        .on_hover_text("Haas spread: delays R channel by 0–12 ms. Creates stereo width from mono unison voices. Keep under 10 ms to avoid comb filtering.");
                    ui.add(egui::Slider::new(&mut self.stereo_width, 0.0_f32..=2.0)
                        .text("Width")
                        .clamp_to_range(true))
                        .on_hover_text("M/S width on the final output. 0 = mono, 1 = unchanged, 2 = maximum stereo expansion.");
                    self.engine.set_stereo_spread(self.stereo_spread);
                    self.engine.set_stereo_width(self.stereo_width);
                });
            });
        });
    }
}
