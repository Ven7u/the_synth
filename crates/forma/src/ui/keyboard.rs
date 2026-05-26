use crate::sequencer::{
    apply_voicing, chord_name, chord_quality, scale_pitch_classes, ChordType, ScaleType, SeqMode,
    VoicingType, CHORD_KB_COLS, CHORD_KB_ROWS, DEGREE_LABELS, NOTE_NAMES,
};
use crate::ui::layout::AppMode;
use crate::SynthApp;
use eframe::egui;
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

const WHITE_SEMITONES: &[i32] = &[0, 2, 4, 5, 7, 9, 11];
const BLACK_SEMITONES: &[Option<i32>] = &[Some(1), Some(3), None, Some(6), Some(8), Some(10), None];

const KEY_MAP: &[(egui::Key, i32)] = &[
    (egui::Key::A, 0),          // C
    (egui::Key::W, 1),          // C#
    (egui::Key::S, 2),          // D
    (egui::Key::E, 3),          // D#
    (egui::Key::D, 4),          // E
    (egui::Key::F, 5),          // F
    (egui::Key::T, 6),          // F#
    (egui::Key::G, 7),          // G
    (egui::Key::Y, 8),          // G#
    (egui::Key::H, 9),          // A
    (egui::Key::U, 10),         // A#
    (egui::Key::J, 11),         // B
    (egui::Key::K, 12),         // C
    (egui::Key::O, 13),         // C#
    (egui::Key::L, 14),         // D
    (egui::Key::P, 15),         // D#
    (egui::Key::Semicolon, 16), // E
    (egui::Key::Quote, 17),     // F
];

/// 88-key piano: A0 (MIDI 21) to C8 (MIDI 108).
const PIANO_FIRST_MIDI: u8 = 21; // A0
const PIANO_LAST_MIDI: u8 = 108; // C8

/// Returns true if a MIDI note is a white key.
fn is_white_key(midi: u8) -> bool {
    matches!(midi % 12, 0 | 2 | 4 | 5 | 7 | 9 | 11)
}

/// Count white keys in the 88-key range.
fn count_white_keys() -> usize {
    (PIANO_FIRST_MIDI..=PIANO_LAST_MIDI)
        .filter(|&m| is_white_key(m))
        .count()
}

impl SynthApp {
    /// Process keyboard input every frame regardless of which tab is visible.
    /// Call this from the main update loop, not from a tab panel.
    pub fn tick_keyboard_input(&mut self, ctx: &egui::Context) {
        const WHITE_KEYS: &[egui::Key] = &[
            egui::Key::A,
            egui::Key::S,
            egui::Key::D,
            egui::Key::F,
            egui::Key::G,
            egui::Key::H,
            egui::Key::J,
        ];

        // F1–F4 → switch focused synth track; F5 → Drum Machine mode.
        for (key, track) in [
            (egui::Key::F1, 0usize),
            (egui::Key::F2, 1),
            (egui::Key::F3, 2),
            (egui::Key::F4, 3),
        ] {
            if ctx.input(|i| i.key_pressed(key)) {
                self.switch_focused_track(track);
                // Bring up LIVE mode so the rig strip is visible.
                if self.app_mode == AppMode::DrumMachine || self.app_mode == AppMode::Studio {
                    self.app_mode = AppMode::Live;
                }
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.app_mode = AppMode::DrumMachine;
        }

        // Space bar toggles freeze (when no text widget has focus).
        let space_pressed = ctx.input(|inp| inp.key_pressed(egui::Key::Space));
        if space_pressed && !ctx.memory(|m| m.focused().is_some()) {
            self.kb_freeze = !self.kb_freeze;
            if !self.kb_freeze {
                let frozen: Vec<u8> = self.frozen_notes.drain().collect();
                for n in frozen {
                    self.push_note_off(n);
                }
            }
        }

        // Retrigger held chord-KB pads if voicing changed since last note-on.
        if self.kb_chord_mode && self.kb_voicing != self.kb_voicing_applied {
            let old_v = self.kb_voicing_applied;
            let new_v = self.kb_voicing;
            // Collect (old_notes, new_notes) for every held pad before any mutable borrows.
            let mut retriggers: Vec<(Vec<u8>, Vec<u8>)> = self
                .chord_kb
                .kb_held
                .iter()
                .map(|&(row, col)| {
                    (
                        apply_voicing(self.chord_kb.chord_notes_for(row, col), old_v),
                        apply_voicing(self.chord_kb.chord_notes_for(row, col), new_v),
                    )
                })
                .collect();
            if let Some((row, col)) = self.chord_kb.held_pad {
                retriggers.push((
                    apply_voicing(self.chord_kb.chord_notes_for(row, col), old_v),
                    apply_voicing(self.chord_kb.chord_notes_for(row, col), new_v),
                ));
            }
            for (old_notes, new_notes) in retriggers {
                for n in &old_notes {
                    self.engine.note_off(*n);
                }
                for n in &new_notes {
                    self.engine.note_on(*n, self.piano_velocity);
                }
            }
            // Update frozen notes to the new voicing so freeze-held chords retrigger cleanly.
            let old_frozen: Vec<u8> = self.frozen_notes.iter().copied().collect();
            if !old_frozen.is_empty() {
                // frozen_notes are individual MIDI values — swap any that match old voiced notes.
                // Rebuild the set with updated values.
                let mut new_frozen = std::collections::HashSet::new();
                for n in old_frozen {
                    // Try to find which pad this note belonged to and remap it.
                    let mut remapped = false;
                    'outer: for row in 0..crate::sequencer::CHORD_KB_ROWS {
                        for col in 0..crate::sequencer::CHORD_KB_COLS {
                            let old_voiced =
                                apply_voicing(self.chord_kb.chord_notes_for(row, col), old_v);
                            if old_voiced.contains(&n) {
                                for m in
                                    apply_voicing(self.chord_kb.chord_notes_for(row, col), new_v)
                                {
                                    self.engine.note_off(n);
                                    self.engine.note_on(m, self.piano_velocity);
                                    new_frozen.insert(m);
                                }
                                remapped = true;
                                break 'outer;
                            }
                        }
                    }
                    if !remapped {
                        new_frozen.insert(n);
                    }
                }
                self.frozen_notes = new_frozen;
            }
            self.kb_voicing_applied = new_v;
        }

        // Arrow keys = momentary voicing (chord mode only); release to return to Root.
        // No arrow = Root  ↑ = 1st  ↓ = 2nd  → = Open  ← = Full
        if self.kb_chord_mode && !ctx.memory(|m| m.focused().is_some()) {
            self.kb_voicing = ctx.input(|inp| {
                if inp.key_down(egui::Key::ArrowUp) {
                    VoicingType::First
                } else if inp.key_down(egui::Key::ArrowDown) {
                    VoicingType::Second
                } else if inp.key_down(egui::Key::ArrowRight) {
                    VoicingType::Open
                } else if inp.key_down(egui::Key::ArrowLeft) {
                    VoicingType::Full
                } else {
                    VoicingType::Root
                }
            });
        } else {
            self.kb_voicing = VoicingType::Root;
        }

        if self.kb_chord_mode {
            // 3 rows × 7 cols grid:
            //   Row 0 (triads):  A S D F G H J
            //   Row 1 (7ths):    Q W E R T Y U
            //   Row 2 (sus/add): Z X C V B N M
            const ROW_KEYS: [[egui::Key; 7]; 3] = [
                [
                    egui::Key::Q,
                    egui::Key::W,
                    egui::Key::E,
                    egui::Key::R,
                    egui::Key::T,
                    egui::Key::Y,
                    egui::Key::U,
                ],
                [
                    egui::Key::A,
                    egui::Key::S,
                    egui::Key::D,
                    egui::Key::F,
                    egui::Key::G,
                    egui::Key::H,
                    egui::Key::J,
                ],
                [
                    egui::Key::Z,
                    egui::Key::X,
                    egui::Key::C,
                    egui::Key::V,
                    egui::Key::B,
                    egui::Key::N,
                    egui::Key::M,
                ],
            ];

            let mut current_pads = std::collections::HashSet::<(usize, usize)>::new();
            ctx.input(|inp| {
                for (row, keys) in ROW_KEYS.iter().enumerate() {
                    for (col, &key) in keys.iter().enumerate() {
                        if inp.key_down(key) {
                            current_pads.insert((row, col));
                        }
                    }
                }
            });

            for &pad in &current_pads {
                if !self.chord_kb.kb_held.contains(&pad) {
                    let (row, col) = pad;
                    self.seq_record_chord_pad(row, col);
                    if self.kb_freeze {
                        let frozen: Vec<u8> = self.frozen_notes.drain().collect();
                        for n in frozen {
                            self.push_note_off(n);
                        }
                        for m in
                            apply_voicing(self.chord_kb.chord_notes_for(row, col), self.kb_voicing)
                        {
                            self.push_note_on(m);
                        }
                    } else {
                        for m in
                            apply_voicing(self.chord_kb.chord_notes_for(row, col), self.kb_voicing)
                        {
                            self.push_note_on(m);
                        }
                    }
                }
            }
            let released: Vec<(usize, usize)> = self
                .chord_kb
                .kb_held
                .iter()
                .filter(|p| !current_pads.contains(p))
                .copied()
                .collect();
            for (row, col) in released {
                let notes = apply_voicing(self.chord_kb.chord_notes_for(row, col), self.kb_voicing);
                if self.kb_freeze {
                    for m in notes {
                        self.frozen_notes.insert(m);
                    }
                } else {
                    for m in notes {
                        self.push_note_off(m);
                    }
                }
            }
            self.chord_kb.kb_held = current_pads;
            let prev_midi: Vec<u8> = self.piano_held_midi.drain().collect();
            for m in prev_midi {
                self.push_note_off(m);
            }
        } else {
            // Piano mode — release chord notes if mode just switched
            if !self.chord_kb.kb_held.is_empty() {
                let held: Vec<(usize, usize)> = self.chord_kb.kb_held.drain().collect();
                for (row, col) in held {
                    for m in apply_voicing(self.chord_kb.chord_notes_for(row, col), self.kb_voicing)
                    {
                        self.push_note_off(m);
                    }
                }
            }
            // ChordSeq live transpose: any key press sets the root note
            if SeqMode::from_u8(self.seq.mode.load(std::sync::atomic::Ordering::Relaxed))
                == SeqMode::ChordSeq
                && self.seq.playing.load(std::sync::atomic::Ordering::Relaxed)
            {
                let mut pressed_semitone: Option<u8> = None;
                ctx.input(|inp| {
                    for &(key, semitone) in KEY_MAP {
                        if inp.key_pressed(key) {
                            pressed_semitone = Some((semitone % 12) as u8);
                        }
                    }
                });
                if let Some(semi) = pressed_semitone {
                    self.seq.chord_seq.lock().unwrap().root = semi;
                }
                let prev: Vec<u8> = self.piano_held_midi.drain().collect();
                for m in prev {
                    self.push_note_off(m);
                }
            } else if !ctx.memory(|m| m.focused().is_some()) {
                // Z/X = octave, C/V = velocity, 1/2 = pitch bend, 3–8 = mod wheel → filter
                let prev_pitch_bend = self.piano_pitch_bend;
                let prev_mod = self.piano_mod_wheel;
                ctx.input(|inp| {
                    if inp.key_pressed(egui::Key::Z) && self.piano_octave > 1 {
                        self.piano_octave -= 1;
                    }
                    if inp.key_pressed(egui::Key::X) && self.piano_octave < 7 {
                        self.piano_octave += 1;
                    }
                    if inp.key_pressed(egui::Key::C) {
                        self.piano_velocity = self.piano_velocity.saturating_sub(10).max(10);
                    }
                    if inp.key_pressed(egui::Key::V) {
                        self.piano_velocity = self.piano_velocity.saturating_add(10).min(127);
                    }
                    // 1 = pitch bend down, 2 = pitch bend up (hold), release = reset
                    let bend_down = inp.key_down(egui::Key::Num1);
                    let bend_up = inp.key_down(egui::Key::Num2);
                    self.piano_pitch_bend = if bend_down && !bend_up {
                        -2
                    } else if bend_up && !bend_down {
                        2
                    } else {
                        0
                    };
                    // 3=off, 4=20%, 5=40%, 6=60%, 7=80%, 8=100% filter offset
                    let mod_keys = [
                        (egui::Key::Num3, 0u8),
                        (egui::Key::Num4, 1),
                        (egui::Key::Num5, 2),
                        (egui::Key::Num6, 3),
                        (egui::Key::Num7, 4),
                        (egui::Key::Num8, 5),
                    ];
                    for (key, level) in mod_keys {
                        if inp.key_pressed(key) {
                            self.piano_mod_wheel = level;
                        }
                    }
                });
                if self.piano_pitch_bend != prev_pitch_bend {
                    let semitones = self.piano_pitch_bend as f32;
                    self.engine.set_lfo_pitch_mult(2_f32.powf(semitones / 12.0));
                }
                if self.piano_mod_wheel != prev_mod {
                    self.engine.set_mod_wheel(self.piano_mod_wheel as f32 / 5.0);
                }
                let mut current_held = std::collections::HashSet::<u8>::new();
                ctx.input(|inp| {
                    for &(key, semitone) in KEY_MAP {
                        if inp.key_down(key) {
                            current_held.insert((self.piano_octave * 12 + semitone) as u8);
                        }
                    }
                });
                for &midi in &current_held {
                    if !self.piano_held_midi.contains(&midi) {
                        // New note: release frozen set now, defer NoteOn to next frame.
                        if self.kb_freeze {
                            let frozen: Vec<u8> = self.frozen_notes.drain().collect();
                            for n in frozen {
                                self.push_note_off(n);
                            }
                            self.push_note_on(midi);
                        } else {
                            self.push_note_on(midi);
                        }
                    }
                }
                let released: Vec<u8> = self
                    .piano_held_midi
                    .iter()
                    .filter(|&&m| !current_held.contains(&m))
                    .copied()
                    .collect();
                for midi in released {
                    if self.kb_freeze {
                        self.frozen_notes.insert(midi);
                    } else {
                        self.push_note_off(midi);
                    }
                }
                self.piano_held_midi = current_held;
            } else {
                // A text widget has focus — release any notes that were held before focus was taken.
                let held: Vec<u8> = self.piano_held_midi.drain().collect();
                for midi in held {
                    if self.kb_freeze {
                        self.frozen_notes.insert(midi);
                    } else {
                        self.push_note_off(midi);
                    }
                }
            }
        }
    }

    /// Render the persistent bottom keyboard strip.
    pub fn ui_keyboard_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Mode toggle: Piano / Chord KB
            let piano_label = egui::RichText::new("Piano")
                .color(if !self.kb_chord_mode { self.theme.c(&self.theme.accent) } else { Color32::GRAY })
                .strong();
            if ui.button(piano_label).on_hover_text("Standard piano — play individual notes.").clicked()
                && self.kb_chord_mode
            {
                self.kb_chord_mode = false;
            }
            let chord_label = egui::RichText::new("Chord KB")
                .color(if self.kb_chord_mode { self.theme.c(&self.theme.accent) } else { Color32::GRAY })
                .strong();
            if ui.button(chord_label).on_hover_text("Chord Keyboard — A–G trigger diatonic chords I–VII.").clicked()
                && !self.kb_chord_mode
            {
                self.kb_chord_mode = true;
            }

            ui.separator();

            // Freeze toggle
            let freeze_color = if self.kb_freeze {
                Color32::from_rgb(80, 160, 240)
            } else {
                Color32::GRAY
            };
            let freeze_label = egui::RichText::new("❄ Freeze")
                .color(freeze_color)
                .strong();
            if ui.button(freeze_label)
                .on_hover_text("Freeze held notes — they keep sounding until a new chord/note is played.\nSpace bar toggles. MIDI CC 64 (sustain pedal) also works.")
                .clicked()
            {
                self.kb_freeze = !self.kb_freeze;
                if !self.kb_freeze {
                    let frozen: Vec<u8> = self.frozen_notes.drain().collect();
                    for n in frozen { self.push_note_off(n); }
                }
            }

            ui.separator();

            if self.kb_chord_mode {
                ui.label("Key:");
                egui::ComboBox::from_id_salt("chord_kb_root")
                    .selected_text(NOTE_NAMES[self.chord_kb.root as usize])
                    .show_ui(ui, |ui| {
                        for (i, name) in NOTE_NAMES.iter().enumerate() {
                            ui.selectable_value(&mut self.chord_kb.root, i as u8, *name);
                        }
                    });
                ui.label("Scale:");
                for &sc in &[ScaleType::Major, ScaleType::Minor] {
                    let active = self.chord_kb.scale == sc;
                    let label = egui::RichText::new(sc.label())
                        .color(if active { self.theme.c(&self.theme.accent_dim) } else { Color32::GRAY });
                    if ui.button(label).clicked() { self.chord_kb.scale = sc; }
                }
                ui.separator();
                // Edit toggle
                let edit_color = if self.chord_kb.edit_mode {
                    Color32::from_rgb(240, 180, 60)
                } else {
                    Color32::GRAY
                };
                let edit_label = egui::RichText::new("✏ Edit").color(edit_color).strong();
                if ui.button(edit_label)
                    .on_hover_text("Edit mode: click any pad to change its chord type.")
                    .clicked()
                {
                    self.chord_kb.edit_mode = !self.chord_kb.edit_mode;
                    self.chord_kb.editing_pad = None;
                }
                let preview_label = egui::RichText::new(
                    if self.chord_kb.show_piano_preview { "KEYS ▾" } else { "KEYS ▸" }
                ).small();
                if ui.button(preview_label).on_hover_text("Toggle piano preview").clicked() {
                    self.chord_kb.show_piano_preview = !self.chord_kb.show_piano_preview;
                }
                ui.separator();
                // Voicing selector (also controlled by ↑/↓ arrow keys)
                // Voicing indicator — arrows are the control, this just shows what's active.
                ui.label(egui::RichText::new("Voicing:").weak().small());
                for &v in VoicingType::all() {
                    let active = self.kb_voicing == v;
                    let color = if active {
                        self.theme.c(&self.theme.accent)
                    } else {
                        Color32::GRAY
                    };
                    ui.label(egui::RichText::new(v.label()).small().color(color))
                        .on_hover_text("Hold arrow keys: no arrow = Root  ↑ = 1st  ↓ = 2nd  → = Open  ← = Full");
                }
                ui.label(egui::RichText::new("  A–J / Q–U / Z–M = rows 1–3").weak().small());
            } else {
                // Octave controls
                ui.label("Oct:").on_hover_text("Keyboard octave (1–7).  Z = down, X = up");
                if ui.button("−").on_hover_text("One octave down  [Z]").clicked() && self.piano_octave > 1 {
                    self.piano_octave -= 1;
                }
                ui.label(format!("C{}", self.piano_octave));
                if ui.button("+").on_hover_text("One octave up  [X]").clicked() && self.piano_octave < 7 {
                    self.piano_octave += 1;
                }

                ui.separator();

                // Velocity controls
                ui.label("Vel:").on_hover_text("Note velocity (10–127).  C = down, V = up");
                if ui.button("−").on_hover_text("Velocity −10  [C]").clicked() {
                    self.piano_velocity = self.piano_velocity.saturating_sub(10).max(10);
                }
                ui.label(format!("{}", self.piano_velocity));
                if ui.button("+").on_hover_text("Velocity +10  [V]").clicked() {
                    self.piano_velocity = self.piano_velocity.saturating_add(10).min(127);
                }

                ui.separator();

                // Pitch bend indicator (keys 1/2)
                let bend_color = if self.piano_pitch_bend != 0 {
                    Color32::from_rgb(100, 180, 255)
                } else {
                    Color32::GRAY
                };
                let bend_text = match self.piano_pitch_bend {
                    -2 => "Bend ▼",
                    2  => "Bend ▲",
                    _  => "Bend",
                };
                ui.label(egui::RichText::new(bend_text).color(bend_color))
                    .on_hover_text("Pitch bend ±2 semitones.  Hold 1 = down,  Hold 2 = up");

                ui.separator();

                // Mod wheel indicator (keys 3–8 → filter cutoff offset)
                let mod_color = if self.piano_mod_wheel > 0 {
                    Color32::from_rgb(180, 120, 255)
                } else {
                    Color32::GRAY
                };
                let mod_bars = ["▁", "▃", "▅", "▆", "█"];
                let mod_label = if self.piano_mod_wheel == 0 {
                    "Mod: off".to_string()
                } else {
                    format!("Mod: {}", mod_bars[(self.piano_mod_wheel as usize).saturating_sub(1).min(4)])
                };
                ui.label(egui::RichText::new(mod_label).color(mod_color))
                    .on_hover_text("Modulation wheel → filter cutoff.  3=off, 4–8=levels  (up to +8 kHz)");

                ui.separator();

                // Scale highlight controls
                ui.label(egui::RichText::new("Scale:").weak().small());
                // "Off" button
                let off_active = self.piano_scale_highlight.is_none();
                let off_label = egui::RichText::new("Off")
                    .small()
                    .color(if off_active { self.theme.c(&self.theme.accent) } else { Color32::GRAY });
                if ui.button(off_label).clicked() {
                    self.piano_scale_highlight = None;
                }
                for &sc in ScaleType::all_highlight() {
                    let active = self.piano_scale_highlight == Some(sc);
                    let label = egui::RichText::new(sc.label())
                        .small()
                        .color(if active { self.theme.c(&self.theme.accent) } else { Color32::GRAY });
                    if ui.button(label).clicked() {
                        self.piano_scale_highlight = Some(sc);
                    }
                }
                if self.piano_scale_highlight.is_some() {
                    ui.label(egui::RichText::new("Root:").weak().small());
                    egui::ComboBox::from_id_salt("piano_scale_root")
                        .selected_text(NOTE_NAMES[self.piano_scale_root as usize])
                        .width(48.0)
                        .show_ui(ui, |ui| {
                            for (i, name) in NOTE_NAMES.iter().enumerate() {
                                ui.selectable_value(&mut self.piano_scale_root, i as u8, *name);
                            }
                        });
                }

                let hint = if SeqMode::from_u8(self.seq.mode.load(std::sync::atomic::Ordering::Relaxed)) == SeqMode::ChordSeq
                    && self.seq.playing.load(std::sync::atomic::Ordering::Relaxed) {
                    "  any key = set root note (live transpose)"
                } else {
                    "  a–' = notes  |  z/x = oct  |  c/v = vel  |  1/2 = bend  |  3–8 = mod"
                };
                ui.label(egui::RichText::new(hint).weak().small());
            }
        });

        if self.kb_chord_mode {
            self.draw_chord_pads(ui);
            if self.chord_kb.show_piano_preview {
                self.draw_chord_piano_preview(ui);
            }
        } else {
            self.draw_piano_88(ui);
        }
    }

    fn draw_chord_pads(&mut self, ui: &mut egui::Ui) {
        const KEY_HINTS: [[&str; CHORD_KB_COLS]; CHORD_KB_ROWS] = [
            ["Q", "W", "E", "R", "T", "Y", "U"],
            ["A", "S", "D", "F", "G", "H", "J"],
            ["Z", "X", "C", "V", "B", "N", "M"],
        ];
        const ROW_LABELS: [&str; CHORD_KB_ROWS] = ["7ths", "Triads", "Sus/Add"];

        let label_w = 48.0_f32;
        let spacing = ui.spacing().item_spacing.x;
        let btn_w = ((ui.available_width() - label_w - spacing * 7.0) / 7.0).max(40.0);
        let btn_h = 52.0;

        // Collect held state snapshot before mutable borrows below
        let held_pad = self.chord_kb.held_pad;
        let edit_mode = self.chord_kb.edit_mode;
        let editing_pad = self.chord_kb.editing_pad;

        egui::Grid::new("chord_kb_grid")
            .num_columns(CHORD_KB_COLS + 1)
            .spacing([spacing, 2.0])
            .show(ui, |ui| {
                for row in 0..CHORD_KB_ROWS {
                    // Fixed-width row label cell
                    ui.allocate_ui_with_layout(
                        Vec2::new(label_w, btn_h),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new(ROW_LABELS[row])
                                    .weak()
                                    .small()
                                    .color(Color32::from_gray(120)),
                            );
                        },
                    );

                    for col in 0..CHORD_KB_COLS {
                        let (resp, painter) =
                            ui.allocate_painter(Vec2::new(btn_w, btn_h), Sense::click_and_drag());
                        let r = resp.rect;

                        let is_held_mouse = held_pad == Some((row, col));
                        let is_held_kb = self.chord_kb.kb_held.contains(&(row, col));
                        let is_held = is_held_mouse || is_held_kb;
                        let is_editing = editing_pad == Some((row, col));

                        let quality = chord_quality(self.chord_kb.scale, col);
                        let bg = if is_held {
                            self.theme.c(&self.theme.seq_current)
                        } else if is_editing {
                            Color32::from_rgb(80, 60, 20)
                        } else if quality == "m" {
                            self.theme.c(&self.theme.seq_kb_minor)
                        } else if quality == "°" {
                            self.theme.c(&self.theme.seq_kb_dim)
                        } else {
                            self.theme.c(&self.theme.seq_kb_major)
                        };
                        painter.rect_filled(r, CornerRadius::same(6), bg);
                        let stroke_color = if is_held {
                            Color32::WHITE
                        } else if is_editing {
                            Color32::from_rgb(240, 180, 60)
                        } else {
                            Color32::from_gray(80)
                        };
                        painter.rect_stroke(
                            r,
                            CornerRadius::same(6),
                            Stroke::new(
                                if is_held || is_editing { 2.0 } else { 1.0 },
                                stroke_color,
                            ),
                            StrokeKind::Middle,
                        );

                        // Chord name
                        let cname = chord_name(self.chord_kb.root, self.chord_kb.scale, col);
                        let chord_type = self.chord_kb.pads[row][col].chord_type;
                        let display = if chord_type == ChordType::Triad {
                            cname
                        } else {
                            format!("{} {}", cname, chord_type.label())
                        };
                        painter.text(
                            egui::pos2(r.center().x, r.top() + 14.0),
                            egui::Align2::CENTER_CENTER,
                            &display,
                            egui::FontId::proportional(11.0),
                            Color32::WHITE,
                        );

                        // Degree label
                        painter.text(
                            egui::pos2(r.center().x, r.top() + 28.0),
                            egui::Align2::CENTER_CENTER,
                            DEGREE_LABELS[col],
                            egui::FontId::monospace(9.0),
                            Color32::from_gray(160),
                        );

                        // Key hint (bottom right corner)
                        painter.text(
                            egui::pos2(r.right() - 5.0, r.bottom() - 4.0),
                            egui::Align2::RIGHT_BOTTOM,
                            KEY_HINTS[row][col],
                            egui::FontId::monospace(8.0),
                            Color32::from_gray(110),
                        );

                        // Interaction
                        if edit_mode {
                            if resp.clicked() {
                                self.chord_kb.editing_pad =
                                    if is_editing { None } else { Some((row, col)) };
                            }
                        } else {
                            if resp.is_pointer_button_down_on() && !is_held_mouse {
                                self.seq_record_chord_pad(row, col);
                                if self.kb_freeze {
                                    let frozen: Vec<u8> = self.frozen_notes.drain().collect();
                                    for n in frozen {
                                        self.push_note_off(n);
                                    }
                                    if let Some((pr, pc)) = self.chord_kb.held_pad {
                                        for m in apply_voicing(
                                            self.chord_kb.chord_notes_for(pr, pc),
                                            self.kb_voicing,
                                        ) {
                                            self.frozen_notes.insert(m);
                                        }
                                    }
                                    for m in apply_voicing(
                                        self.chord_kb.chord_notes_for(row, col),
                                        self.kb_voicing,
                                    ) {
                                        self.push_note_on(m);
                                    }
                                } else {
                                    if let Some((pr, pc)) = self.chord_kb.held_pad {
                                        for m in apply_voicing(
                                            self.chord_kb.chord_notes_for(pr, pc),
                                            self.kb_voicing,
                                        ) {
                                            self.push_note_off(m);
                                        }
                                    }
                                    for m in apply_voicing(
                                        self.chord_kb.chord_notes_for(row, col),
                                        self.kb_voicing,
                                    ) {
                                        self.push_note_on(m);
                                    }
                                }
                                self.chord_kb.held_pad = Some((row, col));
                            }
                            if !resp.is_pointer_button_down_on() && is_held_mouse {
                                self.chord_kb.held_pad = None;
                                let notes = apply_voicing(
                                    self.chord_kb.chord_notes_for(row, col),
                                    self.kb_voicing,
                                );
                                if self.kb_freeze {
                                    for m in notes {
                                        self.frozen_notes.insert(m);
                                    }
                                } else {
                                    for m in notes {
                                        self.push_note_off(m);
                                    }
                                }
                            }
                        }
                    }
                    ui.end_row();
                }
            }); // end Grid

        // Edit popover
        if let Some((row, col)) = self.chord_kb.editing_pad {
            egui::Window::new("Chord type")
                .id(egui::Id::new("chord_kb_edit_popover"))
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label(format!("{} — {}", DEGREE_LABELS[col], ROW_LABELS[row]));
                    ui.separator();
                    for &ct in ChordType::all() {
                        let active = self.chord_kb.pads[row][col].chord_type == ct;
                        let label = egui::RichText::new(ct.label()).color(if active {
                            self.theme.c(&self.theme.accent)
                        } else {
                            Color32::GRAY
                        });
                        if ui.button(label).clicked() {
                            self.chord_kb.pads[row][col].chord_type = ct;
                            self.chord_kb.editing_pad = None;
                            self.chord_kb.edit_mode = false;
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.chord_kb.editing_pad = None;
                        self.chord_kb.edit_mode = false;
                    }
                });
        }
    }

    /// Read-only piano strip showing which MIDI notes are currently active in chord KB mode.
    fn draw_chord_piano_preview(&mut self, ui: &mut egui::Ui) {
        // Collect active notes: frozen + currently held pads
        let mut active: std::collections::HashSet<u8> = self.frozen_notes.clone();
        if let Some((row, col)) = self.chord_kb.held_pad {
            for m in apply_voicing(self.chord_kb.chord_notes_for(row, col), self.kb_voicing) {
                active.insert(m);
            }
        }
        for &(row, col) in &self.chord_kb.kb_held.clone() {
            for m in apply_voicing(self.chord_kb.chord_notes_for(row, col), self.kb_voicing) {
                active.insert(m);
            }
        }

        let num_white = count_white_keys();
        let available_w = ui.available_width();
        let white_w = (available_w / num_white as f32).max(6.0);
        let white_h = 36.0_f32;
        let black_w = white_w * 0.62;
        let black_h = white_h * 0.60;
        let total_width = white_w * num_white as f32;

        let (resp, painter) =
            ui.allocate_painter(Vec2::new(total_width, white_h + 2.0), Sense::hover());
        let origin = resp.rect.left_top();
        let accent = self.theme.c(&self.theme.accent);

        let mut white_key_x: [f32; 128] = [0.0; 128];
        let mut white_x = 0.0_f32;

        // Pass 1: white keys
        for midi in PIANO_FIRST_MIDI..=PIANO_LAST_MIDI {
            if !is_white_key(midi) {
                continue;
            }
            let x = white_x;
            white_key_x[midi as usize] = x;
            white_x += white_w;

            let rect = Rect::from_min_size(
                origin + Vec2::new(x + 0.5, 1.0),
                Vec2::new(white_w - 1.0, white_h - 2.0),
            );
            let pressed = active.contains(&midi);
            let fill = if pressed {
                accent
            } else {
                Color32::from_rgb(230, 230, 230)
            };
            painter.rect_filled(rect, CornerRadius::same(2), fill);
            painter.rect_stroke(
                rect,
                CornerRadius::same(2),
                Stroke::new(0.5, Color32::from_rgb(160, 160, 160)),
                StrokeKind::Middle,
            );

            if midi % 12 == 0 {
                let octave = (midi / 12) as i32 - 1;
                painter.text(
                    Pos2::new(rect.center().x, rect.bottom() - 3.0),
                    egui::Align2::CENTER_BOTTOM,
                    format!("C{octave}"),
                    egui::FontId::proportional(if white_w > 12.0 { 7.0 } else { 6.0 }),
                    Color32::from_rgb(120, 120, 120),
                );
            }
        }

        // Pass 2: black keys
        for midi in PIANO_FIRST_MIDI..=PIANO_LAST_MIDI {
            if is_white_key(midi) {
                continue;
            }
            let white_below = midi - 1;
            if !is_white_key(white_below) {
                continue;
            }
            let x = white_key_x[white_below as usize] + white_w * 0.6;
            let rect = Rect::from_min_size(origin + Vec2::new(x, 1.0), Vec2::new(black_w, black_h));
            let pressed = active.contains(&midi);
            let fill = if pressed {
                Color32::from_rgba_premultiplied(accent.r(), accent.g(), accent.b(), 220)
            } else {
                Color32::from_rgb(25, 25, 25)
            };
            painter.rect_filled(rect, CornerRadius::same(1), fill);
        }
    }

    /// Draw a full 88-key piano (A0–C8) with the active keyboard range highlighted.
    fn draw_piano_88(&mut self, ui: &mut egui::Ui) {
        let scale_pcs: Option<[bool; 12]> = self
            .piano_scale_highlight
            .map(|sc| scale_pitch_classes(sc, self.piano_scale_root));
        let num_white = count_white_keys(); // 52
        let available_w = ui.available_width();
        let white_w = (available_w / num_white as f32).max(6.0);
        let white_h = 64.0_f32;
        let black_w = white_w * 0.62;
        let black_h = white_h * 0.60;
        let total_width = white_w * num_white as f32;

        let (resp, painter) = ui.allocate_painter(
            Vec2::new(total_width, white_h + 4.0),
            Sense::click_and_drag(),
        );
        let origin = resp.rect.left_top();
        let pointer_pos = resp.interact_pointer_pos();
        let mut clicked_midi: Option<u8> = None;

        // The range that the computer keyboard maps to (KEY_MAP: semitones 0–14).
        let kb_max_semitone = KEY_MAP.iter().map(|&(_, s)| s).max().unwrap_or(14);
        let kb_range_start = (self.piano_octave * 12) as u8;
        let kb_range_end = kb_range_start + kb_max_semitone as u8 + 1;

        // --- Pass 1: Draw white keys ---
        let mut white_x = 0.0_f32;
        // Store white key positions for black key placement.
        let mut white_key_x: [f32; 128] = [0.0; 128];

        for midi in PIANO_FIRST_MIDI..=PIANO_LAST_MIDI {
            if !is_white_key(midi) {
                continue;
            }

            let x = white_x;
            white_key_x[midi as usize] = x;
            white_x += white_w;

            let rect = Rect::from_min_size(
                origin + Vec2::new(x + 0.5, 1.0),
                Vec2::new(white_w - 1.0, white_h - 2.0),
            );

            let pressed =
                self.piano_held_midi.contains(&midi) || self.piano_mouse_midi == Some(midi);
            let in_kb_range = midi >= kb_range_start && midi < kb_range_end;
            let pitch_class = (midi % 12) as usize;
            let is_root = scale_pcs.is_some() && pitch_class == self.piano_scale_root as usize;
            let in_scale = scale_pcs.map_or(false, |pcs| pcs[pitch_class]);

            let fill = if pressed {
                self.theme.c(&self.theme.key_white_pressed)
            } else if is_root {
                Color32::from_rgb(255, 210, 80)
            } else if in_scale {
                Color32::from_rgb(200, 240, 210)
            } else if in_kb_range {
                Color32::from_rgb(230, 240, 245)
            } else {
                Color32::from_rgb(245, 245, 245)
            };

            painter.rect_filled(rect, CornerRadius::same(2), fill);
            painter.rect_stroke(
                rect,
                CornerRadius::same(2),
                Stroke::new(0.5, Color32::from_rgb(180, 180, 180)),
                StrokeKind::Middle,
            );

            // C note labels at the bottom of each C key.
            if midi % 12 == 0 {
                let octave = (midi / 12) as i32 - 1;
                painter.text(
                    Pos2::new(rect.center().x, rect.bottom() - 3.0),
                    egui::Align2::CENTER_BOTTOM,
                    format!("C{octave}"),
                    egui::FontId::proportional(if white_w > 12.0 { 8.0 } else { 6.0 }),
                    Color32::from_rgb(140, 140, 140),
                );
            }

            if let Some(pos) = pointer_pos {
                if rect.contains(pos) {
                    clicked_midi = Some(midi);
                }
            }
        }

        // --- Pass 2: Draw black keys (on top) ---
        for midi in PIANO_FIRST_MIDI..=PIANO_LAST_MIDI {
            if is_white_key(midi) {
                continue;
            }

            // Black key sits between the white key below and above.
            // Find the white key just below this black key.
            let white_below = midi - 1;
            if !is_white_key(white_below) {
                continue;
            }
            let x = white_key_x[white_below as usize] + white_w * 0.6;

            let rect = Rect::from_min_size(origin + Vec2::new(x, 1.0), Vec2::new(black_w, black_h));

            let pressed =
                self.piano_held_midi.contains(&midi) || self.piano_mouse_midi == Some(midi);
            let in_kb_range = midi >= kb_range_start && midi < kb_range_end;
            let pitch_class = (midi % 12) as usize;
            let is_root = scale_pcs.is_some() && pitch_class == self.piano_scale_root as usize;
            let in_scale = scale_pcs.map_or(false, |pcs| pcs[pitch_class]);

            let fill = if pressed {
                self.theme.c(&self.theme.key_black_pressed)
            } else if is_root {
                Color32::from_rgb(120, 80, 10)
            } else if in_scale {
                Color32::from_rgb(30, 70, 40)
            } else if in_kb_range {
                Color32::from_rgb(40, 40, 50)
            } else {
                Color32::from_rgb(25, 25, 25)
            };

            painter.rect_filled(rect, CornerRadius::same(1), fill);

            if let Some(pos) = pointer_pos {
                if rect.contains(pos) {
                    clicked_midi = Some(midi);
                }
            }
        }

        // --- Pass 3: Draw keyboard range bracket on top ---
        // A subtle colored bar above the active range.
        {
            // Find pixel x range for the keyboard mapping range.
            let accent = self.theme.c(&self.theme.accent);
            let mut range_left = f32::MAX;
            let mut range_right = 0.0_f32;
            for midi in kb_range_start..kb_range_end.min(PIANO_LAST_MIDI + 1) {
                if midi < PIANO_FIRST_MIDI {
                    continue;
                }
                if is_white_key(midi) {
                    let x = white_key_x[midi as usize];
                    range_left = range_left.min(x);
                    range_right = range_right.max(x + white_w);
                } else {
                    let wb = midi - 1;
                    if is_white_key(wb) {
                        let x = white_key_x[wb as usize] + white_w * 0.6;
                        range_left = range_left.min(x);
                        range_right = range_right.max(x + black_w);
                    }
                }
            }
            if range_left < range_right {
                let bar = Rect::from_min_size(
                    origin + Vec2::new(range_left, 0.0),
                    Vec2::new(range_right - range_left, 2.5),
                );
                painter.rect_filled(bar, CornerRadius::same(1), accent);
            }
        }

        // --- Mouse interaction ---
        if resp.is_pointer_button_down_on() {
            if let Some(midi) = clicked_midi {
                if self.piano_mouse_midi != Some(midi) {
                    if let Some(old) = self.piano_mouse_midi {
                        self.push_note_off(old);
                    }
                    self.piano_mouse_midi = Some(midi);
                    self.push_note_on(midi);
                }
            }
        } else if let Some(midi) = self.piano_mouse_midi.take() {
            self.push_note_off(midi);
        }
    }
}
