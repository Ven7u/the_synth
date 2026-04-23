//! # Example 1 — Sine Wave
//!
//! A WAV file is a sequence of floating-point numbers called *samples*.
//! Each sample represents the displacement of a loudspeaker cone at one instant.
//!
//! Key concepts:
//! - **Sample rate** (44100 Hz): how many samples play back per second.
//!   Must be at least 2× the highest frequency we want to represent
//!   (Nyquist theorem). Human hearing tops out ~20 kHz, so 44100 Hz covers it.
//! - **Frequency** (440 Hz = A4): how many complete oscillation cycles per second.
//! - **Amplitude** (0.5): peak displacement. 1.0 = full scale, 0.0 = silence.
//!   We use 0.5 to leave headroom.
//!
//! Run: cargo run --example 01_sine_wave
//! Output: output/01_sine_wave.wav (open in Audacity or any audio player)

#![allow(clippy::precedence)]

use fundsp::prelude32::*;

fn main() {
    let sample_rate = 44100.0_f64;
    let duration = 3.0_f64; // seconds

    // `sine_hz(f)` creates a sine oscillator at a fixed frequency f.
    // It has 0 inputs and 1 output.
    // `0.5 *` scales the amplitude — in fundsp, `*` on an AudioNode
    // multiplies every output sample by that constant.
    let mut signal = 0.5 * sine_hz(440.0);

    // `Wave::render` runs the graph offline and collects all samples into a buffer.
    // This is the simplest way to hear fundsp without needing a live audio device.
    let wave = Wave::render(sample_rate, duration, &mut signal);

    std::fs::create_dir_all("output").ok();
    let path = std::path::Path::new("output/01_sine_wave.wav");
    wave.save_wav32(path).expect("Could not save WAV");
    println!(
        "Saved {path:?}  ({} samples, {} Hz)",
        wave.len(),
        wave.sample_rate()
    );
    println!("Open it in Audacity — it should look like a smooth S-curve.");
}
