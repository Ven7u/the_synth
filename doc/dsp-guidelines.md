# DSP Developer Guidelines

Rules and patterns to follow when working on the audio engine. Violating these will cause noise, clipping, distortion, or silence.

---

## 1. Signal Flow Topology

### Never duplicate stateful filters across parallel paths

A stateful filter (smoothing, slew limiter, lowpass, delay, etc.) should appear **once** in the signal chain, at the point where the signal is still singular. If you need to fan out a signal to N parallel paths, apply the filter **before** the fan-out, not inside each branch.

**Why:** Every stateful filter instance maintains its own internal memory. When N copies of the same filter process the same logical signal independently, their internal states diverge over time. The outputs drift apart, accumulate different DC offsets, and introduce phase mismatches. Summing these divergent outputs produces beating, noise, and amplitude spikes that sound like distortion.

```
WRONG — filter duplicated per branch:

    signal
    ├── filter_copy_1 → process_A
    ├── filter_copy_2 → process_B
    └── filter_copy_3 → process_C
    → sum

RIGHT — filter once, then fan out:

    signal → filter → fan-out
                       ├── process_A
                       ├── process_B
                       └── process_C
                       → sum
```

**Scale of damage:** This scales with (copies x voices). A synth with 5 unison copies x 3 oscillators x 6 voices = 90 redundant filter instances. That is enough to make the output unrecognizable.

### Keep the graph as flat as possible

Avoid deep chains of operations when a shallow graph achieves the same result. Every node in the graph adds latency and state. The fewer nodes between input and output, the fewer places things can go wrong.

---

## 2. Gain Staging and Amplitude Control

### Always know the peak amplitude at every summing point

Before summing N signals, determine the worst-case peak of the sum. If each signal peaks at A, the sum can peak at N * A. Apply normalization at the summing point, not downstream.

```
N signals of amplitude A each:
  Worst-case peak after sum = N * A
  Apply gain of 1/N at the sum (or 1/sqrt(N) for uncorrelated signals)
```

### Never rely on a final soft-clipper to fix upstream gain problems

A `tanh()` or similar limiter on the output is a **safety net**, not a mixing strategy. If the signal regularly exceeds 1.0 before the clipper, the sound will be audibly compressed and distorted. Fix the gain staging so the signal stays in [-1.0, 1.0] under normal conditions.

### Account for phase coherence in amplitude calculations

When summing N copies of similar signals:
- **Phase-coherent** (same phase): peak = N * single_peak. Normalize by 1/N.
- **Phase-random** (spread phases): average peak ~ sqrt(N) * single_peak. Normalize by 1/sqrt(N) for constant perceived loudness.
- **Uncorrelated** (noise-like): peak ~ sqrt(N) statistically, but transients can spike higher.

When in doubt, use the conservative 1/N normalization.

---

## 3. Phase Management for Unison / Chorus

### Always spread initial phases across unison copies

If multiple oscillators play the same (or nearly the same) frequency, they must start at different phases. Without phase spreading, all copies are coherent: they constructively interfere at peaks and cancel at troughs, producing amplitude beating that sounds like tremolo, wobble, or noise depending on the frequency difference.

```
For N unison copies, assign initial phase = k/N for k in 0..N
This distributes copies evenly across one cycle.
```

### Detune alone is not enough

Small detune values (e.g., +/- 10 cents) mean frequencies are nearly identical. Phase drift between copies is very slow (a few Hz of beat frequency). Without initial phase spreading, the copies spend long periods nearly aligned, then nearly cancelled, creating an unpleasant slow amplitude modulation instead of a smooth chorus effect.

---

## 4. Parameter Updates and Thread Safety

### UI-rate parameters do not need audio-rate smoothing

Parameters updated from UI sliders change at ~60 Hz (frame rate). They are already smooth enough for most purposes. Adding a smoothing filter (slew limiter, lowpass) is only necessary when:
- The parameter is being modulated at audio rate (e.g., by an LFO or envelope)
- The parameter directly controls a discontinuous process (e.g., hard-sync, wavetable index)
- You need portamento/glide behavior

### One smoother per logical signal, not per consumer

If a parameter needs smoothing (e.g., voice frequency for glide), apply the smoother **once** at the source, then distribute the smoothed value to all consumers. Do not put a smoother in front of every oscillator that reads the same frequency.

---

## 5. Graph Sizing and Performance

### Count your nodes

Every node in the DSP graph costs CPU per sample. Before adding nodes, multiply:

```
total_nodes = nodes_per_copy * copies_per_osc * oscs_per_voice * voices
```

For this synth: 1 node * 5 copies * 3 oscs * 6 voices = 90 nodes for oscillators alone. Adding an extra filter per path means 90 additional filters. Always ask: "Does each copy genuinely need its own instance of this node, or can it be shared?"

### Prefer static graphs with dynamic parameters

Build the graph once with all possible paths present. Use `Shared` values (volume = 0.0) to silence unused paths instead of rebuilding the graph. Graph allocation is not real-time safe and will cause audio dropouts if done on the audio thread.

---

## 6. Common Mistakes — Quick Reference

| Mistake | Symptom | Fix |
|---|---|---|
| Stateful filter per unison copy | Noise, distortion, DC drift | Filter once before fan-out |
| No phase spreading on unison | Beating, wobble, amplitude pumping | Spread phases evenly: k/N |
| Missing gain normalization at sum | Clipping, distortion | Scale by 1/N or 1/sqrt(N) |
| Smoothing UI-rate params redundantly | Wasted CPU, potential instability | Only smooth if modulated at audio rate |
| Graph rebuild on audio thread | Clicks, dropouts, silence | Use static graph + Shared params |
| Relying on output tanh() for mixing | Compressed, mushy sound | Fix gain staging upstream |

---

## fundsp-Specific Rules

These rules apply specifically to the fundsp library used in this project.

### `var()` nodes are consumed on use

A `var(&shared)` node is an owned value in the graph. You cannot use the same `var()` binding in multiple branches. Each branch needs its own `var(&shared)` call. The underlying `Shared` can have many readers — the `var()` wrapper is what's single-use.

### `follow(time)` is a slew-rate limiter, not a general filter

`follow(t)` smooths a signal with a given time constant. It is meant for **parameter smoothing** (e.g., smoothing frequency changes for glide). It is **not** a general-purpose lowpass filter. Do not place it in every oscillator path "just in case". It maintains internal state that accumulates over time.

### `>>` is pipe, not fan-out

The `>>` operator connects one node's output to another's input. It does **not** duplicate the signal. To send the same signal to multiple destinations, you need separate `var()` reads of the same `Shared`, or use fundsp's branching combinators (`|`, bus, etc.).

### `Shared` is the correct way to modulate parameters

Use `Shared` + `var()` for any parameter that changes at runtime (volume, frequency, detune, etc.). Writes from the UI thread are atomic and lock-free. The audio thread reads the latest value each sample via `var()`. This is fundsp's intended pattern for real-time parameter control.

### `BlockRateAdapter` reduces per-sample overhead

Wrap the entire graph in `BlockRateAdapter` for the audio callback. This processes samples in blocks instead of one-at-a-time, which is significantly faster for large graphs. Already used in this project — do not remove it.

### `adsr_live` requires a continuous gate signal

`adsr_live(a, d, s, r)` reads a gate input every sample. The gate must be 0.0 (off) or 1.0 (on). Do not feed it intermediate values or modulated signals. The gate controls the envelope state machine — anything other than 0/1 produces undefined behavior.

### Always call `set_sample_rate()` and `allocate()` on the final graph

After building the graph and before using it in the audio callback:
```rust
graph.set_sample_rate(sr);
graph.allocate();
```
`allocate()` pre-allocates internal buffers. Without it, the first few audio callbacks may allocate memory, causing dropouts.

---

## Build Profile: Always Optimize

### Never run a real-time audio app in debug mode

Rust's default `cargo run` uses the `dev` profile with **no optimization** (`opt-level = 0`). This means:
- No inlining of `f32` math, `tanh()`, `sin()`, etc.
- No auto-vectorization (SIMD)
- Every `AudioNode::tick()` call has full function call overhead
- fundsp graph traversal is orders of magnitude slower

For a synth with 90+ oscillator nodes across 6 polyphonic voices, unoptimized code **cannot keep up with real-time audio** at 44.1kHz. The audio callback takes longer than the buffer duration, causing **buffer underruns** — gaps in the output that sound like random clicks, pops, and crackling.

This is deceptive because:
- The signal level looks fine on the meter (it's not clipping)
- The clicks are random and intermittent (depends on CPU load and scheduling)
- It sounds similar to amplitude clipping but has a different character (sharp digital pops vs. waveform distortion)

### How to fix

Add this to `Cargo.toml`:

```toml
[profile.dev]
opt-level = 2
```

This gives optimized code while keeping debug symbols and reasonable compile times. `opt-level = 2` is enough for real-time audio. Use `opt-level = 3` or `--release` if you need maximum performance.

### How to tell the difference

- **Amplitude clipping**: visible on the peak meter (signal > 1.0), consistent distortion that gets worse with volume, waveform looks squashed on the oscilloscope.
- **Buffer underruns**: meter shows normal levels, clicks are random and independent of volume, waveform has sudden discontinuities (gaps).

### Rule of thumb

If you hear random clicks/pops but the peak meter is green, check your build profile first. This applies to any real-time audio project in Rust, not just fundsp.
