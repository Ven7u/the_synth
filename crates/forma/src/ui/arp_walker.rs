use crate::SynthApp;
use eframe::egui;
use egui::{Color32, Pos2, Sense, Stroke, Vec2};
use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::atomic::Ordering;

impl SynthApp {
    pub fn ui_arp_panel(&mut self, ui: &mut egui::Ui) {
        use forma_engine::arp::{ArpMode, ClockDiv};

        let enabled = self.engine.arp_enabled();

        ui.horizontal(|ui| {
            // ── Left: ARP controls ────────────────────────────────────────
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let label = egui::RichText::new("ARP").strong().color(if enabled {
                        self.theme.c(&self.theme.accent)
                    } else {
                        Color32::GRAY
                    });
                    if ui.button(label).clicked() {
                        let new_enabled = !enabled;
                        self.all_notes_off();
                        self.engine.set_arp_enabled(new_enabled);
                        if new_enabled && self.arp_sync_active() {
                            self.apply_clock_sync();
                            self.schedule_or_restart_arp();
                        }
                        if !new_enabled {
                            self.engine.chord_hold(&[]);
                        }
                    }
                    if ui
                        .button("RST")
                        .on_hover_text("Restart arp phase/step from beginning.")
                        .clicked()
                    {
                        self.engine.arp_restart();
                    }
                    let hold = self.engine.arp_hold();
                    let hold_label = egui::RichText::new("HOLD").color(if hold {
                        self.theme.c(&self.theme.accent_hold)
                    } else {
                        Color32::GRAY
                    });
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
                        if sync_active {
                            self.engine
                                .set_arp_bpm(self.seq.bpm.load(Ordering::Relaxed) as f32);
                        }
                        let mut bpm = self.engine.arp_bpm();
                        ui.add_enabled_ui(!sync_active, |ui| {
                            if ui.add(egui::Slider::new(&mut bpm, 20.0..=300.0)).changed() {
                                self.engine.set_arp_bpm(bpm);
                            }
                        });
                        ui.add_enabled_ui(!self.global_sync, |ui| {
                            let sync_label =
                                egui::RichText::new("Sync").color(if self.arp_sync_active() {
                                    self.theme.c(&self.theme.accent)
                                } else {
                                    Color32::GRAY
                                });
                            if ui
                                .button(sync_label)
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
                            if ui
                                .selectable_label(current_mode == i as u8, label)
                                .clicked()
                            {
                                self.engine.set_arp_mode(i as u8);
                            }
                        }
                    });
                    let current_oct = self.engine.arp_octave_range();
                    ui.horizontal(|ui| {
                        ui.label("Oct:");
                        for oct in 1u8..=4 {
                            if ui
                                .selectable_label(current_oct == oct, oct.to_string())
                                .clicked()
                            {
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
            });

            // ── Vertical separator ────────────────────────────────────────
            ui.add_space(6.0);
            let sep_rect = ui.available_rect_before_wrap();
            let sep_x = sep_rect.left();
            ui.painter().vline(
                sep_x,
                sep_rect.y_range(),
                Stroke::new(1.0, Color32::from_gray(45)),
            );
            ui.add_space(8.0);

            // ── Right: Ring gate sequencer ────────────────────────────────
            ui.add_enabled_ui(enabled, |ui| {
                ui.vertical(|ui| {
                    self.ui_arp_ring(ui);
                });
            });
        });
    }

    fn ui_arp_ring(&mut self, ui: &mut egui::Ui) {
        use forma_engine::arp::{euclidean_pattern, RING_PRESETS};

        let accent = self.theme.c(&self.theme.accent);

        // Enable toggle + N steps control
        ui.horizontal(|ui| {
            let ring_col = if self.arp_ring_enabled {
                accent
            } else {
                Color32::GRAY
            };
            if ui
                .button(egui::RichText::new("RING").color(ring_col))
                .on_hover_text("Euclidean ring gate — controls which arp steps fire a note.")
                .clicked()
            {
                self.arp_ring_enabled = !self.arp_ring_enabled;
                self.engine.set_arp_ring_enabled(self.arp_ring_enabled);
            }
            ui.label("N:");
            let mut n = self.arp_ring_steps as usize;
            if ui
                .add(egui::DragValue::new(&mut n).range(2..=16))
                .on_hover_text("Number of steps in the ring (2–16).")
                .changed()
            {
                self.arp_ring_steps = n as u8;
                self.engine.set_arp_ring_steps(n as u8);
                // Mask pattern to new step count so stale high bits don't linger
                let mask = if n >= 32 { u32::MAX } else { (1u32 << n) - 1 };
                self.arp_ring_pattern &= mask;
                self.engine.set_arp_ring_pattern(self.arp_ring_pattern);
            }
            ui.label("  K:");
            let max_k = self.arp_ring_steps as usize;
            let mut k = (self.arp_ring_k as usize).min(max_k);
            if ui
                .add(egui::DragValue::new(&mut k).range(1..=max_k))
                .on_hover_text("Hits to distribute (euclidean generator).")
                .changed()
            {
                self.arp_ring_k = k as u8;
            }
            if ui
                .button("Gen")
                .on_hover_text("Fill ring with K hits spread as evenly as possible (Euclidean).")
                .clicked()
            {
                self.arp_ring_pattern = euclidean_pattern(k, self.arp_ring_steps as usize, 0);
                self.engine.set_arp_ring_pattern(self.arp_ring_pattern);
            }
        });

        // Preset buttons
        ui.horizontal_wrapped(|ui| {
            for &(name, k, n, rot) in RING_PRESETS {
                if ui
                    .small_button(name)
                    .on_hover_text(format!("E({k},{n}) rot={rot}"))
                    .clicked()
                {
                    self.arp_ring_steps = n;
                    self.arp_ring_pattern = euclidean_pattern(k as usize, n as usize, rot as usize);
                    self.arp_ring_k = k;
                    self.engine.set_arp_ring_steps(n);
                    self.engine.set_arp_ring_pattern(self.arp_ring_pattern);
                }
            }
        });

        // Circular ring display
        let ring_size = 120.0;
        let (ring_rect, response) = ui.allocate_exact_size(Vec2::splat(ring_size), Sense::click());
        if ui.is_rect_visible(ring_rect) {
            let painter = ui.painter_at(ring_rect);
            let center = ring_rect.center();
            let ring_r = ring_size * 0.38;
            let n = self.arp_ring_steps as usize;
            let pattern = self.arp_ring_pattern;
            let ring_pos = self.engine.arp_ring_pos() as usize;

            // Background guide circle
            painter.circle_stroke(center, ring_r, Stroke::new(1.0, Color32::from_gray(35)));

            for i in 0..n {
                let angle = i as f32 * TAU / n as f32 - FRAC_PI_2;
                let dot = Pos2::new(
                    center.x + ring_r * angle.cos(),
                    center.y + ring_r * angle.sin(),
                );
                let active = (pattern >> i) & 1 == 1;
                let is_head = self.arp_ring_enabled && ring_pos == i;

                // Playhead halo
                if is_head {
                    painter.circle_filled(dot, 9.0, accent.gamma_multiply(0.25));
                }
                if active {
                    painter.circle_filled(
                        dot,
                        6.0,
                        if is_head {
                            accent
                        } else {
                            accent.gamma_multiply(0.75)
                        },
                    );
                } else {
                    painter.circle_stroke(dot, 5.0, Stroke::new(1.5, Color32::from_gray(65)));
                    if is_head {
                        painter.circle_filled(dot, 3.5, Color32::from_gray(160));
                    }
                }
            }

            // Click: toggle nearest step
            if response.clicked() {
                if let Some(click_pos) = response.interact_pointer_pos() {
                    let mut best = 0;
                    let mut best_d = f32::MAX;
                    for i in 0..n {
                        let angle = i as f32 * TAU / n as f32 - FRAC_PI_2;
                        let dot = Pos2::new(
                            center.x + ring_r * angle.cos(),
                            center.y + ring_r * angle.sin(),
                        );
                        let d = (dot - click_pos).length();
                        if d < best_d {
                            best_d = d;
                            best = i;
                        }
                    }
                    if best_d < 14.0 {
                        self.arp_ring_pattern ^= 1 << best;
                        self.engine.set_arp_ring_pattern(self.arp_ring_pattern);
                    }
                }
            }
        }
    }

    pub fn ui_walker_panel(&mut self, ui: &mut egui::Ui) {
        use forma_engine::arp::{ClockDiv, Scale};

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
                    self.engine
                        .set_walker_bpm(self.seq.bpm.load(Ordering::Relaxed) as f32);
                }
                let mut bpm = self.engine.walker_bpm();
                ui.add_enabled_ui(!sync_active, |ui| {
                    if ui.add(egui::Slider::new(&mut bpm, 20.0..=300.0)).changed() {
                        self.engine.set_walker_bpm(bpm);
                    }
                });
                // Per-component sync toggle (disabled when global sync is on)
                ui.add_enabled_ui(!self.global_sync, |ui| {
                    let sync_label =
                        egui::RichText::new("Sync").color(if self.walker_sync_active() {
                            self.theme.c(&self.theme.accent)
                        } else {
                            Color32::GRAY
                        });
                    if ui
                        .button(sync_label)
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
                    if ui
                        .selectable_label(current_scale == i as u8, label)
                        .clicked()
                    {
                        self.engine.set_walker_scale(i as u8);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Root:");
                let mut root = self.engine.walker_root();
                let note_names = [
                    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
                ];
                let name = note_names[(root % 12) as usize];
                let octave = (root as i32 / 12) - 1;
                ui.label(format!("{}{}", name, octave));
                if ui.add(egui::Slider::new(&mut root, 36u8..=84)).changed() {
                    self.engine.set_walker_root(root);
                }
                ui.label("  Oct:");
                let current_oct = self.engine.walker_octave_range();
                for oct in 1u8..=3 {
                    if ui
                        .selectable_label(current_oct == oct, oct.to_string())
                        .clicked()
                    {
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
