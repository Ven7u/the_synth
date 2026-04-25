# Plugin Signing & Distribution Runbook

## Prerequisites

- Apple Developer account ($99/year): https://developer.apple.com/account
- **Developer ID Application** certificate installed in Keychain Access
- Xcode command-line tools: `xcode-select --install`
- `cargo xtask` alias set up (see `.cargo/config.toml`)

---

## Build

```bash
cargo xtask bundle synth-plugin --release
# Output: target/bundled/TheSynth.clap
#         target/bundled/TheSynth.vst3
#         target/bundled/TheSynth.component   ← Audio Unit (AU)
```

---

## Code Signing

Replace `Your Name (TEAM_ID)` with your actual name and 10-character Team ID
(visible at https://developer.apple.com/account → Membership Details).

```bash
IDENTITY="Developer ID Application: Your Name (TEAM_ID)"

# Sign with hardened runtime (required for notarization)
codesign --sign "$IDENTITY" \
         --timestamp \
         --options runtime \
         --force \
         target/bundled/TheSynth.component

# Verify signature
codesign --verify --verbose target/bundled/TheSynth.component
```

---

## Notarization

Notarization requires an **app-specific password** stored in Keychain:

```bash
# Store credentials once
xcrun notarytool store-credentials "AC_PASSWORD" \
    --apple-id you@example.com \
    --team-id TEAM_ID \
    --password "xxxx-xxxx-xxxx-xxxx"   # app-specific password from appleid.apple.com

# Zip the bundle (notarytool requires a zip or dmg)
ditto -c -k --keepParent \
    target/bundled/TheSynth.component \
    TheSynth.component.zip

# Submit and wait
xcrun notarytool submit TheSynth.component.zip \
    --keychain-profile "AC_PASSWORD" \
    --wait

# Staple the notarization ticket
xcrun stapler staple target/bundled/TheSynth.component
```

---

## Installation

```bash
# User-level (GarageBand + Logic Pro will find it here)
cp -R target/bundled/TheSynth.component \
      ~/Library/Audio/Plug-Ins/Components/

# System-level (all users, requires sudo)
sudo cp -R target/bundled/TheSynth.component \
           /Library/Audio/Plug-Ins/Components/
```

---

## Verification in Logic Pro

1. Open Logic Pro → Settings → Plug-in Manager
2. Click **Reset & Rescan Selection** or **Rescan**
3. Search for "The Synth" — it should appear under Instruments
4. Create an instrument track, insert The Synth, play a MIDI note

---

## Testing Without an Apple Developer Certificate

macOS Gatekeeper blocks unsigned dylibs by default. Workarounds for
**local development only** (never distribute unsigned builds):

```bash
# Option 1: Grant an explicit Gatekeeper exception for this specific bundle
spctl --add --label "TheSynth Dev" target/bundled/TheSynth.component

# Option 2: In Logic Pro's Plug-in Manager, enable "Use Audio Units in
# compatibility mode" — this relaxes validation for testing.

# Option 3: Build and run the standalone app instead:
cargo run -p the-synth --release
```

GarageBand does **not** support unsigned plugins. Logic Pro's compatibility
mode is the only option without a certificate.

---

## Updating the Plugin

After code changes:

```bash
cargo xtask bundle synth-plugin --release
codesign --sign "$IDENTITY" --timestamp --options runtime --force \
    target/bundled/TheSynth.component
xcrun stapler staple target/bundled/TheSynth.component
cp -R target/bundled/TheSynth.component ~/Library/Audio/Plug-Ins/Components/
# Restart Logic Pro / GarageBand to pick up the new build
```
