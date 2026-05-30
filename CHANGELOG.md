# Changelog

All notable changes to Forma are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Versions follow [Semantic Versioning](https://semver.org/).

---

## [Unreleased]

---

## [0.1.0] — 2026-05-30

First public release.

### Sound engine
- 3 oscillators (sine, saw, square with pulse width, triangle, noise) with unison, FM, ring mod, hard sync
- 8-voice polyphony with oldest-first stealing and velocity sensitivity
- Mono / legato mode with glide
- Moog-style 4-pole lowpass filter with resonance, drive, key tracking, and filter ADSR
- Amp ADSR envelope
- Lookahead true-peak limiter on the mix bus
- 8-band parametric EQ (biquad cascade, draggable-dot UI)

### Modulation
- 2 independent LFOs — multiple waveforms, BPM sync, gate-triggered retrigger
- 4-slot mod matrix (any source → any destination)
- Aftertouch and mod wheel routing
- Pulse / gate lanes for rhythmic LFO and amplitude modulation

### FX chain
- Overdrive, Distortion, Chorus, Delay (BPM sync), Reverb (Freeverb / Plate / FDN Hall), Shimmer, Crystallizer

### Sequencer & arpeggiator
- 16-step sequencer with chord and scale modes
- Arpeggiator with multiple modes, up to 4 octaves, BPM sync
- Scale walker — generative random walk within a scale
- Scale highlight — on-screen keyboard marks valid notes
- Chord voicings with voice leading and arrow-key control

### Drum machine
- 8 channels × 16 steps (KICK, SNARE, HAT, CLAP, TOM×2, PERC, NOISE)
- 4 pattern slots (A/B/C/D) with copy, paste, clear
- Per-step velocity via drag
- Euclidean rhythm generator per lane (hits / steps / offset)
- Voice editor — per-channel synthesis params (base freq, sweep, decay, noise mix)
- Solo, Mute, Reverse, Randomize per lane
- Kit preset save/load/export with 3 factory kits

### MIDI
- Auto-connect on launch; rescans every 2 s if disconnected
- MIDI learn — bind any CC to any parameter
- Keyboard presets — Arturia KeyLab MkIII, MiniLab MkIII, Generic
- Patch navigation from hardware (wheel, prev/next, favourites, randomize, Program Change)
- MIDI monitor for live CC inspection

### Patch management
- 171 factory presets across 20 categories
- Tag and category browser with favourites and recents
- Patch history — auto-snapshots every 3 s + named manual pins, persisted to disk

### Interface
- Dockable panels (egui-dock)
- Multiple themes (Midnight, Nord, Warm, and more)
- Oscilloscope with latency meter
- On-screen keyboard with scale highlighting and GarageBand-style key bindings

[Unreleased]: https://github.com/Ven7u/forma-synth/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Ven7u/forma-synth/releases/tag/v0.1.0
