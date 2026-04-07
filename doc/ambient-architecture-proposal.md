# Ambient Synth — Architecture Proposal

This document proposes the architectural evolution of `the_synth` from a single mono-timbral
synthesizer into a modular, multi-layer ambient music engine. It is a design document — no code
changes are implied yet.

---

## 1. Vision

The target experience is a **self-modulating ambient rig** that a musician can control with a
small set of expressive macros. The reference setup (Volca Bass + Zebra-style arp + Legend bass
+ Omnisphere strings + Valhalla Shimmer + SoundToys Crystallizer) demonstrates the key idea:
**four simultaneous instruments with different textural roles, glued by two signature spatial
effects, controlled by a few meaningful knobs**.

The synth should be able to reproduce this archetype without requiring the user to manage four
separate applications or manual routing.

### Textural roles in the reference setup

| Layer role | Character | Rhythmic density | Frequency range |
|---|---|---|---|
| **Foundation** (Legend bass) | Deep, slow, sustained | Near-static | Sub / low |
| **Pulse** (Volca arp) | Analog, melodic, looping | 1/8–1/16 notes | Low-mid |
| **Harmonic color** (Zebra arp) | Dark, metallic, complex | Variable | Mid-high |
| **Glue pad** (Omnisphere strings) | Lush, slow attack, sustain | Slow chord changes | Full range |

These four roles map directly to a **four-layer architecture**, each layer carrying its own
synthesis voice, arpeggiator, and local effects, feeding into shared spatial buses.

---

## 2. Design Principles

### Library-first
The synthesis engine must be usable as a Rust library (`synth-engine`) independently of the
egui UI or the cpal audio backend. This enables embedding in other applications, headless
testing, and future integration with DAW plugin formats (VST3/CLAP via `nih-plug`).

### Real-time safe on the audio thread
No heap allocation, no mutex lock, no blocking I/O on the audio thread. All cross-thread
communication uses `fundsp::Shared` (lock-free atomic f32) or `Arc<AtomicU*>`. This is already
the pattern in the current codebase and must be preserved and extended.

### Static graph with dynamic parameters
DSP graphs are built once at initialisation. Runtime changes (mute a layer, change a patch,
adjust reverb size) are expressed through parameter writes to `Shared` values, never through
graph restructuring. Inactive paths are silenced by setting their volume `Shared` to 0.0.

### Composable, independently testable modules
Each module (oscillator bank, arpeggiator, effects node, macro engine) must compile and be
testable in isolation. No module should import from the UI crate.

### Predictable gain staging
Every summing point must have a defined worst-case amplitude and a corresponding normalization
factor. Layers are mixed before the global buses. The master bus is the last and only safety
net. See `dsp-guidelines.md` for the full rules.

---

## 3. Proposed Crate Structure

The project should evolve from a single crate into a **Cargo workspace** with three crates.

```mermaid
graph TD
    subgraph Workspace["Cargo Workspace — the_synth"]
        DSP["synth-dsp  ·  lib\n───────────────\nDSP primitives\nMultiWaveOsc · LiveAdsr\nMoog filter · FxNodes\nShimmer · Crystallizer\nAll fundsp AudioNodes"]
        ENGINE["synth-engine  ·  lib\n───────────────\nLayer system\nArpeggiator\nMacro engine\nScene management\nMIDI routing\nPatch serialization"]
        APP["synth-app  ·  bin\n───────────────\negui UI panels\ncpal audio stream\nMIDI device mgmt\nFile dialogs"]
    end

    DSP -->|"DSP nodes + Shared params"| ENGINE
    ENGINE -->|"AudioState + event bus"| APP

    style DSP fill:#1a2a3a,stroke:#4a7fa5
    style ENGINE fill:#1a3a2a,stroke:#4aa57f
    style APP fill:#3a2a1a,stroke:#a57f4a
```

### `synth-dsp` (library crate)
Pure signal processing. No I/O, no UI, no serialization. Depends only on `fundsp` and `std`.
This crate contains every `AudioNode` implementation:
- `MultiWaveOsc` — band-limited oscillator (already exists, move here)
- `LiveAdsr` — fully parametric ADSR (already exists, move here)
- `FxChain` — series effects chain (already exists, move here)
- `ShimmerReverb` — new: pitch-shifted feedback reverb
- `Crystallizer` — new: granular pitch-shift delay

### `synth-engine` (library crate)
Assembles DSP nodes into a complete instrument. Manages the layer system, arpeggiator, macro
engine, and scene state. Exposes a clean API: `Engine::new()`, `Engine::process_block()`,
`Engine::set_macro()`, `Engine::load_scene()`. Depends on `synth-dsp`, `serde`, `midir`.

### `synth-app` (binary crate)
The egui application. Creates an `Engine`, opens a cpal stream, drives the MIDI engine, and
renders the UI. Contains no DSP logic — it only translates user actions into `Engine` API calls.
Depends on `synth-engine`, `cpal`, `egui/eframe`, `rfd`.

---

## 4. Layer System

### Concept

A **Layer** is one complete instrument voice running simultaneously with up to three other
layers. Layers are independent: each has its own patch, arpeggiator, MIDI channel, and local
effects. They are mixed together before the global send buses.

Up to **four layers** are supported, matching the four textural roles of the reference setup.
Layers can be muted, soloed, and have independent volume and pan.

### Layer internal signal flow

```mermaid
flowchart TD
    MIDI["MIDI input\n(channel filter)"]
    ARP["Arpeggiator\nup · down · updown\nrandom · as-played"]
    VOICE["Voice Bank\n6-voice polyphony\n(existing architecture)"]
    LOCAL["Local FX Chain\nOverdrive · Distortion\nChorus · Delay"]
    SEND_S["Shimmer send\n0.0 – 1.0"]
    SEND_C["Crystal send\n0.0 – 1.0"]
    DRY["Dry out\nto Layer Mixer"]

    MIDI --> ARP --> VOICE --> LOCAL
    LOCAL --> SEND_S
    LOCAL --> SEND_C
    LOCAL --> DRY
```

### Layer Mixer and Global Buses

```mermaid
flowchart LR
    L1["Layer 1\nFoundation Bass"]
    L2["Layer 2\nPulse Arp"]
    L3["Layer 3\nHarmonic Color"]
    L4["Layer 4\nGlue Pad"]

    MIX(("Layer\nMixer\n÷4"))

    SHB["Shimmer Bus\nShimmerReverb node"]
    CRB["Crystal Bus\nCrystallizer node"]

    MBUS["Master Bus\nLimiter · Saturation\nMaster Volume"]
    OUT(["Stereo Out"])

    L1 -->|dry| MIX
    L2 -->|dry| MIX
    L3 -->|dry| MIX
    L4 -->|dry| MIX

    L1 -->|shimmer send| SHB
    L2 -->|shimmer send| SHB
    L3 -->|shimmer send| SHB
    L4 -->|shimmer send| SHB

    L1 -->|crystal send| CRB
    L2 -->|crystal send| CRB
    L3 -->|crystal send| CRB
    L4 -->|crystal send| CRB

    MIX --> MBUS
    SHB --> MBUS
    CRB --> MBUS
    MBUS --> OUT
```

### Layer data model

```
Layer {
    id:           u8,                 // 0–3
    name:         String,
    enabled:      Shared,             // 0.0 = muted
    volume:       Shared,             // layer fader
    pan:          Shared,             // -1.0 left … +1.0 right
    midi_channel: Arc<AtomicU8>,      // 0 = omni
    patch:        Patch,              // existing Patch struct
    arpeggiator:  ArpState,           // new (see §6)
    shimmer_send: Shared,             // pre-fader send to Shimmer Bus
    crystal_send: Shared,             // pre-fader send to Crystal Bus
    local_fx:     LocalFxParams,      // chorus, delay, overdrive, distortion
}
```

---

## 5. New Effects: Shimmer Reverb

### What it is

Shimmer reverb is an algorithmic reverb with a **pitch shifter inserted in the feedback loop**.
On each reverb recirculation, the signal is transposed up (typically +1 or +2 octaves) before
being fed back into the reverb diffusion network. The result: sustained sounds grow upward into
a halo of pitch-shifted harmonics above themselves. A single bass note becomes a chord. This is
the defining sound of ambient music (Eno, Stars of the Lid, Jonsi).

### Signal flow

```mermaid
flowchart LR
    IN(["Input (wet send)"])
    PRE["Pre-delay\n0 – 80 ms"]
    DIFF["Diffusion Network\n4× comb + 2× allpass\n(Schroeder topology)"]
    PITCH["Pitch Shifter\n+12 or +24 semitones\n(granular, overlapping windows)"]
    FB(("×\nfeedback\n0.0 – 0.95"))
    WET["Wet mix"]
    DRY["Dry pass-through"]
    OUT(["Output"])

    IN --> PRE --> DIFF
    DIFF --> PITCH --> FB --> DIFF
    DIFF --> WET
    IN --> DRY
    WET --> OUT
    DRY --> OUT
```

### Parameters

| Parameter | Range | Role |
|---|---|---|
| `size` | 0.0 – 1.0 | Reverb tail length (scales comb delay times) |
| `damp` | 0.0 – 1.0 | High-frequency absorption in the diffusion network |
| `shimmer` | 0.0 – 1.0 | Amount of pitch-shifted signal in the feedback path |
| `pitch` | 0, +12, +24 st | Pitch shift interval (unison, octave, two octaves) |
| `feedback` | 0.0 – 0.95 | Overall recirculation amount (controls tail decay) |
| `mix` | 0.0 – 1.0 | Wet/dry blend (applied at the bus, not per-note) |

### Pitch shifter approach

The pitch shifter in the feedback path uses an **overlapping-window granular approach** (also
known as a pitch-shift via two read heads on a circular buffer). This avoids FFT complexity
while producing acceptable quality at the moderate rates used in a reverb tail:

```
Circular buffer (e.g. 4096 samples)
  Write head: advances at sample rate
  Read head A: advances at (1.0 / pitch_ratio) × sample rate
  Read head B: offset by buffer_size/2, same rate
  Crossfade A ↔ B with a Hann window when either head wraps
  Output = crossfaded mix of head A and head B
```

At +1 octave, `pitch_ratio = 2.0`, so the read heads advance at half speed — playback speed
halves, pitch doubles. The crossfade eliminates the click at the wrap point.

This implementation belongs in `synth-dsp::shimmer`.

---

## 6. New Effects: Crystallizer

### What it is

The Crystallizer (inspired by SoundToys Crystallizer) is a **granular pitch-shift delay**. It
slices incoming audio into short grains, pitch-shifts each grain by a fixed interval, and
re-emits the grains with a configurable delay and feedback. The result is a cascading, glassy
texture where each sound spawns a pitch-shifted echo of itself.

### Signal flow

```mermaid
flowchart LR
    IN(["Input (wet send)"])
    WBUF["Write\nCircular Buffer\n(~2 s)"]
    GRAIN["Grain Engine\n4–8 simultaneous grains\neach: read + window + pitch shift"]
    SCAT["Scatter\nper-grain time offset\n0 – 200 ms random"]
    FB(("×\nfeedback\n0.0 – 0.85"))
    MIX["Wet mix"]
    OUT(["Output"])

    IN --> WBUF
    WBUF --> GRAIN --> SCAT --> MIX
    SCAT --> FB --> WBUF
    IN --> MIX
    MIX --> OUT
```

### Parameters

| Parameter | Range | Role |
|---|---|---|
| `pitch` | −24 … +24 st | Semitone shift applied to each grain |
| `grain_size` | 20 – 200 ms | Duration of each grain (affects smoothness vs. texture) |
| `scatter` | 0.0 – 1.0 | Time randomisation of grain playback positions |
| `feedback` | 0.0 – 0.85 | Crystallizer tail decay |
| `mix` | 0.0 – 1.0 | Wet/dry blend |

### Key difference from shimmer

Shimmer expands sounds **spatially** (into a reverberant halo of harmonics).
Crystallizer expands sounds **temporally** (into a cascade of pitch-shifted echoes spread
over time). Applied together, they place a sound in a large, shimmering space while also
scattering its echo-image forward in time. The combination is more than the sum of its parts.

---

## 7. Arpeggiator

### Current state

The existing **step sequencer** requires manually programming each note. This is powerful but
requires effort. A **chord-responsive arpeggiator** is a different tool: hold a chord and the
arpeggiator automatically plays its notes in a configurable pattern. This is the source of the
"noodle lead" character in the reference setup.

### Design

```mermaid
flowchart LR
    HELD["Held Note Set\n(from MIDI or keyboard)"]
    SORT["Note Sorter\n(pitch order)"]
    PAT["Pattern Engine\nup · down · updown\nrandom · as-played\nup+1oct · down+1oct"]
    CLK["Clock\nBPM-sync or free\n1/4 · 1/8 · 1/16 · 1/32\ntriplet variants"]
    GATE["Gate Length\n10% – 100% of step"]
    NOTE(["Note On/Off\n→ Voice Bank"])

    HELD --> SORT --> PAT
    CLK --> PAT
    PAT --> GATE --> NOTE
```

### ArpState data model

```
ArpState {
    enabled:      bool,
    mode:         ArpMode,      // Up, Down, UpDown, Random, AsPlayed
    division:     ClockDiv,     // Quarter, Eighth, Sixteenth, Thirtysecond, + Triplet variants
    octave_range: u8,           // 1–4: how many octave repetitions per cycle
    gate:         f32,          // 0.1–1.0: fraction of step duration note is held
    hold:         bool,         // latch: keep last chord when keys released
    sync_to_bpm:  bool,
    bpm:          f32,          // used if not synced to global BPM
}
```

### Relationship to existing sequencer

The step sequencer and arpeggiator are **complementary**, not competing:
- Step sequencer: precise, programmed, repeating patterns (bassline, melody)
- Arpeggiator: reactive to held chords, expressive, improvisational

Both remain available per layer. A layer can use either, both, or neither.

---

## 8. Macro System

### Concept

A **Macro** is a single named control (0.0 – 1.0) that simultaneously drives multiple
synthesizer parameters, each scaled and offset independently. The musician sees "Atmosphere"
or "Motion" — not "shimmer feedback" and "layer 3 filter cutoff" and "layer 2 arp rate".

This is what makes the "few knobs" UX possible for complex ambient textures.

### Macro definition

```
Macro {
    name:    String,               // e.g. "Atmosphere"
    targets: Vec<MacroTarget>,     // list of parameter bindings
}

MacroTarget {
    param:  ParamAddress,          // layer · parameter path
    min:    f32,                   // output value when macro = 0.0
    max:    f32,                   // output value when macro = 1.0
    curve:  MacroCurve,            // Linear, Exponential, SCurve
}
```

### Signal flow

```mermaid
flowchart LR
    KNOB["Macro Knob\n0.0 – 1.0"]
    LFO_OPT["Optional LFO\n(slow auto-modulation\nfor evolution)"]
    SUM("+")
    TABLE["Target Table\n[param, min, max, curve] × N"]
    P1["Layer 1\nShimmer send"]
    P2["Layer 2\nFilter cutoff"]
    P3["Layer 3\nArp rate"]
    P4["Layer 4\nVolume"]
    PN["… more targets"]

    KNOB --> SUM
    LFO_OPT --> SUM
    SUM --> TABLE
    TABLE --> P1 & P2 & P3 & P4 & PN
```

### Example: "Atmosphere" macro

| Value | Shimmer mix | Crystal feedback | Layer 4 (pad) volume | Layer 1 filter cutoff |
|---|---|---|---|---|
| 0.0 (dry) | 0.05 | 0.0 | 0.4 | 800 Hz |
| 0.5 | 0.4 | 0.3 | 0.7 | 3 kHz |
| 1.0 (full ambient) | 0.85 | 0.6 | 1.0 | 8 kHz |

A single knob turn morphs from a fairly dry, rhythmic texture to a fully dissolved ambient wash.

### Scene

A **Scene** is a named snapshot of: all four layer patches, arpeggiator states, all macro
definitions and their current values. Scenes replace the current flat patch system for
multi-layer use.

```
Scene {
    name:    String,
    layers:  [Layer; 4],
    macros:  Vec<Macro>,       // 4–8 macros per scene
    bpm:     f32,
    key:     Note,             // root note for arp scale quantization
    scale:   Scale,            // diatonic scale for arp quantization
}
```

---

## 9. Full System Signal Flow

The complete picture, from MIDI input to stereo output:

```mermaid
flowchart TD
    MIDI(["MIDI / Keyboard"])
    MIDI --> L1 & L2 & L3 & L4

    subgraph L1["Layer 1  ·  Foundation"]
        A1["Arpeggiator"] --> V1["Voice Bank\n6-voice poly"] --> FX1["Local FX"]
    end

    subgraph L2["Layer 2  ·  Pulse"]
        A2["Arpeggiator"] --> V2["Voice Bank\n6-voice poly"] --> FX2["Local FX"]
    end

    subgraph L3["Layer 3  ·  Color"]
        A3["Arpeggiator"] --> V3["Voice Bank\n6-voice poly"] --> FX3["Local FX"]
    end

    subgraph L4["Layer 4  ·  Pad"]
        A4["Arpeggiator"] --> V4["Voice Bank\n6-voice poly"] --> FX4["Local FX"]
    end

    FX1 & FX2 & FX3 & FX4 -->|"dry"| LMIX

    FX1 & FX2 & FX3 & FX4 -->|"shimmer send"| SHIM["Shimmer Bus\nShimmerReverb"]
    FX1 & FX2 & FX3 & FX4 -->|"crystal send"| CRYS["Crystal Bus\nCrystallizer"]

    LMIX(("Layer Mixer\n÷4")) --> MBUS
    SHIM --> MBUS
    CRYS --> MBUS

    MBUS["Master Bus\nLimiter · Saturation\nMaster Volume"] --> OUT(["Stereo Output"])

    MACRO["Macro Engine\n4–8 macro knobs\nper scene"] -.->|"param writes"| L1 & L2 & L3 & L4
    MACRO -.->|"bus param writes"| SHIM & CRYS
```

---

## 10. UI/UX Implications

The "few knobs" UX maps naturally to this architecture:

| UI element | Maps to |
|---|---|
| **Scene selector** | Load a `Scene` (all 4 layers + macros) |
| **Layer tabs** | Per-layer patch editor (existing synth UI, one tab per layer) |
| **Macro panel** | 4–8 large knobs, one per macro, labelled by scene |
| **Shimmer send** | Per-layer knob in the layer strip |
| **Crystal send** | Per-layer knob in the layer strip |
| **Arp controls** | Per-layer: mode, division, octave range, gate, hold |
| **Global BPM** | Drives all arp clocks and delay BPM-sync |

The macro panel is the primary performance surface. The layer/patch editor is for sound design
and remains accessible but not required during performance.

---

## 11. Migration Path

The existing codebase is a sound foundation. The migration should be incremental:

```mermaid
gantt
    title Migration Phases
    dateFormat  YYYY-MM-DD
    axisFormat  Phase %s

    section Phase 1 · Workspace split
    Create Cargo workspace                   :p1a, 2025-01-01, 7d
    Move DSP nodes to synth-dsp              :p1b, after p1a, 7d
    Move engine logic to synth-engine        :p1c, after p1b, 7d
    Verify app builds and audio works        :p1d, after p1c, 3d

    section Phase 2 · Shimmer + Crystallizer
    ShimmerReverb AudioNode                  :p2a, after p1d, 10d
    Crystallizer AudioNode                   :p2b, after p1d, 10d
    Add shimmer + crystal buses to graph     :p2c, after p2a, 5d

    section Phase 3 · Layer system
    Layer struct + LayerMixer                :p3a, after p2c, 10d
    Per-layer MIDI channel routing           :p3b, after p3a, 5d
    Per-layer send knobs in UI               :p3c, after p3b, 5d

    section Phase 4 · Arpeggiator
    ArpState + pattern engine                :p4a, after p3c, 7d
    BPM-sync clock                           :p4b, after p4a, 3d
    Arp UI panel                             :p4c, after p4b, 5d

    section Phase 5 · Macro + Scene
    MacroTarget + evaluation                 :p5a, after p4c, 7d
    Scene serialization                      :p5b, after p5a, 5d
    Macro panel UI                           :p5c, after p5b, 7d
```

### Phase 1 is non-breaking
Moving to a workspace and reorganizing modules does not change any DSP behaviour. It is purely
structural. The audio output should be bit-identical before and after Phase 1.

### Phases 2–5 are additive
Each phase adds new capability without removing existing functionality. The existing single-layer
mono-timbral mode remains operational throughout — it becomes "one layer with shimmer/crystal
sends" after Phase 3.

---

## 12. Open Questions

These decisions should be made before implementation begins:

1. **Pitch shifter quality vs. complexity trade-off.** The granular overlap-add approach is
   simpler but has artefacts on transients. A phase-vocoder (FFT-based) approach is cleaner
   but adds latency and complexity. For a reverb feedback path, the granular approach is
   likely sufficient — the artefacts are buried in the reverb tail.

2. **Four fixed layers or dynamic N layers?** Four fixed layers matches the reference setup and
   simplifies the static graph approach. Dynamic N layers would require graph rebuilds or a
   larger pre-allocated graph. Recommend starting with four.

3. **Macro LFO depth.** Should macros support audio-rate LFO modulation (for tremolo-like
   effects at the scene level), or only UI-rate slow modulation? Audio-rate macro modulation
   requires the callback-side computation pattern from `dsp-guidelines.md §BlockRateAdapter`.

4. **Scene format compatibility with existing patches.** The existing 94 embedded patches are
   single-layer. A Scene wraps four of them. A migration utility that wraps a single patch
   into a Scene (layer 1 = the patch, layers 2–4 silent) would preserve backwards
   compatibility.

---

## 13. Music Generation Library — Revised Architecture

This section supersedes and extends the crate structure proposed in §3. It reflects two
additional use cases that have become priorities: **embedding the engine in a Bevy game** and
**music generation** (generative ambient / adaptive game audio), alongside the original standalone
Mac app goal.

### 13.1 The Three Use Cases

| Use case | Host | Audio thread owner | Control source |
|---|---|---|---|
| **Standalone Mac app** | `synth-app` | cpal | MIDI + keyboard + mouse + UI |
| **Bevy game / interactive** | `synth-bevy` plugin | Bevy audio system | Bevy ECS events + game state |
| **DAW plugin** (future) | VST3/CLAP host | DAW | MIDI + automation |

All three use cases share the same `synth-engine` library. The host is a thin shell — it owns
the audio thread and calls `Engine::process_block()`. It does not contain DSP logic.

### 13.2 Revised Crate Architecture

The original three-crate split (§3) is extended to five crates plus one optional Bevy crate:

```mermaid
graph TD
    subgraph Workspace["Cargo Workspace — the_synth"]
        DSP["synth-dsp  ·  lib\n──────────────────\nAudioNode primitives\nMultiWaveOsc · LiveAdsr\nMoog filter · FxNodes\nShimmer · Crystallizer\nNo I/O · No std::time"]

        VOICE["synth-voice  ·  lib\n──────────────────\nInstrument trait\nSubtractive voice\nSample player voice\nPolyphony + voice stealing\nPatch (parameter snapshot)"]

        ENGINE["synth-engine  ·  lib\n──────────────────\nMulti-track engine\nTrack = Instrument + Arp + sends\nGlobal effect buses\nMacro system · Scene\nGenerative pattern engine\nAutomation\nprocess_block() API"]

        CONTROL["synth-control  ·  lib\n──────────────────\nControlEvent type\nControlSource trait\nMIDI adapter (midir)\nKeyboard adapter\nCC → param mapping table"]

        APP["synth-app  ·  bin\n──────────────────\ncpal stream\negui UI panels\nMIDI device management\nFile dialogs"]

        BEVY["synth-bevy  ·  lib\n──────────────────\nBevy Plugin\nAudioSource registration\nECS ↔ ControlEvent bridge\nBevy Resource wrappers\nDev inspector panel (bevy-egui)"]
    end

    DSP --> VOICE --> ENGINE
    CONTROL --> ENGINE
    ENGINE --> APP
    ENGINE --> BEVY

    style DSP    fill:#1a2a3a,stroke:#4a7fa5
    style VOICE  fill:#1a2a3a,stroke:#4a7fa5
    style ENGINE fill:#1a3a2a,stroke:#4aa57f
    style CONTROL fill:#2a1a3a,stroke:#7f4aa5
    style APP    fill:#3a2a1a,stroke:#a57f4a
    style BEVY   fill:#3a1a2a,stroke:#a54a7f
```

**Dependency rules:**
- `synth-dsp` and `synth-voice` depend only on `fundsp` and `std` — never on any I/O crate
- `synth-engine` depends on `synth-dsp`, `synth-voice`, and `synth-control` — never on cpal, egui, or bevy
- `synth-control` depends on nothing project-internal; it defines the shared event vocabulary
- `synth-app` and `synth-bevy` are the only crates that may depend on I/O (cpal, egui, bevy)

### 13.3 The Four Architectural Layers

The full system is organized in four horizontal layers. Each layer has a single clear
responsibility and a defined interface to the layers above and below it.

```mermaid
flowchart TD
    subgraph CTRL["CONTROL LAYER  — synth-control"]
        MIDI_SRC["MIDI device\n(midir)"]
        KEY_SRC["Keyboard / Mouse"]
        GAME_SRC["Bevy ECS\ngame systems"]
        GEN_SRC["Generative\nalgorithms"]
        MIDI_SRC & KEY_SRC & GAME_SRC & GEN_SRC -->|ControlEvent| BUS["ControlEvent bus\n(lock-free SPSC/MPSC)"]
    end

    subgraph MUS["MUSIC LAYER  — synth-engine"]
        TRACKS["Track 1..N\n(Instrument + Arp + sends)"]
        ARP["Arpeggiator\n(chord-responsive)"]
        GEN_PAT["Generative pattern\n(scale walks · euclidean · probability)"]
        AUTO["Automation\n(param curves over time)"]
        MACRO_ENG["Macro engine\n(scene-level knobs)"]
        BUSES["Global buses\n(Shimmer · Crystal · Master)"]
        PB["process_block()\n← called by host"]
        BUS -->|consume| TRACKS
        TRACKS --> ARP & GEN_PAT & AUTO & MACRO_ENG
        TRACKS --> BUSES --> PB
    end

    subgraph INST["INSTRUMENT LAYER  — synth-voice"]
        INST_TRAIT["Instrument trait\nnote_on · note_off\nset_param · process_block"]
        POLY["Voice pool\n(polyphony + stealing)"]
        PATCH["Patch\n(parameter snapshot)"]
        INST_TRAIT --> POLY --> PATCH
    end

    subgraph DSP_L["DSP LAYER  — synth-dsp"]
        OSC_NODE["Oscillator nodes\n(MultiWaveOsc)"]
        FILT_NODE["Filter nodes\n(Moog ladder)"]
        ENV_NODE["Envelope nodes\n(LiveAdsr)"]
        FX_NODE["FX nodes\n(Shimmer · Crystallizer\nChorus · Delay · Reverb)"]
    end

    MUS -->|note_on / set_param| INST
    INST -->|AudioNode API| DSP_L
```

**The critical boundary** is between the Music layer and the Control layer. The audio thread
only knows about `ControlEvent` — it never sees MIDI bytes, Bevy ECS, or UI widget values.
Everything above that boundary is application code. Everything below is real-time safe.

### 13.4 The Control Layer in Detail

A `ControlEvent` is the universal language between any input source and the engine.

```
enum ControlEvent {
    NoteOn  { track: u8, pitch: u8, velocity: u8 },
    NoteOff { track: u8, pitch: u8 },
    SetParam { address: ParamAddress, value: f32 },
    SetMacro { index: u8, value: f32 },
    SceneLoad { name: String },
    Tempo    { bpm: f32 },
    ChordHold { track: u8, notes: Vec<u8> },   // for arpeggiator
    SceneTransition { from: u8, to: u8, frames: u32 },
}
```

A `ControlSource` is a trait with one method: `poll() -> Option<ControlEvent>`. Every source
implements this trait. The engine's audio callback drains the queue each buffer:

```mermaid
flowchart LR
    MIDI["MidiSource\nimpl ControlSource"]
    KEY["KeyboardSource\nimpl ControlSource"]
    BEVY_SRC["BevyEventSource\nimpl ControlSource"]
    GEN["GenerativeSource\nimpl ControlSource"]

    QUEUE["lock-free\nevent queue\n(ringbuf / crossbeam)"]

    MIDI & KEY & BEVY_SRC & GEN -->|push| QUEUE
    QUEUE -->|drain each buffer| ENGINE_CB["Engine\naudio callback"]
```

The queue is the **only** thread boundary in the real-time path. It must be wait-free on the
consumer side (audio thread). The producer side (UI thread, Bevy system thread) may use a
try-push that drops on overflow — the audio thread must never block.

### 13.5 Bevy Integration Design

The Bevy integration is a thin shell over `synth-engine`. It has three responsibilities:

**1. Audio source registration.**
`SynthPlugin` registers the engine as a Bevy `AudioSource`. Bevy's audio system calls
`process_block()` from its own audio thread, identical to how cpal does in the standalone app.
The engine does not know the difference.

**2. ECS → ControlEvent bridge.**
Game systems post `SynthEvent` Bevy events (regular Bevy events, not real-time). A dedicated
Bevy system reads `SynthEvent`s each frame and translates them into `ControlEvent`s pushed into
the engine's lock-free queue:

```
// Bevy game system example:
fn tension_system(
    tension: Res<GameTension>,
    mut synth: EventWriter<SynthEvent>,
) {
    synth.send(SynthEvent::SetMacro { index: 0, value: tension.0 });
}
```

The musician pre-designs what Macro 0 does (shimmer level, arp rate, pad volume) — the game
just moves the dial. This is the adaptive audio model: game changes the macro, musician defines
the consequence.

**3. Dev inspector panel.**
In development mode, `SynthPlugin` adds a `bevy-egui` panel that exposes the same parameter
controls as the standalone app. This allows a musician/composer to tweak the engine live inside
the game window. The panel is stripped from release builds by a Cargo feature flag.

```mermaid
flowchart LR
    subgraph BEVY["Bevy World"]
        GS["Game Systems\n(tension, zone, pace)"]
        SE["SynthEvent\nBevy events"]
        BRIDGE["BevyBridge\nsystem"]
        INSP["Dev Inspector\n(bevy-egui)"]
    end

    subgraph ENGINE["synth-engine"]
        QUEUE2["lock-free\nControlEvent queue"]
        PB2["process_block()"]
        PARAMS["Shared params\n(Arc atomics)"]
    end

    AUDIO["Bevy audio thread"]

    GS --> SE --> BRIDGE -->|push| QUEUE2
    INSP -->|write| PARAMS
    AUDIO -->|call| PB2
    PB2 -->|drain| QUEUE2
```

### 13.6 Generative Music Engine

For ambient and game use, the engine needs to generate music autonomously, without a human
pressing keys. The generative engine sits inside `synth-engine` as a `ControlSource` that
produces note events from rules rather than from human input.

Three generative primitives cover the ambient / synthwave / game audio range:

**Scale walker** — plays a random walk within a chosen scale and key. Step size, direction
probability, and note range are parameters. At low tempo and wide steps this produces the
"noodle" behavior of a Volca Bass lead. This is the simplest generative layer.

**Euclidean rhythm** — distributes N hits across M steps using Bjorklund's algorithm. Standard
in electronic music for organic-feeling rhythmic patterns that are not 4-on-the-floor.
Configurable per-track; drives the arpeggiator's clock gate.

**Probability table** — a list of (note, probability) pairs evaluated each step. Enables
tension-responsive melody: when `tension` is high, high-probability notes shift toward
dissonant intervals; when low, they return to tonic resolution. The game writes `tension`,
the musician defines the table per scene.

All three primitives are driven by the same global BPM clock that drives the arpeggiators.
They produce `ControlEvent::NoteOn / NoteOff` events into the main queue — indistinguishable
from human MIDI input.

### 13.7 Automation Engine

Automation allows parameters to change over time along pre-defined curves without real-time
human input. It is essential for the slow, continuous evolution of ambient textures.

```
AutomationClip {
    param:    ParamAddress,
    points:   Vec<(beat: f32, value: f32)>,   // sorted by beat
    curve:    InterpolationCurve,              // Linear, Cosine, Hold
    looping:  bool,
}
```

The automation engine runs in the audio callback alongside the arpeggiator. It reads the
current playhead position (in beats, derived from the sample counter and BPM), interpolates
between the nearest two points, and writes the result to the target `Shared`. It is fully
real-time safe: no allocation, no branching on the hot path.

Automation is the "set and forget" modulation layer — it handles the long-period evolution
(e.g., filter cutoff opening over 32 bars) that would be tedious to perform manually and too
slow for an LFO.

### 13.8 Process Block API Contract

The engine's single public audio API must satisfy these constraints across all three host types:

```rust
impl Engine {
    /// Called by the host audio thread — must be real-time safe.
    /// Advances the internal timeline by `frames` samples.
    /// `output` is interleaved stereo: [L0, R0, L1, R1, ...]
    pub fn process_block(&mut self, output: &mut [f32], frames: usize);

    /// Called from any thread. Non-blocking, fails silently on queue full.
    pub fn push_event(&self, event: ControlEvent);

    /// Called from any thread. Writes directly to a lock-free Shared.
    pub fn set_param(&self, address: ParamAddress, value: f32);
}
```

**The host owns the audio clock.** The engine has no internal threads, no `std::thread::sleep`,
no `Instant::now` on the hot path. All timing is derived from the cumulative sample count
passed through `frames`. This is the invariant that makes the engine portable across cpal,
Bevy, and any future DAW host.

---

## 14. Engineering Plan

This section defines the concrete implementation phases. Each phase builds on the previous,
is independently releasable, and does not break existing functionality.

### Current state assessment

| Area | Current status | Gap |
|---|---|---|
| Oscillator bank (3 OSC + unison + FM + ring) | Complete | — |
| Amp ADSR | Complete | — |
| Filter ADSR | State + UI only | Not wired to DSP graph |
| Moog ladder filter | Planned | Not implemented |
| LFO DSP wiring | State + UI only | Not wired to DSP graph |
| FX chain (overdrive, distortion, chorus, delay, reverb) | Complete | — |
| Step sequencer | Complete | — |
| MIDI input | Basic | Channel filtering, multi-track routing missing |
| Crate structure | Single crate | Workspace split needed |
| Multi-track / layer system | None | Full implementation needed |
| Arpeggiator | None | Full implementation needed |
| Shimmer reverb | None | Full implementation needed |
| Crystallizer | None | Full implementation needed |
| Control layer (ControlEvent bus) | None | Full implementation needed |
| Macro system | None | Full implementation needed |
| Scene system | Patches only | Multi-layer scenes needed |
| Generative patterns | None | Full implementation needed |
| Automation engine | None | Full implementation needed |
| Bevy integration | None | Full implementation needed |

### Phase 0 — Complete the single-voice engine (pre-refactor)

**Goal:** Finish the work already started before restructuring anything. These are low-risk
changes within the current single-crate structure. The audio output of the app improves
immediately.

| Task | Description | Complexity |
|---|---|---|
| 0.1 Moog ladder filter | Wire `moog_ladder` fundsp node in place of placeholder | Low |
| 0.2 Filter ADSR DSP | Connect `fenv_*` Shared values to actual filter cutoff modulation | Low |
| 0.3 LFO DSP wiring | Connect `lfo_*` Shared values to pitch / filter / amp | Low |
| 0.4 Glide live param | Replace hardcoded `follow(0.002)` with `glide_time` Shared | Low |
| 0.5 Noise node in graph | Connect noise generator to mixer | Low |

All tasks in Phase 0 are independent. They can be done in any order.

### Phase 1 — Cargo workspace split

**Goal:** Reorganize the codebase into the five-crate workspace without changing any behavior.
Audio output must be bit-identical before and after this phase.

| Task | Description |
|---|---|
| 1.1 Create workspace `Cargo.toml` | Add `[workspace]` manifest at repo root |
| 1.2 Create `synth-dsp` | Move `osc.rs`, `envelope.rs`, and the FX nodes into a new library crate |
| 1.3 Create `synth-voice` | Move the voice pool logic (6-voice poly, `AudioState` core) into a new library crate |
| 1.4 Create `synth-control` | Define `ControlEvent` enum; move `midi.rs` here; define `ControlSource` trait |
| 1.5 Create `synth-engine` | Move `audio.rs` core into a new library crate; expose `process_block` and `push_event` |
| 1.6 `synth-app` becomes a bin crate | Thin shell: opens cpal stream, renders egui, calls engine API |
| 1.7 Verify | Run existing tests; confirm audio output unchanged |

This phase is purely structural. It unlocks all subsequent phases by creating clear dependency
boundaries.

### Phase 2 — Control layer and MIDI routing

**Goal:** All input sources speak `ControlEvent`. The engine has no direct dependency on
`midir` or keyboard state.

| Task | Description |
|---|---|
| 2.1 `ControlEvent` bus | Implement lock-free MPSC queue (ringbuf); integrate into engine callback |
| 2.2 `MidiSource` | Wrap existing `midi.rs` into `ControlSource` impl; map MIDI to `ControlEvent` |
| 2.3 `KeyboardSource` | Wrap existing keyboard handling into `ControlSource` |
| 2.4 CC → param mapping | `HashMap<u8, ParamAddress>` table; CC events write to `set_param` |
| 2.5 Per-track MIDI channel | Route incoming MIDI channel to the corresponding track |

After Phase 2, any input source that implements `ControlSource` can drive the engine. This is
the prerequisite for the Bevy bridge.

### Phase 3 — Multi-track engine and layer system

**Goal:** The engine manages four independent tracks. Each track has its own voice bank,
arpeggiator stub, and effect sends. The existing single-voice behavior becomes "Track 1, all
other tracks silent."

| Task | Description |
|---|---|
| 3.1 `Track` struct | Bundles voice bank, patch, arp state, shimmer send, crystal send |
| 3.2 `TrackMixer` | Sums four tracks with per-track volume; normalizes before buses |
| 3.3 Layer UI | Tab bar in `synth-app`: one tab per track, existing UI panel per tab |
| 3.4 Global shimmer bus | Shimmer `Shared` mix bus; all tracks feed it; output added to master |
| 3.5 Global crystal bus | Crystal `Shared` mix bus (dry delay for now, crystallizer in Phase 5) |
| 3.6 Per-track send knobs | UI: shimmer send and crystal send sliders per track |

After Phase 3 the musician can run four independent synth voices simultaneously, each with
different patches and independent routing to the two effect buses.

### Phase 4 — Arpeggiator

**Goal:** Each track has a chord-responsive arpeggiator. The step sequencer remains available
as an alternative trigger source.

| Task | Description |
|---|---|
| 4.1 `ArpState` | Internal state: held notes, pattern mode, current step, BPM clock phase |
| 4.2 Pattern modes | Up · Down · UpDown · Random · AsPlayed |
| 4.3 BPM-sync clock | Global BPM → per-arp division clock derived from sample counter |
| 4.4 `ChordHold` | `ControlEvent::ChordHold` latches a set of notes for the arp to iterate |
| 4.5 Octave range | Arp transposes the pattern up N octaves and back |
| 4.6 Gate length | Configurable note length as a fraction of the step |
| 4.7 Arp UI panel | Per-track controls: mode, division, octave range, gate, hold toggle |
| 4.8 Generative: scale walker | Basic random walk within scale; produces `NoteOn/Off` events |

The scale walker (4.8) is the simplest generative primitive — it can be added in this phase
as it shares the BPM clock infrastructure with the arpeggiator.

### Phase 5 — Shimmer reverb and Crystallizer

**Goal:** Both signature spatial effects are implemented as proper DSP nodes and wired into
the global buses.

| Task | Description |
|---|---|
| 5.1 `ShimmerReverb` AudioNode | Schroeder reverb + granular pitch-shifter in feedback loop |
| 5.2 Pitch shifter | Circular buffer with moving read head; overlap-add for smoothness |
| 5.3 Shimmer parameters | Room size, damping, pitch interval, shimmer mix, dry/wet |
| 5.4 `Crystallizer` AudioNode | Granular pitch-shift delay: grain size, scatter, pitch ratio, feedback |
| 5.5 Crystal parameters | Grain size, scatter, pitch ratio, feedback, dry/wet |
| 5.6 Wire into global buses | Replace placeholder dry buses with the new nodes |

Both effects are in `synth-dsp` — no dependency on the engine or app. They can be developed
and tested in isolation with simple unit tests feeding silence or a sine wave.

### Phase 6 — Macro and Scene system

**Goal:** The musician can define 4–8 named macro knobs per scene. A macro writes to multiple
parameters simultaneously. Scenes are serializable.

| Task | Description |
|---|---|
| 6.1 `Macro` and `MacroTarget` structs | As defined in §8 |
| 6.2 Macro evaluation in callback | Read macro Shared value; iterate targets; write param Shared values |
| 6.3 `Scene` struct | Bundles four track patches + macro definitions + BPM + key + scale |
| 6.4 Scene serialization | `serde` + JSON/TOML; backward-compatible single-layer migration |
| 6.5 Macro panel UI | 4–8 large knobs labelled by scene; replaces per-parameter sliders during performance |
| 6.6 Scene browser UI | Load / save / name scenes |

### Phase 7 — Bevy integration

**Goal:** A game can embed `synth-engine` and drive it through the ECS event system.

| Task | Description |
|---|---|
| 7.1 Create `synth-bevy` crate | Bevy plugin skeleton; feature-gated `bevy` dependency |
| 7.2 `SynthPlugin` | Registers engine as Bevy `AudioSource`; initializes event channel |
| 7.3 `SynthEvent` Bevy event | Mirror of `ControlEvent` but as a regular Bevy event (heap-ok) |
| 7.4 `BevyBridge` system | Reads `SynthEvent`s, translates, pushes into engine lock-free queue |
| 7.5 `SynthParam` Resource | Wraps `Arc<AtomicF32>` parameter handles; game systems write directly |
| 7.6 Dev inspector panel | `bevy-egui` panel behind `inspector` Cargo feature; matches standalone UI |
| 7.7 Example game | Minimal Bevy app: moving entity → Macro 0; demonstrates adaptive audio loop |

### Phase 8 — Generative patterns and automation (ambient / game audio)

**Goal:** The engine can generate music autonomously from rules. Essential for ambient
background and adaptive game audio.

| Task | Description |
|---|---|
| 8.1 Euclidean rhythm generator | Bjorklund algorithm; configurable N hits / M steps per track |
| 8.2 Probability table generator | (note, probability) table evaluated each step; tension param |
| 8.3 `AutomationClip` | Breakpoint curve; evaluated in audio callback from beat position |
| 8.4 Automation engine | Manages a list of active clips; interpolates and writes Shared params |
| 8.5 Generative source UI | Controls for walker range, euclidean N/M, probability table editor |
| 8.6 Automation UI | Drag-point curve editor per param (simplified — no full DAW timeline) |

### Phase summary and dependencies

```mermaid
graph LR
    P0["Phase 0\nComplete\nsingle-voice"]
    P1["Phase 1\nWorkspace\nsplit"]
    P2["Phase 2\nControl layer\n+ MIDI routing"]
    P3["Phase 3\nMulti-track\n+ layers"]
    P4["Phase 4\nArpeggiator"]
    P5["Phase 5\nShimmer +\nCrystallizer"]
    P6["Phase 6\nMacro +\nScene"]
    P7["Phase 7\nBevy\nintegration"]
    P8["Phase 8\nGenerative +\nautomation"]

    P0 --> P1 --> P2 --> P3
    P3 --> P4
    P3 --> P5
    P4 --> P6
    P5 --> P6
    P2 --> P7
    P6 --> P8
    P4 --> P8

    style P0 fill:#1a3a1a,stroke:#4aa54a
    style P1 fill:#1a2a3a,stroke:#4a7fa5
    style P2 fill:#1a2a3a,stroke:#4a7fa5
    style P3 fill:#2a1a3a,stroke:#7f4aa5
    style P4 fill:#2a1a3a,stroke:#7f4aa5
    style P5 fill:#2a1a3a,stroke:#7f4aa5
    style P6 fill:#3a2a1a,stroke:#a57f4a
    style P7 fill:#3a1a2a,stroke:#a54a7f
    style P8 fill:#3a2a1a,stroke:#a57f4a
```

Phases 0 and 1 are blocking for everything else. Phases 3, 4, and 5 can be parallelized once
Phase 2 is complete. Phase 7 (Bevy) only requires Phase 2 — it does not depend on layers,
arp, or effects being finished. This means Bevy integration can proceed in parallel with the
ambient music feature work.

### Non-goals for all phases

- **iOS / iPadOS** — AUv3 requires a full Swift project shell; out of scope until the Mac
  standalone and Bevy targets are mature.
- **VST3 / CLAP plugin format** — `nih-plug` integration is possible after Phase 1, but is
  not required for the primary use cases. Tracked separately.
- **DAW-style timeline / MIDI clip editor** — the automation UI in Phase 8 is intentionally
  minimal. A full piano-roll is a separate product, not a feature.

---

## 15. Bevy Integration — Developer Guide

This section describes the integration from the perspective of a game developer embedding
`synth-bevy` into a Bevy project. It covers the full picture: plugin setup, the pieces of
architecture involved, how game systems talk to the audio engine, and the thread model.

### 15.1 What the game developer gets

After adding `synth-bevy` as a dependency, a game developer can:

- Play musical notes from any Bevy system with a single event
- Drive continuous audio parameters (filter cutoff, macro levels, layer volumes) directly from
  game state — no audio code required
- Load and switch musical scenes (a full multi-layer patch with macro definitions) at runtime
- In development mode, open an inspector panel to tweak sounds live inside the game window
- Do all of the above without knowing anything about DSP, fundsp, or audio callbacks

### 15.2 Plugin setup

The integration entry point is `SynthPlugin`. A game adds it to the Bevy `App` once:

```rust
// main.rs (game)
use synth_bevy::SynthPlugin;

App::new()
    .add_plugins(DefaultPlugins)
    .add_plugins(SynthPlugin::default())
    .run();
```

`SynthPlugin` performs the following at startup, in order:

1. Creates a `synth-engine` `Engine` instance (allocates DSP graph, voice pools)
2. Wraps the engine in a Bevy `Resource` so any system can borrow it
3. Registers the engine as a Bevy `AudioSource` — Bevy's audio backend calls `process_block()`
   from its audio thread automatically, forever, for the lifetime of the app
4. Registers `SynthEvent` as a Bevy event type
5. Adds the `BevyBridge` system to the `PostUpdate` schedule — translates `SynthEvent`s into
   lock-free `ControlEvent`s each frame
6. If the `inspector` Cargo feature is enabled, adds the `SynthInspector` plugin (bevy-egui panel)

After plugin setup the audio engine is running. It produces silence until notes or parameter
changes arrive.

### 15.3 Pieces of architecture

```mermaid
flowchart TD
    subgraph GAME["Game Code  (game developer writes this)"]
        GS1["TensionSystem\nreads GameState\nwrites SynthEvent"]
        GS2["ZoneTransitionSystem\nwrites SynthEvent::SceneLoad"]
        GS3["CombatSystem\nwrites SynthEvent::NoteOn"]
    end

    subgraph BEVY_INTERNAL["synth-bevy internals"]
        EVT["SynthEvent\nBevy event queue\n(heap-ok, deferred)"]
        BRIDGE["BevyBridge system\n(PostUpdate schedule)\ntranslates events"]
        RES["SynthEngineRes\nBevy Resource\nwraps Arc Engine"]
        INSP["SynthInspector\n(bevy-egui panel)\nfeature = inspector"]
    end

    subgraph ENGINE["synth-engine  (runs on audio thread)"]
        QUEUE["lock-free\nControlEvent queue\n(ringbuf SPSC)"]
        PARAMS["Shared params\nArc AtomicF32 per param"]
        PB["process_block()\ncalled by Bevy audio thread"]
    end

    subgraph BEVY_AUDIO["Bevy audio system"]
        AT["audio thread\n(cpal under the hood)"]
    end

    GS1 & GS2 & GS3 -->|EventWriter| EVT
    EVT -->|EventReader| BRIDGE
    BRIDGE -->|push_event| QUEUE
    INSP -->|set_param| PARAMS
    RES -.->|Arc clone| BRIDGE & INSP
    QUEUE -->|drain each buffer| PB
    PARAMS -->|read by DSP graph| PB
    AT -->|call| PB
```

There are exactly **two cross-thread boundaries**:

| Boundary | Mechanism | Thread safety |
|---|---|---|
| Game thread → audio thread (discrete events) | `ringbuf` lock-free SPSC queue | Wait-free on consumer (audio thread) |
| Game thread → audio thread (continuous params) | `Arc<AtomicF32>` (`fundsp::Shared`) | Atomic store/load, no lock |

The game thread never blocks on the audio thread. The audio thread never blocks on anything.

### 15.4 SynthEvent — the game developer's API

`SynthEvent` is a regular Bevy event. Game systems write it with `EventWriter<SynthEvent>`;
the `BevyBridge` system reads it with `EventReader<SynthEvent>` and converts it to the
engine's internal `ControlEvent`.

```rust
pub enum SynthEvent {
    /// Trigger a note on a specific track (0-indexed).
    NoteOn  { track: u8, pitch: u8, velocity: u8 },
    NoteOff { track: u8, pitch: u8 },

    /// Latch a chord for the track's arpeggiator.
    /// The arp iterates these notes until a new ChordHold arrives.
    ChordHold { track: u8, notes: Vec<u8> },

    /// Set a named macro knob (0.0–1.0).
    /// The current scene defines what parameters this macro controls.
    SetMacro { index: u8, value: f32 },

    /// Write directly to a specific parameter on a specific track.
    SetParam { track: u8, param: ParamId, value: f32 },

    /// Load a named scene (replaces all four track patches + macro definitions).
    SceneLoad { name: String },

    /// Crossfade from the current scene to a new one over N frames.
    SceneTransition { name: String, frames: u32 },

    /// Change global BPM.
    Tempo { bpm: f32 },
}
```

The game developer never calls audio functions directly. They write `SynthEvent`s; the bridge
handles translation.

### 15.5 Driving the engine from game state — patterns

#### Pattern A: Continuous mapping (game value → macro)

The most common pattern for adaptive audio. A game system reads a game-world value and maps
it to a macro knob each frame. The musician has pre-designed what Macro 0 does — the game
only knows it ranges 0–1.

```rust
fn tension_audio_system(
    tension: Res<GameTension>,           // f32, 0.0 = calm, 1.0 = max danger
    mut events: EventWriter<SynthEvent>,
) {
    events.write(SynthEvent::SetMacro {
        index: 0,          // "Atmosphere" macro — defined in the scene
        value: tension.0,
    });
}
```

This system runs every frame. The macro smoothly drives shimmer level, filter cutoff, pad
volume, or whatever the musician mapped to it — without any additional code.

#### Pattern B: Scene transition on zone change

```rust
fn zone_transition_system(
    zone: Res<CurrentZone>,
    mut last_zone: Local<Option<Zone>>,
    mut events: EventWriter<SynthEvent>,
) {
    if Some(zone.0) != *last_zone {
        *last_zone = Some(zone.0);
        events.write(SynthEvent::SceneTransition {
            name: zone.0.scene_name().to_string(),
            frames: 44100 * 4,   // 4-second crossfade at 44.1 kHz
        });
    }
}
```

Each zone has a named scene. When the player crosses a zone boundary the engine crossfades to
the new scene without a hard cut.

#### Pattern C: Rhythmic triggers from game events

```rust
fn combat_hit_system(
    mut hit_events: EventReader<EnemyHitEvent>,
    mut synth: EventWriter<SynthEvent>,
) {
    for hit in hit_events.read() {
        // Track 1 = percussion layer; pitch encodes hit severity
        let pitch = if hit.damage > 50 { 60 } else { 48 };
        synth.write(SynthEvent::NoteOn  { track: 1, pitch, velocity: hit.damage.min(127) });
        synth.write(SynthEvent::NoteOff { track: 1, pitch });
    }
}
```

#### Pattern D: Letting the arpeggiator handle harmony

For ambient zones where the music should follow a harmonic center without individual note
triggers:

```rust
fn ambient_chord_system(
    harmony: Res<HarmonyState>,          // current root + scale
    mut synth: EventWriter<SynthEvent>,
    mut last_chord: Local<Vec<u8>>,
) {
    let chord = harmony.current_chord_midi_notes();
    if chord != *last_chord {
        *last_chord = chord.clone();
        synth.write(SynthEvent::ChordHold { track: 0, notes: chord });
    }
}
```

The arpeggiator on Track 0 iterates the held chord automatically. The game never sends
individual `NoteOn/Off` for the ambient layer — it just tells the arp what chord to play.

### 15.6 The dev inspector

During development the musician / composer can open the inspector panel inside the game
window to design sounds and map macros without leaving Bevy. Enable it with a Cargo feature:

```toml
# Cargo.toml (game)
[dependencies]
synth-bevy = { path = "../synth-bevy", features = ["inspector"] }
```

The panel is a `bevy-egui` window. It exposes:
- All four track tabs with the full patch editor (same UI as the standalone app)
- Macro editor: drag-connect parameters to macro knobs; set min/max/curve per target
- Scene save/load from disk
- Live oscilloscope and peak meter

In a release build the `inspector` feature is not enabled. The panel code is compiled out
entirely — zero overhead.

### 15.7 Thread model summary

```mermaid
sequenceDiagram
    participant GS as Game System<br/>(main thread)
    participant BB as BevyBridge<br/>(PostUpdate, main thread)
    participant Q  as lock-free queue<br/>(shared)
    participant AT as Audio Thread<br/>(Bevy / cpal)
    participant EN as Engine<br/>(process_block)

    GS->>BB: EventWriter<SynthEvent> (deferred)
    Note over BB: PostUpdate: drain SynthEvents
    BB->>Q: push_event(ControlEvent) [non-blocking]
    Note over AT: hardware buffer callback (~5ms)
    AT->>EN: process_block(output, frames)
    EN->>Q: drain all pending ControlEvents
    EN->>EN: advance arp, automation, DSP graph
    EN->>AT: fill output buffer
```

Key properties:
- `SynthEvent` is a normal Bevy event — it can carry heap data (`Vec`, `String`) safely because
  it lives on the game thread until `BevyBridge` converts it to a `ControlEvent`
- `ControlEvent` pushed into the lock-free queue is stack-sized (no heap). The queue is
  pre-allocated; if it is full, the push is dropped silently — the audio thread never blocks
- `process_block()` drains the queue before advancing DSP — events written in frame N take
  effect at the start of the next audio buffer, typically within 5–10 ms

### 15.8 Minimal working example

A complete Bevy app that plays a generative ambient loop, driven by a single tension value:

```rust
use bevy::prelude::*;
use synth_bevy::{SynthPlugin, SynthEvent};

#[derive(Resource)]
struct GameTension(f32);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(SynthPlugin::default())
        .insert_resource(GameTension(0.0))
        .add_systems(Startup, load_scene)
        .add_systems(Update, (oscillate_tension, map_tension_to_audio))
        .run();
}

fn load_scene(mut events: EventWriter<SynthEvent>) {
    events.write(SynthEvent::SceneLoad { name: "ambient_forest".to_string() });
}

fn oscillate_tension(time: Res<Time>, mut tension: ResMut<GameTension>) {
    // Slowly oscillate tension 0→1→0 over 20 seconds (placeholder for real game logic)
    tension.0 = (time.elapsed_secs() * std::f32::consts::TAU / 20.0).sin() * 0.5 + 0.5;
}

fn map_tension_to_audio(
    tension: Res<GameTension>,
    mut events: EventWriter<SynthEvent>,
) {
    events.write(SynthEvent::SetMacro { index: 0, value: tension.0 });
}
```

The `ambient_forest` scene (designed in the standalone app or inspector, saved to disk)
contains four layers — pad, bass, arp, texture — with Macro 0 wired to shimmer level, filter
cutoff, and texture volume. The game code above is the entirety of the audio integration.
