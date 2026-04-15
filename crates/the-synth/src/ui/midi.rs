use crate::SynthApp;
use eframe::egui;
use egui::Color32;

impl SynthApp {
    pub fn ui_midi_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("MIDI").strong().small());

        // Refresh port list button
        if ui.small_button("⟳").on_hover_text("Refresh MIDI device list").clicked() {
            self.midi.list_ports();
        }

        if self.midi.port_names.is_empty() {
            ui.label(egui::RichText::new("No MIDI devices found").weak().small());
            return;
        }

        // Device selector
        let connected = self.midi.connected_port;
        let current_label = connected
            .and_then(|i| self.midi.port_names.get(i))
            .map(|s| s.as_str())
            .unwrap_or("— disconnected —");

        egui::ComboBox::from_id_salt("midi_port")
            .selected_text(egui::RichText::new(current_label).small())
            .show_ui(ui, |ui| {
                let selected = connected.is_none();
                if ui.selectable_label(selected, "— disconnected —")
                    .on_hover_text("Disconnect from all MIDI devices.")
                    .clicked() {
                    self.midi.disconnect();
                }
                let names: Vec<String> = self.midi.port_names.clone();
                for (i, name) in names.iter().enumerate() {
                    let selected = connected == Some(i);
                    if ui.selectable_label(selected, name)
                        .on_hover_text(format!("Connect to MIDI device: {name}"))
                        .clicked() && !selected {
                        if let Err(e) = self.midi.connect(i) {
                            eprintln!("MIDI connect error: {e}");
                        }
                    }
                }
            });

        // Status dot
        let (color, label) = if connected.is_some() {
            (Color32::from_rgb(0, 220, 120), "●")
        } else {
            (Color32::from_gray(80), "○")
        };
        ui.label(egui::RichText::new(label).color(color).small());
    }
}
