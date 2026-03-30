# The Synth — Architecture

A MiniMoog-inspired subtractive synthesizer built in Rust with [fundsp](https://github.com/SamiPerttu/fundsp) (DSP) + [cpal](https://github.com/RustAudio/cpal) (audio I/O) + [egui](https://github.com/emilk/egui) (UI).

---

## Signal Flow Overview

```mermaid
flowchart LR
    KB[Keyboard / Sequencer]

    subgraph CTRL[Controllers]
        LFO[LFO\nrate · depth · shape]
        GLIDE[Glide\nportamento]
    end

    subgraph OSC[Oscillator Bank]
        O1[OSC 1\nwave · octave · detune]
        O2[OSC 2\nwave · octave · detune]
        O3[OSC 3\nwave · octave · detune]
        NOISE[Noise]
    end

    subgraph MIX[Mixer]
        M1[Vol 1]
        M2[Vol 2]
        M3[Vol 3]
        MN[Noise Vol]
    end

    subgraph FILT[Filter]
        FLP[Lowpass\ncutoff · resonance]
        FADSR[Filter ADSR\nenv amount]
    end

    subgraph AMP[Amp]
        AADSR[Amp ADSR\nattack · decay · sustain · release]
    end

    OUT[Output\nmaster volume]

    KB -->|freq + gate| CTRL
    CTRL -->|pitched freq| OSC
    LFO -->|modulates| OSC
    LFO -->|modulates| FILT

    O1 --> M1
    O2 --> M2
    O3 --> M3
    NOISE --> MN

    M1 & M2 & M3 & MN --> FILT
    FADSR -->|cutoff mod| FLP
    FILT --> AADSR
    KB -->|gate| FADSR
    KB -->|gate| AADSR
    AADSR --> OUT
```

---

## Detailed Per-Voice DSP Graph

Each polyphonic voice runs this graph independently. Up to 6 voices play simultaneously.

```mermaid
flowchart TD
    FREQ([freq\nShared])
    GATE([gate\nShared])

    FREQ --> GLIDE[Glide\nsmooth Hz]

    GLIDE -->|Hz| P1[" × oct1 × detune1"]
    GLIDE -->|Hz| P2[" × oct2 × detune2"]
    GLIDE -->|Hz| P3[" × oct3 × detune3"]

    P1 --> W1[OSC 1\nsaw/sq/tri/sin]
    P2 --> W2[OSC 2\nsaw/sq/tri/sin]
    P3 --> W3[OSC 3\nsaw/sq/tri/sin]

    W1 -->|× vol1| SUM((Σ))
    W2 -->|× vol2| SUM
    W3 -->|× vol3| SUM
    NOISE[pink / white\nnoise] -->|× vol_n| SUM

    LFO_PITCH([LFO pitch mod]) --> SUM

    SUM --> FILT[Lowpass Filter\ncutoff + env_mod + lfo_mod\nresonance]

    GATE --> FENV[Filter ADSR\nadsr_live]
    FENV -->|cutoff offset| FILT

    FILT --> AMUL((×))

    GATE --> AENV[Amp ADSR\nadsr_live]
    AENV --> AMUL

    AMUL --> PAN[pan 0.0\nmono→stereo]
    PAN --> OUT([stereo out])
```

---

## LFO Modulation Routing

The LFO is a single low-frequency oscillator that can be routed to one or more destinations.

```mermaid
flowchart LR
    LFO_OSC[LFO\nsin / tri / saw\nrate: 0.1–20 Hz\ndepth: 0.0–1.0]

    LFO_OSC -->|pitch offset Hz| DEST_PITCH[OSC pitch\nvibrato]
    LFO_OSC -->|cutoff offset Hz| DEST_FILT[Filter cutoff\nwah / filter sweep]
    LFO_OSC -->|amp scale| DEST_AMP[Amp\ntremolo]
```

---

## Thread Model

Two threads communicate via lock-free `Shared` atomics (fundsp) and a single `Mutex` for the oscilloscope buffer.

```mermaid
sequenceDiagram
    participant UI as UI Thread\n(egui ~60 fps)
    participant SH as Shared params\n(atomic f32)
    participant AU as Audio Thread\n(cpal ~44100 Hz)

    UI->>SH: key press → voice_freqs[slot].set(hz)
    UI->>SH: key press → voice_gates[slot].set(1.0)
    AU->>SH: read freq + gate every sample (no lock)
    AU->>AU: evaluate DSP graph → stereo sample
    AU->>AU: write to osc_buffer (try_lock)
    UI->>AU: read osc_buffer (lock) → draw oscilloscope
    UI->>SH: key release → voice_gates[slot].set(0.0)
```

---

## UI Layout

```mermaid
block-beta
    columns 1

    block:TOP["Top Panel"]
        columns 4
        OSC1["OSC 1\nwave · oct · detune"]
        OSC2["OSC 2\nwave · oct · detune"]
        OSC3["OSC 3\nwave · oct · detune"]
        MIXER["Mixer\nvol 1-2-3 · noise"]
    end

    block:MID["Middle Panel"]
        columns 4
        LFOB["LFO\nrate · depth · dest"]
        FILTB["Filter\ncutoff · res · env amt"]
        FADSR2["Filter ADSR\nA · D · S · R"]
        AADSR2["Amp ADSR\nA · D · S · R"]
    end

    block:BOT["Bottom Panel"]
        columns 2
        KEYS["Keyboard\n(click or a-l keys)"]
        SEQ["Sequencer\n8 steps · BPM · pattern"]
    end

    block:SCOPE["Oscilloscope strip (always visible)"]
        columns 1
        OSCVIS["~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~"]
    end
```

---

## Module Structure

```mermaid
graph TD
    MAIN[main.rs\nSynthApp · egui loop]

    subgraph audio_rs[audio.rs]
        AS[AudioState\nall Shared params]
        AE[AudioEngine\ncpal stream]
        GRAPHS[DSP Graph builders\nbuild_poly_graph\nbuild_seq_graph]
        CB[Audio Callback\nper-sample evaluation\nLFO computation\ngraph hot-swap]
    end

    MAIN -->|Arc clone| AS
    MAIN -->|owns| AE
    AE -->|owns| CB
    CB -->|reads| AS
    CB -->|calls| GRAPHS
```

---

## Parameter Reference

### Oscillators (×3)

| Parameter | Range | Notes |
|-----------|-------|-------|
| Waveform  | sine / saw / square / triangle | Per OSC |
| Octave    | -2 … +2 oct | Relative to played note |
| Detune    | -100 … +100 cents | Fine pitch offset |
| Mix level | 0.0 … 1.0 | In the mixer |

### Noise

| Parameter | Range | Notes |
|-----------|-------|-------|
| Type      | white / pink | |
| Mix level | 0.0 … 1.0 | |

### LFO

| Parameter | Range | Notes |
|-----------|-------|-------|
| Rate      | 0.1 … 20 Hz | |
| Depth     | 0.0 … 1.0 | Scales modulation amount |
| Shape     | sin / tri / saw | |
| Destination | pitch / filter / amp | |

### Filter

| Parameter | Range | Notes |
|-----------|-------|-------|
| Cutoff    | 80 … 18000 Hz | Logarithmic |
| Resonance | 0.5 … 20 Q | |
| Env Amount | 0.0 … 1.0 | How much filter ADSR opens filter |
| Type      | lowpass (Moog-style) | Fixed for now |

### Amp ADSR

| Parameter | Range |
|-----------|-------|
| Attack    | 1 ms … 2 s |
| Decay     | 1 ms … 2 s |
| Sustain   | 0.0 … 1.0 |
| Release   | 1 ms … 4 s |

### Glide

| Parameter | Range | Notes |
|-----------|-------|-------|
| Time      | 0 … 500 ms | 0 = instant (off) |

---

## What Changes from v0.1

| Area | Before | After |
|------|--------|-------|
| Oscillators | 1 per voice, 1 waveform | 3 per voice, each configurable |
| Mixer | None | Per-OSC + noise volume |
| Filter | Separate tab, no chain | Integrated in signal chain |
| LFO | None | Rate / depth / destination |
| Glide | None | Portamento between notes |
| UI | 4 tabs (educational) | 1 unified synth panel |
| Sequencer + Keyboard | Drive separate graphs | Both drive same full chain |
