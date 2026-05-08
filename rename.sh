#!/usr/bin/env bash
# Rename all synth-* / forma crates to forma-* / forma.
# Requires: sd  (cargo install sd)
#
# Run from the workspace root:
#   ./rename.sh          # dry-run: shows what would change
#   ./rename.sh --apply  # actually makes the changes
#
# After running with --apply:
#   cargo check          # verify nothing was missed
#   git diff --stat      # review scope before committing
set -euo pipefail

APPLY=false
if [[ "${1:-}" == "--apply" ]]; then
    APPLY=true
fi

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

# ── Sanity check ────────────────────────────────────────────────────────────
if ! command -v sd &>/dev/null; then
    echo "✗ 'sd' not found. Install with: cargo install sd"
    exit 1
fi

# ── Rename map ───────────────────────────────────────────────────────────────
# Format: "old-hyphen-name|old_underscore_name|new-hyphen-name|new_underscore_name"
RENAMES=(
    "forma-engine|forma_engine|forma-engine|forma_engine"
    "forma-dsp|forma_dsp|forma-dsp|forma_dsp"
    "forma-control|forma_control|forma-control|forma_control"
    "forma-common|forma_common|forma-common|forma_common"
    "forma-bevy|forma_bevy|forma-bevy|forma_bevy"
    "forma-ui|forma_ui|forma-ui|forma_ui"
    "forma-bench|forma_bench|forma-bench|forma_bench"
    "forma|forma|forma|forma"
    "forma-ambient|forma_ambient|forma-ambient|forma_ambient"
)

echo ""
echo "════════════════════════════════════════════════════"
echo "  Forma rename — $([ "$APPLY" = true ] && echo 'APPLYING' || echo 'DRY RUN')"
echo "════════════════════════════════════════════════════"
echo ""

# ── Step 1: text substitutions across all files ──────────────────────────────
# File patterns to touch (excludes target/, .git, binary assets)
FILES_TOML=$(find crates -name "Cargo.toml")
FILES_RS=$(find crates -name "*.rs" -not -path "*/target/*")
FILES_MD=$(find . -name "*.md" -not -path "*/target/*" -not -path "*/.git/*")
FILES_SH=$(find . -name "*.sh" -not -path "*/target/*")

for entry in "${RENAMES[@]}"; do
    IFS='|' read -r old_hyph old_under new_hyph new_under <<< "$entry"

    # Count occurrences before replacing
    hits_hyph=$(grep -rl "$old_hyph" $FILES_TOML $FILES_RS $FILES_MD $FILES_SH 2>/dev/null | wc -l | tr -d ' ')
    hits_under=$(grep -rl "$old_under" $FILES_RS $FILES_MD $FILES_SH 2>/dev/null | wc -l | tr -d ' ')

    echo "  $old_hyph  →  $new_hyph   (${hits_hyph} files)"
    echo "  $old_under  →  $new_under   (${hits_under} files)"

    if [ "$APPLY" = true ]; then
        # Hyphenated form: Cargo.toml, .rs, .md, .sh
        sd "$old_hyph" "$new_hyph" $FILES_TOML $FILES_RS $FILES_MD $FILES_SH 2>/dev/null || true
        # Underscored form: .rs, .md, .sh only
        sd "$old_under" "$new_under" $FILES_RS $FILES_MD $FILES_SH 2>/dev/null || true
    fi
    echo ""
done

# ── Step 2: rename directories ───────────────────────────────────────────────
echo "Directory renames:"
DIR_RENAMES=(
    "crates/forma-engine|crates/forma-engine"
    "crates/forma-dsp|crates/forma-dsp"
    "crates/forma-control|crates/forma-control"
    "crates/forma-common|crates/forma-common"
    "crates/forma-bevy|crates/forma-bevy"
    "crates/forma-ui|crates/forma-ui"
    "crates/forma-bench|crates/forma-bench"
    "crates/forma|crates/forma"
    "crates/forma-ambient|crates/forma-ambient"
)

for entry in "${DIR_RENAMES[@]}"; do
    IFS='|' read -r old_dir new_dir <<< "$entry"
    if [ -d "$old_dir" ]; then
        echo "  mv $old_dir  →  $new_dir"
        if [ "$APPLY" = true ]; then
            mv "$old_dir" "$new_dir"
        fi
    fi
done

echo ""

# ── Step 3: rename bin source file for forma-bench ───────────────────────────
if [ "$APPLY" = true ] && [ -f "crates/forma-bench/src/bin/synth-perf.rs" ]; then
    mv "crates/forma-bench/src/bin/synth-perf.rs" "crates/forma-bench/src/bin/forma-perf.rs"
    # Fix the bin path reference in Cargo.toml
    sd 'src/bin/synth-perf.rs' 'src/bin/forma-perf.rs' crates/forma-bench/Cargo.toml
    sd '"synth-perf"' '"forma-perf"' crates/forma-bench/Cargo.toml
    echo "  renamed synth-perf.rs → forma-perf.rs"
fi

# ── Step 4: update bundle identifier ─────────────────────────────────────────
if [ "$APPLY" = true ] && [ -f "crates/forma/Cargo.toml" ]; then
    sd 'com.francescoventura.forma' 'com.francescoventura.forma' crates/forma/Cargo.toml
    echo "  updated bundle identifier"
fi

# ── Step 5: rename bundle.sh output references ───────────────────────────────
if [ "$APPLY" = true ] && [ -f "bundle.sh" ]; then
    sd 'forma' 'forma' bundle.sh
    sd 'The Synth' 'Forma' bundle.sh
    sd 'TheSynth' 'Forma' bundle.sh
    echo "  updated bundle.sh"
fi

echo ""
if [ "$APPLY" = true ]; then
    echo "✓ Done. Run 'cargo check' to verify."
else
    echo "Dry run complete. Run './rename.sh --apply' to apply changes."
fi
