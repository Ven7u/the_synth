# Oscillators

The synth has 3 independent oscillators per voice. Each is a direct digital synthesis (DDS) oscillator implemented as a custom fundsp `AudioNode` (`MultiWaveOsc` in `src/osc.rs`).

---

## Controls (per oscillator)

| Control | Range | Default | Notes |
|---|---|---|---|
| On/Off | toggle | OSC 1+2 on, OSC 3 off | Header button, green = on |
| Waveform | Sin / Saw / Sqr / Tri | Sin | See waveforms below |
| Octave | −2 … +2 | 0 | Relative to played note |
| Detune | −100 … +100 ¢ | 0 | Fine pitch offset in cents |
| Volume | 0.0 … 1.0 | OSC1: 0.5, OSC2: 0.5, OSC3: 0.3 | In the mixer panel |
| PW on/off | toggle (Sqr only) | off | Pulse width control |
| Uni on/off | toggle | off | Unison / spread |
| Sync→2 | toggle (OSC 1 only) | off | Hard sync OSC 1 → OSC 2 |

---

## Waveforms

### Sine
A pure single-frequency tone. No harmonics, no aliasing. The smoothest, least bright waveform. Good for sub-bass, soft pads, or FM modulation.

### Saw (Sawtooth)
A linear ramp from −1 to +1, then an instant reset. Rich in harmonics — all partials present (1/n amplitude). The classic "buzz" of brass and strings. Most common waveform for subtractive synthesis. Uses **PolyBLEP** band-limiting to eliminate aliasing at the reset discontinuity.

### Sqr (Square)
Equal time at +1 and −1. Contains only odd harmonics, giving a hollow, woody, clarinet-like sound at 50% duty cycle. Uses **PolyBLEP** band-limiting at both edges.

Supports **Pulse Width** control (see below).

### Tri (Triangle)
A linear ramp up then down. Only odd harmonics, falling off fast (1/n²). Softer than square, rounder than saw. Alias-free by nature — no PolyBLEP needed.

---

## Pulse Width (Square only)

| Control | Range | Default | Notes |
|---|---|---|---|
| PW on/off | toggle | off | Only visible when Sqr selected |
| Width | 0.01 … 0.99 | 0.5 | Duty cycle of the square wave |

At 0.5 (50%) you get a standard square wave. Narrowing the width (e.g. 0.1) produces a thin, nasal, reed-like tone. Disabling resets to 0.5.

**PolyBLEP** is applied at both the rising edge (phase = 0) and the falling edge (phase = width), so the band-limiting adapts correctly at any duty cycle.

---

## Unison / Spread

Runs multiple detuned copies of the oscillator simultaneously, summed and normalised. Creates a thick, chorused, "wide" sound. Up to 5 copies per OSC slot.

| Control | Range | Default | Notes |
|---|---|---|---|
| Uni on/off | toggle | off | Green = active |
| Voices (v) | 2 … 5 | 2 | Number of simultaneous copies |
| Spread (¢) | 0 … 50 ¢ | 20 | Total pitch spread across all copies |

Copies are spread symmetrically. With 3 voices at 20¢: −10¢, 0¢, +10¢. The centre copy (when count is odd) is always at 0 detune so the fundamental pitch stays anchored. All 5 graph nodes are always present in the DSP — inactive copies have volume 0.0, so there is no graph rebuild when toggling.

---

## Hard Sync (OSC 1 → OSC 2)

Every time OSC 1 completes a full cycle, it forces OSC 2 to restart its phase from zero — regardless of where OSC 2 is in its own cycle. The result is a complex, harmonically rich timbre whose character changes dramatically as OSC 2's pitch is swept relative to OSC 1. Classic sound on Moog leads and the intro of "Jump" (Van Halen).

| Control | Range | Default | Notes |
|---|---|---|---|
| Sync→2 | toggle | off | On OSC 1 panel only, amber = active |

**How to use:** Enable OSC 2, set it to a higher pitch than OSC 1 (e.g. octave +1 or large detune), then engage `Sync→2`. Sweeping OSC 2's pitch while sync is on produces the characteristic hard sync sweep sound.

**Works with unison:** All 5 unison copies of OSC 2 are slaves — they all reset in the same sample when OSC 1's master copy wraps.

### Hard sync signal flow

```mermaid
flowchart LR
    OSC1["OSC 1 copy 0\n(Master)"]
    GEN["sync_gen\nAtomicU8\nper voice"]
    OSC2_0["OSC 2 copy 0\n(Slave)"]
    OSC2_1["OSC 2 copy 1\n(Slave)"]
    OSC2_N["OSC 2 copy …\n(Slave)"]

    OSC1 -->|"phase wrap\n→ gen++"| GEN
    GEN -->|"gen changed?\n→ reset phase"| OSC2_0
    GEN -->|"gen changed?\n→ reset phase"| OSC2_1
    GEN -->|"gen changed?\n→ reset phase"| OSC2_N
```

The generation counter approach avoids the need to clear a flag — each slave independently compares its `last_gen` to the shared counter every sample. When `Sync→2` is off, master and slaves skip all sync logic with no CPU overhead.

---

## Signal flow (per OSC slot)

```mermaid
flowchart LR
    FREQ([voice freq Hz])
    FM[× osc_freq_mult\noctave + detune]

    subgraph UNISON[Unison — 5 copies]
        U0[× detune0\n→ MultiWaveOsc\n× vol0]
        U1[× detune1\n→ MultiWaveOsc\n× vol1]
        U2[× detune2\n→ MultiWaveOsc\n× vol2]
        U3[× detune3\n→ MultiWaveOsc\n× vol3]
        U4[× detune4\n→ MultiWaveOsc\n× vol4]
    end

    SUM((Σ))
    VOL[× osc_vol]

    FREQ --> FM --> U0 & U1 & U2 & U3 & U4 --> SUM --> VOL
```

Inactive unison copies have `vol = 0.0` — they are always present in the graph but contribute nothing.

---

## MultiWaveOsc internals

```mermaid
flowchart TD
    IN([freq Hz input])
    DT[dt = freq / sr]
    ACC[phase += dt\nphase mod 1.0]
    WAVE{WaveShape}
    SIN[sin·p·2π]
    SAW[2p − 1\n− polyBLEP]
    SQR[naive·threshold·pw\n+ polyBLEP rising\n− polyBLEP falling]
    TRI[4p−1 or 3−4p]
    OUT([sample out])

    IN --> DT --> ACC --> WAVE
    WAVE -->|Sine| SIN --> OUT
    WAVE -->|Saw| SAW --> OUT
    WAVE -->|Square| SQR --> OUT
    WAVE -->|Triangle| TRI --> OUT
```

### PolyBLEP band-limiting

Saw and Square contain discontinuities (instantaneous jumps) that alias at high pitches. PolyBLEP applies a quadratic polynomial correction within ±1 sample of each discontinuity, smoothing the step without a lookup table. Triangle and Sine are alias-free by nature.

### Graph architecture

Waveform switching uses a single static `MultiWaveOsc` node — no graph rebuild. The `AtomicU8` waveform selector is written by the UI thread and read by the audio thread every sample.

Unison uses 5 `MultiWaveOsc` instances per OSC slot, always present in the graph. Spread is controlled by `osc_unison_detune[osc][copy]: Shared` (freq multiplier) and `osc_unison_vol[osc][copy]: Shared` (weight). The UI writes these atomically — no graph rebuild, no glitch.
