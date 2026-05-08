# Forma

A polyphonic software synthesizer written in Rust — built for expressive sound design, generative music, and game engine integration.

> **Status:** Active development. macOS only. Tested on macOS 15 Sequoia.

---

## What it is

Forma is a MiniMoog-inspired synthesizer that runs as a native desktop application. It pairs a clean, realtime UI with a headless audio engine designed to embed in any Rust host — an egui app today, a Bevy game tomorrow.

The engine and the interface are fully decoupled. Every parameter is a lock-free atomic. There is no shared mutable state between the UI thread and the audio thread.

---

## Features

### Sound engine
- **3 oscillators per voice** — sine, saw, square (with pulse width), triangle
- **6-voice polyphony** with oldest-first voice stealing
- **Unison mode** per oscillator — up to 5 detuned copies with spread control
- **Hard sync** (OSC 1 → OSC 2)
- **FM synthesis** — OSC 2 modulates OSC 1 at audio rate
- **Ring modulation** — OSC 1 × OSC 2
- **Noise generator**
- **Moog-style 4-pole lowpass filter** with resonance, drive (asymmetric tanh saturation), key tracking, and filter envelope (ADSR)
- **Amplitude envelope** (ADSR) per voice
- **Glide / portamento**
- **2 independent LFOs** — sine, triangle, saw; routable to pitch, filter, or amplitude; BPM-syncable
- **Master volume** and **output limiter** (lookahead true-peak)

### Effects chain
- **Overdrive** — asymmetric soft-clip with tone and bias controls
- **Distortion** — hard clip with pre-filter and tone shaping
- **Chorus** — stereo BBD-style
- **Delay** — stereo with BPM sync
- **Reverb** — three algorithms: Freeverb, Plate, FDN Hall
- **Shimmer reverb** — pitch-shifted feedback reverb with stereo spread
- **Crystallizer** — granular pitch-shifting delay

### Performance
- **Arpeggiator** — Up, Down, Up/Down, Random, As Played; BPM-syncable; octave range up to 4
- **Scale walker** — autonomous random walk within any scale (Major, Minor, Dorian, Pentatonic, Blues, Chromatic, and more)
- **Chord keyboard** — 3×7 pad grid triggers diatonic chords; triads, 7ths, sus, add variants
- **Step sequencer** — 16-step with per-step gate, transposable
- **MIDI input** — note on/off, pitch bend, sustain pedal (CC64)

### Interface
- **Keyboard shortcuts** matching GarageBand's software keyboard layout
  - `A`–`'` white keys, `W E T Y U O P` sharps (two-octave span)
  - `Z` / `X` — octave down / up
  - `C` / `V` — velocity down / up
  - Hold `1` / `2` — pitch bend ±2 semitones
  - `3`–`8` — modulation wheel (routed to filter cutoff)
  - `Space` — freeze (sustain all held notes)
- **Oscilloscope** — realtime waveform display
- **Patch library** — 171 factory presets across 20 categories including Ambient, Cinematic, Synth, Lead, Pad, Bass, and artist-inspired collections

### Architecture
The workspace is structured as a set of focused crates:

| Crate | Role |
|---|---|
| `forma-engine` | Headless audio engine: DSP graph, voice allocator, patch schema |
| `forma-dsp` | DSP primitives: envelopes, oscillators, effects, crystallizer, shimmer |
| `forma-control` | Lock-free control protocol: events, parameters, MIDI parsing |
| `forma-common` | Shared types (clock divisions, scales) |
| `forma` | egui/eframe desktop application |
| `forma-bevy` | Bevy plugin *(roadmap)* |

---

## Building

**Requirements**
- Rust 1.89 or later (`rustup update stable`)
- macOS 15 Sequoia or later (earlier versions may work; not tested)
- Default system audio output

```bash
git clone https://github.com/francescoventura/forma
cd forma
cargo run -p forma --release
```

**Package as a .app and DMG**
```bash
cargo install cargo-bundle --locked
./bundle.sh
# → dist/TheSynth.dmg
```

---

## Patch library

Presets are plain JSON files in `assets/patches/`. Each folder is a category — drop a new `.json` file anywhere in that tree and it appears in the library at next launch. No database, no indexing step.

To save a patch from within the app: tweak the sound, type a name in the patch field, and save. Patches round-trip through the engine's typed schema so older files load safely with `#[serde(default)]` on every new field.

---

## Roadmap

- [ ] Bevy plugin for in-game use
- [ ] Generative engine (Markov chains, harmonic sequencing)
- [ ] Filter modes: BP, HP, Notch
- [ ] CLAP / VST plugin shell
- [ ] Linux and Windows support

---

## License

MIT — see [LICENSE](LICENSE).
