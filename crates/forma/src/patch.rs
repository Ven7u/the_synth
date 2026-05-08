//! Patch re-export + disk loader.
//!
//! The `Patch` struct itself lives in [`forma_engine::patch`] so that any
//! frontend (egui, Bevy, Swift, web, DAW plugin) can round-trip patches
//! through the engine handle without re-implementing the schema. This
//! module keeps the `crate::patch::Patch` import path working and owns the
//! UI-side concern of scanning `assets/patches/**/*.json` from disk.

pub use forma_engine::Patch;

// ---------------------------------------------------------------------------
// Default patches — scanned from assets/patches/**/*.json at runtime.
// Drop any .json file into a subfolder and it appears in both apps automatically.
// ---------------------------------------------------------------------------

fn collect_patch_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if p.is_dir() {
            collect_patch_files(&p, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("json"))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
}

/// Locate the patches directory, trying (in order):
///   1. `assets/patches` relative to CWD  — works with `cargo run`
///   2. `../Resources/patches` relative to the executable — works inside a .app bundle
///      (executable lives at `Contents/MacOS/`, resources at `Contents/Resources/`)
fn patches_dir() -> std::path::PathBuf {
    let cwd_path = std::path::Path::new("assets/patches");
    if cwd_path.is_dir() {
        return cwd_path.to_path_buf();
    }
    if let Ok(exe) = std::env::current_exe() {
        let bundle_path = exe
            .parent() // Contents/MacOS
            .and_then(|p| p.parent()) // Contents
            .map(|p| p.join("Resources").join("assets").join("patches"));
        if let Some(p) = bundle_path {
            if p.is_dir() {
                return p;
            }
        }
    }
    cwd_path.to_path_buf()
}

pub fn default_patches() -> Vec<Patch> {
    let mut files = Vec::new();
    collect_patch_files(&patches_dir(), &mut files);
    files.sort();
    files
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|s| serde_json::from_str(&s).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every on-disk preset in `assets/patches/**/*.json` must continue to
    /// parse via the migrated `Patch` struct (now in `forma-engine`).
    /// Catches serde-field drift during the Stage 3 move.
    #[test]
    fn every_bundled_patch_json_deserialises() {
        // Walk relative to the workspace root; `cargo test` sets CWD there.
        let root = std::env::var_os("CARGO_MANIFEST_DIR")
            .map(std::path::PathBuf::from)
            .and_then(|p| p.parent().map(|w| w.parent().unwrap().to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let patches_dir = root.join("assets/patches");
        if !patches_dir.exists() {
            // Skip silently on environments without assets.
            return;
        }

        let mut files = Vec::new();
        collect_patch_files(&patches_dir, &mut files);
        assert!(
            !files.is_empty(),
            "no patch files under {}",
            patches_dir.display()
        );

        let mut failures = Vec::new();
        for f in &files {
            let body = match std::fs::read_to_string(f) {
                Ok(s) => s,
                Err(e) => {
                    failures.push(format!("{}: read error {}", f.display(), e));
                    continue;
                }
            };
            if let Err(e) = serde_json::from_str::<Patch>(&body) {
                failures.push(format!("{}: {}", f.display(), e));
            }
        }
        assert!(
            failures.is_empty(),
            "{}/{} patch file(s) failed to deserialise:\n{}",
            failures.len(),
            files.len(),
            failures.join("\n"),
        );
    }
}
