//! # Example 3 — ADSR Envelope
//!
//! Without an envelope, a tone starts and stops instantly — unnatural and clicky.
//! An ADSR envelope shapes *amplitude over time*:
//!
//! - **Attack** (0.05 s): fade in from 0 → 1 when the note starts
//! - **Decay** (0.1 s): fall from 1 → sustain level
//! - **Sustain** (0.7): hold at this fraction of peak while note is held
//! - **Release** (0.8 s): fade out from sustain → 0 after note ends
//!
//! `lfo(|t| ...)` creates a time-varying signal from a closure. `t` is time in
//! seconds. We use it here to manually compute the ADSR curve.
//!
//! fundsp also has `adsr_live(a, d, s, r)` for live/MIDI use — it takes a
//! *gate signal* as its input (1.0 = note on, 0.0 = note off). The interactive
//! app uses this; here we use `lfo` because it is clearer for offline rendering.
//!
//! Run: cargo run --example 03_envelope
//! Output: output/03_raw.wav (no envelope) and output/03_envelope.wav (with ADSR)

#![allow(clippy::precedence)]

use fundsp::prelude32::*;

fn main() {
    std::fs::create_dir_all("output").ok();
    // --- Without envelope ---
    // Clicks at start and end because amplitude steps instantly to/from 0.
    let wave = Wave::render(44100.0, 3.0, &mut (0.5 * saw_hz(220.0)));
    wave.save_wav32(std::path::Path::new("output/03_raw.wav"))
        .unwrap();
    println!("Saved output/03_raw.wav (notice the click at start/end)");

    // --- With ADSR envelope ---
    let attack = 0.05_f32;
    let decay = 0.10_f32;
    let sustain = 0.70_f32;
    let release = 0.80_f32;
    let note_length = 1.50_f32; // gate open for 1.5 s

    // `lfo` has 0 inputs and 1 output. The closure receives elapsed time in seconds.
    let amp_curve = lfo(move |t: f32| {
        if t < attack {
            // Attack: linear ramp up
            t / attack
        } else if t < attack + decay {
            // Decay: linear ramp from 1.0 down to sustain level
            1.0 - (1.0 - sustain) * (t - attack) / decay
        } else if t < note_length {
            // Sustain: hold at sustain level
            sustain
        } else if t < note_length + release {
            // Release: linear fade to silence
            sustain * (1.0 - (t - note_length) / release)
        } else {
            0.0
        }
    });

    // Multiply the oscillator by the amplitude curve sample-by-sample.
    // `>>` pipes the mono signal into `pan(0.0)` which creates a stereo output.
    let mut enveloped = (saw_hz(220.0) * amp_curve) >> pan(0.0);
    let wave = Wave::render(44100.0, 3.5, &mut enveloped);
    wave.save_wav32(std::path::Path::new("output/03_envelope.wav"))
        .unwrap();
    println!("Saved output/03_envelope.wav (smooth attack and release)");
    println!("\nCompare the two files — the enveloped version sounds much more natural.");
}
