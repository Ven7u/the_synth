use egui::{Context, Id, Pos2, Rect};
use std::collections::HashMap;

const SNAP_DISTANCE: f32 = 12.0;

/// Window names — must match the egui::Window titles exactly.
pub const WINDOW_NAMES: &[&str] = &[
    "Oscillators",
    "Modulation & Filter",
    "Keyboard",
    "Sequencer",
    "Arpeggiator & Walker",
    "FX Chain",
    "Oscilloscope",
    "MIDI & Latency",
];

/// Compute snap offsets for each window. Returns a map of window name → snapped position.
/// Call this before rendering windows and pass the positions via `.current_pos()`.
pub fn compute_snap_positions(ctx: &Context) -> HashMap<&'static str, Pos2> {
    let mut result = HashMap::new();

    // Collect current window rects.
    let mut entries: Vec<(&str, Id, Rect)> = Vec::new();
    for &name in WINDOW_NAMES {
        let id = Id::new(name);
        if let Some(rect) = ctx.memory(|mem| mem.area_rect(id)) {
            entries.push((name, id, rect));
        }
    }

    if entries.is_empty() {
        return result;
    }

    // Don't snap while dragging.
    let is_dragging = ctx.input(|i| i.pointer.any_down());
    if is_dragging {
        return result;
    }

    let viewport = ctx.screen_rect();

    for i in 0..entries.len() {
        let (name, _id, rect) = entries[i];
        let mut new_pos = rect.min;
        let mut snapped_x = false;
        let mut snapped_y = false;

        // Snap to viewport edges.
        if !snapped_x && (rect.left() - viewport.left()).abs() < SNAP_DISTANCE {
            new_pos.x = viewport.left();
            snapped_x = true;
        }
        if !snapped_x && (rect.right() - viewport.right()).abs() < SNAP_DISTANCE {
            new_pos.x = viewport.right() - rect.width();
            snapped_x = true;
        }
        if !snapped_y && (rect.top() - viewport.top()).abs() < SNAP_DISTANCE {
            new_pos.y = viewport.top();
            snapped_y = true;
        }
        if !snapped_y && (rect.bottom() - viewport.bottom()).abs() < SNAP_DISTANCE {
            new_pos.y = viewport.bottom() - rect.height();
            snapped_y = true;
        }

        // Snap to other windows.
        for j in 0..entries.len() {
            if i == j {
                continue;
            }
            let (_, _, other) = entries[j];

            let overlap_y = rect.top() < other.bottom() && rect.bottom() > other.top();
            let overlap_x = rect.left() < other.right() && rect.right() > other.left();

            if !snapped_x && overlap_y {
                if (rect.left() - other.right()).abs() < SNAP_DISTANCE {
                    new_pos.x = other.right();
                    snapped_x = true;
                }
                if !snapped_x && (rect.right() - other.left()).abs() < SNAP_DISTANCE {
                    new_pos.x = other.left() - rect.width();
                    snapped_x = true;
                }
            }
            if !snapped_y && overlap_x {
                if (rect.top() - other.bottom()).abs() < SNAP_DISTANCE {
                    new_pos.y = other.bottom();
                    snapped_y = true;
                }
                if !snapped_y && (rect.bottom() - other.top()).abs() < SNAP_DISTANCE {
                    new_pos.y = other.top() - rect.height();
                    snapped_y = true;
                }
            }

            // Align edges.
            if !snapped_x && overlap_y {
                if (rect.left() - other.left()).abs() < SNAP_DISTANCE {
                    new_pos.x = other.left();
                    snapped_x = true;
                }
                if !snapped_x && (rect.right() - other.right()).abs() < SNAP_DISTANCE {
                    new_pos.x = other.right() - rect.width();
                    snapped_x = true;
                }
            }
            if !snapped_y && overlap_x {
                if (rect.top() - other.top()).abs() < SNAP_DISTANCE {
                    new_pos.y = other.top();
                    snapped_y = true;
                }
                if !snapped_y && (rect.bottom() - other.bottom()).abs() < SNAP_DISTANCE {
                    new_pos.y = other.bottom() - rect.height();
                    snapped_y = true;
                }
            }
        }

        // Only store if actually snapped.
        if (new_pos.x - rect.min.x).abs() > 0.5 || (new_pos.y - rect.min.y).abs() > 0.5 {
            result.insert(name, new_pos);
        }
    }

    result
}
