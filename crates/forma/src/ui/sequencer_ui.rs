use crate::sequencer::{
    chord_name, chord_quality, ScaleType, SeqClockDiv, SeqMode, DEGREE_LABELS, NOTE_NAMES,
};
use crate::SynthApp;
use eframe::egui;
use egui::{Color32, CornerRadius, Sense, Stroke, StrokeKind, Vec2};
use std::sync::atomic::Ordering;
use std::sync::Arc;

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
                let bar_quantize = self.seq.bar_quantize.load(Ordering::Relaxed);
                let btn = if seq_playing {
                    "■ Stop"
                } else if self.seq_pending_start {
                    "… Bar"
                } else {
                    "▶ Play"
                };
                let tip = if self.seq_pending_start {
                    "Waiting for the next bar boundary — click to cancel."
                } else if seq_playing {
                    "Stop the sequencer."
                } else if bar_quantize {
                    "Start the sequencer on the next bar boundary (bar-quantize is on)."
                } else {
                    "Start the sequencer immediately."
                };
                if ui.button(btn).on_hover_text(tip).clicked() {
                    if seq_playing || self.seq_pending_start {
                        // Stop: reset to step 0 so next Play starts from the beginning.
                        self.seq.playing.store(false, Ordering::Relaxed);
                        self.seq.current_step.store(0, Ordering::Relaxed);
                        self.seq_pending_start = false;
                    } else if bar_quantize {
                        // Defer: sequencer fires on next bar boundary detected in tick_metronome.
                        self.seq_pending_start = true;
                        self.seq.current_step.store(0, Ordering::Relaxed);
                    } else {
                        // Start from step 0, align metronome to beat 1.
                        self.seq.current_step.store(0, Ordering::Relaxed);
                        self.seq.playing.store(true, Ordering::Relaxed);
                        self.metro_reset();
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

                // Record button — step entry (stopped) or live overdub (playing).
                // NoteSeq records notes directly; ChordSeq maps played notes to scale degrees.
                if seq_mode != crate::sequencer::SeqMode::ChordKb {
                    let recording = self.seq.recording.load(Ordering::Relaxed);
                    let rec_label = egui::RichText::new("● REC")
                        .color(if recording {
                            egui::Color32::from_rgb(220, 50, 50)
                        } else {
                            Color32::GRAY
                        })
                        .strong();
                    let rec_tip = if recording {
                        if seq_playing {
                            "Live overdub active — notes you play overwrite the current step. Click to stop recording."
                        } else {
                            "Step entry active — each key press fills the highlighted step and advances. Click to stop."
                        }
                    } else if seq_playing {
                        "Start live overdub — notes you play will overwrite steps as the sequencer runs."
                    } else {
                        "Start step entry — press keys to fill steps one by one."
                    };
                    if ui.button(rec_label).on_hover_text(rec_tip).clicked() {
                        let next = !recording;
                        self.seq.recording.store(next, Ordering::Relaxed);
                        if next && !seq_playing {
                            // Reset step cursor to beginning when starting step entry.
                            self.seq.rec_step.store(0, Ordering::Relaxed);
                        }
                    }

                    // REST and ← only matter in step-entry mode (stopped + recording).
                    if recording && !seq_playing {
                        if ui
                            .button("REST")
                            .on_hover_text("Insert a rest (empty step) and advance.")
                            .clicked()
                        {
                            self.seq_record_rest();
                        }
                        if ui
                            .button("←")
                            .on_hover_text("Go back one step.")
                            .clicked()
                        {
                            self.seq_record_back();
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

                // Clock division selector
                ui.label("Div:")
                    .on_hover_text("Duration of each step. Short values = fast sequencing; long values = slow chord changes.");
                let (cur_div, div_atomic) = match seq_mode {
                    SeqMode::NoteSeq => (
                        self.seq.note_div.load(Ordering::Relaxed),
                        Arc::clone(&self.seq.note_div),
                    ),
                    SeqMode::ChordSeq => (
                        self.seq.chord_div.load(Ordering::Relaxed),
                        Arc::clone(&self.seq.chord_div),
                    ),
                    SeqMode::ChordKb => unreachable!(),
                };
                for (i, &label) in SeqClockDiv::LABELS.iter().enumerate() {
                    let active = cur_div == i as u8;
                    let col = if active {
                        self.theme.c(&self.theme.accent_dim)
                    } else {
                        Color32::GRAY
                    };
                    if ui
                        .button(egui::RichText::new(label).color(col))
                        .on_hover_text(format!("Step duration: {} note/bar(s).", label))
                        .clicked()
                        && !active
                    {
                        div_atomic.store(i as u8, Ordering::Relaxed);
                        // Also persist to SynthApp mirror
                        match seq_mode {
                            SeqMode::NoteSeq => self.note_seq_div = i as u8,
                            SeqMode::ChordSeq => self.chord_seq_div = i as u8,
                            SeqMode::ChordKb => {}
                        }
                    }
                }

                // Gate length and timing humanization.
                ui.separator();
                ui.label("Gate:").on_hover_text("Note gate length — how long each note is held within its step. 100% = hold until next step, lower values = shorter, more staccato notes.");
                let mut gate = self.seq.gate.load(Ordering::Relaxed);
                if ui
                    .add(egui::Slider::new(&mut gate, 1u8..=100).suffix("%").fixed_decimals(0))
                    .on_hover_text("1% = very staccato, 90% = slight separation (default), 100% = legato/tied.")
                    .changed()
                {
                    self.seq.gate.store(gate, Ordering::Relaxed);
                }
                ui.label("Human:").on_hover_text("Timing humanization — each step fires slightly early or late by a random amount. 0 = perfectly on-grid.");
                let mut human = self.seq.humanize.load(Ordering::Relaxed);
                if ui
                    .add(egui::Slider::new(&mut human, 0u8..=100).suffix("%").fixed_decimals(0))
                    .on_hover_text("0% = robot-tight, 100% = maximum humanization (±45% of step duration).")
                    .changed()
                {
                    self.seq.humanize.store(human, Ordering::Relaxed);
                }

                ui.separator();

                // Random fill
                if ui
                    .button("RND")
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

                // Euclidean rhythm generator.
                let euclid_label = egui::RichText::new("EUCLID").color(
                    if self.seq_euclid_open { self.theme.c(&self.theme.accent) } else { Color32::GRAY }
                );
                if ui.button(euclid_label).on_hover_text("Generate a Euclidean (evenly-spaced) rhythm pattern.").clicked() {
                    self.seq_euclid_open = !self.seq_euclid_open;
                    if self.seq_euclid_open {
                        // Pre-fill hits to half the current pattern length.
                        let cur_len = match seq_mode {
                            SeqMode::NoteSeq => self.seq.note_seq.lock().unwrap().length,
                            SeqMode::ChordSeq => self.seq.chord_seq.lock().unwrap().length,
                            SeqMode::ChordKb => 8,
                        };
                        self.seq_euclid_hits = self.seq_euclid_hits.min(cur_len);
                    }
                }
            }

            // Euclidean controls (shown inline below toolbar when open).
            if self.seq_euclid_open && seq_mode != SeqMode::ChordKb {
                let cur_len = match seq_mode {
                    SeqMode::NoteSeq => self.seq.note_seq.lock().unwrap().length,
                    SeqMode::ChordSeq => self.seq.chord_seq.lock().unwrap().length,
                    SeqMode::ChordKb => 8,
                };
                self.seq_euclid_hits = self.seq_euclid_hits.clamp(1, cur_len);
                self.seq_euclid_offset = self.seq_euclid_offset % cur_len;
                ui.horizontal(|ui| {
                    ui.label("Hits:");
                    let mut h = self.seq_euclid_hits;
                    if ui.add(egui::Slider::new(&mut h, 1..=cur_len)).changed() {
                        self.seq_euclid_hits = h;
                    }
                    ui.label("Offset:");
                    let mut off = self.seq_euclid_offset;
                    if ui.add(egui::Slider::new(&mut off, 0..=cur_len - 1)).changed() {
                        self.seq_euclid_offset = off;
                    }
                    if ui.button("Apply").on_hover_text("Fill the step on/off pattern with the Euclidean rhythm.").clicked() {
                        let pattern = crate::sequencer::bjorklund(self.seq_euclid_hits, cur_len, self.seq_euclid_offset);
                        match seq_mode {
                            SeqMode::NoteSeq => {
                                let mut ns = self.seq.note_seq.lock().unwrap();
                                for (i, &on) in pattern.iter().enumerate() {
                                    ns.steps[i] = on;
                                }
                            }
                            SeqMode::ChordSeq => {
                                let mut cs = self.seq.chord_seq.lock().unwrap();
                                for (i, &on) in pattern.iter().enumerate() {
                                    cs.steps[i] = on;
                                }
                            }
                            SeqMode::ChordKb => {}
                        }
                        self.seq_euclid_open = false;
                    }
                    if ui.button("✕").clicked() {
                        self.seq_euclid_open = false;
                    }
                });
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
        let recording = self.seq.recording.load(Ordering::Relaxed);
        let rec_step = self.seq.rec_step.load(Ordering::Relaxed);

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
                    // Step-entry cursor: highlight when stopped + recording.
                    let is_rec_cursor = recording && !seq_playing && rec_step == i;
                    let note_f = note as f32;

                    // Pitch bar
                    let (bar_resp, painter) =
                        ui.allocate_painter(Vec2::new(step_w, bar_area_h), Sense::click_and_drag());
                    let r = bar_resp.rect;
                    painter.rect_filled(
                        r,
                        CornerRadius::same(4),
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
                    painter.rect_filled(bar_rect, CornerRadius::same(3), bar_color);
                    // Red border on rec cursor step.
                    if is_rec_cursor {
                        painter.rect_stroke(
                            r,
                            CornerRadius::same(4),
                            Stroke::new(2.0, egui::Color32::from_rgb(220, 50, 50)),
                            StrokeKind::Middle,
                        );
                    }
                    painter.text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        super::midi_note_name(note),
                        egui::FontId::monospace(10.0),
                        if is_on { Color32::WHITE } else { Color32::GRAY },
                    );

                    if bar_resp.dragged() {
                        let mut ns = self.seq.note_seq.lock().unwrap();
                        ns.drag_accum[i] -= bar_resp.drag_delta().y * 0.3;
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
                    // Scroll wheel: 1 tick = 1 semitone.
                    if bar_resp.hovered() {
                        let scroll = ui.input(|inp| inp.smooth_scroll_delta.y);
                        if scroll != 0.0 {
                            let mut ns = self.seq.note_seq.lock().unwrap();
                            let pos = SEQ_CHROMATIC
                                .iter()
                                .position(|&n| n == ns.notes[i])
                                .unwrap_or(0) as i32;
                            let delta = if scroll > 0.0 { 1i32 } else { -1 };
                            let new_pos =
                                (pos + delta).clamp(0, SEQ_CHROMATIC.len() as i32 - 1) as usize;
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
                    painter.rect_filled(r.rect, CornerRadius::same(5), fill);
                    painter.rect_stroke(
                        r.rect,
                        CornerRadius::same(5),
                        Stroke::new(
                            1.0,
                            if is_current {
                                Color32::WHITE
                            } else {
                                Color32::GRAY
                            },
                        ),
                        StrokeKind::Middle,
                    );
                    if r.clicked() {
                        let mut ns = self.seq.note_seq.lock().unwrap();
                        ns.steps[i] = !ns.steps[i];
                    }

                    // Velocity bar — click/drag sets 0-127 by x position.
                    let vel_h = 14.0;
                    let vel = self.seq.note_seq.lock().unwrap().velocities[i];
                    let (vel_resp, painter) =
                        ui.allocate_painter(Vec2::new(step_w, vel_h), Sense::click_and_drag());
                    let vr = vel_resp.rect;
                    painter.rect_filled(vr, CornerRadius::same(2), Color32::from_rgb(30, 30, 40));
                    let fill_w = vel as f32 / 127.0 * vr.width();
                    painter.rect_filled(
                        egui::Rect::from_min_size(vr.min, Vec2::new(fill_w, vel_h)),
                        CornerRadius::same(2),
                        Color32::from_rgb(80, 140, 200),
                    );
                    painter.text(
                        vr.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{vel}"),
                        egui::FontId::monospace(8.0),
                        Color32::from_rgb(180, 180, 180),
                    );
                    if vel_resp.dragged() || vel_resp.clicked() {
                        if let Some(pos) = vel_resp.interact_pointer_pos() {
                            let t = ((pos.x - vr.min.x) / vr.width()).clamp(0.0, 1.0);
                            self.seq.note_seq.lock().unwrap().velocities[i] =
                                (t * 127.0).round() as u8;
                        }
                    }

                    // Probability bar — click/drag sets 0-100 by x position.
                    let prob_h = 10.0;
                    let prob = self.seq.note_seq.lock().unwrap().probabilities[i];
                    let (prob_resp, painter) =
                        ui.allocate_painter(Vec2::new(step_w, prob_h), Sense::click_and_drag());
                    let pr = prob_resp.rect;
                    painter.rect_filled(pr, CornerRadius::same(2), Color32::from_rgb(30, 30, 40));
                    let pfill_w = prob as f32 / 100.0 * pr.width();
                    let prob_color = if prob >= 100 {
                        Color32::from_rgb(60, 160, 80)
                    } else if prob >= 50 {
                        Color32::from_rgb(180, 140, 40)
                    } else {
                        Color32::from_rgb(180, 70, 50)
                    };
                    painter.rect_filled(
                        egui::Rect::from_min_size(pr.min, Vec2::new(pfill_w, prob_h)),
                        CornerRadius::same(2),
                        prob_color,
                    );
                    if prob_resp.dragged() || prob_resp.clicked() {
                        if let Some(pos) = prob_resp.interact_pointer_pos() {
                            let t = ((pos.x - pr.min.x) / pr.width()).clamp(0.0, 1.0);
                            self.seq.note_seq.lock().unwrap().probabilities[i] =
                                (t * 100.0).round() as u8;
                        }
                    }
                });
            }
        });
    }

    fn ui_chord_seq(&mut self, ui: &mut egui::Ui) {
        let bar_area_h = 64.0;
        let seq_playing = self.seq.playing.load(Ordering::Relaxed);
        let seq_current_step = self.seq.current_step.load(Ordering::Relaxed);
        let recording = self.seq.recording.load(Ordering::Relaxed);
        let rec_step = self.seq.rec_step.load(Ordering::Relaxed);

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
                    let (is_on, degree, chord_type, oct_off) = {
                        let cs = self.seq.chord_seq.lock().unwrap();
                        (
                            cs.steps[i],
                            cs.degrees[i],
                            cs.chord_types[i],
                            cs.octave_offsets[i],
                        )
                    };
                    let is_current = seq_playing && seq_current_step == i;
                    let is_rec_cursor = recording && !seq_playing && rec_step == i;

                    let (bar_resp, painter) =
                        ui.allocate_painter(Vec2::new(step_w, bar_area_h), Sense::click_and_drag());
                    let r = bar_resp.rect;
                    painter.rect_filled(
                        r,
                        CornerRadius::same(4),
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
                    painter.rect_filled(bar_rect, CornerRadius::same(3), bar_color);
                    // Rec cursor border.
                    if is_rec_cursor {
                        painter.rect_stroke(
                            r,
                            CornerRadius::same(4),
                            Stroke::new(2.0, egui::Color32::from_rgb(220, 50, 50)),
                            StrokeKind::Middle,
                        );
                    }

                    let cname = chord_name(root, scale, degree);
                    painter.text(
                        egui::pos2(r.center().x, r.min.y + 10.0),
                        egui::Align2::CENTER_CENTER,
                        &cname,
                        egui::FontId::monospace(9.0),
                        if is_on { Color32::WHITE } else { Color32::GRAY },
                    );
                    painter.text(
                        egui::pos2(r.center().x, r.min.y + 22.0),
                        egui::Align2::CENTER_CENTER,
                        DEGREE_LABELS[degree],
                        egui::FontId::monospace(8.0),
                        if is_on {
                            Color32::from_rgb(180, 180, 180)
                        } else {
                            Color32::from_rgb(80, 80, 80)
                        },
                    );
                    // Chord type label (bottom of bar).
                    painter.text(
                        egui::pos2(r.center().x, r.max.y - 8.0),
                        egui::Align2::CENTER_CENTER,
                        chord_type.label(),
                        egui::FontId::monospace(7.0),
                        if is_on {
                            Color32::from_rgb(160, 200, 160)
                        } else {
                            Color32::from_rgb(70, 90, 70)
                        },
                    );

                    // Left drag: change degree.
                    if bar_resp.dragged() {
                        let mut cs = self.seq.chord_seq.lock().unwrap();
                        cs.drag_accum[i] -= bar_resp.drag_delta().y * 0.6;
                        let steps = cs.drag_accum[i] as i32;
                        if steps != 0 {
                            cs.drag_accum[i] -= steps as f32;
                            cs.degrees[i] = (degree as i32 + steps).clamp(0, 6) as usize;
                        }
                    }
                    if bar_resp.drag_stopped() {
                        self.seq.chord_seq.lock().unwrap().drag_accum[i] = 0.0;
                    }
                    // Scroll wheel: change degree.
                    if bar_resp.hovered() {
                        let scroll = ui.input(|inp| inp.smooth_scroll_delta.y);
                        if scroll != 0.0 {
                            let delta = if scroll > 0.0 { 1i32 } else { -1 };
                            let mut cs = self.seq.chord_seq.lock().unwrap();
                            cs.degrees[i] = (degree as i32 + delta).clamp(0, 6) as usize;
                        }
                    }
                    // Right-click: cycle chord type.
                    if bar_resp.secondary_clicked() {
                        use crate::sequencer::ChordType;
                        let all = ChordType::all();
                        let cur_idx = all.iter().position(|&t| t == chord_type).unwrap_or(0);
                        let next = all[(cur_idx + 1) % all.len()];
                        self.seq.chord_seq.lock().unwrap().chord_types[i] = next;
                    }

                    let fill = if is_current {
                        self.theme.c(&self.theme.seq_current)
                    } else if is_on {
                        self.theme.c(&self.theme.seq_step_on)
                    } else {
                        self.theme.c(&self.theme.seq_step_off)
                    };
                    let (r, painter) = ui.allocate_painter(Vec2::new(step_w, 28.0), Sense::click());
                    painter.rect_filled(r.rect, CornerRadius::same(5), fill);
                    painter.rect_stroke(
                        r.rect,
                        CornerRadius::same(5),
                        Stroke::new(
                            1.0,
                            if is_current {
                                Color32::WHITE
                            } else {
                                Color32::GRAY
                            },
                        ),
                        StrokeKind::Middle,
                    );
                    if r.clicked() {
                        let mut cs = self.seq.chord_seq.lock().unwrap();
                        cs.steps[i] = !cs.steps[i];
                    }

                    // Velocity bar.
                    let vel_h = 14.0;
                    let vel = self.seq.chord_seq.lock().unwrap().velocities[i];
                    let (vel_resp, painter) =
                        ui.allocate_painter(Vec2::new(step_w, vel_h), Sense::click_and_drag());
                    let vr = vel_resp.rect;
                    painter.rect_filled(vr, CornerRadius::same(2), Color32::from_rgb(30, 30, 40));
                    let fill_w = vel as f32 / 127.0 * vr.width();
                    painter.rect_filled(
                        egui::Rect::from_min_size(vr.min, Vec2::new(fill_w, vel_h)),
                        CornerRadius::same(2),
                        Color32::from_rgb(80, 140, 200),
                    );
                    painter.text(
                        vr.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{vel}"),
                        egui::FontId::monospace(8.0),
                        Color32::from_rgb(180, 180, 180),
                    );
                    if vel_resp.dragged() || vel_resp.clicked() {
                        if let Some(pos) = vel_resp.interact_pointer_pos() {
                            let t = ((pos.x - vr.min.x) / vr.width()).clamp(0.0, 1.0);
                            self.seq.chord_seq.lock().unwrap().velocities[i] =
                                (t * 127.0).round() as u8;
                        }
                    }

                    // Probability bar.
                    let prob_h = 10.0;
                    let prob = self.seq.chord_seq.lock().unwrap().probabilities[i];
                    let (prob_resp, painter) =
                        ui.allocate_painter(Vec2::new(step_w, prob_h), Sense::click_and_drag());
                    let pr = prob_resp.rect;
                    painter.rect_filled(pr, CornerRadius::same(2), Color32::from_rgb(30, 30, 40));
                    let pfill_w = prob as f32 / 100.0 * pr.width();
                    let prob_color = if prob >= 100 {
                        Color32::from_rgb(60, 160, 80)
                    } else if prob >= 50 {
                        Color32::from_rgb(180, 140, 40)
                    } else {
                        Color32::from_rgb(180, 70, 50)
                    };
                    painter.rect_filled(
                        egui::Rect::from_min_size(pr.min, Vec2::new(pfill_w, prob_h)),
                        CornerRadius::same(2),
                        prob_color,
                    );
                    if prob_resp.dragged() || prob_resp.clicked() {
                        if let Some(pos) = prob_resp.interact_pointer_pos() {
                            let t = ((pos.x - pr.min.x) / pr.width()).clamp(0.0, 1.0);
                            self.seq.chord_seq.lock().unwrap().probabilities[i] =
                                (t * 100.0).round() as u8;
                        }
                    }

                    // Octave offset row: −2 to +2, click left/right half to dec/inc.
                    let oct_h = 14.0;
                    let (oct_resp, painter) =
                        ui.allocate_painter(Vec2::new(step_w, oct_h), Sense::click());
                    let or_ = oct_resp.rect;
                    let oct_t = (oct_off + 2) as f32 / 4.0; // map −2..+2 → 0..1
                    painter.rect_filled(or_, CornerRadius::same(2), Color32::from_rgb(30, 30, 40));
                    let oct_fill_w = oct_t * or_.width();
                    painter.rect_filled(
                        egui::Rect::from_min_size(or_.min, Vec2::new(oct_fill_w, oct_h)),
                        CornerRadius::same(2),
                        Color32::from_rgb(120, 80, 180),
                    );
                    let oct_label = match oct_off {
                        0 => "oct".to_string(),
                        n if n > 0 => format!("+{n}"),
                        n => format!("{n}"),
                    };
                    painter.text(
                        or_.center(),
                        egui::Align2::CENTER_CENTER,
                        oct_label,
                        egui::FontId::monospace(8.0),
                        Color32::from_rgb(200, 180, 220),
                    );
                    if oct_resp.clicked() {
                        if let Some(pos) = oct_resp.interact_pointer_pos() {
                            let mid = or_.center().x;
                            let new_off = if pos.x > mid {
                                (oct_off + 1).min(2)
                            } else {
                                (oct_off - 1).max(-2)
                            };
                            self.seq.chord_seq.lock().unwrap().octave_offsets[i] = new_off;
                        }
                    }
                });
            }
        });
    }
}
