//! # Example 4 — Filters
//!
//! A filter passes some frequencies and attenuates others.
//!
//! - **Lowpass** (LP): passes frequencies *below* the cutoff. Sounds: muffled, warm.
//! - **Highpass** (HP): passes frequencies *above* the cutoff. Sounds: airy, thin.
//! - **Bandpass** (BP): passes a narrow band around the center frequency.
//!
//! `Q` (quality factor) controls steepness and resonance:
//! - Low Q (≈ 0.7): gentle rolloff
//! - High Q (> 5): sharp rolloff + resonant peak at the cutoff frequency
//!
//! `pink()` is the ideal test signal: equal energy per octave.
//!
//! `>>` pipes the noise into the filter (serial connection).
//! `|` stacks two nodes side-by-side into a multi-channel bus.
//!
//! Run: cargo run --example 04_filter
//! Output: several WAV files in output/ — open in Audacity, use Analyze → Plot Spectrum.

#![allow(clippy::precedence)]

use fundsp::prelude32::*;

fn save(node: &mut impl AudioUnit, name: &str) {
    let wave = Wave::render(44100.0, 2.0, node);
    let path = format!("output/{name}");
    wave.save_wav32(std::path::Path::new(&path)).unwrap();
    println!("Saved {path}");
}

fn main() {
    std::fs::create_dir_all("output").ok();
    // Pink noise (unfiltered, for reference) — convert mono to stereo via pan
    save(&mut (pink() * 0.5 >> pan(0.0)), "04_noise_raw.wav");

    // Lowpass at 800 Hz — everything above 800 Hz is attenuated
    save(
        &mut (pink() * 0.5 >> lowpass_hz(800.0, 1.0) >> pan(0.0)),
        "04_lowpass_800.wav",
    );

    // Highpass at 2000 Hz — everything below 2000 Hz is removed
    save(
        &mut (pink() * 0.5 >> highpass_hz(2000.0, 1.0) >> pan(0.0)),
        "04_highpass_2k.wav",
    );

    // Bandpass centred at 1000 Hz, Q=5 — narrow window, whistling quality
    save(
        &mut (pink() * 0.5 >> bandpass_hz(1000.0, 5.0) >> pan(0.0)),
        "04_bandpass_1k.wav",
    );

    // High resonance Q=12 — strong ring at the cutoff frequency
    save(
        &mut (pink() * 0.5 >> lowpass_hz(1200.0, 12.0) >> pan(0.0)),
        "04_resonant.wav",
    );

    // --- Dynamic filter sweep ---
    // `lfo(|t| ...)` generates a time-varying cutoff Hz value (0 inputs, 1 output).
    // `lowpass_q(q)` is the signal-driven variant: takes (audio, cutoff_hz) as two
    // inputs via the `|` stack operator, and outputs filtered audio.
    let sweep = lfo(|t: f32| 200.0 + 7800.0 * (t / 4.0).min(1.0));
    let audio = saw_hz(110.0) * 0.5;
    let mut filter_sweep = (audio | sweep) >> lowpass_q(1.0) >> pan(0.0);
    let wave = Wave::render(44100.0, 4.5, &mut filter_sweep);
    wave.save_wav32(std::path::Path::new("output/04_filter_sweep.wav"))
        .unwrap();
    println!("Saved output/04_filter_sweep.wav — the classic synthesizer 'filter sweep' sound");

    println!("\nTip: open in Audacity and use Analyze → Plot Spectrum to visualize.");
}
