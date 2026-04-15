use crate::SynthApp;
use crate::sequencer::SeqMode;
use eframe::egui;
use egui::{Color32, Pos2, Rect, Rounding, Sense, Stroke, Vec2};

const WHITE_SEMITONES: &[i32] = &[0, 2, 4, 5, 7, 9, 11];
const BLACK_SEMITONES: &[Option<i32>] = &[Some(1), Some(3), None, Some(6), Some(8), Some(10), None];

const KEY_MAP: &[(egui::Key, i32)] = &[
    (egui::Key::A, 0),
    (egui::Key::W, 1),
    (egui::Key::S, 2),
    (egui::Key::E, 3),
    (egui::Key::D, 4),
    (egui::Key::F, 5),
    (egui::Key::T, 6),
    (egui::Key::G, 7),
    (egui::Key::Y, 8),
    (egui::Key::H, 9),
    (egui::Key::U, 10),
    (egui::Key::J, 11),
    (egui::Key::K, 12),
    (egui::Key::L, 14),
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
    pub fn ui_keyboard_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Octave:").on_hover_text("Keyboard octave range (1–7). Shifts the computer keyboard mapping up or down.");
            if ui.button("−").on_hover_text("One octave down").clicked() && self.piano_octave > 1 {
                self.piano_octave -= 1;
            }
            ui.label(format!("{}", self.piano_octave)).on_hover_text("Current keyboard octave.");
            if ui.button("+").on_hover_text("One octave up").clicked() && self.piano_octave < 7 {
                self.piano_octave += 1;
            }
            let hint = if self.seq_mode == SeqMode::ChordKb {
                "  a s d f g h j = chords I–VII"
            } else if self.seq_mode == SeqMode::ChordSeq && self.seq_playing {
                "  any key = set root note (live transpose)"
            } else {
                "  a–l = white keys, w e t y u = sharps"
            };
            ui.label(egui::RichText::new(hint).weak().small());
        });

        const WHITE_KEYS: &[egui::Key] = &[
            egui::Key::A, egui::Key::S, egui::Key::D, egui::Key::F,
            egui::Key::G, egui::Key::H, egui::Key::J,
        ];

        if self.seq_mode == SeqMode::ChordSeq && self.seq_playing {
            let mut pressed_semitone: Option<u8> = None;
            ui.input(|inp| {
                for &(key, semitone) in KEY_MAP {
                    if inp.key_pressed(key) {
                        pressed_semitone = Some((semitone % 12) as u8);
                    }
                }
            });
            if let Some(semi) = pressed_semitone {
                self.chord_seq.root = semi;
            }
            let prev: Vec<u8> = self.piano_held_midi.drain().collect();
            for m in prev { self.push_note_off(m); }
        } else if self.seq_mode == SeqMode::ChordKb {
            let mut current_degrees = std::collections::HashSet::<usize>::new();
            ui.input(|inp| {
                for (degree, &key) in WHITE_KEYS.iter().enumerate() {
                    if inp.key_down(key) { current_degrees.insert(degree); }
                }
            });
            for &deg in &current_degrees {
                if !self.chord_kb.kb_held.contains(&deg) {
                    for m in self.chord_kb.chord_notes(deg) { self.push_note_on(m); }
                }
            }
            let released: Vec<usize> = self.chord_kb.kb_held.iter()
                .filter(|&&d| !current_degrees.contains(&d))
                .copied().collect();
            for deg in released {
                for m in self.chord_kb.chord_notes(deg) { self.push_note_off(m); }
            }
            self.chord_kb.kb_held = current_degrees;
            let prev_midi: Vec<u8> = self.piano_held_midi.drain().collect();
            for m in prev_midi { self.push_note_off(m); }
        } else {
            if !self.chord_kb.kb_held.is_empty() {
                let held: Vec<usize> = self.chord_kb.kb_held.drain().collect();
                for deg in held {
                    for m in self.chord_kb.chord_notes(deg) { self.push_note_off(m); }
                }
            }
            let mut current_held = std::collections::HashSet::<u8>::new();
            ui.input(|inp| {
                for &(key, semitone) in KEY_MAP {
                    if inp.key_down(key) {
                        current_held.insert((self.piano_octave * 12 + semitone) as u8);
                    }
                }
            });
            for &midi in &current_held {
                if !self.piano_held_midi.contains(&midi) { self.push_note_on(midi); }
            }
            let released: Vec<u8> = self.piano_held_midi.iter()
                .filter(|&&m| !current_held.contains(&m))
                .copied().collect();
            for midi in released { self.push_note_off(midi); }
            self.piano_held_midi = current_held;
        }

        self.draw_piano_88(ui);
    }

    /// Draw a full 88-key piano (A0–C8) with the active keyboard range highlighted.
    fn draw_piano_88(&mut self, ui: &mut egui::Ui) {
        let num_white = count_white_keys(); // 52
        let available_w = ui.available_width();
        let white_w = (available_w / num_white as f32).max(6.0).min(20.0);
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
            if !is_white_key(midi) { continue; }

            let x = white_x;
            white_key_x[midi as usize] = x;
            white_x += white_w;

            let rect = Rect::from_min_size(
                origin + Vec2::new(x + 0.5, 1.0),
                Vec2::new(white_w - 1.0, white_h - 2.0),
            );

            let pressed = self.piano_held_midi.contains(&midi)
                || self.piano_mouse_midi == Some(midi);
            let in_kb_range = midi >= kb_range_start && midi < kb_range_end;

            let fill = if pressed {
                self.theme.c(&self.theme.key_white_pressed)
            } else if in_kb_range {
                // Subtle highlight for the active keyboard range.
                Color32::from_rgb(230, 240, 245)
            } else {
                Color32::from_rgb(245, 245, 245)
            };

            painter.rect_filled(rect, Rounding::same(2.0), fill);
            painter.rect_stroke(rect, Rounding::same(2.0),
                Stroke::new(0.5, Color32::from_rgb(180, 180, 180)));

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
            if is_white_key(midi) { continue; }

            // Black key sits between the white key below and above.
            // Find the white key just below this black key.
            let white_below = midi - 1;
            if !is_white_key(white_below) { continue; }
            let x = white_key_x[white_below as usize] + white_w * 0.6;

            let rect = Rect::from_min_size(
                origin + Vec2::new(x, 1.0),
                Vec2::new(black_w, black_h),
            );

            let pressed = self.piano_held_midi.contains(&midi)
                || self.piano_mouse_midi == Some(midi);
            let in_kb_range = midi >= kb_range_start && midi < kb_range_end;

            let fill = if pressed {
                self.theme.c(&self.theme.key_black_pressed)
            } else if in_kb_range {
                Color32::from_rgb(40, 40, 50)
            } else {
                Color32::from_rgb(25, 25, 25)
            };

            painter.rect_filled(rect, Rounding::same(1.5), fill);

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
                if midi < PIANO_FIRST_MIDI { continue; }
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
                painter.rect_filled(bar, Rounding::same(1.0), accent);
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
