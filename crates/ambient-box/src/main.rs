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
use synth_engine::arp::{ArpMode, ArpState, ClockDiv, Scale, ScaleWalker};
use synth_control::{ControlEvent, ControlSender, make_control_channel};
use synth_control::midi::{MidiEngine, MidiEvent};
use std::sync::atomic::Ordering;

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

    // Per-track arpeggiator and scale walker (audio-thread state only)
    let mut arp_states:    [ArpState;    TRACK_COUNT] = std::array::from_fn(|_| ArpState::new());
    let mut walker_states: [ScaleWalker; TRACK_COUNT] = std::array::from_fn(|_| ScaleWalker::new());

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
                        if eng.arp_configs[ti].enabled.load(Ordering::Relaxed) {
                            arp_states[ti].note_on(pitch);
                        } else {
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
                    }
                    ControlEvent::NoteOff { pitch, track } => {
                        let ti = track as usize % TRACK_COUNT;
                        if eng.arp_configs[ti].enabled.load(Ordering::Relaxed) {
                            let hold = eng.arp_configs[ti].hold.load(Ordering::Relaxed);
                            arp_states[ti].note_off(pitch, hold);
                        } else {
                            for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                                if *note == Some(pitch) {
                                    eng.tracks[ti].voice_gates[slot].set(0.0);
                                    break;
                                }
                            }
                        }
                    }
                    ControlEvent::SetParam { .. } => {}
                    ControlEvent::ChordHold { track, notes } => {
                        let ti = track as usize % TRACK_COUNT;
                        arp_states[ti].set_chord(&notes);
                    }
                    ControlEvent::ArpRestart { track } => {
                        let ti = track as usize % TRACK_COUNT;
                        if let Some(pitch) = arp_states[ti].restart() {
                            for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                                if *note == Some(pitch) {
                                    eng.tracks[ti].voice_gates[slot].set(0.0);
                                    break;
                                }
                            }
                        }
                    }
                    ControlEvent::WalkerRestart { track } => {
                        let ti = track as usize % TRACK_COUNT;
                        if let Some(pitch) = walker_states[ti].restart() {
                            for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                                if *note == Some(pitch) {
                                    eng.tracks[ti].voice_gates[slot].set(0.0);
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            // --- Arp + walker tick (once per buffer, per track) ---
            for ti in 0..TRACK_COUNT {
                for ev in [
                    arp_states[ti].tick(&eng.arp_configs[ti], frames, sr),
                    walker_states[ti].tick(&eng.walker_configs[ti], frames, sr),
                ] {
                    if let Some(pitch) = ev.note_off {
                        for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                            if *note == Some(pitch) {
                                eng.tracks[ti].voice_gates[slot].set(0.0);
                                break;
                            }
                        }
                    }
                    if let Some(pitch) = ev.note_on {
                        if !voice_notes[ti].iter().enumerate().any(|(s, &n)| {
                            n == Some(pitch) && eng.tracks[ti].voice_gates[s].value() > 0.5
                        }) {
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
                    }
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

    // Per-track arp UI state (mirrors AtomicU8 config in engine)
    arp_bpm:      [f32; TRACK_COUNT],
    arp_mode:     [u8;  TRACK_COUNT],
    arp_division: [u8;  TRACK_COUNT],
    arp_oct:      [u8;  TRACK_COUNT],
    arp_gate:     [f32; TRACK_COUNT],

    // Per-track scale walker UI state
    walker_bpm:   [f32; TRACK_COUNT],
    walker_scale: [u8;  TRACK_COUNT],
    walker_root:  [u8;  TRACK_COUNT],
    walker_oct:   [u8;  TRACK_COUNT],
    walker_div:   [u8;  TRACK_COUNT],
    walker_gate:  [f32; TRACK_COUNT],

    // Global shimmer UI state
    shimmer_on:    bool,
    shimmer_mix:   f32,
    shimmer_amt:   f32,
    shimmer_size:  f32,
    shimmer_damp:  f32,
    shimmer_pitch: u8,

    // Global crystallizer UI state
    crystal_on:       bool,
    crystal_mix:      f32,
    crystal_grain_ms: f32,
    crystal_scatter:  f32,
    crystal_feedback: f32,
    crystal_delay_ms: f32,
    crystal_pitch:    u8,
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
            arp_bpm:      [120.0; TRACK_COUNT],
            arp_mode:     [0;     TRACK_COUNT],
            arp_division: [1;     TRACK_COUNT],
            arp_oct:      [1;     TRACK_COUNT],
            arp_gate:     [0.7;   TRACK_COUNT],
            walker_bpm:   [120.0; TRACK_COUNT],
            walker_scale: [0;     TRACK_COUNT],
            walker_root:  [60;    TRACK_COUNT],
            walker_oct:   [2;     TRACK_COUNT],
            walker_div:   [1;     TRACK_COUNT],
            walker_gate:  [0.6;   TRACK_COUNT],
            shimmer_on:    false,
            shimmer_mix:   0.4,
            shimmer_amt:   0.5,
            shimmer_size:  0.6,
            shimmer_damp:  0.5,
            shimmer_pitch: 1,
            crystal_on:       false,
            crystal_mix:      0.35,
            crystal_grain_ms: 120.0,
            crystal_scatter:  0.25,
            crystal_feedback: 0.35,
            crystal_delay_ms: 260.0,
            crystal_pitch:    2,
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

                ui.separator();

                // --- Shimmer reverb global bus ---
                ui.horizontal(|ui| {
                    let col = egui::Color32::from_rgb(120, 200, 255);
                    let lbl = egui::RichText::new("SHIMMER").strong()
                        .color(if self.shimmer_on { col } else { egui::Color32::GRAY });
                    if ui.button(lbl).on_hover_text("Global shimmer reverb bus.").clicked() {
                        self.shimmer_on = !self.shimmer_on;
                        eng.shimmer.mix.set(if self.shimmer_on { self.shimmer_mix } else { 0.0 });
                    }
                    if synth_ui::knob(ui, "Mix", &mut self.shimmer_mix, 0.0, 1.0) {
                        if self.shimmer_on { eng.shimmer.mix.set(self.shimmer_mix); }
                    }
                    if synth_ui::knob(ui, "Shim", &mut self.shimmer_amt, 0.0, 1.0) {
                        eng.shimmer.shimmer.set(self.shimmer_amt);
                    }
                    if synth_ui::knob(ui, "Size", &mut self.shimmer_size, 0.0, 1.0) {
                        eng.shimmer.size.set(self.shimmer_size);
                    }
                    if synth_ui::knob(ui, "Damp", &mut self.shimmer_damp, 0.0, 1.0) {
                        eng.shimmer.damp.set(self.shimmer_damp);
                    }
                    ui.label("Pitch:");
                    for (i, lbl) in ["0", "+12", "+24"].iter().enumerate() {
                        if ui.selectable_label(self.shimmer_pitch == i as u8, *lbl).clicked() {
                            self.shimmer_pitch = i as u8;
                            eng.shimmer.pitch.store(i as u8, Ordering::Relaxed);
                        }
                    }
                });

                ui.separator();

                // --- Crystal global bus ---
                ui.horizontal(|ui| {
                    let col = egui::Color32::from_rgb(255, 170, 90);
                    let lbl = egui::RichText::new("CRYSTAL").strong()
                        .color(if self.crystal_on { col } else { egui::Color32::GRAY });
                    if ui.button(lbl).on_hover_text("Global crystallizer bus (granular pitch-shift delay).").clicked() {
                        self.crystal_on = !self.crystal_on;
                        eng.crystal.mix.set(if self.crystal_on { self.crystal_mix } else { 0.0 });
                    }
                    if synth_ui::knob(ui, "Mix", &mut self.crystal_mix, 0.0, 1.0) {
                        if self.crystal_on { eng.crystal.mix.set(self.crystal_mix); }
                    }
                    if synth_ui::knob(ui, "Grain", &mut self.crystal_grain_ms, 10.0, 400.0) {
                        eng.crystal.grain_ms.set(self.crystal_grain_ms);
                    }
                    if synth_ui::knob(ui, "Scatter", &mut self.crystal_scatter, 0.0, 1.0) {
                        eng.crystal.scatter.set(self.crystal_scatter);
                    }
                    if synth_ui::knob(ui, "Delay", &mut self.crystal_delay_ms, 20.0, 1200.0) {
                        eng.crystal.delay_ms.set(self.crystal_delay_ms);
                    }
                    if synth_ui::knob(ui, "Feedback", &mut self.crystal_feedback, 0.0, 0.95) {
                        eng.crystal.feedback.set(self.crystal_feedback);
                    }
                    ui.label("Pitch:");
                    for (i, lbl) in ["0.5x", "1x", "2x", "4x"].iter().enumerate() {
                        if ui.selectable_label(self.crystal_pitch == i as u8, *lbl).clicked() {
                            self.crystal_pitch = i as u8;
                            eng.crystal.pitch.store(i as u8, Ordering::Relaxed);
                        }
                    }
                });
            }

            ui.separator();

            // --- Per-track arp + walker ---
            if let Ok(eng) = self.engine.try_lock() {
                let ti = self.active_track;
                let arp_cfg    = &eng.arp_configs[ti];
                let walker_cfg = &eng.walker_configs[ti];

                ui.columns(2, |cols| {
                    // Arp column
                    let arp_on = arp_cfg.enabled.load(Ordering::Relaxed);
                    cols[0].horizontal(|ui| {
                        let lbl = egui::RichText::new("ARP").strong()
                            .color(if arp_on { egui::Color32::from_rgb(0,220,160) } else { egui::Color32::GRAY });
                        if ui.button(lbl).clicked() {
                            let new_on = !arp_on;
                            arp_cfg.enabled.store(new_on, Ordering::Relaxed);
                            if !new_on {
                                let _ = self.control.try_send(ControlEvent::ChordHold {
                                    track: ti as u8,
                                    notes: Vec::new(),
                                });
                            }
                        }
                        if ui.button("RST").on_hover_text("Restart arp phase/step.").clicked() {
                            let _ = self.control.try_send(ControlEvent::ArpRestart { track: ti as u8 });
                        }
                        let hold = arp_cfg.hold.load(Ordering::Relaxed);
                        let hl = egui::RichText::new("HOLD")
                            .color(if hold { egui::Color32::from_rgb(255,200,0) } else { egui::Color32::GRAY });
                        if ui.button(hl).clicked() {
                            let new_hold = !hold;
                            arp_cfg.hold.store(new_hold, Ordering::Relaxed);
                            if !new_hold {
                                let _ = self.control.try_send(ControlEvent::ChordHold {
                                    track: ti as u8,
                                    notes: Vec::new(),
                                });
                            }
                        }
                    });
                    cols[0].add_enabled_ui(arp_on, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("BPM:");
                            if ui.add(egui::Slider::new(&mut self.arp_bpm[ti], 20.0..=300.0)).changed() {
                                arp_cfg.bpm.set(self.arp_bpm[ti]);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Div:");
                            for (i, &lbl) in ClockDiv::LABELS.iter().enumerate() {
                                if ui.selectable_label(self.arp_division[ti] == i as u8, lbl).clicked() {
                                    self.arp_division[ti] = i as u8;
                                    arp_cfg.division.store(i as u8, Ordering::Relaxed);
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Mode:");
                            for (i, &lbl) in ArpMode::LABELS.iter().enumerate() {
                                if ui.selectable_label(self.arp_mode[ti] == i as u8, lbl).clicked() {
                                    self.arp_mode[ti] = i as u8;
                                    arp_cfg.mode.store(i as u8, Ordering::Relaxed);
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Oct:");
                            for oct in 1u8..=4 {
                                if ui.selectable_label(self.arp_oct[ti] == oct, oct.to_string()).clicked() {
                                    self.arp_oct[ti] = oct;
                                    arp_cfg.octave_range.store(oct, Ordering::Relaxed);
                                }
                            }
                            ui.label(" Gate:");
                            if ui.add(egui::Slider::new(&mut self.arp_gate[ti], 0.05..=1.0)).changed() {
                                arp_cfg.gate.set(self.arp_gate[ti]);
                            }
                        });
                    });

                    // Walker column
                    let walk_on = walker_cfg.enabled.load(Ordering::Relaxed);
                    cols[1].horizontal(|ui| {
                        let lbl = egui::RichText::new("WALKER").strong()
                            .color(if walk_on { egui::Color32::from_rgb(100,180,255) } else { egui::Color32::GRAY });
                        if ui.button(lbl).clicked() {
                            walker_cfg.enabled.store(!walk_on, Ordering::Relaxed);
                        }
                        if ui.button("RST").on_hover_text("Restart walker phase/index.").clicked() {
                            let _ = self.control.try_send(ControlEvent::WalkerRestart { track: ti as u8 });
                        }
                    });
                    cols[1].add_enabled_ui(walk_on, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("BPM:");
                            if ui.add(egui::Slider::new(&mut self.walker_bpm[ti], 20.0..=300.0)).changed() {
                                walker_cfg.bpm.set(self.walker_bpm[ti]);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Div:");
                            for (i, &lbl) in ClockDiv::LABELS.iter().enumerate() {
                                if ui.selectable_label(self.walker_div[ti] == i as u8, lbl).clicked() {
                                    self.walker_div[ti] = i as u8;
                                    walker_cfg.division.store(i as u8, Ordering::Relaxed);
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Scale:");
                            for (i, &lbl) in Scale::LABELS.iter().enumerate() {
                                if ui.selectable_label(self.walker_scale[ti] == i as u8, lbl).clicked() {
                                    self.walker_scale[ti] = i as u8;
                                    walker_cfg.scale.store(i as u8, Ordering::Relaxed);
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Root:");
                            if ui.add(egui::Slider::new(&mut self.walker_root[ti], 36u8..=84)).changed() {
                                walker_cfg.root.store(self.walker_root[ti], Ordering::Relaxed);
                            }
                            ui.label(" Oct:");
                            for oct in 1u8..=3 {
                                if ui.selectable_label(self.walker_oct[ti] == oct, oct.to_string()).clicked() {
                                    self.walker_oct[ti] = oct;
                                    walker_cfg.octave_range.store(oct, Ordering::Relaxed);
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Gate:");
                            if ui.add(egui::Slider::new(&mut self.walker_gate[ti], 0.05..=1.0)).changed() {
                                walker_cfg.gate.set(self.walker_gate[ti]);
                            }
                        });
                    });
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
