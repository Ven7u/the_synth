//! Scene save/load — a scene is a complete snapshot of the 4-track rig.
//!
//! Scenes are stored as a JSON array in `forma-scenes.json` next to the
//! working directory (same convention as `forma-layout.json`).

use crate::audio::TRACK_COUNT;
use crate::patch::Patch;
use crate::ui::drum_machine_ui::DrumMachineState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Scene struct ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub name: String,
    pub global_bpm: u32,
    pub track_names: [String; TRACK_COUNT],
    pub track_patches: [Patch; TRACK_COUNT],
    pub track_volumes: [f32; TRACK_COUNT],
    pub track_pans: [f32; TRACK_COUNT],
    pub track_muted: [bool; TRACK_COUNT],
    pub drums: DrumMachineState,
    /// Keyboard split: inclusive low note per track (default 0).
    #[serde(default = "default_key_lo")]
    pub track_key_lo: [u8; TRACK_COUNT],
    /// Keyboard split: inclusive high note per track (default 127).
    #[serde(default = "default_key_hi")]
    pub track_key_hi: [u8; TRACK_COUNT],
    /// MIDI channel filter per track: 0 = omni, 1–16 = specific channel.
    #[serde(default)]
    pub track_midi_ch: [u8; TRACK_COUNT],
}

fn default_key_lo() -> [u8; TRACK_COUNT] {
    [0u8; TRACK_COUNT]
}
fn default_key_hi() -> [u8; TRACK_COUNT] {
    [127u8; TRACK_COUNT]
}

// ── Disk I/O ─────────────────────────────────────────────────────────────────

fn scenes_path() -> PathBuf {
    PathBuf::from("forma-scenes.json")
}

pub fn save_scenes(scenes: &[Scene]) {
    if let Ok(json) = serde_json::to_string_pretty(scenes) {
        let _ = std::fs::write(scenes_path(), json);
    }
}

pub fn load_scenes() -> Vec<Scene> {
    let path = scenes_path();
    if let Ok(json) = std::fs::read_to_string(&path) {
        serde_json::from_str(&json).unwrap_or_default()
    } else {
        Vec::new()
    }
}
