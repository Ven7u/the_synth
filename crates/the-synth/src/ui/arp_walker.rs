use crate::SynthApp;
use eframe::egui;
use egui::Color32;
use std::sync::atomic::Ordering;

impl SynthApp {
    pub fn ui_arp_panel(&mut self, ui: &mut egui::Ui) {
        use synth_engine::arp::{ArpMode, ClockDiv};

        let enabled = self.engine.arp_enabled();
        ui.horizontal(|ui| {
            let label = egui::RichText::new("ARP").strong()
                .color(if enabled { self.theme.c(&self.theme.accent) } else { Color32::GRAY });
            if ui.button(label).clicked() {
                let new_enabled = !enabled;
                self.engine.set_arp_enabled(new_enabled);
                if new_enabled && self.arp_sync_active() {
                    self.apply_clock_sync();
                    self.schedule_or_restart_arp();
                }
                if !new_enabled {
                    self.engine.chord_hold(&[]);
                }
            }
            if ui.button("RST").on_hover_text("Restart arp phase/step from beginning.").clicked() {
                self.engine.arp_restart();
            }
            let hold = self.engine.arp_hold();
            let hold_label = egui::RichText::new("HOLD")
                .color(if hold { self.theme.c(&self.theme.accent_hold) } else { Color32::GRAY });
            if ui.button(hold_label).clicked() {
                let new_hold = !hold;
                self.engine.set_arp_hold(new_hold);
                if !new_hold {
                    self.engine.chord_hold(&[]);
                }
            }
        });

        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("BPM:");
                let sync_active = self.arp_sync_active();
                // When synced, force engine bpm to the global value before we read it.
                if sync_active {
                    self.engine.set_arp_bpm(self.seq.bpm.load(Ordering::Relaxed) as f32);
                }
                let mut bpm = self.engine.arp_bpm();
                ui.add_enabled_ui(!sync_active, |ui| {
                    if ui.add(egui::Slider::new(&mut bpm, 20.0..=300.0)).changed() {
                        self.engine.set_arp_bpm(bpm);
                    }
                });
                // Per-component sync toggle (disabled when global sync is on)
                ui.add_enabled_ui(!self.global_sync, |ui| {
                    let sync_label = egui::RichText::new("Sync")
                        .color(if self.arp_sync_active() { self.theme.c(&self.theme.accent) } else { Color32::GRAY });
                    if ui.button(sync_label)
                        .on_hover_text("Lock Arp BPM to the Global BPM.")
                        .clicked()
                    {
                        self.arp_sync = !self.arp_sync;
                        if self.arp_sync {
                            self.apply_clock_sync();
                            self.schedule_or_restart_arp();
                        } else {
                            self.seq.arp_restart.store(false, Ordering::Relaxed);
                        }
                    }
                });
            });
            let current_div = self.engine.arp_division();
            ui.horizontal(|ui| {
                ui.label("Div:");
                for (i, &label) in ClockDiv::LABELS.iter().enumerate() {
                    if ui.selectable_label(current_div == i as u8, label).clicked() {
                        self.engine.set_arp_division(i as u8);
                    }
                }
            });
            let current_mode = self.engine.arp_mode();
            ui.horizontal(|ui| {
                ui.label("Mode:");
                for (i, &label) in ArpMode::LABELS.iter().enumerate() {
                    if ui.selectable_label(current_mode == i as u8, label).clicked() {
                        self.engine.set_arp_mode(i as u8);
                    }
                }
            });
            let current_oct = self.engine.arp_octave_range();
            ui.horizontal(|ui| {
                ui.label("Oct:");
                for oct in 1u8..=4 {
                    if ui.selectable_label(current_oct == oct, oct.to_string()).clicked() {
                        self.engine.set_arp_octave_range(oct);
                    }
                }
                ui.label("  Gate:");
                let mut gate = self.engine.arp_gate();
                if ui.add(egui::Slider::new(&mut gate, 0.05..=1.0)).changed() {
                    self.engine.set_arp_gate(gate);
                }
            });
        });
    }

    pub fn ui_walker_panel(&mut self, ui: &mut egui::Ui) {
        use synth_engine::arp::{Scale, ClockDiv};

        let enabled = self.engine.walker_enabled();
        ui.horizontal(|ui| {
            let label = egui::RichText::new("WALKER").strong()
                .color(if enabled { self.theme.c(&self.theme.accent_walker) } else { Color32::GRAY });
            if ui.button(label).on_hover_text("Scale Walker — autonomous random walk within a scale. Generates notes independently of keyboard input.").clicked() {
                let new_enabled = !enabled;
                self.engine.set_walker_enabled(new_enabled);
                if new_enabled && self.walker_sync_active() {
                    self.apply_clock_sync();
                    self.schedule_or_restart_walker();
                }
            }
            if ui.button("RST").on_hover_text("Restart walker phase/index from beginning.").clicked() {
                self.engine.walker_restart();
            }
        });

        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("BPM:");
                let sync_active = self.walker_sync_active();
                if sync_active {
                    self.engine.set_walker_bpm(self.seq.bpm.load(Ordering::Relaxed) as f32);
                }
                let mut bpm = self.engine.walker_bpm();
                ui.add_enabled_ui(!sync_active, |ui| {
                    if ui.add(egui::Slider::new(&mut bpm, 20.0..=300.0)).changed() {
                        self.engine.set_walker_bpm(bpm);
                    }
                });
                // Per-component sync toggle (disabled when global sync is on)
                ui.add_enabled_ui(!self.global_sync, |ui| {
                    let sync_label = egui::RichText::new("Sync")
                        .color(if self.walker_sync_active() { self.theme.c(&self.theme.accent) } else { Color32::GRAY });
                    if ui.button(sync_label)
                        .on_hover_text("Lock Walker BPM to the Global BPM.")
                        .clicked()
                    {
                        self.walker_sync = !self.walker_sync;
                        if self.walker_sync {
                            self.apply_clock_sync();
                            self.schedule_or_restart_walker();
                        } else {
                            self.seq.walker_restart.store(false, Ordering::Relaxed);
                        }
                    }
                });
            });
            let current_div = self.engine.walker_division();
            ui.horizontal(|ui| {
                ui.label("Div:");
                for (i, &label) in ClockDiv::LABELS.iter().enumerate() {
                    if ui.selectable_label(current_div == i as u8, label).clicked() {
                        self.engine.set_walker_division(i as u8);
                    }
                }
            });
            let current_scale = self.engine.walker_scale();
            ui.horizontal(|ui| {
                ui.label("Scale:");
                for (i, &label) in Scale::LABELS.iter().enumerate() {
                    if ui.selectable_label(current_scale == i as u8, label).clicked() {
                        self.engine.set_walker_scale(i as u8);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Root:");
                let mut root = self.engine.walker_root();
                let note_names = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];
                let name = note_names[(root % 12) as usize];
                let octave = (root as i32 / 12) - 1;
                ui.label(format!("{}{}", name, octave));
                if ui.add(egui::Slider::new(&mut root, 36u8..=84)).changed() {
                    self.engine.set_walker_root(root);
                }
                ui.label("  Oct:");
                let current_oct = self.engine.walker_octave_range();
                for oct in 1u8..=3 {
                    if ui.selectable_label(current_oct == oct, oct.to_string()).clicked() {
                        self.engine.set_walker_octave_range(oct);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Gate:");
                let mut gate = self.engine.walker_gate();
                if ui.add(egui::Slider::new(&mut gate, 0.05..=1.0)).changed() {
                    self.engine.set_walker_gate(gate);
                }
            });
        });
    }
}
