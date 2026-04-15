use crate::SynthApp;
use crate::sequencer::SeqMode;
use eframe::egui;
use egui::{Color32, Rect, Rounding, Sense, Stroke, Vec2};

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

impl SynthApp {
    pub fn ui_keyboard_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Octave:").on_hover_text("Keyboard octave range (1–7). Shifts all keys up or down by one octave.");
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

        self.draw_piano(ui);
    }

    fn draw_piano(&mut self, ui: &mut egui::Ui) {
        let white_w = 32.0_f32;
        let white_h = 90.0_f32;
        let black_w = 20.0_f32;
        let black_h = 56.0_f32;
        let num_white = 14;
        let total_width = white_w * num_white as f32;

        let (resp, painter) = ui.allocate_painter(
            Vec2::new(total_width, white_h + 4.0),
            Sense::click_and_drag(),
        );
        let origin = resp.rect.left_top();
        let pointer_pos = resp.interact_pointer_pos();
        let mut clicked_midi: Option<u8> = None;

        for oct in 0..2_i32 {
            for (wi, &semi) in WHITE_SEMITONES.iter().enumerate() {
                let x = (oct * 7 + wi as i32) as f32 * white_w;
                let rect = Rect::from_min_size(
                    origin + Vec2::new(x + 1.0, 1.0),
                    Vec2::new(white_w - 2.0, white_h - 2.0),
                );
                let midi = ((self.piano_octave + oct) * 12 + semi) as u8;
                let pressed =
                    self.piano_held_midi.contains(&midi) || self.piano_mouse_midi == Some(midi);
                let fill = if pressed {
                    self.theme.c(&self.theme.key_white_pressed)
                } else {
                    Color32::WHITE
                };
                painter.rect_filled(rect, Rounding::same(3.0), fill);
                painter.rect_stroke(
                    rect,
                    Rounding::same(3.0),
                    Stroke::new(1.0, Color32::DARK_GRAY),
                );
                if let Some(pos) = pointer_pos {
                    if rect.contains(pos) {
                        clicked_midi = Some(midi);
                    }
                }
            }
        }
        for oct in 0..2_i32 {
            for (bi, semi_opt) in BLACK_SEMITONES.iter().enumerate() {
                let Some(semi) = semi_opt else { continue };
                let x = (oct * 7 + bi as i32) as f32 * white_w + white_w * 0.6;
                let rect =
                    Rect::from_min_size(origin + Vec2::new(x, 1.0), Vec2::new(black_w, black_h));
                let midi = ((self.piano_octave + oct) * 12 + semi) as u8;
                let pressed =
                    self.piano_held_midi.contains(&midi) || self.piano_mouse_midi == Some(midi);
                let fill = if pressed {
                    self.theme.c(&self.theme.key_black_pressed)
                } else {
                    Color32::BLACK
                };
                painter.rect_filled(rect, Rounding::same(2.0), fill);
                if let Some(pos) = pointer_pos {
                    if rect.contains(pos) {
                        clicked_midi = Some(midi);
                    }
                }
            }
        }

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
