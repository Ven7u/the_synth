# Contributing to Forma

Thank you for your interest in contributing. This document covers how to report bugs, propose features, and submit code.

---

## Before you start

Forma is a personal project in early development. The architecture is still evolving, so please **open an issue before starting significant work** — this avoids duplicate effort and ensures the direction fits the project.

For small fixes (typos, documentation, obvious bugs) feel free to open a PR directly.

---

## Reporting bugs

Use the **Bug report** issue template. The most helpful reports include:

- macOS version and audio device
- Steps to reproduce reliably
- What you expected vs. what happened
- If it's audio-related: MIDI device, sample rate, buffer size (visible in the MIDI & Latency tab)

---

## Proposing features

Use the **Feature request** issue template. Describe the musical problem you're trying to solve, not just the solution — there's often a better way to approach it.

---

## Development setup

```sh
git clone https://github.com/Ven7u/forma-synth.git
cd forma-synth
cargo run -p forma --release
```

The pre-commit hooks run `rustfmt` and `cargo check` automatically. Install them once:

```sh
pip install pre-commit
pre-commit install
```

---

## Code guidelines

**Architecture invariant:** the audio engine (`forma-engine`, `forma-dsp`) must remain headless — no `egui`, no windowing, no `std::io`. The UI (`forma`) must not depend on `forma-engine` directly; all communication goes through `forma-control`'s lock-free atomics.

**Style:**
- Run `cargo fmt` before committing (the pre-commit hook does this automatically)
- No clippy warnings — `RUSTFLAGS=-D warnings` is enforced in CI
- Comments only when the *why* is non-obvious — don't explain what the code does
- Prefer editing existing files over creating new ones
- No half-finished implementations — if something isn't wired up, leave a `// TODO:` rather than adding dead code

**DSP code:**
- Audio thread code must never allocate, block, or lock a mutex
- All UI → audio communication uses atomics (`AtomicU32` for f32 bits, `AtomicBool`, etc.)
- New effects go in `forma-dsp`; new parameters get a `ParamId` variant in `forma-control`

**Commits:**
- One logical change per commit
- Present-tense summary line: `Add chorus depth parameter`, not `Added` or `Adding`
- Reference issue numbers where relevant: `Fix filter cutoff at high resonance (#42)`

---

## Pull requests

- Target `main`
- CI must pass (fmt, clippy, build)
- Include a short description of what changed and why
- Screenshots or audio examples are very welcome for UI or sound changes

---

## License

By submitting a contribution you agree that your code will be licensed under the same **MIT OR Apache-2.0** terms as the rest of the project.
