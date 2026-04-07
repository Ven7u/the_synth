//! ambient-box — multi-track ambient/electronic music maker.
//!
//! Thin shell over `ambient-engine`. Opens a cpal stream and a minimal egui window.
//! Track selector routes keyboard/MIDI input to the active track.

#![allow(clippy::precedence)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use eframe::egui;
use fundsp::prelude::midi_hz;
use std::sync::Arc;

use ambient_engine::{AmbientEngine, TRACK_COUNT, VOICE_COUNT};
use synth_control::{ControlEvent, ControlSender, make_control_channel};
use synth_control::midi::{MidiEngine, MidiEvent};

fn main() -> eframe::Result {
    let sr = get_default_sr().unwrap_or(44100.0);
    let engine = Arc::new(std::sync::Mutex::new(AmbientEngine::new(sr)));
    let (tx, rx) = make_control_channel(1024);

    let _stream = build_stream(Arc::clone(&engine), rx, sr).expect("Failed to build audio stream");
    _stream.play().expect("Failed to start audio stream");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 600.0])
            .with_title("Ambient Box"),
        ..Default::default()
    };

    eframe::run_native(
        "Ambient Box",
        options,
        Box::new(move |_cc| Ok(Box::new(AmbientBoxApp::new(engine, tx, _stream)))),
    )
}

fn get_default_sr() -> Option<f64> {
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let config = device.default_output_config().ok()?;
    Some(config.sample_rate().0 as f64)
}

// ---------------------------------------------------------------------------
// cpal stream
// ---------------------------------------------------------------------------

fn build_stream(
    engine: Arc<std::sync::Mutex<AmbientEngine>>,
    rx: synth_control::ControlReceiver,
    sr: f64,
) -> anyhow::Result<Stream> {
    let host = cpal::default_host();
    let device = host.default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No output device"))?;
    let config = device.default_output_config()?;

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => make_stream::<f32>(&device, &config.into(), engine, rx, sr)?,
        cpal::SampleFormat::I16 => make_stream::<i16>(&device, &config.into(), engine, rx, sr)?,
        cpal::SampleFormat::U16 => make_stream::<u16>(&device, &config.into(), engine, rx, sr)?,
        _ => anyhow::bail!("Unsupported sample format"),
    };
    Ok(stream)
}

fn make_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    engine: Arc<std::sync::Mutex<AmbientEngine>>,
    rx: synth_control::ControlReceiver,
    sr: f64,
) -> anyhow::Result<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;

    // Per-track voice allocation state on the audio thread
    let mut voice_notes: [[Option<u8>; VOICE_COUNT]; TRACK_COUNT] =
        [[None; VOICE_COUNT]; TRACK_COUNT];
    let mut steal_idx: [usize; TRACK_COUNT] = [0; TRACK_COUNT];

    // Per-track LFO phase
    let mut lfo_phases: [f32; TRACK_COUNT] = [0.0; TRACK_COUNT];

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let frames = data.len() / channels;

            let Ok(mut eng) = engine.try_lock() else { return; };

            // --- Release cleanup ---
            for ti in 0..TRACK_COUNT {
                for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                    if note.is_some()
                        && eng.tracks[ti].voice_gates[slot].value() < 0.5
                        && eng.tracks[ti].amp_cursors[slot].value() < 0.5
                    {
                        *note = None;
                    }
                }
            }

            // --- Drain control events ---
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    ControlEvent::NoteOn { pitch, velocity: _, track } => {
                        let ti = track as usize % TRACK_COUNT;
                        if voice_notes[ti].iter().enumerate().any(|(s, &n)| {
                            n == Some(pitch) && eng.tracks[ti].voice_gates[s].value() > 0.5
                        }) {
                            continue;
                        }
                        let slot = voice_notes[ti].iter().position(|&n| n == Some(pitch))
                            .or_else(|| voice_notes[ti].iter().position(|n| n.is_none()))
                            .unwrap_or_else(|| {
                                let s = steal_idx[ti] % VOICE_COUNT;
                                steal_idx[ti] += 1;
                                s
                            });
                        voice_notes[ti][slot] = Some(pitch);
                        eng.tracks[ti].voice_freq_targets[slot].set(midi_hz(pitch as f64) as f32);
                        eng.tracks[ti].voice_gates[slot].set(1.0);
                    }
                    ControlEvent::NoteOff { pitch, track } => {
                        let ti = track as usize % TRACK_COUNT;
                        for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                            if *note == Some(pitch) {
                                eng.tracks[ti].voice_gates[slot].set(0.0);
                                break;
                            }
                        }
                    }
                    ControlEvent::SetParam { .. } => {}
                }
            }

            // --- Glide ---
            eng.tick_glide(frames);

            // --- Per-sample output ---
            for frame in data.chunks_mut(channels) {
                // Advance LFO for each track
                for ti in 0..TRACK_COUNT {
                    let lfo_rate = eng.tracks[ti].lfo_rate.value();
                    lfo_phases[ti] = (lfo_phases[ti] + lfo_rate / sr as f32).fract();
                    eng.tick_lfo_sample(ti, lfo_phases[ti]);
                }

                let (l, r) = eng.get_stereo();
                let left  = T::from_sample(l);
                let right = T::from_sample(r);
                for (i, smp) in frame.iter_mut().enumerate() {
                    *smp = if i & 1 == 0 { left } else { right };
                }
            }
        },
        |err| eprintln!("audio error: {err}"),
        None,
    )?;
    Ok(stream)
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct AmbientBoxApp {
    engine:       Arc<std::sync::Mutex<AmbientEngine>>,
    control:      ControlSender,
    _stream:      Stream,
    midi:         MidiEngine,
    active_track: usize,

    // Per-track keyboard state
    held_midi: [std::collections::HashSet<u8>; TRACK_COUNT],
    piano_octave: i32,
}

impl AmbientBoxApp {
    fn new(
        engine: Arc<std::sync::Mutex<AmbientEngine>>,
        control: ControlSender,
        stream: Stream,
    ) -> Self {
        let mut midi = MidiEngine::new();
        midi.list_ports();
        Self {
            engine,
            control,
            _stream: stream,
            midi,
            active_track: 0,
            held_midi: std::array::from_fn(|_| std::collections::HashSet::new()),
            piano_octave: 4,
        }
    }

    fn push_note_on(&self, midi: u8) {
        let _ = self.control.try_send(ControlEvent::NoteOn {
            pitch: midi,
            velocity: 100,
            track: self.active_track as u8,
        });
    }

    fn push_note_off(&self, midi: u8) {
        let _ = self.control.try_send(ControlEvent::NoteOff {
            pitch: midi,
            track: self.active_track as u8,
        });
    }

    fn tick_midi(&mut self) {
        let events = self.midi.drain();
        for ev in events {
            match ev {
                MidiEvent::NoteOn { note, .. } => self.push_note_on(note),
                MidiEvent::NoteOff { note, .. } => self.push_note_off(note),
                _ => {}
            }
        }
    }
}

// Keyboard note mapping (same as the_synth)
const KEY_MAP: &[(egui::Key, i32)] = &[
    (egui::Key::A, 0), (egui::Key::W, 1), (egui::Key::S, 2),
    (egui::Key::E, 3), (egui::Key::D, 4), (egui::Key::F, 5),
    (egui::Key::T, 6), (egui::Key::G, 7), (egui::Key::Y, 8),
    (egui::Key::H, 9), (egui::Key::U, 10), (egui::Key::J, 11),
    (egui::Key::K, 12),
];

impl eframe::App for AmbientBoxApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick_midi();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Ambient Box");
            ui.separator();

            // --- Track selector ---
            ui.horizontal(|ui| {
                ui.label("Track:");
                for i in 0..TRACK_COUNT {
                    let label = format!("Track {}", i + 1);
                    let selected = self.active_track == i;
                    let color = if selected {
                        egui::Color32::from_rgb(0, 200, 140)
                    } else {
                        egui::Color32::GRAY
                    };
                    if ui.button(egui::RichText::new(&label).color(color).strong()).clicked() {
                        // Release all held notes on current track before switching
                        let prev: Vec<u8> = self.held_midi[self.active_track].drain().collect();
                        for m in prev { self.push_note_off(m); }
                        self.active_track = i;
                    }
                }
            });

            ui.separator();

            // --- Active track basic controls ---
            if let Ok(eng) = self.engine.try_lock() {
                let track = &eng.tracks[self.active_track];
                ui.horizontal(|ui| {
                    let mut vol = track.track_vol.value();
                    if synth_ui::knob(ui, "Volume", &mut vol, 0.0, 1.0) {
                        track.track_vol.set(vol);
                    }
                    let mut cutoff = track.cutoff.value();
                    if synth_ui::knob(ui, "Cutoff", &mut cutoff, 80.0, 18000.0) {
                        track.cutoff.set(cutoff);
                    }
                    let mut res = track.resonance.value();
                    if synth_ui::knob(ui, "Resonance", &mut res, 0.1, 10.0) {
                        track.resonance.set(res);
                    }
                    let mut shim = track.shimmer_send.value();
                    if synth_ui::knob(ui, "Shimmer", &mut shim, 0.0, 1.0) {
                        track.shimmer_send.set(shim);
                    }
                    let mut crys = track.crystal_send.value();
                    if synth_ui::knob(ui, "Crystal", &mut crys, 0.0, 1.0) {
                        track.crystal_send.set(crys);
                    }
                });

                ui.separator();

                // Global controls
                ui.horizontal(|ui| {
                    let mut mvol = eng.master_vol.value();
                    if synth_ui::knob(ui, "Master Vol", &mut mvol, 0.0, 1.0) {
                        eng.master_vol.set(mvol);
                    }
                });
            }

            ui.separator();

            // --- Keyboard input ---
            let mut current_held = std::collections::HashSet::<u8>::new();
            ui.input(|inp| {
                for &(key, semitone) in KEY_MAP {
                    if inp.key_down(key) {
                        current_held.insert((self.piano_octave * 12 + semitone) as u8);
                    }
                }
            });
            for &midi in &current_held {
                if !self.held_midi[self.active_track].contains(&midi) {
                    self.push_note_on(midi);
                }
            }
            let released: Vec<u8> = self.held_midi[self.active_track]
                .iter()
                .filter(|&&m| !current_held.contains(&m))
                .copied()
                .collect();
            for midi in released { self.push_note_off(midi); }
            self.held_midi[self.active_track] = current_held;

            ui.label(egui::RichText::new("Keys: A-K = C to C (one octave)").small().weak());
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}
