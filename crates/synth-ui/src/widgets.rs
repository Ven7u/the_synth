//! Common widget helpers.
//!
//! Placeholder module — widgets will be extracted from the_synth in later iterations.
//! For now this crate establishes the dependency boundary.

use egui::Ui;

/// A minimal labelled knob (drag vertical to change value).
/// `value`: current value in [min, max].
/// Returns true if the value changed.
pub fn knob(ui: &mut Ui, label: &str, value: &mut f32, min: f32, max: f32) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        let resp = ui.add(
            egui::DragValue::new(value)
                .range(min..=max)
                .speed((max - min) * 0.005)
                .max_decimals(3),
        );
        ui.label(egui::RichText::new(label).small());
        changed = resp.changed();
    });
    changed
}
