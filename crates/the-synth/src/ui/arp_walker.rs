use crate::SynthApp;
use synth_control::ControlEvent;
use eframe::egui;
use egui::Color32;
use std::sync::atomic::Ordering;

impl SynthApp {
    pub fn ui_arp_panel(&mut self, ui: &mut egui::Ui) {
        use synth_engine::arp::{ArpMode, ClockDiv};

        let enabled = self.state.arp.enabled.load(Ordering::Relaxed);
        ui.horizontal(|ui| {
            let label = egui::RichText::new("ARP").strong()
                .color(if enabled { Color32::from_rgb(0, 220, 160) } else { Color32::GRAY });
            if ui.button(label).clicked() {
                let new_enabled = !enabled;
                self.state.arp.enabled.store(new_enabled, Ordering::Relaxed);
                if new_enabled && self.seq_clock_sync {
                    self.apply_clock_sync();
                    self.schedule_or_restart_arp();
                }
                if !new_enabled {
                    let _ = self.control.try_send(ControlEvent::ChordHold { track: 0, notes: Vec::new() });
                }
            }
            if ui.button("RST").on_hover_text("Restart arp phase/step from beginning.").clicked() {
                let _ = self.control.try_send(ControlEvent::ArpRestart { track: 0 });
            }
            let hold = self.state.arp.hold.load(Ordering::Relaxed);
            let hold_label = egui::RichText::new("HOLD")
                .color(if hold { Color32::from_rgb(255, 200, 0) } else { Color32::GRAY });
            if ui.button(hold_label).clicked() {
                let new_hold = !hold;
                self.state.arp.hold.store(new_hold, Ordering::Relaxed);
                if !new_hold {
                    let _ = self.control.try_send(ControlEvent::ChordHold { track: 0, notes: Vec::new() });
                }
            }
        });

        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("BPM:");
                if self.seq_clock_sync {
                    self.arp_bpm = self.seq_bpm as f32;
                }
                ui.add_enabled_ui(!self.seq_clock_sync, |ui| {
                    if ui.add(egui::Slider::new(&mut self.arp_bpm, 20.0..=300.0)).changed() {
                        self.state.arp.bpm.set(self.arp_bpm);
                    }
                });
                if self.seq_clock_sync {
                    ui.label(egui::RichText::new("SYNC").small().color(Color32::GRAY));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Div:");
                for (i, &label) in ClockDiv::LABELS.iter().enumerate() {
                    if ui.selectable_label(self.arp_division == i as u8, label).clicked() {
                        self.arp_division = i as u8;
                        self.state.arp.division.store(i as u8, Ordering::Relaxed);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Mode:");
                for (i, &label) in ArpMode::LABELS.iter().enumerate() {
                    if ui.selectable_label(self.arp_mode == i as u8, label).clicked() {
                        self.arp_mode = i as u8;
                        self.state.arp.mode.store(i as u8, Ordering::Relaxed);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Oct:");
                for oct in 1u8..=4 {
                    if ui.selectable_label(self.arp_octave_range == oct, oct.to_string()).clicked() {
                        self.arp_octave_range = oct;
                        self.state.arp.octave_range.store(oct, Ordering::Relaxed);
                    }
                }
                ui.label("  Gate:");
                if ui.add(egui::Slider::new(&mut self.arp_gate, 0.05..=1.0)).changed() {
                    self.state.arp.gate.set(self.arp_gate);
                }
            });
        });
    }

    pub fn ui_walker_panel(&mut self, ui: &mut egui::Ui) {
        use synth_engine::arp::{Scale, ClockDiv};

        let enabled = self.state.walker.enabled.load(Ordering::Relaxed);
        ui.horizontal(|ui| {
            let label = egui::RichText::new("WALKER").strong()
                .color(if enabled { Color32::from_rgb(100, 180, 255) } else { Color32::GRAY });
            if ui.button(label).on_hover_text("Scale Walker — autonomous random walk within a scale. Generates notes independently of keyboard input.").clicked() {
                let new_enabled = !enabled;
                self.state.walker.enabled.store(new_enabled, Ordering::Relaxed);
                if new_enabled && self.seq_clock_sync {
                    self.apply_clock_sync();
                    self.schedule_or_restart_walker();
                }
            }
            if ui.button("RST").on_hover_text("Restart walker phase/index from beginning.").clicked() {
                let _ = self.control.try_send(ControlEvent::WalkerRestart { track: 0 });
            }
        });

        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("BPM:");
                if self.seq_clock_sync {
                    self.walker_bpm = self.seq_bpm as f32;
                }
                ui.add_enabled_ui(!self.seq_clock_sync, |ui| {
                    if ui.add(egui::Slider::new(&mut self.walker_bpm, 20.0..=300.0)).changed() {
                        self.state.walker.bpm.set(self.walker_bpm);
                    }
                });
                if self.seq_clock_sync {
                    ui.label(egui::RichText::new("SYNC").small().color(Color32::GRAY));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Div:");
                for (i, &label) in ClockDiv::LABELS.iter().enumerate() {
                    if ui.selectable_label(self.walker_division == i as u8, label).clicked() {
                        self.walker_division = i as u8;
                        self.state.walker.division.store(i as u8, Ordering::Relaxed);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Scale:");
                for (i, &label) in Scale::LABELS.iter().enumerate() {
                    if ui.selectable_label(self.walker_scale == i as u8, label).clicked() {
                        self.walker_scale = i as u8;
                        self.state.walker.scale.store(i as u8, Ordering::Relaxed);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Root:");
                let note_names = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];
                let name = note_names[(self.walker_root % 12) as usize];
                let octave = (self.walker_root as i32 / 12) - 1;
                ui.label(format!("{}{}", name, octave));
                if ui.add(egui::Slider::new(&mut self.walker_root, 36u8..=84)).changed() {
                    self.state.walker.root.store(self.walker_root, Ordering::Relaxed);
                }
                ui.label("  Oct:");
                for oct in 1u8..=3 {
                    if ui.selectable_label(self.walker_oct == oct, oct.to_string()).clicked() {
                        self.walker_oct = oct;
                        self.state.walker.octave_range.store(oct, Ordering::Relaxed);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Gate:");
                if ui.add(egui::Slider::new(&mut self.walker_gate, 0.05..=1.0)).changed() {
                    self.state.walker.gate.set(self.walker_gate);
                }
            });
        });
    }
}
