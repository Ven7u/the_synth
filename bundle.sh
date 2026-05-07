#!/usr/bin/env bash
# Build a release .app bundle and package it as a DMG.
# Usage:  ./bundle.sh [--skip-build]
set -euo pipefail

CRATE_DIR="crates/the-synth"
APP_NAME="The Synth"
DMG_NAME="TheSynth"
OUT_DIR="$(pwd)/dist"

# ── 1. Build release .app ────────────────────────────────────────────────────
if [[ "${1:-}" != "--skip-build" ]]; then
    echo "▶ Building release bundle…"
    cargo bundle --release -p the-synth
fi

APP_PATH="target/release/bundle/osx/${APP_NAME}.app"

if [[ ! -d "$APP_PATH" ]]; then
    echo "✗ Expected .app not found at $APP_PATH"
    exit 1
fi

echo "✓ Bundle: $APP_PATH"

# ── 2. Package as DMG ────────────────────────────────────────────────────────
mkdir -p "$OUT_DIR"
DMG_PATH="$OUT_DIR/${DMG_NAME}.dmg"
STAGING="$(mktemp -d)"

cp -r "$APP_PATH" "$STAGING/"
ln -s /Applications "$STAGING/Applications"

echo "▶ Creating DMG…"
hdiutil create \
    -volname "$APP_NAME" \
    -srcfolder "$STAGING" \
    -ov \
    -format UDZO \
    "$DMG_PATH"

rm -rf "$STAGING"

echo "✓ Done: $DMG_PATH"
echo "  Size: $(du -sh "$DMG_PATH" | cut -f1)"
