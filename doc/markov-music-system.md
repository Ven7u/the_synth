# Markov Music System — Design Document

**Status:** Design phase (pre-implementation)
**Replaces:** Phase 8.2 generators (Euclidean, ProbTable) — those become sub-components
**Target crates:** `ambient-engine` (core), `synth-bevy` (integration), `ambient-box` (UI)

---

## 1. Motivation

Euclidean and probability-table generators are rhythmically mechanical and harmonically naive.
Each step is independent — there is no memory, no phrase shape, no relationship between voices.
The result sounds like a random arpeggiator, not music.

Markov chains model music as a sequence of **state transitions** where history shapes the
next state. A note follows from the previous note. A rest follows from a dense passage.
A chord resolves because the transition matrix encodes resolution probability.
The result has temporal coherence, phrasing, and style — without requiring authored sequences.

---

## 2. Core Design Principles

1. **Tonality-agnostic matrices.** All chains operate on relative quantities: scale degrees
   (1–7), rhythmic states (Rest/Hold/Single/Double/Accent), harmonic function (I/IV/V/etc.).
   Absolute pitch is resolved at output time from `(root, scale, degree)`. The same matrices
   work in any key; modulation is an external input, not a matrix property.

2. **External control of tonality and mood.** The game engine or user sets the root note,
   scale, mood blend, and density. The chains respond to these inputs but do not encode them.
   This separation means a single set of matrices can express any key and any mood through
   interpolation and parameter injection.

3. **Voice roles enforce harmonic coherence (Strategy 1+2).** A global harmonic chain sets
   the current chord function for all voices. Each voice has a fixed role (Bass, Pad, Melody,
   Texture) that constrains its register and scale-degree preference. Within those constraints
   the melodic chain provides organic variety. Structural clashes are prevented by role design,
   not by real-time conflict detection.

4. **Phrase boundaries prevent stasis.** A global phrase counter advances every N bars.
   At phrase boundaries the harmonic chain is permitted wider transitions (e.g. relative major
   modulation). This gives the music a sense of going somewhere rather than hovering.

5. **Trainable matrices.** Every probability matrix is a plain `[[f32; N]; N]` array and can
   be updated from analysis of existing music. A training pass reads MIDI or note-event logs,
   counts observed state transitions, and normalizes to produce new matrices. The runtime and
   the learner share the same data format.

6. **Launchpad visualization.** The UI shows N voice rows × M step columns. Cells light up
   in real time as the chains produce output. No steps are pre-programmed — the grid is a
   live display of generative output. This is the primary feedback loop for musicians and
   game developers alike.

---

## 3. Architecture Overview

```
External inputs
  ├─ root note (MIDI pitch 0-127)        ← keyboard, game event, MIDI
  ├─ scale (Major/Minor/Dorian/…)        ← scene config, game event
  ├─ mood blend (f32 per named mood)     ← game engine, macro knob
  ├─ density (f32 0-1)                   ← macro knob, game event
  └─ BPM                                 ← transport, game event

Global (shared across all voices)
  ├─ BeatClock          — sample-accurate timing source
  ├─ HarmonicChain      — current chord function, advances per phrase
  └─ PhraseCounter      — counts bars, triggers harmonic permission gates

Per voice (N voices)
  ├─ VoiceRole          — Bass / Pad / Melody / Texture
  ├─ RhythmicChain      — current rhythmic state, advances per subdivision
  ├─ MelodicChain       — current scale degree, advances when rhythmic fires
  ├─ Patch              — synth patch + effect params
  └─ LaunchpadRow       — M-cell display buffer (read by UI)

Output
  └─ NoteOn / NoteOff events → voice allocator → audio graph
```

---

## 4. The Three Markov Chains

### 4.1 HarmonicChain (global, advances per bar or phrase boundary)

**States — harmonic functions (7):**

| Index | Function | Example in C major | Example in A minor |
|---|---|---|---|
| 0 | `I` / `i` | Cmaj7 | Am7 |
| 1 | `II` / `ii` | Dm7 | Bm7b5 |
| 2 | `III` / `bIII` | Em7 | Cmaj7 |
| 3 | `IV` / `iv` | Fmaj7 | Dm7 |
| 4 | `V` / `V7` | G7 | E7 |
| 5 | `VI` / `bVI` | Am7 | Fmaj7 |
| 6 | `VII` / `bVII` | Bm7b5 | Gmaj7 |

States encode *function*, not pitch. Resolution to actual chord tones happens at
NoteOn time using the current root and scale.

**Normal transition matrix (rows = from, columns = to):**
Used within a phrase. Biased toward authentic cadences and common progressions.

```
         I     ii    iii   IV    V     vi    vii
I       0.25  0.10  0.05  0.25  0.20  0.10  0.05
ii      0.05  0.15  0.05  0.15  0.40  0.15  0.05
iii     0.10  0.10  0.10  0.25  0.20  0.20  0.05
IV      0.25  0.10  0.05  0.20  0.25  0.10  0.05
V       0.45  0.05  0.05  0.10  0.15  0.15  0.05
vi      0.15  0.20  0.05  0.20  0.25  0.10  0.05
vii     0.35  0.10  0.05  0.15  0.20  0.10  0.05
```

Key properties:
- V resolves to I with 45% probability (dominant resolution)
- vii resolves to I with 35% probability (leading-tone resolution)
- IV→V and ii→V are the most common subdominant→dominant moves
- I has moderate self-loop (tonic stability)

**Phrase-boundary transition matrix:**
Used only at phrase boundaries (every N bars). Allows wider moves — relative major/minor
shifts, modal interchange, unexpected pivots.

```
         I     ii    iii   IV    V     vi    vii
I       0.10  0.10  0.10  0.15  0.15  0.25  0.15   ← can jump to vi (relative minor)
ii      0.10  0.05  0.10  0.15  0.25  0.25  0.10
iii     0.10  0.10  0.05  0.20  0.15  0.25  0.15
IV      0.15  0.10  0.10  0.05  0.20  0.25  0.15
V       0.25  0.10  0.10  0.15  0.05  0.25  0.10
vi      0.25  0.15  0.10  0.15  0.15  0.10  0.10   ← vi→I creates relative shift
vii     0.20  0.10  0.10  0.15  0.20  0.15  0.10
```

### 4.2 RhythmicChain (per voice, advances per subdivision)

**States (5):**

| Index | State | Description | Typical duration |
|---|---|---|---|
| 0 | `Rest` | Silence | 1+ subdivisions |
| 1 | `Hold` | Sustain previous note | 1+ subdivisions |
| 2 | `Single` | One attack, normal velocity | 1 subdivision |
| 3 | `Double` | Two rapid attacks (2× 32nd) | 1 subdivision |
| 4 | `Accent` | One attack, high velocity | 1 subdivision |

**Calm matrix** (sparse, breathing, mostly resting):

```
          Rest  Hold  Single  Double  Accent
Rest      0.40  0.20  0.35    0.03    0.02
Hold      0.15  0.45  0.35    0.03    0.02
Single    0.25  0.20  0.40    0.10    0.05
Double    0.30  0.10  0.45    0.10    0.05
Accent    0.35  0.15  0.40    0.08    0.02
```

**Tense matrix** (dense, accented, restless):

```
          Rest  Hold  Single  Double  Accent
Rest      0.15  0.05  0.40    0.25    0.15
Hold      0.10  0.20  0.40    0.20    0.10
Single    0.10  0.10  0.35    0.30    0.15
Double    0.15  0.05  0.35    0.35    0.10
Accent    0.20  0.05  0.35    0.25    0.15
```

**Density control:** A density scalar `d ∈ [0,1]` modifies the effective matrix at
query time without changing the stored matrix:

```
effective_rest_prob = stored_rest_prob * (1 - d)   // suppress rest
// renormalize row to sum to 1.0
```

At `d=0` the matrix is used as-is. At `d=1` rest transitions are zeroed and the voice
plays almost continuously.

**Double** and **Accent** states always have high self-loop suppression — they are
naturally transient (a fill should not loop into another fill indefinitely).

### 4.3 MelodicChain (per voice, advances when rhythmic chain fires an attack)

**States — scale degrees (7):** `1 2 3 4 5 6 7`

All degrees are relative to the current scale. Actual MIDI pitch = `root + octave_offset +
scale.intervals[degree]`. The chain is tonality-agnostic by construction.

**Stepwise / Calm style** (small intervals, tonic pull, consonant):

```
       1     2     3     4     5     6     7
1     0.25  0.30  0.15  0.05  0.15  0.05  0.05
2     0.20  0.20  0.30  0.15  0.08  0.05  0.02
3     0.10  0.25  0.20  0.25  0.12  0.05  0.03
4     0.05  0.15  0.20  0.20  0.25  0.10  0.05
5     0.20  0.08  0.12  0.15  0.25  0.15  0.05
6     0.10  0.05  0.10  0.10  0.20  0.25  0.20
7     0.30  0.05  0.05  0.05  0.15  0.15  0.25
```

Key properties:
- Degree 7 resolves to 1 with 30% probability (leading tone)
- Degree 4 moves to 3 or 5 (avoids tritone stasis against 7)
- Tonic (1) has moderate self-loop — settled but moves on
- Adjacent degree transitions dominate (stepwise motion)

**Leaping / Heroic style** (large intervals, active):

```
       1     2     3     4     5     6     7
1     0.15  0.10  0.20  0.05  0.30  0.10  0.10
2     0.10  0.10  0.15  0.10  0.25  0.20  0.10
3     0.20  0.10  0.10  0.10  0.25  0.15  0.10
4     0.15  0.10  0.15  0.10  0.30  0.10  0.10
5     0.25  0.05  0.15  0.10  0.15  0.15  0.15
6     0.20  0.05  0.10  0.10  0.25  0.15  0.15
7     0.35  0.05  0.10  0.05  0.25  0.10  0.10
```

Key properties:
- Larger probability mass on non-adjacent degrees (4ths, 5ths)
- Degree 5 (dominant) is a common leap target from any state
- Degree 7 still resolves to 1 strongly (35%)

---

## 5. Voice Roles

Each voice has a fixed role that applies two constraints:
1. **Register** — octave range within which pitches are resolved
2. **Degree bias** — a per-degree weight vector multiplied into the melodic chain row

### Role definitions

**Bass**
- Register: octaves 1–2 (MIDI 24–47)
- Allowed degrees: `{1, 2, 5}` — root and fifth primarily, passing second
- Bias vector: `[3.0, 0.8, 0.0, 0.0, 2.5, 0.0, 0.0]`
  - Degree 4 = 0 (tritone risk against bass), degree 3/6/7 = 0 (weak in bass)
- Rhythmic style: slower subdivision clock (advances every 2 subdivisions)

**Pad**
- Register: octaves 3–4 (MIDI 48–71)
- Allowed degrees: chord tones of current harmonic state `{1, 3, 5}` ± 2 passing tones
- Bias vector: `[2.0, 0.5, 2.0, 0.3, 2.0, 0.5, 0.3]`
- Rhythmic style: prefers `Hold` and `Single`, rare `Accent`

**Melody**
- Register: octaves 4–5 (MIDI 60–83)
- Allowed degrees: all `{1, 2, 3, 4, 5, 6, 7}` — full melodic freedom
- Bias vector: `[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]` (unbiased)
- Rhythmic style: most rhythmically active, all states available

**Texture**
- Register: octaves 5–6 (MIDI 72–95)
- Allowed degrees: upper extensions `{2, 3, 6, 7}` — color tones, no root doubling
- Bias vector: `[0.2, 1.5, 1.2, 0.5, 0.5, 2.0, 1.8]`
- Rhythmic style: very sparse, mostly `Rest` and `Hold`

### Degree bias application

At each melodic step, the chain's transition row is element-wise multiplied by the role's
bias vector, then renormalized to sum to 1.0, before sampling. This means the role shapes
probabilities softly — it doesn't hard-exclude degrees (except where bias = 0), it just
makes some degrees much less likely.

---

## 6. Mood Blending

A mood is a named tuple of three matrices:
```
Mood {
    name: &str,
    harmonic: [[f32; 7]; 7],
    rhythmic: [[f32; 5]; 5],
    melodic:  [[f32; 7]; 7],
}
```

### Predefined moods

| Mood | Harmonic character | Rhythmic character | Melodic character |
|---|---|---|---|
| `Calm` | Tonic-heavy, IV→I resolutions | Sparse, breath, lots of rest/hold | Stepwise, tonic pull |
| `Tense` | Dominant-heavy, unresolved V | Dense, accented, syncopated | Leaping, dissonant degrees |
| `Dark` | Minor functions, bVI/bVII modal | Sparse but with sudden accents | Low register, chromatic passing |
| `Euphoric` | Fast IV→I lifts, major resolutions | Steady pulse, syncopated doubles | Ascending leaps, high register |
| `Floating` | Slow harmonic rhythm, modal | Very sparse, long holds | Narrow range, minimal motion |

### Interpolation

At runtime, the active matrix for any chain is computed as a weighted blend:

```
active_rhythmic = sum(mood_blend[i] * mood[i].rhythmic for i in moods)
// where sum(mood_blend) = 1.0
```

This is done per-row, per-sample-of-chain-state (i.e. only when a transition is queried,
not every audio sample). Cost: N_states × N_moods multiplications at transition time.

**Example:** 70% Calm + 30% Tense gives a rhythmic matrix where rest probability is
`0.7 * calm_rest + 0.3 * tense_rest` — mostly breathing but occasionally bursting.

---

## 7. Phrase Boundary System

```
PhraseConfig {
    bars_per_phrase: u32,      // e.g. 4 or 8
    phrases_per_section: u32,  // after N phrases, allow large harmonic jump
}
```

**Per bar:** `bar_counter++`. If `bar_counter % bars_per_phrase == 0` → phrase boundary.
At a phrase boundary, the harmonic chain uses the **phrase-boundary transition matrix**
instead of the normal matrix. This allows larger harmonic jumps (relative modulation, etc.).

**Per section** (every N phrases): a larger reset that can trigger a scale mode change
(e.g. major → dorian) or root shift. This is an **external event** — the phrase counter
fires an event upward to the host (game or ambient-box), which decides whether to act on it.
The system does not auto-modulate; it asks permission.

---

## 8. Tonality Control (External Input)

All of the following are runtime parameters, not matrix properties:

| Parameter | Type | Who sets it |
|---|---|---|
| `root` | MIDI note 0–127 | Keyboard, game event, scene |
| `scale` | enum (Major/Minor/Dorian/…) | Scene config, game event |
| `octave_offset` | i32 per voice | Voice role default, game event |
| `mood_blend` | `[f32; N_MOODS]` (sums to 1) | Macro knob, game event |
| `density` | f32 0–1 | Macro knob, game event |
| `phrase_length` | u32 (bars) | Scene config, game event |

**Keyboard input:** playing a note on the keyboard shifts the root to that note (or adds it
to a chord set that the harmonic chain uses as an anchor). The matrices don't change —
the resolution mapping changes. This lets a performer modulate a generative piece in real
time by pressing a new root note.

**Game engine:** sends `SynthEvent::SetRoot(pitch)`, `SynthEvent::SetMoodBlend([…])`,
`SynthEvent::SetDensity(f32)`. These write to atomics; the audio thread reads them at
each chain transition.

---

## 9. Matrix Training from Existing Music

### Training process

A training pass takes a sequence of observed events (from a MIDI file or live performance)
and counts state transitions:

```
for each consecutive pair (state_a, state_b) in observed sequence:
    count_matrix[state_a][state_b] += 1.0

// Normalize each row:
for row in count_matrix:
    total = sum(row)
    if total > 0: row /= total
```

For the **harmonic chain**: the input is a sequence of chord symbols (analyzed from MIDI or
provided as annotations). Each chord is mapped to a harmonic function (I/ii/iii/IV/V/vi/vii)
relative to the key. The transition count matrix is built from consecutive chord pairs.

For the **rhythmic chain**: the input is a sequence of note events on a rhythmic grid. Each
cell is classified as Rest/Hold/Single/Double/Accent. Consecutive cells build the count matrix.

For the **melodic chain**: the input is a sequence of scale degrees (absolute pitch → scale
degree given known key). Consecutive degree pairs build the count matrix.

### Smoothing

Raw count matrices from short songs have many zero entries, which create hard blocks in
generation (certain transitions can never occur). Apply Laplace smoothing:

```
smoothed[i][j] = (count[i][j] + alpha) / (row_total + alpha * N_states)
// alpha = 0.1 to 0.5 depending on desired smoothing strength
```

### Blending learned matrices with built-in moods

A trained matrix can be used as a new named mood and blended with existing moods. A
game could ship with `Calm`, `Tense`, `Dark` built-in and let designers train a
`HeroTheme` matrix from the game's own soundtrack. The mood blend knob then mixes
`50% Calm + 50% HeroTheme`.

### Format

Trained matrices are serialized as JSON (same format as scene files):

```json
{
  "name": "HeroTheme",
  "source": "midi/hero_theme.mid",
  "harmonic": [[0.25, 0.10, ...], ...],
  "rhythmic": [[0.40, 0.20, ...], ...],
  "melodic":  [[0.25, 0.30, ...], ...]
}
```

---

## 10. The Launchpad UI

### Grid layout

```
[Track 1] [●][○][●][○][●][○][●][○]  [Patch] [Vol] [Density] [Style]
[Track 2] [○][●][●][○][○][●][○][●]  [Patch] [Vol] [Density] [Style]
[Track 3] [●][○][○][○][●][○][○][○]  [Patch] [Vol] [Density] [Style]
[Track 4] [○][○][●][○][○][○][●][○]  [Patch] [Vol] [Density] [Style]

[BPM: 120]  [Key: Dm]  [Mood: ━━━━●━━━]  [Density: ━━●━━━━━]
```

### Cell states

| Display | Meaning |
|---|---|
| Dark | Rest |
| Dim white | Hold (sustaining) |
| Bright color | Single attack (color = voice role) |
| Bright + large | Accent |
| Flashing | Double (two rapid attacks) |

Cells are **not clickable to pre-program** — they are a read-only live display of chain output.
Future: optionally allow override (click to force a step, overriding the chain for that one cycle).

### Per-voice controls (right of grid)

- Patch selector (dropdown)
- Volume knob
- Density knob (overrides global for this voice)
- Style knob (Stepwise ↔ Leaping melodic style blend)
- Role indicator (Bass / Pad / Melody / Texture — clickable to change)

### Global controls (below grid)

- BPM slider
- Key (root note buttons: C C# D … B)
- Scale (Major / Minor / Dorian / …)
- Mood XY pad or blend knobs (one per named mood)
- Master density
- Shimmer / Crystal sends

---

## 11. Implementation Plan

### Phase 8.3 — Core Markov engine (`ambient-engine`)

| Task | Description |
|---|---|
| 8.3.1 `HarmonicChain` | 7-state chain + normal/phrase-boundary matrices + current chord output |
| 8.3.2 `RhythmicChain` | 5-state chain + density control + Calm/Tense base matrices |
| 8.3.3 `MelodicChain` | 7-state degree chain + role bias + pitch resolution |
| 8.3.4 `VoiceRole` enum | Bass/Pad/Melody/Texture + register + bias vector |
| 8.3.5 `PhraseCounter` | Bar counter + phrase boundary event |
| 8.3.6 `MoodSet` | Named mood = 3 matrices; blend function |
| 8.3.7 `MarkovVoice` | Combines rhythmic + melodic chain for one voice |
| 8.3.8 `MarkovEngine` | N voices + global harmonic chain + phrase counter |

### Phase 8.4 — Matrix training (`ambient-engine` or standalone tool)

| Task | Description |
|---|---|
| 8.4.1 MIDI parser | Read `.mid` format 0/1, extract note events |
| 8.4.2 Chord analyzer | Infer harmonic function from simultaneous notes + key |
| 8.4.3 Rhythmic classifier | Map note events on grid → rhythmic states |
| 8.4.4 `TransitionCounter` | Build count matrices from event sequences |
| 8.4.5 Laplace smoothing | Prevent zero-probability transitions |
| 8.4.6 JSON export | Serialize trained mood to file |

### Phase 8.5 — Bevy integration (`synth-bevy`)

| Task | Description |
|---|---|
| 8.5.1 `SynthEvent::SetRoot` | Update root note atomically |
| 8.5.2 `SynthEvent::SetMoodBlend` | Update mood blend vector |
| 8.5.3 `SynthEvent::SetDensity` | Update per-voice or global density |
| 8.5.4 `SynthEvent::SetPhraseLength` | Update phrase boundary config |
| 8.5.5 `MarkovState` Resource | Exposes current harmonic state, phrase position to Bevy systems |

### Phase 8.6 — Launchpad UI (`ambient-box`)

| Task | Description |
|---|---|
| 8.6.1 Grid widget | N×M cell grid, real-time update from chain output |
| 8.6.2 Per-voice controls | Role, patch, volume, density, style |
| 8.6.3 Global controls | Key, scale, mood blend, BPM |
| 8.6.4 Mood editor | Load/save mood JSON, blend sliders |
| 8.6.5 Training UI | Load MIDI, run training, preview matrix, export mood |

---

## 12. Design Decisions Log

| Decision | Rationale |
|---|---|
| Tonality-agnostic matrices | Same matrices work in any key; modulation is input, not matrix property |
| Strategy 1+2 for coherence | Global harmonic chain + voice roles; simplest approach that guarantees no clashes |
| Strategy 3 (constraint solver) deferred | Adds complexity without clear audible benefit at this stage |
| Phrase boundary as event, not auto-modulate | Host (game/performer) decides whether to act on phrase boundary; engine doesn't auto-modulate |
| Laplace smoothing for training | Prevents hard probability blocks from short training sets |
| `Double` self-loop suppression | Fills must be transient; a loop of doubles sounds mechanical |
| Degree 4 = 0 in Bass role | Tritone against degree 7 is the most common harmonic clash; safest to exclude from bass |
| Mood blend by matrix interpolation | Linear interpolation between matrix pairs is cheap and perceptually smooth |
| Launchpad as read-only live display | Real-time feedback without pre-programming; future: optional step override |
