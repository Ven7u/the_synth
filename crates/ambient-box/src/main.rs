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

use ambient_engine::{
    ACTIVE_MACRO_KNOBS, AmbientEngine, AmbientPatch, MacroSetKind, Scene, MACRO_COUNT, TRACK_COUNT,
    VOICE_COUNT, load_scene_json, save_scene_json,
};
use synth_engine::arp::{ArpMode, ArpState, ClockDiv, Scale, ScaleWalker};
use synth_control::{ControlEvent, ControlSender, make_control_channel};
use synth_control::midi::{MidiEngine, MidiEvent};
use synth_common::{RestartBatch, SyncTransport};
use std::sync::atomic::Ordering;

#[derive(Clone)]
struct PatchEntry {
    path: String,
    category: String,
    name: String,
    patch: AmbientPatch,
}

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
    // Last valid stereo sample, used as a smooth fallback if UI briefly owns the lock.
    let mut last_out_l: f32 = 0.0;
    let mut last_out_r: f32 = 0.0;

    // Per-track arpeggiator and scale walker (audio-thread state only)
    let mut arp_states:    [ArpState;    TRACK_COUNT] = std::array::from_fn(|_| ArpState::new());
    let mut walker_states: [ScaleWalker; TRACK_COUNT] = std::array::from_fn(|_| ScaleWalker::new());

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let frames = data.len() / channels;

            let mut eng_guard = engine.try_lock().ok();
            if eng_guard.is_none() {
                // Short retry window: avoids full-buffer fallback when contention is only
                // a few microseconds at the UI/audio boundary.
                for _ in 0..64 {
                    std::hint::spin_loop();
                    if let Ok(g) = engine.try_lock() {
                        eng_guard = Some(g);
                        break;
                    }
                }
            }

            let Some(mut eng) = eng_guard else {
                // If UI briefly holds the engine lock, avoid hard mute jumps (clicks).
                // Hold and gently decay the last valid sample for this callback.
                let mut l = last_out_l;
                let mut r = last_out_r;
                for frame in data.chunks_mut(channels) {
                    let left = T::from_sample(l);
                    let right = T::from_sample(r);
                    for (i, smp) in frame.iter_mut().enumerate() {
                        *smp = if i & 1 == 0 { left } else { right };
                    }
                    l *= 0.9995;
                    r *= 0.9995;
                }
                last_out_l = l;
                last_out_r = r;
                return;
            };

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
                last_out_l = l;
                last_out_r = r;
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
    arp_enabled_ui: [bool; TRACK_COUNT],
    arp_hold_ui:    [bool; TRACK_COUNT],
    walker_enabled_ui: [bool; TRACK_COUNT],
    transport_sync: SyncTransport<TRACK_COUNT>,

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

    // Macro + scene UI state (Phase 6)
    macro_ui_values: [f32; MACRO_COUNT],
    macro_set_ui: MacroSetKind,
    scene_name: String,
    scene_bpm: u32,
    scene_key: u8,
    scene_scale_minor: bool,
    scene_status: String,
    scene_files: Vec<String>,
    scene_selected: usize,

    // UI cache for lock-contented engine fields.
    track_vol_ui: [f32; TRACK_COUNT],
    track_cutoff_ui: [f32; TRACK_COUNT],
    track_resonance_ui: [f32; TRACK_COUNT],
    track_shimmer_send_ui: [f32; TRACK_COUNT],
    track_crystal_send_ui: [f32; TRACK_COUNT],
    master_vol_ui: f32,
    macro_ui_names: [String; MACRO_COUNT],
    patch_library: Vec<PatchEntry>,
    track_patch_choice: [usize; TRACK_COUNT],
    track_patch_last_synced_path: [String; TRACK_COUNT],
}

impl AmbientBoxApp {
    fn collect_patch_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return; };
        for ent in entries.flatten() {
            let p = ent.path();
            if p.is_dir() {
                Self::collect_patch_files(&p, out);
            } else if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
                out.push(p);
            }
        }
    }

    fn load_patch_library() -> Vec<PatchEntry> {
        let mut files = Vec::new();
        Self::collect_patch_files(std::path::Path::new("assets/patches"), &mut files);
        let mut out = Vec::new();
        for p in files {
            if let Ok(patch) = AmbientPatch::from_file(&p) {
                out.push(PatchEntry {
                    path: p.to_string_lossy().to_string(),
                    category: patch.category.clone(),
                    name: patch.name.clone(),
                    patch,
                });
            }
        }
        out.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }

    fn list_scene_files() -> Vec<String> {
        let mut out = Vec::new();
        let dir = std::path::Path::new("scenes");
        let Ok(entries) = std::fs::read_dir(dir) else { return out; };
        for ent in entries.flatten() {
            let p = ent.path();
            if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
                if let Some(s) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(s.to_string());
                }
            }
        }
        out.sort();
        out
    }

    fn new(
        engine: Arc<std::sync::Mutex<AmbientEngine>>,
        control: ControlSender,
        stream: Stream,
    ) -> Self {
        let mut midi = MidiEngine::new();
        midi.list_ports();
        let patch_library = Self::load_patch_library();
        let scene_files = Self::list_scene_files();
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
            arp_enabled_ui: [false; TRACK_COUNT],
            arp_hold_ui: [false; TRACK_COUNT],
            walker_enabled_ui: [false; TRACK_COUNT],
            transport_sync: SyncTransport::new(120),
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
            macro_ui_values: [0.0; MACRO_COUNT],
            macro_set_ui: MacroSetKind::AmbientCore,
            scene_name: "ambient_01".to_string(),
            scene_bpm: 120,
            scene_key: 0,
            scene_scale_minor: false,
            scene_status: String::new(),
            scene_files,
            scene_selected: 0,
            track_vol_ui: [1.0; TRACK_COUNT],
            track_cutoff_ui: [3000.0; TRACK_COUNT],
            track_resonance_ui: [0.3; TRACK_COUNT],
            track_shimmer_send_ui: [0.0; TRACK_COUNT],
            track_crystal_send_ui: [0.0; TRACK_COUNT],
            master_vol_ui: 0.7,
            macro_ui_names: std::array::from_fn(|i| format!("Macro {}", i + 1)),
            patch_library,
            track_patch_choice: [0; TRACK_COUNT],
            track_patch_last_synced_path: std::array::from_fn(|_| String::new()),
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

    fn dispatch_restarts(&self, batch: RestartBatch<TRACK_COUNT>) {
        for ti in 0..TRACK_COUNT {
            if batch.arp[ti] {
                let _ = self.control.try_send(ControlEvent::ArpRestart { track: ti as u8 });
            }
            if batch.walker[ti] {
                let _ = self.control.try_send(ControlEvent::WalkerRestart { track: ti as u8 });
            }
        }
    }

    fn sync_transport_now(&mut self) {
        let batch = self.transport_sync.sync_now();
        self.dispatch_restarts(batch);
    }

    fn schedule_or_restart_arp(&mut self, track: usize) {
        if self.transport_sync.schedule_or_restart_arp(track) {
            let _ = self.control.try_send(ControlEvent::ArpRestart { track: track as u8 });
        }
    }

    fn schedule_or_restart_walker(&mut self, track: usize) {
        if self.transport_sync.schedule_or_restart_walker(track) {
            let _ = self.control.try_send(ControlEvent::WalkerRestart { track: track as u8 });
        }
    }

    fn tick_transport_sync(&mut self) {
        let batch = self.transport_sync.tick();
        self.dispatch_restarts(batch);
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
        self.tick_transport_sync();

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

            if let Ok(eng) = self.engine.try_lock() {
                let ti = self.active_track;
                let track = &eng.tracks[ti];
                self.track_vol_ui[ti] = track.track_vol.value();
                self.track_cutoff_ui[ti] = track.cutoff.value();
                self.track_resonance_ui[ti] = track.resonance.value();
                self.track_shimmer_send_ui[ti] = track.shimmer_send.value();
                self.track_crystal_send_ui[ti] = track.crystal_send.value();
                self.master_vol_ui = eng.master_vol.value();
                for i in 0..MACRO_COUNT {
                    self.macro_ui_values[i] = eng.macro_value(i);
                    self.macro_ui_names[i] = eng.macro_names[i].clone();
                }
                self.macro_set_ui = eng.macro_set_kind();
                let arp_cfg = &eng.arp_configs[ti];
                self.arp_enabled_ui[ti] = arp_cfg.enabled.load(Ordering::Relaxed);
                self.arp_hold_ui[ti] = arp_cfg.hold.load(Ordering::Relaxed);
                self.arp_bpm[ti] = arp_cfg.bpm.value();
                self.arp_division[ti] = arp_cfg.division.load(Ordering::Relaxed);
                self.arp_mode[ti] = arp_cfg.mode.load(Ordering::Relaxed);
                self.arp_oct[ti] = arp_cfg.octave_range.load(Ordering::Relaxed);
                self.arp_gate[ti] = arp_cfg.gate.value();

                let walker_cfg = &eng.walker_configs[ti];
                self.walker_enabled_ui[ti] = walker_cfg.enabled.load(Ordering::Relaxed);
                self.walker_bpm[ti] = walker_cfg.bpm.value();
                self.walker_div[ti] = walker_cfg.division.load(Ordering::Relaxed);
                self.walker_scale[ti] = walker_cfg.scale.load(Ordering::Relaxed);
                self.walker_root[ti] = walker_cfg.root.load(Ordering::Relaxed);
                self.walker_oct[ti] = walker_cfg.octave_range.load(Ordering::Relaxed);
                self.walker_gate[ti] = walker_cfg.gate.value();

                let engine_patch_path = &eng.track_patch_paths[ti];
                if self.track_patch_last_synced_path[ti] != *engine_patch_path {
                    if let Some(idx) = self.patch_library.iter().position(|p| p.path == *engine_patch_path) {
                        self.track_patch_choice[ti] = idx;
                    }
                    self.track_patch_last_synced_path[ti] = engine_patch_path.clone();
                }
            }

            let mut request_patch_load: Option<(usize, usize)> = None;
            let mut request_scene_save = false;
            let mut request_scene_load_path: Option<String> = None;
            let mut trigger_arp_restart: Option<usize> = None;
            let mut trigger_walker_restart: Option<usize> = None;
            let mut request_scene_load: Option<(String, Scene)> = None;
            let mut pending_scene_save: Option<(String, Scene, String)> = None;

            // --- Per-track patch slot (Phase 6.7) ---
            let ti = self.active_track;
            ui.horizontal(|ui| {
                ui.label("Patch Slot:");
                if self.patch_library.is_empty() {
                    ui.label(egui::RichText::new("No patches found in assets/patches").small().weak());
                } else {
                    if self.track_patch_choice[ti] >= self.patch_library.len() {
                        self.track_patch_choice[ti] = 0;
                    }
                    let selected = &self.patch_library[self.track_patch_choice[ti]];
                    egui::ComboBox::from_id_salt(format!("patch_slot_track_{ti}"))
                        .selected_text(format!("{} / {}", selected.category, selected.name))
                        .show_ui(ui, |ui| {
                            for (i, p) in self.patch_library.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.track_patch_choice[ti],
                                    i,
                                    format!("{} / {}", p.category, p.name),
                                );
                            }
                        });
                    if ui.button("Load To Track").clicked() {
                        request_patch_load = Some((ti, self.track_patch_choice[ti]));
                    }
                }
            });

            // --- Active track basic controls ---
            ui.horizontal(|ui| {
                let _ = synth_ui::knob(ui, "Volume", &mut self.track_vol_ui[ti], 0.0, 1.0);
                let _ = synth_ui::knob(ui, "Cutoff", &mut self.track_cutoff_ui[ti], 80.0, 18000.0);
                let _ = synth_ui::knob(ui, "Resonance", &mut self.track_resonance_ui[ti], 0.1, 10.0);
                let _ = synth_ui::knob(ui, "Shimmer", &mut self.track_shimmer_send_ui[ti], 0.0, 1.0);
                let _ = synth_ui::knob(ui, "Crystal", &mut self.track_crystal_send_ui[ti], 0.0, 1.0);
            });

            ui.separator();

            // Global controls
            ui.horizontal(|ui| {
                let _ = synth_ui::knob(ui, "Master Vol", &mut self.master_vol_ui, 0.0, 1.0);
            });

            ui.horizontal(|ui| {
                let btn = if self.transport_sync.playing { "Stop" } else { "Play" };
                if ui.button(btn).on_hover_text("Start or stop the global sync transport.").clicked() {
                    let due = self.transport_sync.set_playing(!self.transport_sync.playing);
                    self.dispatch_restarts(due);
                }
                ui.label("BPM:");
                ui.add(egui::Slider::new(&mut self.transport_sync.bpm, 40..=300));
                if ui.checkbox(&mut self.transport_sync.clock_sync_enabled, "Clock Sync")
                    .on_hover_text("Lock all track ARP/WALKER BPM to global transport BPM and align phase.")
                    .changed()
                {
                    self.transport_sync.set_clock_sync(self.transport_sync.clock_sync_enabled);
                    if self.transport_sync.clock_sync_enabled {
                        self.sync_transport_now();
                    }
                }
                ui.add_enabled_ui(self.transport_sync.clock_sync_enabled, |ui| {
                    ui.checkbox(&mut self.transport_sync.bar_quantize_start, "Bar Quantize")
                        .on_hover_text("When enabled, ARP/WALKER restart on next bar instead of immediately.");
                });
                if ui.button("SYNC NOW")
                    .on_hover_text("Restart all ARP/WALKER engines together.")
                    .clicked()
                {
                    self.sync_transport_now();
                }
            });

            ui.separator();

            // --- Macro panel (Phase 6) ---
            ui.label(egui::RichText::new("MACROS").strong());
            ui.horizontal(|ui| {
                ui.label("Set:");
                egui::ComboBox::from_id_salt("macro_set_selector")
                    .selected_text(self.macro_set_ui.label())
                    .show_ui(ui, |ui| {
                        for set in AmbientEngine::macro_set_catalog() {
                            ui.selectable_value(&mut self.macro_set_ui, *set, set.label());
                        }
                    });
            });
            ui.horizontal(|ui| {
                for i in 0..ACTIVE_MACRO_KNOBS {
                    let label = self.macro_ui_names[i].clone();
                    let _ = synth_ui::knob(ui, &label, &mut self.macro_ui_values[i], 0.0, 1.0);
                }
            });

            ui.separator();

            // --- Scene panel (Phase 6) ---
            ui.label(egui::RichText::new("SCENE").strong());
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut self.scene_name);
                ui.label("BPM:");
                ui.add(egui::Slider::new(&mut self.scene_bpm, 40..=300));
                ui.label("Key:");
                let note_names = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];
                egui::ComboBox::from_id_salt("scene_key")
                    .selected_text(note_names[(self.scene_key % 12) as usize])
                    .show_ui(ui, |ui| {
                        for (i, name) in note_names.iter().enumerate() {
                            ui.selectable_value(&mut self.scene_key, i as u8, *name);
                        }
                    });
                ui.checkbox(&mut self.scene_scale_minor, "Minor");
            });
            ui.horizontal(|ui| {
                let scene_path = format!("scenes/{}.json", self.scene_name.trim());
                if ui.button("Save Scene").clicked() {
                    request_scene_save = true;
                }
                if ui.button("Load Scene").clicked() {
                    request_scene_load_path = Some(scene_path.clone());
                }
                if ui.button("Refresh").clicked() {
                    self.scene_files = Self::list_scene_files();
                    if self.scene_selected >= self.scene_files.len() {
                        self.scene_selected = 0;
                    }
                }
                if !self.scene_files.is_empty() {
                    egui::ComboBox::from_id_salt("scene_file_list")
                        .selected_text(self.scene_files[self.scene_selected].clone())
                        .show_ui(ui, |ui| {
                            for (i, name) in self.scene_files.iter().enumerate() {
                                ui.selectable_value(&mut self.scene_selected, i, name.clone());
                            }
                        });
                    if ui.button("Load Selected").clicked() {
                        let name = self.scene_files[self.scene_selected].clone();
                        let selected_path = format!("scenes/{name}.json");
                        request_scene_load_path = Some(selected_path);
                    }
                }
                if !self.scene_status.is_empty() {
                    ui.label(egui::RichText::new(&self.scene_status).small());
                }
            });

            if let Some(path) = request_scene_load_path.take() {
                match load_scene_json(&path) {
                    Ok(scene) => request_scene_load = Some((path, scene)),
                    Err(e) => self.scene_status = format!("Load failed: {e}"),
                }
            }

            ui.separator();

            // --- Shimmer reverb global bus ---
            ui.horizontal(|ui| {
                let col = egui::Color32::from_rgb(120, 200, 255);
                let lbl = egui::RichText::new("SHIMMER").strong()
                    .color(if self.shimmer_on { col } else { egui::Color32::GRAY });
                if ui.button(lbl).on_hover_text("Global shimmer reverb bus.").clicked() {
                    self.shimmer_on = !self.shimmer_on;
                }
                if synth_ui::knob(ui, "Mix", &mut self.shimmer_mix, 0.0, 1.0) {
                }
                if synth_ui::knob(ui, "Shim", &mut self.shimmer_amt, 0.0, 1.0) {
                }
                if synth_ui::knob(ui, "Size", &mut self.shimmer_size, 0.0, 1.0) {
                }
                if synth_ui::knob(ui, "Damp", &mut self.shimmer_damp, 0.0, 1.0) {
                }
                ui.label("Pitch:");
                for (i, lbl) in ["0", "+12", "+24"].iter().enumerate() {
                    if ui.selectable_label(self.shimmer_pitch == i as u8, *lbl).clicked() {
                        self.shimmer_pitch = i as u8;
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
                }
                if synth_ui::knob(ui, "Mix", &mut self.crystal_mix, 0.0, 1.0) {
                }
                if synth_ui::knob(ui, "Grain", &mut self.crystal_grain_ms, 10.0, 400.0) {
                }
                if synth_ui::knob(ui, "Scatter", &mut self.crystal_scatter, 0.0, 1.0) {
                }
                if synth_ui::knob(ui, "Delay", &mut self.crystal_delay_ms, 20.0, 1200.0) {
                }
                if synth_ui::knob(ui, "Feedback", &mut self.crystal_feedback, 0.0, 0.95) {
                }
                ui.label("Pitch:");
                for (i, lbl) in ["0.5x", "1x", "2x", "4x"].iter().enumerate() {
                    if ui.selectable_label(self.crystal_pitch == i as u8, *lbl).clicked() {
                        self.crystal_pitch = i as u8;
                    }
                }
            });

            ui.separator();

            // --- Per-track arp + walker ---
            let ti = self.active_track;
            ui.columns(2, |cols| {
                // Arp column
                let arp_on = self.arp_enabled_ui[ti];
                cols[0].horizontal(|ui| {
                    let lbl = egui::RichText::new("ARP").strong()
                        .color(if arp_on { egui::Color32::from_rgb(0,220,160) } else { egui::Color32::GRAY });
                    if ui.button(lbl).clicked() {
                        self.arp_enabled_ui[ti] = !arp_on;
                        if !self.arp_enabled_ui[ti] {
                            let _ = self.control.try_send(ControlEvent::ChordHold {
                                track: ti as u8,
                                notes: Vec::new(),
                            });
                        }
                    }
                    if ui.button("RST").on_hover_text("Restart arp phase/step.").clicked() {
                        let _ = self.control.try_send(ControlEvent::ArpRestart { track: ti as u8 });
                    }
                    let hold = self.arp_hold_ui[ti];
                    let hl = egui::RichText::new("HOLD")
                        .color(if hold { egui::Color32::from_rgb(255,200,0) } else { egui::Color32::GRAY });
                    if ui.button(hl).clicked() {
                        self.arp_hold_ui[ti] = !hold;
                        if !self.arp_hold_ui[ti] {
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
                        if self.transport_sync.clock_sync_enabled {
                            self.arp_bpm[ti] = self.transport_sync.bpm_f32();
                        }
                        ui.add_enabled_ui(!self.transport_sync.clock_sync_enabled, |ui| {
                            ui.add(egui::Slider::new(&mut self.arp_bpm[ti], 20.0..=300.0));
                        });
                        if self.transport_sync.clock_sync_enabled {
                            ui.label(egui::RichText::new("SYNC").small().weak());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Div:");
                        for (i, &lbl) in ClockDiv::LABELS.iter().enumerate() {
                            if ui.selectable_label(self.arp_division[ti] == i as u8, lbl).clicked() {
                                self.arp_division[ti] = i as u8;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Mode:");
                        for (i, &lbl) in ArpMode::LABELS.iter().enumerate() {
                            if ui.selectable_label(self.arp_mode[ti] == i as u8, lbl).clicked() {
                                self.arp_mode[ti] = i as u8;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Oct:");
                        for oct in 1u8..=4 {
                            if ui.selectable_label(self.arp_oct[ti] == oct, oct.to_string()).clicked() {
                                self.arp_oct[ti] = oct;
                            }
                        }
                        ui.label(" Gate:");
                        ui.add(egui::Slider::new(&mut self.arp_gate[ti], 0.05..=1.0));
                    });
                });

                // Walker column
                let walk_on = self.walker_enabled_ui[ti];
                cols[1].horizontal(|ui| {
                    let lbl = egui::RichText::new("WALKER").strong()
                        .color(if walk_on { egui::Color32::from_rgb(100,180,255) } else { egui::Color32::GRAY });
                    if ui.button(lbl).clicked() {
                        self.walker_enabled_ui[ti] = !walk_on;
                    }
                    if ui.button("RST").on_hover_text("Restart walker phase/index.").clicked() {
                        let _ = self.control.try_send(ControlEvent::WalkerRestart { track: ti as u8 });
                    }
                });
                cols[1].add_enabled_ui(walk_on, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("BPM:");
                        if self.transport_sync.clock_sync_enabled {
                            self.walker_bpm[ti] = self.transport_sync.bpm_f32();
                        }
                        ui.add_enabled_ui(!self.transport_sync.clock_sync_enabled, |ui| {
                            ui.add(egui::Slider::new(&mut self.walker_bpm[ti], 20.0..=300.0));
                        });
                        if self.transport_sync.clock_sync_enabled {
                            ui.label(egui::RichText::new("SYNC").small().weak());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Div:");
                        for (i, &lbl) in ClockDiv::LABELS.iter().enumerate() {
                            if ui.selectable_label(self.walker_div[ti] == i as u8, lbl).clicked() {
                                self.walker_div[ti] = i as u8;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Scale:");
                        for (i, &lbl) in Scale::LABELS.iter().enumerate() {
                            if ui.selectable_label(self.walker_scale[ti] == i as u8, lbl).clicked() {
                                self.walker_scale[ti] = i as u8;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Root:");
                        ui.add(egui::Slider::new(&mut self.walker_root[ti], 36u8..=84));
                        ui.label(" Oct:");
                        for oct in 1u8..=3 {
                            if ui.selectable_label(self.walker_oct[ti] == oct, oct.to_string()).clicked() {
                                self.walker_oct[ti] = oct;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Gate:");
                        ui.add(egui::Slider::new(&mut self.walker_gate[ti], 0.05..=1.0));
                    });
                });
            });
            if let Ok(mut eng) = self.engine.try_lock() {
                let track = &eng.tracks[ti];
                track.track_vol.set(self.track_vol_ui[ti].clamp(0.0, 1.0));
                track.cutoff.set(self.track_cutoff_ui[ti].clamp(80.0, 18000.0));
                track.resonance.set(self.track_resonance_ui[ti].clamp(0.1, 10.0));
                track.shimmer_send.set(self.track_shimmer_send_ui[ti].clamp(0.0, 1.0));
                track.crystal_send.set(self.track_crystal_send_ui[ti].clamp(0.0, 1.0));

                eng.master_vol.set(self.master_vol_ui.clamp(0.0, 1.0));

                if eng.macro_set_kind() != self.macro_set_ui {
                    eng.set_macro_set(self.macro_set_ui);
                }
                for i in 0..ACTIVE_MACRO_KNOBS {
                    eng.set_macro_value(i, self.macro_ui_values[i]);
                }

                let arp_cfg = &eng.arp_configs[ti];
                let prev_arp_enabled = arp_cfg.enabled.load(Ordering::Relaxed);
                arp_cfg.enabled.store(self.arp_enabled_ui[ti], Ordering::Relaxed);
                arp_cfg.hold.store(self.arp_hold_ui[ti], Ordering::Relaxed);
                arp_cfg.bpm.set(self.arp_bpm[ti]);
                arp_cfg.division.store(self.arp_division[ti], Ordering::Relaxed);
                arp_cfg.mode.store(self.arp_mode[ti], Ordering::Relaxed);
                arp_cfg.octave_range.store(self.arp_oct[ti], Ordering::Relaxed);
                arp_cfg.gate.set(self.arp_gate[ti]);
                if !prev_arp_enabled && self.arp_enabled_ui[ti] && self.transport_sync.clock_sync_enabled {
                    trigger_arp_restart = Some(ti);
                }

                let walker_cfg = &eng.walker_configs[ti];
                let prev_walker_enabled = walker_cfg.enabled.load(Ordering::Relaxed);
                walker_cfg.enabled.store(self.walker_enabled_ui[ti], Ordering::Relaxed);
                walker_cfg.bpm.set(self.walker_bpm[ti]);
                walker_cfg.division.store(self.walker_div[ti], Ordering::Relaxed);
                walker_cfg.scale.store(self.walker_scale[ti], Ordering::Relaxed);
                walker_cfg.root.store(self.walker_root[ti], Ordering::Relaxed);
                walker_cfg.octave_range.store(self.walker_oct[ti], Ordering::Relaxed);
                walker_cfg.gate.set(self.walker_gate[ti]);
                if !prev_walker_enabled && self.walker_enabled_ui[ti] && self.transport_sync.clock_sync_enabled {
                    trigger_walker_restart = Some(ti);
                }

                if self.transport_sync.clock_sync_enabled {
                    let bpm = self.transport_sync.bpm_f32();
                    for track_idx in 0..TRACK_COUNT {
                        self.arp_bpm[track_idx] = bpm;
                        self.walker_bpm[track_idx] = bpm;
                        eng.arp_configs[track_idx].bpm.set(bpm);
                        eng.walker_configs[track_idx].bpm.set(bpm);
                    }
                }

                eng.shimmer.shimmer.set(self.shimmer_amt);
                eng.shimmer.size.set(self.shimmer_size);
                eng.shimmer.damp.set(self.shimmer_damp);
                eng.shimmer.pitch.store(self.shimmer_pitch, Ordering::Relaxed);
                eng.shimmer.mix.set(if self.shimmer_on {
                    self.shimmer_mix.clamp(0.0, 1.0)
                } else { 0.0 });

                eng.crystal.grain_ms.set(self.crystal_grain_ms);
                eng.crystal.scatter.set(self.crystal_scatter);
                eng.crystal.delay_ms.set(self.crystal_delay_ms);
                eng.crystal.feedback.set(self.crystal_feedback);
                eng.crystal.pitch.store(self.crystal_pitch, Ordering::Relaxed);
                eng.crystal.mix.set(if self.crystal_on {
                    self.crystal_mix.clamp(0.0, 1.0)
                } else { 0.0 });

                if let Some((track_idx, patch_idx)) = request_patch_load.take() {
                    if patch_idx < self.patch_library.len() {
                        let p = &self.patch_library[patch_idx];
                        eng.apply_patch_to_track(track_idx, p.path.clone(), &p.patch);
                        self.scene_status = format!("Loaded patch '{}' on Track {}", p.name, track_idx + 1);
                    }
                }

                if request_scene_save {
                    let scene_path = format!("scenes/{}.json", self.scene_name.trim());
                    let scale = if self.scene_scale_minor { "minor" } else { "major" };
                    let scene = eng.capture_scene(
                        self.scene_name.clone(),
                        self.scene_bpm,
                        self.scene_key,
                        scale,
                    );
                    pending_scene_save = Some((scene_path, scene, self.scene_name.trim().to_string()));
                }

                if let Some((path, scene)) = request_scene_load.take() {
                    self.scene_name = scene.name.clone();
                    self.scene_bpm = scene.bpm;
                    self.scene_key = scene.key % 12;
                    self.scene_scale_minor = scene.scale.eq_ignore_ascii_case("minor");
                    eng.apply_scene(&scene);
                    self.scene_status = format!("Loaded {path}");
                }
            } else {
                if request_patch_load.is_some() {
                    self.scene_status = "Patch load skipped: engine busy".to_string();
                }
                if request_scene_save || request_scene_load.is_some() {
                    self.scene_status = "Scene action skipped: engine busy".to_string();
                }
            }

            if let Some((scene_path, scene, selected_name)) = pending_scene_save.take() {
                match save_scene_json(&scene_path, &scene) {
                    Ok(()) => {
                        self.scene_status = format!("Saved {scene_path}");
                        self.scene_files = Self::list_scene_files();
                        if let Some(pos) = self.scene_files.iter().position(|s| s == &selected_name) {
                            self.scene_selected = pos;
                        }
                    }
                    Err(e) => self.scene_status = format!("Save failed: {e}"),
                }
            }

            if let Some(track_idx) = trigger_arp_restart {
                self.schedule_or_restart_arp(track_idx);
            }
            if let Some(track_idx) = trigger_walker_restart {
                self.schedule_or_restart_walker(track_idx);
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
