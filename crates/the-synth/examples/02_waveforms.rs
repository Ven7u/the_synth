//! # Example 2 — Waveform Shapes & Timbre
//!
//! A sine wave has a single frequency (one "partial"). Every other periodic
//! waveform is the sum of multiple sines at integer multiples of the fundamental —
//! these are called *harmonics*.
//!
//! - **Sawtooth**: all harmonics (1, 2, 3, 4 …), amplitudes 1/n. Bright, buzzy.
//! - **Square**: only *odd* harmonics (1, 3, 5 …), amplitudes 1/n. Hollow, reedy.
//! - **Triangle**: only odd harmonics, amplitudes 1/n². Softer than square.
//!
//! fundsp's oscillators are "bandlimited": they remove harmonics above the
//! Nyquist frequency to prevent *aliasing* (false low-frequency artefacts).
//!
//! Run: cargo run --example 02_waveforms
//! Output: four WAV files in output/ — open them in Audacity and compare the waveform shapes
//!         and the frequency spectrum (Analyze → Plot Spectrum).

#![allow(clippy::precedence)]

use fundsp::prelude32::*;

fn render(node: &mut impl AudioUnit, name: &str) {
    let wave = Wave::render(44100.0, 2.0, node);
    let path = format!("output/{name}");
    wave.save_wav32(std::path::Path::new(&path))
        .expect("Could not save");
    println!("Saved {path}");
}

fn main() {
    std::fs::create_dir_all("output").ok();
    // All at 220 Hz (A3) — low enough that several harmonics are clearly audible.
    render(&mut (0.5 * sine_hz(220.0)), "02_sine.wav");
    render(&mut (0.5 * saw_hz(220.0)), "02_saw.wav");
    render(&mut (0.5 * square_hz(220.0)), "02_square.wav");
    render(&mut (0.5 * triangle_hz(220.0)), "02_triangle.wav");

    println!("\nListen to each file. The sine is clean; the saw is bright and buzzy;");
    println!("the square has a hollow, video-game quality; the triangle is the softest.");
}
