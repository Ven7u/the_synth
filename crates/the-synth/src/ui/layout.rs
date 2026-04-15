use serde::{Deserialize, Serialize};

/// Persisted layout state — saved/loaded from disk.
#[derive(Clone, Serialize, Deserialize)]
pub struct LayoutState {
    pub theme_name: String,
    pub panels: PanelVisibilityState,
}

/// Serializable mirror of PanelVisibility.
#[derive(Clone, Serialize, Deserialize)]
pub struct PanelVisibilityState {
    pub oscillators: bool,
    pub modulation: bool,
    pub keyboard: bool,
    pub sequencer: bool,
    pub arp_walker: bool,
    pub fx_chain: bool,
    pub scope: bool,
    pub midi: bool,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            theme_name: "Midnight".into(),
            panels: PanelVisibilityState {
                oscillators: true,
                modulation: true,
                keyboard: true,
                sequencer: true,
                arp_walker: true,
                fx_chain: true,
                scope: true,
                midi: true,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Layout presets
// ---------------------------------------------------------------------------

pub struct LayoutPreset {
    pub name: &'static str,
    pub description: &'static str,
    pub panels: PanelVisibilityState,
}

/// Sound Design — all panels visible (default).
pub fn preset_sound_design() -> LayoutPreset {
    LayoutPreset {
        name: "Sound Design",
        description: "All panels visible for patch creation.",
        panels: PanelVisibilityState {
            oscillators: true,
            modulation: true,
            keyboard: true,
            sequencer: true,
            arp_walker: true,
            fx_chain: true,
            scope: true,
            midi: true,
        },
    }
}

/// Performance — minimal: keyboard, FX, scope.
pub fn preset_performance() -> LayoutPreset {
    LayoutPreset {
        name: "Performance",
        description: "Keyboard + FX + Scope for live playing.",
        panels: PanelVisibilityState {
            oscillators: false,
            modulation: false,
            keyboard: true,
            sequencer: false,
            arp_walker: false,
            fx_chain: true,
            scope: true,
            midi: false,
        },
    }
}

/// Sequencer — sequencer, arp/walker, scope.
pub fn preset_sequencer() -> LayoutPreset {
    LayoutPreset {
        name: "Sequencer",
        description: "Sequencer + Arp/Walker + Scope for pattern work.",
        panels: PanelVisibilityState {
            oscillators: false,
            modulation: false,
            keyboard: true,
            sequencer: true,
            arp_walker: true,
            fx_chain: false,
            scope: true,
            midi: false,
        },
    }
}

pub fn builtin_presets() -> Vec<LayoutPreset> {
    vec![preset_sound_design(), preset_performance(), preset_sequencer()]
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

fn layout_path() -> std::path::PathBuf {
    std::path::PathBuf::from("the-synth-layout.json")
}

pub fn save_layout(state: &LayoutState) {
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(layout_path(), json);
    }
}

pub fn load_layout() -> LayoutState {
    let path = layout_path();
    if let Ok(json) = std::fs::read_to_string(&path) {
        serde_json::from_str(&json).unwrap_or_default()
    } else {
        LayoutState::default()
    }
}
