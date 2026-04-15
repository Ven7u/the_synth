use crate::SynthApp;
use crate::sequencer::{
    SeqMode, ScaleType, NOTE_NAMES, DEGREE_LABELS, chord_name, chord_quality,
};
use synth_control::ControlEvent;
use eframe::egui;
use egui::{Color32, Rounding, Sense, Stroke, Vec2};

const SEQ_CHROMATIC: &[u8] = &[
    36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
    48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59,
    60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71,
    72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83,
    84,
];

impl SynthApp {
    pub fn ui_sequencer_panel(&mut self, ui: &mut egui::Ui) {
        // --- Shared toolbar ---
        ui.horizontal(|ui| {
            // Mode tabs
            for &mode in &[SeqMode::NoteSeq, SeqMode::ChordSeq, SeqMode::ChordKb] {
                let active = self.seq_mode == mode;
                let label = egui::RichText::new(mode.label())
                    .color(if active { self.theme.c(&self.theme.accent) } else { Color32::GRAY })
                    .strong();
                let tip = match mode {
                    SeqMode::NoteSeq  => "Note Sequencer — step-sequence individual notes.",
                    SeqMode::ChordSeq => "Chord Sequencer — step-sequence chords from a diatonic scale.",
                    SeqMode::ChordKb  => "Chord Keyboard — play chords live via keyboard keys (A–J = degrees I–VII).",
                };
                if ui.button(label).on_hover_text(tip).clicked() && !active {
                    let prev: Vec<u8> = self.seq_prev_notes.drain(..).collect();
                    for m in prev { self.push_note_off(m); }
                    self.seq_playing = false;
                    self.seq_current_step = 0;
                    self.seq_mode = mode;
                }
            }

            ui.separator();

            // Play/Stop — only for sequencer modes
            if self.seq_mode != SeqMode::ChordKb {
                let btn = if self.seq_playing { "⏹ Stop" } else { "▶ Play" };
                if ui.button(btn).on_hover_text("Start or stop the sequencer.").clicked() {
                    self.seq_playing = !self.seq_playing;
                    if self.seq_playing {
                        self.seq_last_tick = std::time::Instant::now();
                        if self.seq_bar_quantize_start && self.seq_current_step == 0 {
                            if self.arp_restart_pending {
                                let _ = self.control.try_send(ControlEvent::ArpRestart { track: 0 });
                                self.arp_restart_pending = false;
                            }
                            if self.walker_restart_pending {
                                let _ = self.control.try_send(ControlEvent::WalkerRestart { track: 0 });
                                self.walker_restart_pending = false;
                            }
                        }
                    } else {
                        let prev: Vec<u8> = self.seq_prev_notes.drain(..).collect();
                        for m in prev { self.push_note_off(m); }
                    }
                }
                ui.label("BPM:").on_hover_text("Sequencer tempo in beats per minute.");
                ui.add(egui::Slider::new(&mut self.seq_bpm, 40..=600))
                    .on_hover_text("Sequencer tempo (40–600 BPM).");
                if self.seq_clock_sync {
                    self.apply_clock_sync();
                }

                // Step length selector
                let cur_length = match self.seq_mode {
                    SeqMode::NoteSeq  => &mut self.note_seq.length,
                    SeqMode::ChordSeq => &mut self.chord_seq.length,
                    SeqMode::ChordKb  => unreachable!(),
                };
                ui.label("Steps:").on_hover_text("Number of steps in the sequencer pattern.");
                for &len in &[8usize, 16, 24] {
                    let active = *cur_length == len;
                    let label = egui::RichText::new(format!("{len}"))
                        .color(if active { self.theme.c(&self.theme.accent_dim) } else { Color32::GRAY });
                    if ui.button(label).on_hover_text(format!("Set pattern length to {len} steps.")).clicked() {
                        *cur_length = len;
                        if self.seq_current_step >= len { self.seq_current_step = 0; }
                    }
                }

                // Random fill
                if ui.button("🎲").on_hover_text("Randomly fill all steps with notes.").clicked() {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut h = DefaultHasher::new();
                    std::time::SystemTime::now().hash(&mut h);
                    let seed = h.finish();
                    match self.seq_mode {
                        SeqMode::NoteSeq => {
                            let len = self.note_seq.length;
                            for i in 0..len {
                                self.note_seq.steps[i] = seed.wrapping_shr(i as u32) & 1 == 1;
                                self.note_seq.notes[i] = SEQ_CHROMATIC[
                                    (seed.wrapping_shr((i * 3) as u32) & 0xff) as usize
                                    % SEQ_CHROMATIC.len()
                                ];
                            }
                        }
                        SeqMode::ChordSeq => {
                            let len = self.chord_seq.length;
                            for i in 0..len {
                                self.chord_seq.steps[i] = seed.wrapping_shr(i as u32) & 1 == 1;
                                self.chord_seq.degrees[i] =
                                    (seed.wrapping_shr((i * 4) as u32) & 0xff) as usize % 7;
                            }
                        }
                        SeqMode::ChordKb => {}
                    }
                }
            }

            ui.separator();
            if ui.checkbox(&mut self.seq_clock_sync, "Clock Sync")
                .on_hover_text("Lock ARP + Walker BPM to sequencer BPM and align phases.")
                .changed() && self.seq_clock_sync
            {
                self.apply_clock_sync();
                self.sync_transport_now();
            }
            if !self.seq_clock_sync {
                self.arp_restart_pending = false;
                self.walker_restart_pending = false;
            }
            ui.add_enabled_ui(self.seq_clock_sync, |ui| {
                ui.checkbox(&mut self.seq_bar_quantize_start, "Bar Quantize")
                    .on_hover_text("When enabled, ARP/WALKER restart on the next bar instead of immediately.");
            });
            if ui.button("SYNC NOW")
                .on_hover_text("Restart sequencer, arpeggiator, and walker phase together.")
                .clicked()
            {
                self.apply_clock_sync();
                self.sync_transport_now();
            }

            // Chord key/scale selector (ChordSeq and ChordKb)
            if self.seq_mode == SeqMode::ChordSeq || self.seq_mode == SeqMode::ChordKb {
                ui.separator();
                let (root, scale) = match self.seq_mode {
                    SeqMode::ChordSeq => (&mut self.chord_seq.root, &mut self.chord_seq.scale),
                    SeqMode::ChordKb  => (&mut self.chord_kb.root,  &mut self.chord_kb.scale),
                    _ => unreachable!(),
                };
                ui.label("Key:").on_hover_text("Root note for the chord scale.");
                egui::ComboBox::from_id_salt("chord_root")
                    .selected_text(NOTE_NAMES[*root as usize])
                    .show_ui(ui, |ui| {
                        for (i, name) in NOTE_NAMES.iter().enumerate() {
                            ui.selectable_value(root, i as u8, *name);
                        }
                    });
                ui.label("Scale:").on_hover_text("Diatonic scale used to build chords (Major = bright, Minor = dark).");
                for &sc in &[ScaleType::Major, ScaleType::Minor] {
                    let active = *scale == sc;
                    let label = egui::RichText::new(sc.label())
                        .color(if active { self.theme.c(&self.theme.accent_dim) } else { Color32::GRAY });
                    if ui.button(label).on_hover_text(match sc {
                        ScaleType::Major => "Major scale — bright, happy feel.",
                        ScaleType::Minor => "Minor scale — dark, moody feel.",
                    }).clicked() { *scale = sc; }
                }
            }
        });

        ui.add_space(4.0);

        match self.seq_mode {
            SeqMode::NoteSeq  => self.ui_note_seq(ui),
            SeqMode::ChordSeq => self.ui_chord_seq(ui),
            SeqMode::ChordKb  => self.ui_chord_kb(ui),
        }
    }

    fn ui_note_seq(&mut self, ui: &mut egui::Ui) {
        let bar_area_h = 64.0;
        let n = self.note_seq.length as f32;
        let spacing = ui.spacing().item_spacing.x;
        let step_w = ((ui.available_width() - spacing * (n - 1.0)) / n).max(28.0);
        let midi_min = *SEQ_CHROMATIC.first().unwrap() as f32;
        let midi_max = *SEQ_CHROMATIC.last().unwrap() as f32;

        ui.horizontal(|ui| {
            for i in 0..self.note_seq.length {
                ui.vertical(|ui| {
                    ui.set_width(step_w);
                    let is_current = self.seq_playing && self.seq_current_step == i;
                    let is_on = self.note_seq.steps[i];
                    let note = self.note_seq.notes[i] as f32;

                    // Pitch bar
                    let (bar_resp, painter) = ui.allocate_painter(
                        Vec2::new(step_w, bar_area_h), Sense::click_and_drag());
                    let r = bar_resp.rect;
                    painter.rect_filled(r, Rounding::same(4.0), self.theme.c(&self.theme.bg_seq_bar));
                    let t = (note - midi_min) / (midi_max - midi_min);
                    let bar_h = (t * (bar_area_h - 4.0)).max(4.0);
                    let bar_rect = egui::Rect::from_min_size(
                        egui::pos2(r.min.x + 2.0, r.max.y - bar_h - 2.0),
                        Vec2::new(step_w - 4.0, bar_h),
                    );
                    let bar_color = if is_current { self.theme.c(&self.theme.seq_current) }
                        else if is_on { self.theme.c(&self.theme.seq_note_bar_on) }
                        else { self.theme.c(&self.theme.seq_note_bar_off) };
                    painter.rect_filled(bar_rect, Rounding::same(3.0), bar_color);
                    painter.text(r.center(), egui::Align2::CENTER_CENTER,
                        super::midi_note_name(self.note_seq.notes[i]),
                        egui::FontId::monospace(10.0),
                        if is_on { Color32::WHITE } else { Color32::GRAY });

                    if bar_resp.dragged() {
                        self.note_seq.drag_accum[i] -= bar_resp.drag_delta().y;
                        let steps = self.note_seq.drag_accum[i] as i32;
                        if steps != 0 {
                            self.note_seq.drag_accum[i] -= steps as f32;
                            let pos = SEQ_CHROMATIC.iter()
                                .position(|&n| n == self.note_seq.notes[i]).unwrap_or(0) as i32;
                            let new_pos = (pos + steps).clamp(0, SEQ_CHROMATIC.len() as i32 - 1) as usize;
                            self.note_seq.notes[i] = SEQ_CHROMATIC[new_pos];
                        }
                    }
                    if bar_resp.drag_stopped() { self.note_seq.drag_accum[i] = 0.0; }

                    // Step button
                    let fill = if is_current { self.theme.c(&self.theme.seq_current) }
                        else if is_on { self.theme.c(&self.theme.seq_step_on) }
                        else { self.theme.c(&self.theme.seq_step_off) };
                    let (r, painter) = ui.allocate_painter(Vec2::new(step_w, 28.0), Sense::click());
                    painter.rect_filled(r.rect, Rounding::same(5.0), fill);
                    painter.rect_stroke(r.rect, Rounding::same(5.0),
                        Stroke::new(1.0, if is_current { Color32::WHITE } else { Color32::GRAY }));
                    if r.clicked() { self.note_seq.steps[i] = !self.note_seq.steps[i]; }
                });
            }
        });
    }

    fn ui_chord_seq(&mut self, ui: &mut egui::Ui) {
        let bar_area_h = 64.0;
        let n = self.chord_seq.length as f32;
        let spacing = ui.spacing().item_spacing.x;
        let step_w = ((ui.available_width() - spacing * (n - 1.0)) / n).max(28.0);

        ui.horizontal(|ui| {
            for i in 0..self.chord_seq.length {
                ui.vertical(|ui| {
                    ui.set_width(step_w);
                    let is_current = self.seq_playing && self.seq_current_step == i;
                    let is_on = self.chord_seq.steps[i];
                    let degree = self.chord_seq.degrees[i];

                    let (bar_resp, painter) = ui.allocate_painter(
                        Vec2::new(step_w, bar_area_h), Sense::click_and_drag());
                    let r = bar_resp.rect;
                    painter.rect_filled(r, Rounding::same(4.0), self.theme.c(&self.theme.bg_seq_bar));
                    let t = degree as f32 / 6.0;
                    let bar_h = (t * (bar_area_h - 4.0)).max(4.0);
                    let bar_rect = egui::Rect::from_min_size(
                        egui::pos2(r.min.x + 2.0, r.max.y - bar_h - 2.0),
                        Vec2::new(step_w - 4.0, bar_h),
                    );
                    let quality = chord_quality(self.chord_seq.scale, degree);
                    let bar_color = if is_current { self.theme.c(&self.theme.seq_current) }
                        else if !is_on { self.theme.c(&self.theme.seq_note_bar_off) }
                        else if quality == "m" { self.theme.c(&self.theme.seq_chord_minor) }
                        else if quality == "°" { self.theme.c(&self.theme.seq_chord_dim) }
                        else { self.theme.c(&self.theme.seq_chord_major) };
                    painter.rect_filled(bar_rect, Rounding::same(3.0), bar_color);

                    let cname = chord_name(self.chord_seq.root, self.chord_seq.scale, degree);
                    painter.text(egui::pos2(r.center().x, r.center().y - 6.0),
                        egui::Align2::CENTER_CENTER, &cname,
                        egui::FontId::monospace(9.0),
                        if is_on { Color32::WHITE } else { Color32::GRAY });
                    painter.text(egui::pos2(r.center().x, r.center().y + 7.0),
                        egui::Align2::CENTER_CENTER, DEGREE_LABELS[degree],
                        egui::FontId::monospace(8.0),
                        if is_on { Color32::from_rgb(180, 180, 180) } else { Color32::from_rgb(80,80,80) });

                    if bar_resp.dragged() {
                        self.chord_seq.drag_accum[i] -= bar_resp.drag_delta().y;
                        let steps = self.chord_seq.drag_accum[i] as i32;
                        if steps != 0 {
                            self.chord_seq.drag_accum[i] -= steps as f32;
                            self.chord_seq.degrees[i] =
                                (degree as i32 + steps).clamp(0, 6) as usize;
                        }
                    }
                    if bar_resp.drag_stopped() { self.chord_seq.drag_accum[i] = 0.0; }

                    let fill = if is_current { self.theme.c(&self.theme.seq_current) }
                        else if is_on { self.theme.c(&self.theme.seq_step_on) }
                        else { self.theme.c(&self.theme.seq_step_off) };
                    let (r, painter) = ui.allocate_painter(Vec2::new(step_w, 28.0), Sense::click());
                    painter.rect_filled(r.rect, Rounding::same(5.0), fill);
                    painter.rect_stroke(r.rect, Rounding::same(5.0),
                        Stroke::new(1.0, if is_current { Color32::WHITE } else { Color32::GRAY }));
                    if r.clicked() { self.chord_seq.steps[i] = !self.chord_seq.steps[i]; }
                });
            }
        });
    }

    fn ui_chord_kb(&mut self, ui: &mut egui::Ui) {
        let spacing = ui.spacing().item_spacing.x;
        let btn_w = ((ui.available_width() - spacing * 6.0) / 7.0).max(40.0);
        let btn_h = 90.0;

        ui.horizontal(|ui| {
            for degree in 0..7 {
                let (resp, painter) = ui.allocate_painter(
                    Vec2::new(btn_w, btn_h), Sense::click_and_drag());
                let r = resp.rect;

                let is_held_mouse = self.chord_kb.held_degree == Some(degree);
                let is_held_kb    = self.chord_kb.kb_held.contains(&degree);
                let is_held = is_held_mouse || is_held_kb;
                let quality = chord_quality(self.chord_kb.scale, degree);
                let bg = if is_held { self.theme.c(&self.theme.seq_current) }
                    else if quality == "m" { self.theme.c(&self.theme.seq_kb_minor) }
                    else if quality == "°" { self.theme.c(&self.theme.seq_kb_dim) }
                    else { self.theme.c(&self.theme.seq_kb_major) };
                painter.rect_filled(r, Rounding::same(8.0), bg);
                painter.rect_stroke(r, Rounding::same(8.0),
                    Stroke::new(if is_held { 2.0 } else { 1.0 },
                    if is_held { Color32::WHITE } else { Color32::from_gray(80) }));

                let cname = chord_name(self.chord_kb.root, self.chord_kb.scale, degree);
                painter.text(egui::pos2(r.center().x, r.center().y - 10.0),
                    egui::Align2::CENTER_CENTER, &cname,
                    egui::FontId::proportional(14.0), Color32::WHITE);
                painter.text(egui::pos2(r.center().x, r.center().y + 10.0),
                    egui::Align2::CENTER_CENTER, DEGREE_LABELS[degree],
                    egui::FontId::monospace(10.0), Color32::from_gray(180));

                if resp.is_pointer_button_down_on() && !is_held_mouse {
                    if let Some(prev) = self.chord_kb.held_degree {
                        for m in self.chord_kb.chord_notes(prev) { self.push_note_off(m); }
                    }
                    self.chord_kb.held_degree = Some(degree);
                    for m in self.chord_kb.chord_notes(degree) { self.push_note_on(m); }
                }
                if !resp.is_pointer_button_down_on() && is_held_mouse {
                    self.chord_kb.held_degree = None;
                    for m in self.chord_kb.chord_notes(degree) { self.push_note_off(m); }
                }
            }
        });
    }
}
