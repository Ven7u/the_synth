# Performance Roadmap

Reference document for forma's DSP CPU budget: what we found when we
profiled the engine, what's been shipped, and what's left to do — ordered
by ROI.

## Why this doc exists

The engine was heavier than it needed to be for the target use cases: running
alongside a Bevy game, on modest hardware, and eventually as a DAW plugin.
First audit (pre-fix) showed **RTF ≈ 24× regardless of note count** — meaning
the synth used ~4% of one CPU core just existing, with or without notes
being played. That's a fragile floor for plugin / game-engine contexts where
every percent matters and many instances may run in parallel.

## What we found (the audit)

**Single biggest cost centre: the oscillator bank ticks every sample,
regardless of gate state.**

Per-voice cost summary:

- 3 OSC slots × 5 unison copies = **15 `MultiWaveOsc` per voice**
- × 6 voices = **90 oscillators, always running**
- Plus 6 Moog lowpass filters, 12 `LiveAdsr`

When a voice's gate is 0, the amp envelope multiplies the output by 0, but
the 15 oscillators and filter still evaluate every sample. The zero at the
end hides it, but the cost is paid.

Secondary cost centres (in order):

| Cost | Impact | Location |
|---|---|---|
| Denormal floats in reverb tails | 10–100× CPU spike on Intel | no FTZ/DAZ setup anywhere |
| Default release profile | 5–15% perf left on the table | no `[profile.release]` overrides |
| Shimmer runs full algorithm at `amt=0` | 400–600 ns/sample wasted | [crates/forma-dsp/src/shimmer.rs](../crates/forma-dsp/src/shimmer.rs) |
| Crystallizer grain scheduler runs at low mix | 200–400 ns/sample | [crates/forma-dsp/src/crystallizer.rs](../crates/forma-dsp/src/crystallizer.rs) |
| Per-sample `sin`, `powf`, `tanh` | ~20–60 ns each | LFO phase, FX tone LPs, output soft-clip |
| No voice-activity tracking | idle cost = fully-active cost | the dominant problem |

## What's shipped (current baseline)

Four changes already applied, documented here for completeness:

### 1. Release profile tuning — [Cargo.toml](../Cargo.toml)

```toml
[profile.release]
lto           = "fat"       # inline across crate boundaries (fundsp + engine)
codegen-units = 1           # best code layout / inlining decisions
```

Costs more compile time. No runtime cost. 5–15% perf on Intel; within
measurement noise on Apple Silicon.

### 2. FTZ/DAZ denormal protection — [crates/forma-engine/src/denormals.rs](../crates/forma-engine/src/denormals.rs)

`enable_ftz_on_current_thread()` sets flush-to-zero + denormals-are-zero on
x86/x86_64 (MXCSR) and aarch64 (FPCR bit 24). Called from the cpal callback's
first invocation and from the benchmark's `main`. Eliminates the Intel/AMD
subnormal cliff during reverb tails. No audible effect; silent insurance.

### 3. Voice activation gating — Stage 7

- **New** [crates/forma-engine/src/gated_voice.rs](../crates/forma-engine/src/gated_voice.rs) — `GatedVoice<X>` AudioNode wraps each voice's sub-graph. Reads a per-voice `AtomicBool` flag per tick; returns zeros and skips the inner graph when the flag is `false`. On false→true transition calls `inner.reset()` for a clean wake-up.
- **Edit** [crates/forma-engine/src/voice.rs](../crates/forma-engine/src/voice.rs) — new `VoiceAllocator::update_audibility(state)` method publishes the flag per voice: audible iff `gate > 0.5 OR amp_cursor > 0.5 OR retrigger_countdown > 0`. Runs once per audio buffer after event drain.
- **Edit** [crates/forma-engine/src/audio.rs](../crates/forma-engine/src/audio.rs) — each voice wrapped with `An(GatedVoice::new(make_voice(vi).0, Arc::clone(&state.voice_audible[vi])))`.

### 4. Benchmark harness — [crates/forma-bench/src/bin/synth-perf.rs](../crates/forma-bench/src/bin/synth-perf.rs)

```text
cargo run --release -p forma-bench --bin synth-perf
```

Renders 10 seconds of audio per scenario and reports realtime factor (RTF),
µs/buffer, active voice count. Six scenarios: idle, 1 note, 6 notes clean,
6 + mod FX, 6 + heavy FX, 6 + unison. Sample rate 48 kHz, 256-sample blocks.
Use as regression guard: any future DSP change should be measured before/after.

## Current numbers (after Stage 7, Apple Silicon M-series)

| Scenario | RTF | µs/buffer | Notes |
|---|---|---|---|
| idle (no notes, default FX) | **189.7×** | 28 | ~0.5% CPU |
| 1 note held | 90.3× | 59 | 1 voice active, 5 skipped |
| 6 notes clean | 23.3× | 228 | all voices active, no gating possible |
| 6 + mod FX | 20.7× | 258 | reverb + delay + chorus @ 0.35 mix |
| 6 + heavy FX | 16.3× | 326 | + shimmer + crystal |
| 6 + unison 5× | 22.3× | 239 | 5 unison copies on all 3 osc |

**Idle improvement from pre-Stage-7: 24× → 190× RTF (7.8× faster).**
Cost per active voice: ~33 µs/buffer regardless of other state.

## Remaining roadmap (ordered by ROI)

### Quick wins — small effort, known gain

**1. Unison copy gating** — same pattern as voice gating, one layer deeper.
Today each active voice unconditionally ticks 5 unison copies per oscillator;
the default patch has copies 1–4 at `vol=0`. Adding a per-copy `AtomicBool`
(or a per-osc `active_copies: u8` bitmask) would skip 4 of 5 copies in the
common case.
- Expected: **20–40% on active voices when unison is off**
- Effort: small (mirror of `GatedVoice` at unison-copy level)
- Risk: low

**2. Shimmer/Crystal internal bypass when `amt = 0`** — both guard output
mix with `if mix > 0.0001` but still run comb filters / grain scheduler
internally when `shimmer_amt = 0` or equivalent Crystal parameters are zero.
- Expected: **5–15% on heavy-FX scenarios**
- Files: [crates/forma-dsp/src/shimmer.rs](../crates/forma-dsp/src/shimmer.rs), [crates/forma-dsp/src/crystallizer.rs](../crates/forma-dsp/src/crystallizer.rs)
- Effort: small
- Risk: low

**3. Per-buffer atomic reads** — the DSP graph reads `osc_vol`, `osc_unison_detune`,
etc. per sample via fundsp's `var(&shared)` pattern. For slider-rate values,
one read per buffer is sufficient; a thin cache wrapper preserves semantics.
- Expected: **5–10% overall**
- Effort: small
- Risk: low

**4. LFO lookup table** — replace `(phase * TAU).sin()` (per LFO, per sample)
with a 256-entry table + linear interpolation. Same audible result, ~30 ns
saved × 2 LFOs × 48k.
- Expected: **3–7% overall**
- Location: the inner sample loop in [crates/forma/src/audio.rs](../crates/forma/src/audio.rs)
- Effort: small
- Risk: low

**5. Approximate `tanh`** — the `raw_l.tanh()` soft-clip at the output runs
per sample. Padé 2/3 or 3/5 approximation is ~5× faster, inaudibly different.
- Expected: **3–5% overall**
- Effort: small
- Risk: low-to-mid (quality test on extreme signals)

### Medium effort — meaningful gains

**6. Block-rate LFO modulation** — `state.lfo_pitch_mult` and
`state.effective_cutoff` are written per sample from the LFO trig. Compute
them once every ~32 samples and linearly interpolate in between. Audible
modulation is smooth regardless.
- Expected: **5–10% overall**
- Effort: medium
- Risk: low

**7. Reverb downsample stage** — shimmer, plate, FDN hall all run at full
engine rate. Dropping the reverb tail to 24 kHz and upsampling the output
is industry standard; indistinguishable audibly.
- Expected: **30–50% off reverb cost** (heavy-FX scenario moves from 16× to
  ~22–25×)
- Effort: medium-large (needs polyphase filters + state handling)
- Risk: medium (requires quality A/B testing)

### Large moves — only if needed

**8. SIMD the unison copies** — 5 copies × 3 osc × per-voice is perfectly
shaped for 4-wide SSE / NEON. Biggest single remaining optimisation but
largest implementation cost.
- Expected: **2–4× on active voices with unison**; brings heavy-FX
  scenario into professional-plugin territory (~60–80× RTF)
- Effort: large (restructure `MultiWaveOsc` to operate on `f32x4` vectors,
  validate numerics against scalar)
- Risk: medium
- Prerequisite: ideally combine with #1 (unison masking) first

**9. Option B voice summation** — bypass fundsp's `v0 + v1 + ... + v5`
summation; render active voices into a mix buffer directly in the callback.
Makes Stage 7's voice gating more efficient at the cost of leaving fundsp's
composable graph for the voice layer.
- Expected: marginal on top of Stage 7 alone; needed as a clean base for #8
- Effort: large
- Risk: mid

## What's not worth doing

- **Replacing fundsp.** It's well-optimised; our hotspots are in the way
  we wire nodes, not in the nodes themselves.
- **Double → single precision.** Already single.
- **Per-voice filter sharing.** Complex inter-voice coupling; would sound wrong.
- **Lower cpal buffer size.** That's a latency knob, not a throughput knob.

## The honest ceiling

Current state — comfortable for Bevy coexistence, acceptable for a casual
8-instance DAW session.

After #1–6 (all quick + medium effort): **idle ~250× RTF, heavy FX ~25×**.
That's the ceiling of the "easy" path.

After #7 + #8 (committed week of work): **heavy FX ~60–80× RTF**. That's
professional-plugin territory. Whether the work pays off depends entirely
on whether you actually ship as a DAW plugin or in scenarios with many
simultaneous instances.

## Regression-guarding

Any future DSP-related change should be measured with the harness:

```
cargo run --release -p forma-bench --bin synth-perf
```

Targets:
- idle RTF should stay **≥ 150×**. A drop indicates voice gating regressed.
- 6-notes-clean RTF should stay **≥ 20×**. A drop indicates a per-voice cost regression.
- 6-notes-heavy-FX RTF should stay **≥ 15×**. A drop indicates FX chain regression.

If any threshold slips, git-bisect to the commit and decide whether to fix
or accept.

## Recommendation

Ship what's here. Current numbers already meet the stated targets (game
coexistence, modest hardware, casual multi-instance). Come back to the
bucket list when a concrete workload says you must — not as premature
optimisation. If that day comes, work top-down from item #1; measure after
each change; stop climbing the list when the workload is satisfied.
