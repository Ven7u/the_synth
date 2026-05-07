# Click & Pop Prevention — Engine-Wide Plan

## Why this exists

Several places in the engine multiply control-rate values into the audio path without bandlimiting them. When the user moves a slider, toggles a destination, or a trigger fires, the control value steps to a new value within one sample. Multiplied with audio, that step becomes broadband energy — perceived as a click, pop, or zipper.

The fix is well-known DSP hygiene: **anything multiplied with audio must change smoothly** (≥ ~1 ms transition). The codebase already applies this to FX-chain mix/drive params (via `SmoothedParam` in [crates/synth-engine/src/audio.rs](../crates/synth-engine/src/audio.rs)) and to voice retrigger (via `retrigger_countdown` in [crates/synth-engine/src/voice.rs](../crates/synth-engine/src/voice.rs)). It is **not** applied uniformly across the rest of the modulation graph.

This plan is a phased audit + fix + prevention pass to bring the whole engine up to that bar.

Reference: see also [doc/dsp-guidelines.md](dsp-guidelines.md) for general DSP rules.

---

## Phase 0 — Tooling (do first, before any fixes)

Without a measurement tool, every fix is a listening test, every regression is invisible, and progress is unprovable. Build the detector before doing anything else.

- [ ] **Offline click-detector test** in `synth-bench` or a new `synth-tests/clicks` crate.
  - Renders the engine for N seconds with a sustained voice playing.
  - Sweeps each `Shared` / atomic from min → max in 10-sample steps.
  - Records output, asserts `max(|sample[n] - sample[n-1]|) < THRESHOLD`.
  - Reports the offending param + sample index on failure.
  - Exit criterion: a failing test for at least one known-clicky param (LFO depth slider) before fixes start; a passing baseline for already-smoothed params (FX mix).
- [ ] **FFT companion check** in the same harness — reject runs whose output diff has broadband energy above 4 kHz during a param sweep (proxy for "broadband click").
- [ ] **Runtime click meter** widget in the dev UI — peak `|Δsample|` over a 200 ms window, visible while moving sliders. Cheap, immediate feedback during manual tuning.
- [ ] **Wire the detector into CI** as a regression gate (after Phase 1 establishes a clean baseline).

---

## Phase 1 — Audit (catalog every audio-path multiplier)

Mechanical pass over the engine. For each `Shared` / `Atomic*` field on `AudioState`, classify it:

| Category | Meaning | Action |
|---|---|---|
| **A. Smoothed already** | Wrapped in `SmoothedParam` or smoothed inline | Verify; mark in source |
| **B. Bandlimited by physics** | A frequency / phase / cutoff (the moog filter is itself a smoother) | No action; mark as exempt |
| **C. Multiplicative gain, unsmoothed** | Multiplied with audio raw | **Smoothing required** |
| **D. Mode/destination switch** | Selects between branches that are then multiplied | **Crossfade required** |
| **E. Trigger / gate edge** | Discontinuous event that drives a multiplier | **Attack ramp required** |
| **F. Control-thread only** | Read by UI/sequencer, never multiplied with audio | No action; mark as exempt |

Output of this phase: a table in this doc with one row per field, classified, with the action item and the file/line of the call site.

### Likely Cat C suspects (to confirm during audit)
- [ ] `lfo_depth` — read per-buffer, used per-sample as audio multiplier (amp dest).
- [ ] `lfo2_depth` — same.
- [ ] `osc_vol[0..3]` — multiplied via `var(...)` inside fundsp graph.
- [ ] `osc_unison_vol[i][c]` — same.
- [ ] `noise_vol`.
- [ ] `fm_depth`, `ring_depth`.
- [ ] `filter_env_amount`.
- [ ] `voice_velocities[..]` — multiplied at voice mix.

### Likely Cat D suspects
- [ ] `lfo_dest`, `lfo2_dest` (instant redirect of LFO output).
- [ ] `lfo_shape`, `lfo2_shape` (instant waveform swap inside the LFO computation).
- [ ] `hard_sync_enabled` (changes whether OSC2 is sync'd).
- [ ] `lfo_sync` (free vs synced rate path).
- [ ] `fx_delay_sync` (free vs synced delay time).
- [ ] `fx_reverb_type` (Freeverb / Plate / FDN swap).
- [ ] `fx_shimmer.pitch`, `fx_crystal.pitch` (pitch-shift mode swap).

### Likely Cat E suspects (most already handled)
- [ ] Voice gates / note-on / note-off — handled by `retrigger_countdown` + ADSR.
- [ ] Filter env retrigger — handled by `LiveAdsr`.
- [ ] Gate-lane Pulse trigger — **fixed** in the asymmetric envelope pass.
- [ ] Future gate-lane retriggers (LFO1, LFO2, filter env, per-osc VCA) — **must apply the same attack-ramp pattern**.

### Patch-load case (high-leverage single fix)
- [ ] When the user loads a scene, dozens of params change in one frame. Currently `apply_patch` writes them all without coordination. Suspected source of the loudest clicks.

---

## Phase 2 — Fix (apply the standard catalog)

For each Cat C / D / E entry, apply the appropriate fix. Patterns ranked by leverage:

### 2.1 Master-bus fade-around for patch loads (single biggest win)
- [ ] Wrap `apply_patch` in a "fade `global_vol` to 0 over ~20 ms → apply → fade back" sequence.
- [ ] Guarantees no patch-load click regardless of internal state changes — bypasses needing to smooth every individual param touched during load.
- [ ] Implementation: one new `Shared` (`patch_load_mute_gain`), driven by a host-side timer; multiplied into the master output alongside `global_vol_smooth`.

### 2.2 SmoothedParam wrap for Cat C gains
- [ ] `lfo_depth` — 5–10 ms TC. Big win, slider is heavily used.
- [ ] `lfo2_depth` — 5–10 ms TC.
- [ ] `noise_vol` — 5 ms TC.
- [ ] `fm_depth` — 5 ms TC.
- [ ] `ring_depth` — 5 ms TC.
- [ ] `osc_vol[0..3]` — 5 ms TC. **Verify** how `var(&Shared)` interpolates inside fundsp before writing inline smoothing — fundsp's `var` may already smooth.
- [ ] `osc_unison_vol[i][c]` — 5 ms TC; same fundsp caveat.
- [ ] `filter_env_amount` — 10 ms TC.

> The pattern: either add a `SmoothedParam` wrapper inside `FxChain`-style consumers, or smooth inline in the audio callback (mirroring `voice_gain_smooth` / `global_vol_smooth`). Keep `Shared` as the control-thread-writable atom; smoothing lives audio-side.

### 2.3 Equal-power crossfades for Cat D mode switches

For each: when the atomic changes, fade out the old branch's contribution and fade in the new one over ~10 ms.

- [ ] `lfo_dest` — fade between pitch / filter / amp destinations.
- [ ] `lfo2_dest`.
- [ ] `lfo_shape`, `lfo2_shape` — likely inaudible at low depth, audit before fixing.
- [ ] `hard_sync_enabled` — soft transition between sync'd and free-running OSC2.
- [ ] `lfo_sync` toggle — already smoothed by the rate-write pattern, but confirm.
- [ ] `fx_delay_sync` — switching between two delay-time smoothers, already crossfade-safe via independent smoothers (verify).
- [ ] `fx_reverb_type` — crossfade between reverb engines.

### 2.4 Attack ramps on triggers
- [ ] Pulse gate-lane — **done**.
- [ ] Future gate-lane lanes (LFO1/2 retrigger, filter-env retrigger, per-osc VCA) — apply the same asymmetric-envelope pattern from Pulse.

---

## Phase 3 — Prevention (architectural disciplines)

Three levels, increasingly invasive. Apply Level 1 always, Level 2 once a few smoothing wrappers exist, Level 3 if the engine grows materially.

### Level 1 — Convention + invariant comments (always)
- [ ] Annotate every `Shared` / `Atomic*` field on `AudioState` with one of:
  - `// SMOOTHED — wrap in SmoothedParam before any audio-path multiply.`
  - `// BANDLIMITED — frequency/phase, no smoothing needed.`
  - `// CONTROL — never read on audio thread.`
- [ ] Add a section to [doc/dsp-guidelines.md](dsp-guidelines.md): "Anything multiplied with audio must transition over ≥ 1 ms. Use `SmoothedParam` or an inline one-pole."

### Level 2 — A type that enforces the rule (medium)
- [ ] Define `SmoothedShared` (or `AudioGain`) wrapping a `Shared` + smoothing TC. Only `next() -> f32` is exposed; `.value()` is hidden.
- [ ] Migrate all Cat C fields from `Shared` to `SmoothedShared` on `AudioState`.
- [ ] Migration is mechanical once the type exists; can be done one field at a time.

### Level 3 — Audit script in CI (heavy, defer until justified)
- [ ] Small `xtask` that walks audio-thread closures, greps `state.<field>.value()` / `.load()`, compares against an allowlist of Cat B/F fields, fails CI on new violations.
- [ ] Worth it once the engine has 30+ audio-path params or multiple developers are touching the audio thread.

---

## Phase 4 — Long-term: Modulation-bus refactor (optional, not v1)

If `SmoothedParam` wrappers proliferate, drift toward this design:

- [ ] **Two atom layers**: control-thread atomics (raw user values) → "control smoother" stage at top of audio buffer → audio-thread `Shared`s (smoothed values) consumed by the DSP graph.
- [ ] DSP graph is *forbidden* from reading raw control atomics — only smoothed bus values.
- [ ] Click prevention becomes structural rather than per-call-site discipline.
- [ ] Cost: one indirection per param. Benefit: the click problem becomes a property of the bus stage, not scattered.

Not a v1 task. Note here so future-you remembers the option.

---

## Suggested execution order

1. **Day 1** — Phase 0 detector + FFT check. Run it against current engine. Capture failing params.
2. **Day 1 cont'd** — Phase 2.1 master-bus fade-around `apply_patch`. Hides the loudest single source.
3. **Day 2** — Phase 1 audit (mechanical pass; fill in the Cat tables in this doc).
4. **Day 2 cont'd** — Phase 2.2 smoothing for Cat C gains. Re-run detector; tick off entries.
5. **Day 3** — Phase 2.3 crossfades for the Cat D switches that the detector flags as audible.
6. **Day 3 cont'd** — Phase 3 Level 1 invariant comments on every `AudioState` field.
7. **Day 4** — Wire Phase 0 detector into CI as regression gate.

Stop after step 7 unless the detector or listening reveal more. Phase 3 Level 2 / Phase 4 are reserved for if/when the engine grows.

---

## Definition of done

- [ ] Phase 0 detector passes on a baseline scene with all relevant param sweeps.
- [ ] Patch load (any preset → any preset) is click-free.
- [ ] LFO1/LFO2 depth sliders moving from 0 → 1 are zipper-free.
- [ ] OSC volume sliders are zipper-free.
- [ ] Mode switches (LFO dest, reverb type) cause no audible pop on a sustained pad.
- [ ] Detector test runs in CI and blocks regressions.
- [ ] Every `AudioState` field is annotated with its smoothing-class invariant.
