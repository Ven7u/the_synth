use crate::sequencer::{chord_name, chord_quality, ScaleType, SeqMode, DEGREE_LABELS, NOTE_NAMES};
use crate::SynthApp;
use eframe::egui;
use egui::{Color32, Rounding, Sense, Stroke, Vec2};
use std::sync::atomic::Ordering;

const SEQ_CHROMATIC: &[u8] = &[
    36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59,
    60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83,
    84,
];

impl SynthApp {
    pub fn ui_sequencer_panel(&mut self, ui: &mut egui::Ui) {
        let seq_mode = SeqMode::from_u8(self.seq.mode.load(Ordering::Relaxed));
        let seq_playing = self.seq.playing.load(Ordering::Relaxed);

        // --- Shared toolbar ---
        ui.horizontal(|ui| {
            // Mode tabs
            for &mode in &[SeqMode::NoteSeq, SeqMode::ChordSeq] {
                let active = seq_mode == mode;
                let label = egui::RichText::new(mode.label())
                    .color(if active {
                        self.theme.c(&self.theme.accent)
                    } else {
                        Color32::GRAY
                    })
                    .strong();
                let tip = match mode {
                    SeqMode::NoteSeq => "Note Sequencer — step-sequence individual notes.",
                    SeqMode::ChordSeq => {
                        "Chord Sequencer — step-sequence chords from a diatonic scale."
                    }
                    SeqMode::ChordKb => unreachable!(),
                };
                if ui.button(label).on_hover_text(tip).clicked() && !active {
                    self.seq.playing.store(false, Ordering::Relaxed);
                    self.seq.current_step.store(0, Ordering::Relaxed);
                    self.seq.mode.store(mode.to_u8(), Ordering::Relaxed);
                }
            }

            ui.separator();

            // Play/Stop
            {
                let btn = if seq_playing { "⏹ Stop" } else { "▶ Play" };
                if ui
                    .button(btn)
                    .on_hover_text("Start or stop the sequencer.")
                    .clicked()
                {
                    let new_playing = !seq_playing;
                    self.seq.playing.store(new_playing, Ordering::Relaxed);
                    if new_playing {
                        let bar_quantize = self.seq.bar_quantize.load(Ordering::Relaxed);
                        let current_step = self.seq.current_step.load(Ordering::Relaxed);
                        if bar_quantize && current_step == 0 {
                            if self.seq.arp_restart.load(Ordering::Relaxed) {
                                self.engine.arp_restart();
                                self.seq.arp_restart.store(false, Ordering::Relaxed);
                            }
                            if self.seq.walker_restart.load(Ordering::Relaxed) {
                                self.engine.walker_restart();
                                self.seq.walker_restart.store(false, Ordering::Relaxed);
                            }
                        }
                    }
                }

                // Sequencer BPM — locked to global when seq_sync is active
                let seq_sync_on = self.seq_sync_active();
                if seq_sync_on {
                    self.seq.bpm.store(self.global_bpm, Ordering::Relaxed);
                }
                let mut bpm_val = self.seq.bpm.load(Ordering::Relaxed);
                ui.label("BPM:")
                    .on_hover_text("Sequencer tempo. Follows Global BPM when Sync is enabled.");
                ui.add_enabled_ui(!seq_sync_on, |ui| {
                    if ui
                        .add(egui::Slider::new(&mut bpm_val, 40..=600))
                        .on_hover_text("Sequencer tempo (40–600 BPM).")
                        .changed()
                    {
                        self.seq.bpm.store(bpm_val, Ordering::Relaxed);
                    }
                });
                ui.add_enabled_ui(!self.global_sync, |ui| {
                    let sync_label = egui::RichText::new("Sync").color(if self.seq_sync_active() {
                        self.theme.c(&self.theme.accent)
                    } else {
                        Color32::GRAY
                    });
                    if ui
                        .button(sync_label)
                        .on_hover_text("Lock sequencer BPM to the Global BPM.")
                        .clicked()
                    {
                        self.seq_sync = !self.seq_sync;
                        if self.seq_sync {
                            self.apply_clock_sync();
                        }
                    }
                });

                // Step length selector
                let cur_length = match seq_mode {
                    SeqMode::NoteSeq => self.seq.note_seq.lock().unwrap().length,
                    SeqMode::ChordSeq => self.seq.chord_seq.lock().unwrap().length,
                    SeqMode::ChordKb => unreachable!(),
                };
                ui.label("Steps:")
                    .on_hover_text("Number of steps in the sequencer pattern.");
                for &len in &[8usize, 16, 24] {
                    let active = cur_length == len;
                    let label = egui::RichText::new(format!("{len}")).color(if active {
                        self.theme.c(&self.theme.accent_dim)
                    } else {
                        Color32::GRAY
                    });
                    if ui
                        .button(label)
                        .on_hover_text(format!("Set pattern length to {len} steps."))
                        .clicked()
                    {
                        match seq_mode {
                            SeqMode::NoteSeq => self.seq.note_seq.lock().unwrap().length = len,
                            SeqMode::ChordSeq => self.seq.chord_seq.lock().unwrap().length = len,
                            SeqMode::ChordKb => {}
                        }
                        let current = self.seq.current_step.load(Ordering::Relaxed);
                        if current >= len {
                            self.seq.current_step.store(0, Ordering::Relaxed);
                        }
                    }
                }

                // Random fill
                if ui
                    .button("🎲")
                    .on_hover_text("Randomly fill all steps with notes.")
                    .clicked()
                {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut h = DefaultHasher::new();
                    std::time::SystemTime::now().hash(&mut h);
                    let seed = h.finish();
                    match seq_mode {
                        SeqMode::NoteSeq => {
                            let mut ns = self.seq.note_seq.lock().unwrap();
                            let len = ns.length;
                            for i in 0..len {
                                ns.steps[i] = seed.wrapping_shr(i as u32) & 1 == 1;
                                ns.notes[i] = SEQ_CHROMATIC[(seed.wrapping_shr((i * 3) as u32)
                                    & 0xff)
                                    as usize
                                    % SEQ_CHROMATIC.len()];
                            }
                        }
                        SeqMode::ChordSeq => {
                            let mut cs = self.seq.chord_seq.lock().unwrap();
                            let len = cs.length;
                            for i in 0..len {
                                cs.steps[i] = seed.wrapping_shr(i as u32) & 1 == 1;
                                cs.degrees[i] =
                                    (seed.wrapping_shr((i * 4) as u32) & 0xff) as usize % 7;
                            }
                        }
                        SeqMode::ChordKb => {}
                    }
                }
            }

            // Chord key/scale selector (ChordSeq only)
            if seq_mode == SeqMode::ChordSeq {
                ui.separator();
                ui.label("Key:")
                    .on_hover_text("Root note for the chord scale.");
                let cur_root = self.seq.chord_seq.lock().unwrap().root;
                egui::ComboBox::from_id_salt("chord_root")
                    .selected_text(NOTE_NAMES[cur_root as usize])
                    .show_ui(ui, |ui| {
                        let mut root = cur_root;
                        for (i, name) in NOTE_NAMES.iter().enumerate() {
                            if ui.selectable_value(&mut root, i as u8, *name).changed() {
                                self.seq.chord_seq.lock().unwrap().root = root;
                            }
                        }
                    });
                ui.label("Scale:");
                let cur_scale = self.seq.chord_seq.lock().unwrap().scale;
                for &sc in &[ScaleType::Major, ScaleType::Minor] {
                    let active = cur_scale == sc;
                    let label = egui::RichText::new(sc.label()).color(if active {
                        self.theme.c(&self.theme.accent_dim)
                    } else {
                        Color32::GRAY
                    });
                    if ui
                        .button(label)
                        .on_hover_text(match sc {
                            ScaleType::Major => "Major scale — bright, happy feel.",
                            ScaleType::Minor => "Minor scale — dark, moody feel.",
                        })
                        .clicked()
                    {
                        self.seq.chord_seq.lock().unwrap().scale = sc;
                    }
                }
            }
        });

        ui.add_space(4.0);

        match seq_mode {
            SeqMode::NoteSeq => self.ui_note_seq(ui),
            SeqMode::ChordSeq => self.ui_chord_seq(ui),
            SeqMode::ChordKb => {} // handled in keyboard strip
        }
    }

    fn ui_note_seq(&mut self, ui: &mut egui::Ui) {
        let bar_area_h = 64.0;
        let seq_playing = self.seq.playing.load(Ordering::Relaxed);
        let seq_current_step = self.seq.current_step.load(Ordering::Relaxed);

        let (length, midi_min, midi_max) = {
            let ns = self.seq.note_seq.lock().unwrap();
            (
                ns.length,
                *SEQ_CHROMATIC.first().unwrap() as f32,
                *SEQ_CHROMATIC.last().unwrap() as f32,
            )
        };

        let n = length as f32;
        let spacing = ui.spacing().item_spacing.x;
        let step_w = ((ui.available_width() - spacing * (n - 1.0)) / n).max(28.0);

        ui.horizontal(|ui| {
            for i in 0..length {
                ui.vertical(|ui| {
                    ui.set_width(step_w);
                    let (is_on, note) = {
                        let ns = self.seq.note_seq.lock().unwrap();
                        (ns.steps[i], ns.notes[i])
                    };
                    let is_current = seq_playing && seq_current_step == i;
                    let note_f = note as f32;

                    // Pitch bar
                    let (bar_resp, painter) =
                        ui.allocate_painter(Vec2::new(step_w, bar_area_h), Sense::click_and_drag());
                    let r = bar_resp.rect;
                    painter.rect_filled(
                        r,
                        Rounding::same(4.0),
                        self.theme.c(&self.theme.bg_seq_bar),
                    );
                    let t = (note_f - midi_min) / (midi_max - midi_min);
                    let bar_h = (t * (bar_area_h - 4.0)).max(4.0);
                    let bar_rect = egui::Rect::from_min_size(
                        egui::pos2(r.min.x + 2.0, r.max.y - bar_h - 2.0),
                        Vec2::new(step_w - 4.0, bar_h),
                    );
                    let bar_color = if is_current {
                        self.theme.c(&self.theme.seq_current)
                    } else if is_on {
                        self.theme.c(&self.theme.seq_note_bar_on)
                    } else {
                        self.theme.c(&self.theme.seq_note_bar_off)
                    };
                    painter.rect_filled(bar_rect, Rounding::same(3.0), bar_color);
                    painter.text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        super::midi_note_name(note),
                        egui::FontId::monospace(10.0),
                        if is_on { Color32::WHITE } else { Color32::GRAY },
                    );

                    if bar_resp.dragged() {
                        let mut ns = self.seq.note_seq.lock().unwrap();
                        ns.drag_accum[i] -= bar_resp.drag_delta().y;
                        let steps = ns.drag_accum[i] as i32;
                        if steps != 0 {
                            ns.drag_accum[i] -= steps as f32;
                            let pos = SEQ_CHROMATIC
                                .iter()
                                .position(|&n| n == ns.notes[i])
                                .unwrap_or(0) as i32;
                            let new_pos =
                                (pos + steps).clamp(0, SEQ_CHROMATIC.len() as i32 - 1) as usize;
                            ns.notes[i] = SEQ_CHROMATIC[new_pos];
                        }
                    }
                    if bar_resp.drag_stopped() {
                        self.seq.note_seq.lock().unwrap().drag_accum[i] = 0.0;
                    }

                    // Step button
                    let fill = if is_current {
                        self.theme.c(&self.theme.seq_current)
                    } else if is_on {
                        self.theme.c(&self.theme.seq_step_on)
                    } else {
                        self.theme.c(&self.theme.seq_step_off)
                    };
                    let (r, painter) = ui.allocate_painter(Vec2::new(step_w, 28.0), Sense::click());
                    painter.rect_filled(r.rect, Rounding::same(5.0), fill);
                    painter.rect_stroke(
                        r.rect,
                        Rounding::same(5.0),
                        Stroke::new(
                            1.0,
                            if is_current {
                                Color32::WHITE
                            } else {
                                Color32::GRAY
                            },
                        ),
                    );
                    if r.clicked() {
                        let mut ns = self.seq.note_seq.lock().unwrap();
                        ns.steps[i] = !ns.steps[i];
                    }
                });
            }
        });
    }

    fn ui_chord_seq(&mut self, ui: &mut egui::Ui) {
        let bar_area_h = 64.0;
        let seq_playing = self.seq.playing.load(Ordering::Relaxed);
        let seq_current_step = self.seq.current_step.load(Ordering::Relaxed);

        let (length, scale, root) = {
            let cs = self.seq.chord_seq.lock().unwrap();
            (cs.length, cs.scale, cs.root)
        };

        let n = length as f32;
        let spacing = ui.spacing().item_spacing.x;
        let step_w = ((ui.available_width() - spacing * (n - 1.0)) / n).max(28.0);

        ui.horizontal(|ui| {
            for i in 0..length {
                ui.vertical(|ui| {
                    ui.set_width(step_w);
                    let (is_on, degree) = {
                        let cs = self.seq.chord_seq.lock().unwrap();
                        (cs.steps[i], cs.degrees[i])
                    };
                    let is_current = seq_playing && seq_current_step == i;

                    let (bar_resp, painter) =
                        ui.allocate_painter(Vec2::new(step_w, bar_area_h), Sense::click_and_drag());
                    let r = bar_resp.rect;
                    painter.rect_filled(
                        r,
                        Rounding::same(4.0),
                        self.theme.c(&self.theme.bg_seq_bar),
                    );
                    let t = degree as f32 / 6.0;
                    let bar_h = (t * (bar_area_h - 4.0)).max(4.0);
                    let bar_rect = egui::Rect::from_min_size(
                        egui::pos2(r.min.x + 2.0, r.max.y - bar_h - 2.0),
                        Vec2::new(step_w - 4.0, bar_h),
                    );
                    let quality = chord_quality(scale, degree);
                    let bar_color = if is_current {
                        self.theme.c(&self.theme.seq_current)
                    } else if !is_on {
                        self.theme.c(&self.theme.seq_note_bar_off)
                    } else if quality == "m" {
                        self.theme.c(&self.theme.seq_chord_minor)
                    } else if quality == "°" {
                        self.theme.c(&self.theme.seq_chord_dim)
                    } else {
                        self.theme.c(&self.theme.seq_chord_major)
                    };
                    painter.rect_filled(bar_rect, Rounding::same(3.0), bar_color);

                    let cname = chord_name(root, scale, degree);
                    painter.text(
                        egui::pos2(r.center().x, r.center().y - 6.0),
                        egui::Align2::CENTER_CENTER,
                        &cname,
                        egui::FontId::monospace(9.0),
                        if is_on { Color32::WHITE } else { Color32::GRAY },
                    );
                    painter.text(
                        egui::pos2(r.center().x, r.center().y + 7.0),
                        egui::Align2::CENTER_CENTER,
                        DEGREE_LABELS[degree],
                        egui::FontId::monospace(8.0),
                        if is_on {
                            Color32::from_rgb(180, 180, 180)
                        } else {
                            Color32::from_rgb(80, 80, 80)
                        },
                    );

                    if bar_resp.dragged() {
                        let mut cs = self.seq.chord_seq.lock().unwrap();
                        cs.drag_accum[i] -= bar_resp.drag_delta().y;
                        let steps = cs.drag_accum[i] as i32;
                        if steps != 0 {
                            cs.drag_accum[i] -= steps as f32;
                            cs.degrees[i] = (degree as i32 + steps).clamp(0, 6) as usize;
                        }
                    }
                    if bar_resp.drag_stopped() {
                        self.seq.chord_seq.lock().unwrap().drag_accum[i] = 0.0;
                    }

                    let fill = if is_current {
                        self.theme.c(&self.theme.seq_current)
                    } else if is_on {
                        self.theme.c(&self.theme.seq_step_on)
                    } else {
                        self.theme.c(&self.theme.seq_step_off)
                    };
                    let (r, painter) = ui.allocate_painter(Vec2::new(step_w, 28.0), Sense::click());
                    painter.rect_filled(r.rect, Rounding::same(5.0), fill);
                    painter.rect_stroke(
                        r.rect,
                        Rounding::same(5.0),
                        Stroke::new(
                            1.0,
                            if is_current {
                                Color32::WHITE
                            } else {
                                Color32::GRAY
                            },
                        ),
                    );
                    if r.clicked() {
                        let mut cs = self.seq.chord_seq.lock().unwrap();
                        cs.steps[i] = !cs.steps[i];
                    }
                });
            }
        });
    }
}
