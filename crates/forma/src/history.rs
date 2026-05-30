//! Two-layer patch history: rolling auto-snapshots + named manual pins.

use crate::patch::Patch;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// How many auto snapshots to keep before evicting the oldest.
pub const AUTO_CAPACITY: usize = 30;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchSnapshot {
    /// Unix timestamp (seconds) — used for display only.
    pub timestamp: u64,
    /// None = auto, Some = manual pin.
    pub label: Option<String>,
    pub patch: Patch,
}

impl PatchSnapshot {
    pub fn is_manual(&self) -> bool {
        self.label.is_some()
    }

    /// Human-readable age string: "just now", "2 min ago", "1 h ago", …
    pub fn age_str(&self, now: u64) -> String {
        let secs = now.saturating_sub(self.timestamp);
        if secs < 10 {
            "just now".into()
        } else if secs < 60 {
            format!("{}s ago", secs)
        } else if secs < 3600 {
            format!("{}m ago", secs / 60)
        } else {
            format!("{}h ago", secs / 3600)
        }
    }
}

/// The full history store: manual pins + rolling auto-snapshots, newest first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PatchHistory {
    pub entries: Vec<PatchSnapshot>,
}

impl PatchHistory {
    /// Add an auto-snapshot. Evicts oldest auto entries beyond `AUTO_CAPACITY`.
    pub fn push_auto(&mut self, patch: Patch) {
        self.entries.insert(
            0,
            PatchSnapshot {
                timestamp: now_secs(),
                label: None,
                patch,
            },
        );
        // Evict excess auto entries (keep all manual pins).
        let mut auto_count = 0;
        self.entries.retain(|e| {
            if e.is_manual() {
                true
            } else {
                auto_count += 1;
                auto_count <= AUTO_CAPACITY
            }
        });
    }

    /// Add a manual pin with a user-supplied label.
    pub fn push_manual(&mut self, patch: Patch, label: impl Into<String>) {
        self.entries.insert(
            0,
            PatchSnapshot {
                timestamp: now_secs(),
                label: Some(label.into()),
                patch,
            },
        );
    }

    /// Replace the label of the entry at `index`.
    pub fn rename(&mut self, index: usize, label: impl Into<String>) {
        if let Some(e) = self.entries.get_mut(index) {
            e.label = Some(label.into());
        }
    }

    /// Delete an entry by index.
    pub fn remove(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
        }
    }
}

// ── Persistence ───────────────────────────────────────────────────────────────

fn history_path() -> std::path::PathBuf {
    std::path::PathBuf::from("forma-patch-history.json")
}

pub fn load_history() -> PatchHistory {
    std::fs::read_to_string(history_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_history(h: &PatchHistory) {
    if let Ok(json) = serde_json::to_string_pretty(h) {
        let _ = std::fs::write(history_path(), json);
    }
}
