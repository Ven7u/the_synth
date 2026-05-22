# Multi-Track Live Performance Design
**forma — the hardware rig in software**

Status: Design / pre-implementation
Scope: `forma` (not forma-ambient)

---

## 1. Philosophy

The target experience is **sitting in front of a hardware rig** — not opening a DAW project.

In a real setup (TR-8S + TD-3 + DB-01 + mixer, or Minilogue + Volca Drum + BigSky), every
instrument is always powered on. You play them simultaneously. You twist a filter knob on
one while another loops its arp. The mixer is always in front of you. Nothing is "armed" or
"in focus" — everything is live.

`forma` should feel exactly like that:

- Every track runs independently and simultaneously at all times.
- You focus on a track to **edit** it, not to make it play.
- The keyboard always routes to the focused track, but other tracks keep looping.
- A single master clock binds everything together.

**What this is NOT:**
- Not a DAW (no arrangement timeline, no clips, no automation lanes).
- Not a multi-timbral workstation (no GM patches, no song mode).
- Not a DAW-in-disguise (no "armed track" model).

**What this IS:**
- A virtual version of the hardware rigs described below.
- Playable, improvisational, and composition-friendly.
- Self-contained: everything needed for a full piece is inside one window.

---

## 2. Core Design Decisions (resolved)

These are settled — the rest of the document is built on top of them.

### 2.1 The rig is a shell *around* the existing forma UI
The current forma synth UI is **not modified**. It becomes the per-track editor. A new
"Rig" shell (a thin strip + a mixer panel) sits above and beside it. The user can toggle
the shell on/off — when off, forma is exactly what it is today.

### 2.2 Each track is a fully independent synth
Each of the 4 synth tracks is an independent `SynthEngineHandle` instance. No shared voice
pool, no shared filter, no shared FX. A track *is* a complete forma synth.

### 2.3 FX live inside each track, not on a shared bus
The current `Patch` struct already carries a complete FX chain. That stays. Loading a patch
on a track recalls its sound *and* its FX — exactly like the current forma. No shared FX bus
in the initial design. (A master send bus is a possible Phase 2 *addition*, never a replacement.)

### 2.4 The drum machine is the 5th track
A specialized engine with its own UI (the step grid), but architecturally it occupies the
same "slot" as any synth track: independent volume/pan/mute/solo, follows the master clock,
appears as a strip in the mixer.

### 2.5 Three-mode toggle: STUDIO | DRUM MACHINE | LIVE
The existing top-left toggle is expanded from two states to three:

- **STUDIO** — current forma, unchanged. Single-synth deep editing, no rig strip. Focused
  track is whichever was last selected (default: Track 1).
- **DRUM MACHINE** — full-screen drum programming view. Step grid, all 8 channels, pattern
  management, and per-channel voice editor all have full vertical real estate. The synth
  tracks keep playing; only the *editing surface* changes.
- **LIVE** — rig shell. Track strip + mixer panel + focused synth editor. Performance view.

All three modes share the same running audio engine. Switching modes changes what you're
editing, never what's playing.

### 2.6 Default state preserves zero-regression behavior
On a fresh launch:
- **STUDIO mode** is the default.
- **Track 1** is focused, active, holds the default patch — identical to current forma.
- **Tracks 2, 3, 4** exist in memory but are *muted* (default Init patch, 0.0 volume).
- **Drums** exist but are disabled.

A user who never clicks LIVE never knows multi-track exists. The forma single-synth
experience is bit-for-bit unchanged.

### 2.7 All engine instances are always alive
All 4 synth engines + the drum engine are instantiated at startup and run continuously
in the audio thread. Muted tracks contribute 0.0 to the mix bus — there is no dynamic
creation/teardown of engines when toggling tracks. Idle engines are essentially free
(processing silence with negligible CPU).

### 2.8 Focus is shared state across modes
The "focused synth track" is a single piece of state, not per-mode. Switching between
STUDIO, DRUM MACHINE, and LIVE does not change which synth track you're editing — only
what surface is visible. F1–F4 switch the focused synth track in STUDIO and LIVE modes.
F5 is a shortcut to DRUM MACHINE mode (equivalent to clicking the toggle).

---

## 3. Reference Hardware Setups

These three archetypes define the genre range forma should cover.

### 3.1 Techno (Power & Raw Energy)
```
Roland TR-8S ──────────────────┐
Behringer TD-3 (Acid Bass) ────┤
Erica Synths DB-01 (Lead) ─────┤──→ Mixer ──→ Master
```
Each instrument has its own filter and onboard FX. Master clock locks everything.

### 3.2 Ambient (Atmosphere & Texture)
```
Korg Minilogue XD (Pads/Lead) ─┐
Korg Volca Drum (Noise/Perc) ──┤──→ Output
Strymon BigSky / Microcosm ────┘   (master reverb on the desk)
```
Each synth has its own onboard FX. The master reverb is the *only* shared layer.

### 3.3 Synthwave (80s Nostalgia)
```
Behringer DeepMind 12 (Pads, built-in chorus+reverb) ─┐
Novation Bass Station II (mono, built-in distortion) ─┤──→ Mixer ──→ Master
Elektron Digitakt (Drums + sample FX) ─────────────────┤
```
Every instrument carries its own FX identity. The mixer just balances them.

### 3.4 What these setups share

| Element | Role |
|---------|------|
| 1 drum/percussion engine | Rhythmic foundation |
| 1–2 poly-synths | Pads, chords, harmonic color |
| 1 bass/mono synth | Low-end anchor |
| 1 lead/arp synth | Melody, hooks |
| 1 mixer | Balance + master, *not* an FX hub |
| Shared clock | Everything in sync |
| Per-instrument FX | Each synth has its own character |

**→ forma target: 4 synth tracks + 1 drum machine + 1 mixer. Each synth carries its own FX.**

---

## 4. Signal Architecture

### 4.1 Per-track signal flow (each track is self-contained)

```
                            ┌─────────────────────────────────┐
                            │         SYNTH TRACK             │
                            │                                  │
   MIDI / Keyboard ───────► │  OSC ─→ FILTER ─→ ADSR ─→ AMP   │
   (when focused)           │         │                        │
                            │         LFO 1, LFO 2             │
                            │         ARP, SEQ, WALKER         │
                            │                                  │
                            │         FX chain (full):         │
                            │         OVERDRIVE → CHORUS →     │
                            │         DELAY → REVERB →         │
                            │         SHIMMER → CRYSTAL        │
                            │                                  │
                            │  ────────────────────► out (L/R) │
                            └─────────────────────────────────┘
```

This is **exactly the current forma signal path**. No change. Loading a patch loads the whole
chain. Each of the 4 synth tracks runs an instance of this.

### 4.2 Full rig signal flow

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                              MASTER CLOCK                                     │
│                  BPM ───── Bar boundary ───── Subdivisions                    │
└──────────────────────────────────────────────────────────────────────────────┘
                                       │
        ┌──────────────┬───────────────┼───────────────┬──────────────┐
        │              │               │               │              │
        ▼              ▼               ▼               ▼              ▼
   ┌─────────┐  ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────────┐
   │ TRACK 1 │  │ TRACK 2  │   │ TRACK 3  │   │ TRACK 4  │   │ DRUM MACHINE │
   │  Lead   │  │   Pad    │   │   Bass   │   │   Keys   │   │ 8-ch, 16-stp │
   │         │  │          │   │          │   │          │   │              │
   │ Patch + │  │ Patch +  │   │ Patch +  │   │ Patch +  │   │ Synth voices │
   │  own FX │  │  own FX  │   │  own FX  │   │  own FX  │   │  own FX (TBD)│
   │         │  │          │   │          │   │          │   │              │
   │  L  R   │  │  L  R    │   │  L  R    │   │  L  R    │   │  L  R        │
   └────┬────┘  └────┬─────┘   └────┬─────┘   └────┬─────┘   └──────┬───────┘
        │            │              │               │                │
        │            │              │               │                │
        │       per-track:  Volume · Pan · Mute · Solo                │
        │            │              │               │                │
        └────────────┴──────────────┴───────────────┴────────────────┘
                                    │
                                    ▼  (simple stereo sum)
                          ┌──────────────────────┐
                          │     MASTER STAGE      │
                          │  Limiter + Output     │
                          └──────────────────────┘

  (Phase 2 future option: a master FX send bus alongside, NOT replacing per-track FX.)
```

The mixer is **only** a sum-and-balance stage. No FX routing. Each track's FX is part of
that track's identity.

### 4.3 Audio thread model

```
audio_callback(frames):
    for each synth track T in [1..4]:
        if T.muted or (any.soloed and not T.soloed):  output silence
        else:
            T.engine.tick(frames) → stereo buffer       ← full forma engine, FX included
            apply track gain * track pan → sum into mix_bus

    if !drums.muted:
        drum_engine.tick(frames) → stereo buffer        ← drum synth + FX
        apply gain + pan → sum into mix_bus

    limiter.process(mix_bus)
    output ← mix_bus
```

Cost: ~4x current forma CPU at full polyphony. Modern hardware handles this.

### 4.4 MIDI routing

In a hardware rig, each synth is on a different MIDI channel. forma mirrors this:

| Track | Default MIDI Channel | Keyboard shortcut |
|-------|---------------------|-------------------|
| Synth 1 | Ch 1 | F1 |
| Synth 2 | Ch 2 | F2 |
| Synth 3 | Ch 3 | F3 |
| Synth 4 | Ch 4 | F4 |
| Drums | Ch 10 (GM standard) | F5 |

The on-screen keyboard always plays the focused track. External MIDI respects channel routing
(allowing a multi-track MIDI controller to play all synths simultaneously).

---

## 5. UI Layout

### 5.1 The shell concept

The top-left toggle is the global mode switch. It has three states. All three share the
same running audio engine — switching never stops playback.

```
STUDIO MODE  (current forma, unchanged — small breadcrumb added)
┌──────────────────────────────────────────────────────────────────────┐
│ [STUDIO] [DRUM MACHINE] [LIVE]   BPM 120  SYNC  BAR                  │
│  T1: Lead · "Gilmour Lead"                                            │
├──────────────────────────────────────────────────────────────────────┤
│                                                                       │
│       THE CURRENT FORMA UI, EXACTLY AS IT IS TODAY                    │
│       (single synth, no rig strip, no mixer)                          │
│                                                                       │
└──────────────────────────────────────────────────────────────────────┘

DRUM MACHINE MODE  (full-screen drum programming)
┌──────────────────────────────────────────────────────────────────────┐
│ [STUDIO] [DRUM MACHINE] [LIVE]   BPM 120  SYNC  BAR                  │
├──────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  DRUMS ● ON   Pattern: [Four-on-Floor ▼]  [+NEW] [DUP] [DEL]        │
│  Div: 1/16   Swing: 0%   [▶] [RST]                                   │
│                                                                       │
│       1  2  3  4  5  6  7  8  9  10 11 12 13 14 15 16               │
│ KICK [■][ ][ ][ ][■][ ][ ][ ][■][ ][ ][ ][■][ ][ ][ ]              │
│ SNARE[ ][ ][ ][ ][■][ ][ ][ ][ ][ ][ ][ ][■][ ][ ][ ]              │
│ HAT  [▪][ ][▪][ ][▪][ ][▪][ ][▪][ ][▪][ ][▪][ ][▪][ ]             │
│ CLAP [ ][ ][ ][ ][ ][ ][ ][○][ ][ ][ ][ ][ ][ ][ ][○]              │
│ TOM1 [ ][ ][ ][■][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ]              │
│ TOM2 [ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ]              │
│ PERC [ ][ ][ ][ ][ ][ ][▪][ ][ ][ ][ ][ ][ ][ ][▪][ ]              │
│ NOISE[ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ][ ]              │
│                           ▲ playhead                                  │
│                                                                       │
│ ── VOICE EDITOR (click a channel name above to expand) ───────────── │
│  KICK  Freq: 60Hz  Sweep: 60→20Hz / 80ms  Noise: 10%                 │
│        Attack: 2ms  Decay: 120ms   Filter: LP 200Hz                   │
│                                                                       │
└──────────────────────────────────────────────────────────────────────┘

LIVE MODE  (rig strip + focused synth editor)
┌──────────────────────────────────────────────────────────────────────┐
│ [STUDIO] [DRUM MACHINE] [LIVE]   BPM 120  SYNC  BAR                  │
├──────────────────────────────────────────────────────────────────────┤
│  T1●Lead [M][S]   T2●Pad [M][S]   T3○Bass [M][S]   T4●Keys [M][S]   │
│  DRUMS● [M][S]    ♩ 120 BPM  [▶ ALL] [■ ALL]   MIX▸                 │
├──────────────────────────────────────────────────────────────────────┤
│                                                                       │
│         ╔════════════════════════════════════════════════╗            │
│         ║       THE CURRENT FORMA UI, UNCHANGED           ║          │
│         ║       (shows the focused track's synth)         ║          │
│         ║   OSC · FILTER · ENV · LFO · FX · ARP · SEQ    ║          │
│         ╚════════════════════════════════════════════════╝            │
│                                                                       │
└──────────────────────────────────────────────────────────────────────┘
```

The breadcrumb in STUDIO mode (`T1: Lead · "Gilmour Lead"`) reminds the user which track's
patch they're editing when multiple tracks are loaded. Hidden on fresh launch (only Track 1
active, single-synth experience identical to today).

In all three modes the rig is *running* — switching modes changes the editing surface only.

### 5.2 Rig strip detail

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────┐               │
│ │ T1 Lead  │ │ T2 Pad   │ │ T3 Bass  │ │ T4 Keys  │ │ DRUMS  │  ♩ 120  ▶■ │
│ │ ●  ▐▐▐▐ │ │ ○  ▐▐▐  │ │ ●  ▐▐▐▐ │ │ ●  ▐▐▐  │ │ ●  ▐▐ │  SYNC  MIX▸ │
│ │ [M] [S]  │ │ [M] [S]  │ │ [M] [S]  │ │ [M] [S]  │ │ [M][S] │              │
│ │ Gilmour  │ │ Pad42    │ │ Sub Thb  │ │ Rhodes   │ │ 909    │              │
│ └──────────┘ └──────────┘ └──────────┘ └──────────┘ └────────┘               │
│   focused      idle        focused       idle        idle                     │
└──────────────────────────────────────────────────────────────────────────────┘
```

Per track strip:
- Click anywhere on the strip → focus this track (keyboard + editor switches to it)
- ● / ○ = playing / stopped (independent per track)
- ▐▐▐ = real-time VU meter
- [M] = mute, [S] = solo
- Patch name (current loaded patch)

### 5.3 Mixer side panel (toggle with `MIX▸` button)

```
┌──────────┐
│  MIXER   │ ← slides in from the right when "MIX" is clicked
├──────────┤
│ T1 ████  │
│  0.80 ●  │
│  ─●──    │ ← pan
│ [M] [S]  │
├──────────┤
│ T2 ████  │
│  0.60 ●  │
│  ──●─    │
│ [M] [S]  │
├──────────┤
│ T3 ████  │
│  0.70 ●  │
│  ─●──    │
│ [M] [S]  │
├──────────┤
│ T4 ████  │
│  0.50 ●  │
│  ─●──    │
│ [M] [S]  │
├──────────┤
│ DRM ████ │
│  0.75 ●  │
│  ─●──    │
│ [M] [S]  │
├══════════┤
│ MASTER   │
│ ████████ │
│  LIMIT   │
└──────────┘
```

Pure volume/pan/mute/solo + master. No sends. No FX routing. Hardware-style channel strips.

### 5.4 Drum machine view
This is a dedicated full-screen mode accessed via the **DRUM MACHINE** toggle (see §5.1
mockup). It is not a sub-panel inside LIVE or STUDIO — it has the full window to itself,
giving the step grid and voice editor room to breathe.

The voice editor is inline: clicking a channel name expands its synthesis parameters
(envelope, pitch sweep, noise mix, filter) directly below the grid — no modal, no popover.
Only one channel editor is open at a time; clicking another collapses the previous one.

---

## 6. Drum Machine Design

### 6.1 Synthesis model

Each drum channel is a dedicated synthesized voice — no samples. Parametric, tunable,
patchable (consistent with the synth tracks).

```
Channel architecture (per drum voice):

TRIGGER ──→ ENVELOPE (fast Attack + Decay, no Sustain/Release)
                │
                ▼
         OSCILLATOR ←── PITCH ENVELOPE (sweep: high→low on kick)
                │
                ▼
           NOISE MIX (blend: pure tone ←──→ pure noise)
                │
                ▼
            FILTER (resonant LP/BP — shapes the timbre)
                │
                ▼
             OUTPUT → mixer channel
```

Default voice types (tunable):

| Channel | Base freq | Noise mix | Sweep | Filter | Character |
|---------|-----------|-----------|-------|--------|-----------|
| Kick    | 60 Hz     | 10%       | 60→20Hz, 80ms | LP 200Hz | Deep thud |
| Snare   | 200 Hz    | 70%       | none  | HP 500Hz | Crack + air |
| Hi-Hat  | 8kHz      | 90%       | none  | BP 8kHz | Crisp metallic |
| Clap    | —         | 95%       | none  | HP 2kHz | Short noise burst |
| Tom 1   | 120 Hz    | 20%       | 120→60Hz | LP 400Hz | Mid tom |
| Tom 2   | 80 Hz     | 20%       | 80→40Hz  | LP 300Hz | Floor tom |
| Perc    | 400 Hz    | 40%       | 400→200Hz | BP | Clave / Rim |
| Noise   | —         | 100%      | none  | LP sweep | Cymbal / FX |

### 6.2 Step sequencer (per channel)

- 16 steps (expandable to 32)
- Per-step: on/off, velocity (0–127), probability (0–100%)
- Swing: global or per-channel offset
- Pattern slots: up to 8 named patterns, switchable live on bar boundary
- Pattern chain: A→B→C→... for longer arrangements

### 6.3 Drum machine FX

**Resolved: dedicated drum-bus FX chain (no per-channel FX).** Character per drum voice
comes from synthesis params (envelope, filter, noise mix) — not from FX. The drum machine
has a single shared FX strip applied to the full drum mix:

- Compressor (glue the kit together)
- Drive / saturation (dirt and punch)
- Small room reverb (air, not wash)

This is exactly the TR-8S model: one insert on the master drum bus. It keeps the drum
engine simple and the CPU cost bounded.

---

## 7. Preset Scenes ("Rigs")

A Scene bundles: 4 synth patches + drum patterns + mixer settings + BPM.
Loading a scene = recreating the entire hardware rig in one click.

| Scene | T1 | T2 | T3 | T4 | Drums | Genre |
|-------|----|----|----|----|----|-------|
| Techno Rig | Acid Bass | Industrial Lead | Sub Bass | — | Four-on-floor + offbeat | Techno |
| Ambient Rig | Shimmer Pad | Walker Texture | Drone Bass | Bell Lead | Sparse perc | Ambient |
| Synthwave Rig | Juno Pad | Brass Stab | Rolling Bass | Lead Melody | LinnDrum | Synthwave |
| Minimal Rig | Pad | — | Bass | — | Minimal | Dark Ambient |

Scenes are saved/loaded as JSON in `assets/scenes/`, alongside the existing `assets/patches/`.

---

## 8. Data Model

### 8.1 Track

```
Track {
    name: String,                   // "Lead", "Pad", etc. — user-editable
    engine: SynthEngineHandle,      // full forma synth, independent instance
    midi_channel: u8,               // 1–16 MIDI routing
    volume: f32,                    // 0.0–1.0
    pan: f32,                       // -1.0 to +1.0
    muted: bool,
    solo: bool,
    playing: bool,                  // independent transport per track
    patch_name: String,             // for display
    // NOTE: FX is part of the patch loaded into `engine`, not a separate field
}
```

### 8.2 Drum Machine

```
DrumMachine {
    enabled: bool,
    volume: f32,
    pan: f32,
    channels: [DrumChannel; 8],
    patterns: Vec<DrumPattern>,     // up to 8
    active_pattern: usize,
    swing: f32,                     // 0.0–0.75
    step_count: usize,              // 16 or 32
    current_step: usize,            // read by UI for playhead
    // possibly: drum_fx: DrumFXChain (small dedicated FX strip)
}

DrumChannel {
    name: String,
    voice: DrumVoice,               // synthesis params
    steps: [DrumStep; 32],
    muted: bool,
    volume: f32,
}

DrumStep { active: bool, velocity: f32, probability: f32 }

DrumVoice {
    base_freq: f32,
    pitch_sweep: Option<(f32, f32, f32)>,  // start, end, time_ms
    noise_mix: f32,
    filter_mode: FilterMode,
    filter_cutoff: f32,
    filter_q: f32,
    attack_ms: f32,
    decay_ms: f32,
}
```

### 8.3 Scene

```
Scene {
    name: String,
    bpm: u32,
    tracks: [TrackSnapshot; 4],     // each carries a full Patch (incl. FX)
    drums: DrumMachineSnapshot,
    mixer: MixerSnapshot,           // just volume/pan/mute/solo per track + master
    rig_mode: bool,                 // whether the rig UI was visible
}
```

---

## 9. Implementation Plan

### Phase 0 — Mode toggle wiring (UI shell only, no engine changes)
- Expand the existing **STUDIO / LIVE** toggle to three states: **STUDIO | DRUM MACHINE | LIVE**.
- STUDIO: current forma UI, unchanged. Add a focused-track breadcrumb (`T1: Init`).
- DRUM MACHINE: placeholder view (empty grid, static layout — no engine yet).
- LIVE: placeholder rig strip above the existing forma UI (empty track cells only).
- Goal: prove all three shells render and switch cleanly without breaking the existing UI.
  **No multi-track, no drum engine yet.**

### Phase 1 — Multi-track audio foundation
- Refactor the audio callback to instantiate 4 `SynthEngineHandle`s.
- Sum their stereo outputs with per-track gain + pan into the mix bus.
- The focused track (index 0 by default) is what the existing UI controls.
- Per-track mute (zero gain) and solo (zero all others).
- Goal: 4 tracks playing simultaneously; UI still edits only track 0.

### Phase 2 — Track focus + rig strip
- Make the rig strip cells live: click → switch which track the UI edits.
- Wire keyboard to focused track.
- Per-track play/stop, mute, solo from the strip.
- Per-track VU meters reading real-time levels.

### Phase 3 — Mixer panel
- Side-panel mixer (slides in/out with `MIX▸` button).
- Per-track volume slider, pan, mute, solo.
- Master volume + limiter.

### Phase 4 — Scene save/load
- Serialize all 4 tracks (each carrying its full patch + FX) + mixer + BPM to JSON.
- Scene browser (similar to patch browser).
- Built-in scenes: Techno Rig, Ambient Rig, Synthwave Rig, Minimal Rig.

### Phase 5 — Drum machine
- `DrumEngine` with 8 synthesized channels + 16-step sequencer.
- Drum view replaces synth editor when DRUMS is focused.
- Master clock sync.
- Drum patterns saved in scenes.

### Phase 6 — Polish
- Pattern chains for the drum machine.
- Per-track MIDI channel routing for external controllers.
- Keyboard split / layer mode (per-track MIDI note ranges).

### Phase 7 (optional, much later) — Master FX send bus
- *Optional addition*, not a replacement for per-track FX.
- Master reverb + delay accessible via per-track send knobs in the mixer.
- The "BigSky on the master bus" pattern. Bonus layer, not the core architecture.

---

## 10. CPU Efficiency Guidelines

These are non-negotiable constraints that apply from Phase 1 onward. They are ordered by
impact — the first two cover the overwhelming majority of cases.

### 10.1 Silence-gated DSP (biggest win)
When a track is muted or has no active voices, return a zeroed buffer immediately — do NOT
run the FundDSP audio graph. FundDSP's `AudioUnit::process()` is called explicitly, so it's
trivial to guard:

```rust
if track.muted || track.all_voices_silent() {
    out_buf.fill(0.0);
    continue;
}
track.engine.process(&mut out_buf);
```

A voice is "silent" when its amplitude envelope has decayed below a threshold (~-90 dB).
This handles the 4-idle-engines startup case essentially for free — no CPU cost at rest.

### 10.2 Per-track polyphony caps (bounded worst case)
Each track gets a `max_voices: u8` field. Defaults:

| Track role | Default max voices |
|------------|-------------------|
| Lead / mono | 1 |
| Bass | 1–2 |
| Pad | 2–4 |
| Keys / general | 4 |

Voice stealing (steal oldest or quietest) is local per track, never global. The hard ceiling
means worst-case CPU per track is predictable and can be summed across tracks to get a total
budget, regardless of how the user plays.

**Note on unison:** when 3+ tracks are active, the UI may auto-cap unison voices (e.g., max
4 copies per track in LIVE mode vs. the normal 8). This is a UI-level policy — no engine
change required.

### 10.3 Denormal flushing per audio thread
The existing `denormals.rs` sets FTZ/DAZ on startup. Ensure it runs on **every audio thread**,
not just the main thread. A reverb tail on an idle track can burn 10× CPU from denormals on
x86 without this. Rule: any thread that calls `AudioUnit::process()` must call
`denormals::disable()` before its first buffer.

### 10.4 Do not pre-optimize graph topology
Do NOT introduce a shared reverb bus or shared FX routing to save CPU. Per-track FX is a
core architectural invariant (§2.3). If profiling shows reverb is the bottleneck, the fix is
a lighter algorithm (e.g., Freeverb → cheaper Schroeder variant), not shared state. Shared
state would violate the independence guarantee and complicate patch save/load.

### 10.5 Drum machine voices are cheap — sample playback is not
Each drum channel is a synthesized voice (one trigger → short envelope). This is very cheap.
If sample playback is ever added (Phase 6+), profile it separately: buffer reads +
interpolation × 8 channels adds non-trivial cost on the audio thread.

### 10.6 Profiling strategy
- **Do not profile before Phase 1** — the real patch mix is unknown until multiple tracks
  are running.
- **Profile at end of Phase 1** with a worst-case scenario: max unison on all 4 synth tracks,
  reverb + delay active on each, drum machine at 200 BPM.
- Tool: `cargo flamegraph` (cross-platform) or Instruments on macOS, targeting the audio
  callback thread specifically (not the UI thread).
- If the Phase 1 worst case is acceptable, ship it and move on. Only optimize if measured.

---

## 11. Resolved Decisions

All questions are settled. Listed here for reference.

1. **Where does the mode toggle live, and how many states?**
   **→ Expand the existing top-left STUDIO / LIVE toggle to three states: STUDIO | DRUM MACHINE | LIVE.**
   STUDIO = single-synth deep editing (current forma, unchanged). DRUM MACHINE = full-screen
   drum programming view. LIVE = rig strip + mixer + focused synth editor. All three share
   the same running audio engine. F1–F4 switch focused synth track in STUDIO and LIVE; F5
   is a shortcut to DRUM MACHINE mode.

2. **Drum machine FX: dedicated chain vs. per-channel?**
   **→ Dedicated drum-bus FX chain.** Drum effects (compression, drive, room) work on the
   whole drum mix, not per-channel — consistent with how a TR-8S or a hardware mixer insert
   works. Per-channel FX is not needed because drum voice character comes from synthesis
   parameters (envelope, filter, noise mix), not FX. The drum-bus chain is separate from
   all synth tracks and has its own lightweight strip (compressor + drive + small reverb).

3. **Scene vs. patch — do scenes load patches by reference or by value?**
   **→ By value.** Each scene embeds a full snapshot of all 4 patches + drum patterns +
   mixer state. The scene is self-contained and portable. Editing a source patch file after
   saving a scene does not alter the scene — intentional. Consistent with how the current
   patch system already works.

4. **Keyboard split / layer mode.**
   **→ Deferred to a later phase; one track at a time for now.** The on-screen keyboard
   and external MIDI route to the focused track only. Future feature (Phase 6+): split the
   keyboard into ranges (e.g., left-hand bass / right-hand lead) or support multiple
   physical keyboards each locked to a track. Design is kept open for this — avoid hard-coding
   assumptions that would make it harder to add later.

5. **External MIDI input on all channels simultaneously.**
   **→ Channel-based routing from day one.** Each track has a fixed default MIDI channel
   (T1→Ch1, T2→Ch2, T3→Ch3, T4→Ch4, Drums→Ch10). An external controller sending on Ch2
   plays T2 regardless of which track is focused. The on-screen keyboard always plays the
   focused track (channel-agnostic). These two paths are explicitly separate in the MIDI
   engine and must never be conflated.

---

## 12. Visual Inspiration

```
Hardware rig (physical):              forma (virtual, Rig Mode ON):

[TR-8S]  ─────────────────────────→  [DRUMS] track (step grid)
[TD-3 Acid Bass] ─────────────────→  [T3: Bass] (forma synth with TB-303-style patch)
[DeepMind 12 Pad] ────────────────→  [T2: Pad] (forma synth with Juno-style patch)
[Prophet-6 Lead] ─────────────────→  [T1: Lead] (forma synth with unison patch)
[Keys/Strings]   ─────────────────→  [T4: Keys] (forma synth with Rhodes-style patch)

[Mixer console] ──────────────────→  Mixer panel
 Channel strips                        Volume + Pan + M/S per track
 Master fader                          Master + Limiter
```

The user experience is: **you are the performer at the desk**. The software disappears.
The rig shell is just the studio around the synths you already know.
