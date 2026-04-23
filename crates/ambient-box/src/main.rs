//! ambient-box — multi-track ambient/electronic music maker.
//!
//! Thin shell over `ambient-engine`. Opens a cpal stream and a minimal egui window.
//! Track selector routes keyboard/MIDI input to the active track.

#![allow(clippy::precedence)]

mod midi_recorder;
mod recorder;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use eframe::egui;
use fundsp::prelude::midi_hz;
use std::sync::Arc;

use ambient_engine::{
    load_scene_json, save_scene_json, AmbientEngine, AmbientPatch, BeatClock, BeatClockShared,
    EuclideanGen, GenerativeMode, HarmonicFunction, MacroSetKind, MarkovEngine, ProbTableGen,
    RhythmicState, Scale as MarkovScale, Scene, VoiceRole, ACTIVE_MACRO_KNOBS, HARMONIC_STATES,
    LAUNCHPAD_COLS, MACRO_COUNT, MELODIC_STATES, MOOD_CALM, MOOD_COSMIC, MOOD_DARK, MOOD_EUPHORIC,
    MOOD_GRAVITY, MOOD_TENSE, N_MOODS, RHYTHMIC_STATES, TRACK_COUNT, VOICE_COUNT,
};
use std::sync::atomic::Ordering;
use synth_common::{ClockDivision, RestartBatch, SyncTransport};
use synth_control::midi::{MidiEngine, MidiEvent};
use synth_control::{make_control_channel, ControlEvent, ControlSender};
use synth_engine::arp::{ArpMode, ArpState, ClockDiv, Scale, ScaleWalker};

#[derive(Clone)]
struct PatchEntry {
    path: String,
    category: String,
    name: String,
    patch: AmbientPatch,
}

#[derive(Clone)]
struct PendingSceneSave {
    path: String,
    name: String,
    bpm: u32,
    key: u8,
    scale_minor: bool,
}

fn main() -> eframe::Result {
    let sr = get_default_sr().unwrap_or(44100.0);
    let engine = Arc::new(std::sync::Mutex::new(AmbientEngine::new(sr)));
    let (tx, rx) = make_control_channel(1024);

    let recorder_sink: RecorderSink = Arc::new(std::sync::Mutex::new(None));
    let midi_sink_shared: MidiSinkShared = Arc::new(std::sync::Mutex::new(None));
    let (beat_clock_shared, _stream) = build_stream(
        Arc::clone(&engine),
        rx,
        sr,
        Arc::clone(&recorder_sink),
        Arc::clone(&midi_sink_shared),
    )
    .expect("Failed to build audio stream");
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
        Box::new(move |cc| {
            let last_scene = cc.storage.and_then(|s| s.get_string("last_scene"));
            Ok(Box::new(AmbientBoxApp::new(
                engine,
                tx,
                _stream,
                beat_clock_shared,
                last_scene,
                recorder_sink,
                midi_sink_shared,
                sr as u32,
            )))
        }),
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

type RecorderSink = Arc<std::sync::Mutex<Option<recorder::Recorder>>>;
/// Shared MIDI sink: set to Some(sink) when MIDI recording is active.
type MidiSinkShared = Arc<std::sync::Mutex<Option<midi_recorder::MidiSink>>>;

fn build_stream(
    engine: Arc<std::sync::Mutex<AmbientEngine>>,
    rx: synth_control::ControlReceiver,
    sr: f64,
    recorder_sink: RecorderSink,
    midi_sink: MidiSinkShared,
) -> anyhow::Result<(BeatClockShared, Stream)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No output device"))?;
    let config = device.default_output_config()?;

    let (shared, stream) = match config.sample_format() {
        cpal::SampleFormat::F32 => make_stream::<f32>(
            &device,
            &config.into(),
            engine,
            rx,
            sr,
            recorder_sink,
            midi_sink,
        )?,
        cpal::SampleFormat::I16 => make_stream::<i16>(
            &device,
            &config.into(),
            engine,
            rx,
            sr,
            recorder_sink,
            midi_sink,
        )?,
        cpal::SampleFormat::U16 => make_stream::<u16>(
            &device,
            &config.into(),
            engine,
            rx,
            sr,
            recorder_sink,
            midi_sink,
        )?,
        _ => anyhow::bail!("Unsupported sample format"),
    };
    Ok((shared, stream))
}

fn fill_buffer_with_silence<T>(
    data: &mut [T],
    channels: usize,
    last_out_l: &mut f32,
    last_out_r: &mut f32,
) where
    T: SizedSample + FromSample<f32>,
{
    let mut l = *last_out_l;
    let mut r = *last_out_r;
    for frame in data.chunks_mut(channels) {
        let left = T::from_sample(l);
        let right = T::from_sample(r);
        for (i, smp) in frame.iter_mut().enumerate() {
            *smp = if i & 1 == 0 { left } else { right };
        }
        l *= 0.9995;
        r *= 0.9995;
    }
    *last_out_l = l;
    *last_out_r = r;
}

fn make_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    engine: Arc<std::sync::Mutex<AmbientEngine>>,
    rx: synth_control::ControlReceiver,
    sr: f64,
    recorder_sink: RecorderSink,
    midi_sink: MidiSinkShared,
) -> anyhow::Result<(BeatClockShared, Stream)>
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
    let mut arp_states: [ArpState; TRACK_COUNT] = std::array::from_fn(|_| ArpState::new());
    let mut walker_states: [ScaleWalker; TRACK_COUNT] = std::array::from_fn(|_| ScaleWalker::new());

    // Per-track generative pattern generators (Phase 8.2)
    let mut euclidean_states: [EuclideanGen; TRACK_COUNT] =
        std::array::from_fn(|_| EuclideanGen::new());
    let mut prob_table_states: [ProbTableGen; TRACK_COUNT] =
        std::array::from_fn(|i| ProbTableGen::new(0xDEAD_BEEF ^ i as u64));
    // Global Markov engine (Phase 8.3) — audio-thread state only.
    let mut markov_engine = MarkovEngine::new(TRACK_COUNT, 0xC0DE_CAFE_BEEF_1234);
    // Clock division counter: Markov steps only fire every N subdivisions.
    let mut markov_subdiv_counter: u8 = 0;
    let beat_clock_shared = BeatClockShared::new(120.0);
    let beat_clock_shared_clone = beat_clock_shared.clone();
    let mut beat_clock = BeatClock::default();
    let mut was_playing = false;

    // Running sample counter for MIDI timestamp.
    let mut sample_pos: u64 = 0;

    // Debug counters (audio-thread local, printed to stderr periodically).
    let mut dbg_callback_count: u64 = 0;
    let mut dbg_steal_count: [u32; TRACK_COUNT] = [0; TRACK_COUNT];
    let mut dbg_miss_noteoff: [u32; TRACK_COUNT] = [0; TRACK_COUNT];
    let mut dbg_nan_count: u32 = 0;

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
                fill_buffer_with_silence(data, channels, &mut last_out_l, &mut last_out_r);
                return;
            };

            // --- Release cleanup ---
            // Free a slot as soon as its gate drops (not when fully silent).
            // LiveAdsr handles retrigger-from-current-level cleanly, so releasing
            // voices can be reused immediately. Waiting for amp_cursor to reach
            // near-zero (old behaviour) caused all 6 slots to stay occupied during
            // long pad releases, forcing steals on sustaining voices → clicks.
            for ti in 0..TRACK_COUNT {
                for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                    if note.is_some()
                        && eng.tracks[ti].voice_gates[slot].value() < 0.5
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
                                // Prefer a releasing slot over a hard steal on a sustaining voice
                                .or_else(|| (0..VOICE_COUNT).find(|&s| eng.tracks[ti].voice_gates[s].value() < 0.5))
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
                                // Prefer a releasing slot over a hard steal on a sustaining voice
                                .or_else(|| (0..VOICE_COUNT).find(|&s| eng.tracks[ti].voice_gates[s].value() < 0.5))
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

            // --- Stop: silence all voices on playing→stopped transition ---
            let now_playing = beat_clock_shared.is_playing();
            if was_playing && !now_playing {
                for ti in 0..TRACK_COUNT {
                    for slot in 0..VOICE_COUNT {
                        eng.tracks[ti].voice_gates[slot].set(0.0);
                    }
                    voice_notes[ti] = [None; VOICE_COUNT];
                }
                markov_subdiv_counter = 0;
            }
            was_playing = now_playing;

            // --- Generative pattern generators (BeatClock-driven) ---
            {
                let beat_ev = beat_clock.tick(frames, sr, &beat_clock_shared);

                // Markov mode is active when track 0 is set to GenerativeMode::Markov.
                let markov_active = GenerativeMode::from_u8(
                    eng.generative_modes[0].load(std::sync::atomic::Ordering::Relaxed)
                ) == GenerativeMode::Markov;

                if beat_ev.bar {
                    for ti in 0..TRACK_COUNT {
                        euclidean_states[ti].reset();
                        prob_table_states[ti].reset();
                    }
                    if markov_active {
                        markov_engine.on_bar(&eng.markov_shared);
                    }
                }
                if beat_ev.subdivision {
                    if markov_active {
                        // Apply clock division: only step the engine every N subdivisions.
                        markov_subdiv_counter += 1;
                        let clock_div = eng.markov_shared.clock_div();
                        let step_now = markov_subdiv_counter >= clock_div;
                        if step_now { markov_subdiv_counter = 0; }

                        let voice_events = if step_now {
                            markov_engine.on_subdivision(&eng.markov_shared)
                        } else {
                            // No new step — still need to check for note_off from gate expiry.
                            // Return empty events (voices handle gate age internally on next real step).
                            vec![Default::default(); TRACK_COUNT]
                        };
                        // Snapshot MIDI sink once per subdivision block.
                        let midi_tap = midi_sink.try_lock().ok()
                            .and_then(|g| g.as_ref().map(|s| s.clone()));

                        for (ti, voice_ev) in voice_events.iter().enumerate().take(TRACK_COUNT) {
                            if let Some(pitch) = voice_ev.note_off {
                                let found = voice_notes[ti].iter_mut().enumerate().any(|(slot, note)| {
                                    if *note == Some(pitch) {
                                        eng.tracks[ti].voice_gates[slot].set(0.0);
                                        true
                                    } else { false }
                                });
                                if !found {
                                    dbg_miss_noteoff[ti] += 1;
                                }
                                if let Some(ref sink) = midi_tap {
                                    sink.note_off(sample_pos, ti as u8, pitch);
                                }
                            }
                            if let Some(pitch) = voice_ev.note_on {
                                if !voice_notes[ti].iter().enumerate().any(|(s, &n)| {
                                    n == Some(pitch) && eng.tracks[ti].voice_gates[s].value() > 0.5
                                }) {
                                    let slot = voice_notes[ti].iter().position(|&n| n == Some(pitch))
                                        .or_else(|| voice_notes[ti].iter().position(|n| n.is_none()))
                                        .unwrap_or_else(|| {
                                            let s = steal_idx[ti] % VOICE_COUNT;
                                            steal_idx[ti] += 1;
                                            dbg_steal_count[ti] += 1;
                                            s
                                        });
                                    voice_notes[ti][slot] = Some(pitch);
                                    eng.tracks[ti].voice_freq_targets[slot].set(midi_hz(pitch as f64) as f32);
                                    let velocity = if voice_ev.accent { 1.0f32 } else { 0.75f32 };
                                    eng.tracks[ti].voice_gates[slot].set(velocity);
                                    if let Some(ref sink) = midi_tap {
                                        let vel_u8 = (velocity * 127.0) as u8;
                                        sink.note_on(sample_pos, ti as u8, pitch, vel_u8);
                                    }
                                }
                            }
                        }
                    } else {
                        for ti in 0..TRACK_COUNT {
                            let mode = GenerativeMode::from_u8(
                                eng.generative_modes[ti].load(std::sync::atomic::Ordering::Relaxed)
                            );
                            let gen_ev = match mode {
                                GenerativeMode::Euclidean =>
                                    euclidean_states[ti].on_subdivision(&eng.euclidean_configs[ti]),
                                GenerativeMode::ProbTable =>
                                    prob_table_states[ti].on_subdivision(&eng.prob_table_configs[ti]),
                                _ => continue,
                            };
                            if let Some(pitch) = gen_ev.note_off {
                                for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                                    if *note == Some(pitch) {
                                        eng.tracks[ti].voice_gates[slot].set(0.0);
                                        break;
                                    }
                                }
                            }
                            if let Some(pitch) = gen_ev.note_on {
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
                }
            }

            // --- BPM-sync: update delay synced-time and LFO synced-rate each buffer ---
            {
                let bpm = beat_clock_shared.bpm();
                for ti in 0..TRACK_COUNT {
                    // Delay sync
                    let track = &eng.tracks[ti];
                    if track.fx_delay_sync.load(std::sync::atomic::Ordering::Relaxed) != 0 {
                        let div = ClockDivision::from_u8(
                            track.fx_delay_division.load(std::sync::atomic::Ordering::Relaxed)
                        );
                        track.fx_delay_synced_time.set(div.seconds(bpm));
                    }
                    // LFO sync
                    if track.lfo_sync.load(std::sync::atomic::Ordering::Relaxed) != 0 {
                        let div = ClockDivision::from_u8(
                            track.lfo_division.load(std::sync::atomic::Ordering::Relaxed)
                        );
                        track.lfo_rate.set(div.hz(bpm));
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

                // NaN/Inf guard: if DSP produces a bad value, zero it and count.
                let (l, r) = if l.is_finite() && r.is_finite() {
                    (l, r)
                } else {
                    dbg_nan_count += 1;
                    (0.0f32, 0.0f32)
                };

                // Recorder tap — non-blocking, won't affect real-time thread.
                if let Ok(rec) = recorder_sink.try_lock() {
                    if let Some(ref r_handle) = *rec {
                        r_handle.push(l, r);
                    }
                }

                last_out_l = l;
                last_out_r = r;
                let left  = T::from_sample(l);
                let right = T::from_sample(r);
                for (i, smp) in frame.iter_mut().enumerate() {
                    *smp = if i & 1 == 0 { left } else { right };
                }
                sample_pos += 1;
            }

            // --- Periodic debug dump every ~5 s (at 512-frame buffers / 44100 Hz ≈ 430 callbacks/s) ---
            dbg_callback_count += 1;
            let dump_interval = (sr as u64 * 5) / frames.max(1) as u64;
            if dbg_callback_count % dump_interval.max(1) == 0 {
                let occupied: [usize; TRACK_COUNT] = std::array::from_fn(|ti| {
                    voice_notes[ti].iter().filter(|n| n.is_some()).count()
                });
                let gates: [f32; TRACK_COUNT] = std::array::from_fn(|ti| {
                    eng.tracks[ti].voice_gates.iter().map(|g| g.value()).sum::<f32>()
                });
                eprintln!(
                    "[markov-dbg] cb={} | slots_occupied={:?} | gate_sum={:.2?} | steals={:?} | miss_noteoff={:?} | nan={}",
                    dbg_callback_count, occupied, gates, dbg_steal_count, dbg_miss_noteoff, dbg_nan_count
                );
            }
        },
        |err| eprintln!("audio error: {err}"),
        None,
    )?;
    Ok((beat_clock_shared_clone, stream))
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct AmbientBoxApp {
    engine: Arc<std::sync::Mutex<AmbientEngine>>,
    control: ControlSender,
    _stream: Stream,
    midi: MidiEngine,
    active_track: usize,
    beat_clock_shared: BeatClockShared,

    // Per-track keyboard state
    held_midi: [std::collections::HashSet<u8>; TRACK_COUNT],
    piano_octave: i32,

    // Per-track arp UI state (mirrors AtomicU8 config in engine)
    arp_bpm: [f32; TRACK_COUNT],
    arp_mode: [u8; TRACK_COUNT],
    arp_division: [u8; TRACK_COUNT],
    arp_oct: [u8; TRACK_COUNT],
    arp_gate: [f32; TRACK_COUNT],

    // Per-track scale walker UI state
    walker_bpm: [f32; TRACK_COUNT],
    walker_scale: [u8; TRACK_COUNT],
    walker_root: [u8; TRACK_COUNT],
    walker_oct: [u8; TRACK_COUNT],
    walker_div: [u8; TRACK_COUNT],
    walker_gate: [f32; TRACK_COUNT],
    arp_enabled_ui: [bool; TRACK_COUNT],
    arp_hold_ui: [bool; TRACK_COUNT],
    walker_enabled_ui: [bool; TRACK_COUNT],
    transport_sync: SyncTransport<TRACK_COUNT>,

    // Per-track generative mode UI state (Phase 8.2)
    gen_mode_ui: [u8; TRACK_COUNT],
    euclidean_hits_ui: [u8; TRACK_COUNT],
    euclidean_steps_ui: [u8; TRACK_COUNT],
    euclidean_root_ui: [u8; TRACK_COUNT],
    euclidean_rotation_ui: [u8; TRACK_COUNT],
    prob_table_tension_ui: [f32; TRACK_COUNT],

    // Markov engine UI state (Phase 8.6)
    markov_launchpad_cache: [[u8; LAUNCHPAD_COLS]; TRACK_COUNT],
    markov_launchpad_col_cache: usize,
    markov_root_ui: u8,
    markov_scale_ui: u8,
    markov_density_ui: f32,
    markov_mood_ui: [f32; N_MOODS],
    markov_bars_per_phrase_ui: u32,
    markov_voice_density_ui: [f32; TRACK_COUNT],
    markov_voice_role_ui: [u8; TRACK_COUNT],
    markov_voice_enabled_ui: [bool; TRACK_COUNT],
    markov_chord_attraction_ui: f32,
    markov_bass_lock_ui: bool,
    markov_dissonance_resolve_ui: bool,
    markov_dissonance_threshold_ui: u8,
    markov_register_drift_ui: f32,
    markov_clock_div_ui: u8,
    // Matrix heatmap tab: 0=Harmonic, 1=Rhythmic, 2=Melodic
    markov_heatmap_tab: usize,
    // Cached clone of markov_shared for heatmap rendering (updated each frame when lock is held)
    markov_shared_cache: Option<ambient_engine::MarkovEngineShared>,
    // Harmonic sequence — up to SEQ_MAX slots
    markov_seq_len_ui: usize,
    markov_seq_roots_ui: [u8; 8],
    markov_seq_scales_ui: [u8; 8],
    markov_seq_phrases_ui: [u8; 8],

    // Timeline (Phase 8.7)
    timeline: ambient_engine::Timeline,
    /// Last observed phrase_epoch from the audio thread.
    timeline_last_epoch: usize,
    /// Cached timeline status for UI display (updated each frame).
    timeline_status: Option<ambient_engine::TimelineStatus>,

    // Global shimmer UI state
    shimmer_on: bool,
    shimmer_mix: f32,
    shimmer_amt: f32,
    shimmer_size: f32,
    shimmer_damp: f32,
    shimmer_pitch: u8,

    // Global crystallizer UI state
    crystal_on: bool,
    crystal_mix: f32,
    crystal_grain_ms: f32,
    crystal_scatter: f32,
    crystal_feedback: f32,
    crystal_delay_ms: f32,
    crystal_pitch: u8,

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
    scene_preview_patch_names: [String; TRACK_COUNT],

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
    pending_patch_load: std::collections::VecDeque<(usize, usize)>,
    pending_scene_save: Option<PendingSceneSave>,
    pending_scene_load_path: Option<String>,
    pending_scene_load: Option<(String, Scene)>,

    // Transport extras
    swing_ui: f32, // 0–50 (maps to 0.0–0.5 in BeatClockShared)

    // LFO per-voice UI state
    lfo_enabled_ui: [bool; TRACK_COUNT],
    lfo_depth_ui: [f32; TRACK_COUNT],
    lfo_dest_ui: [u8; TRACK_COUNT],  // 0=Pitch 1=Filter 2=Amp
    lfo_shape_ui: [u8; TRACK_COUNT], // 0=Sin 1=Tri 2=Saw
    lfo_sync_ui: [bool; TRACK_COUNT],
    lfo_division_ui: [u8; TRACK_COUNT], // ClockDivision index
    lfo_rate_ui: [f32; TRACK_COUNT],    // Hz when free

    // Motif lock per-voice UI state
    motif_lock_ui: [bool; TRACK_COUNT],
    motif_length_ui: [u8; TRACK_COUNT], // 4/8/16/32

    // Crystal BPM-sync
    crystal_delay_sync_ui: bool,
    crystal_delay_division_ui: u8, // ClockDivision index

    // WAV recording
    recorder_sink: RecorderSink,
    rec_status: String,
    // MIDI recording
    midi_sink_shared: MidiSinkShared,
    midi_recorder: Option<midi_recorder::MidiRecorder>,
    midi_rec_status: String,
    sample_rate: u32,
}

impl AmbientBoxApp {
    fn collect_patch_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for ent in entries.flatten() {
            let p = ent.path();
            if p.is_dir() {
                Self::collect_patch_files(&p, out);
            } else if p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("json"))
                .unwrap_or(false)
            {
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
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for ent in entries.flatten() {
            let p = ent.path();
            if p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("json"))
                .unwrap_or(false)
            {
                if let Some(s) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(s.to_string());
                }
            }
        }
        out.sort();
        out
    }

    fn scene_preview_for_name(name: &str) -> [String; TRACK_COUNT] {
        let mut out: [String; TRACK_COUNT] = std::array::from_fn(|_| "—".to_string());
        let path = format!("scenes/{name}.json");
        let Ok(scene) = load_scene_json(&path) else {
            return out;
        };
        for (i, t) in scene.tracks.iter().enumerate() {
            let nm = t.patch.name.trim();
            out[i] = if !nm.is_empty() {
                nm.to_string()
            } else if !t.patch_path.trim().is_empty() {
                t.patch_path.clone()
            } else {
                "Init".to_string()
            };
        }
        out
    }

    fn new(
        engine: Arc<std::sync::Mutex<AmbientEngine>>,
        control: ControlSender,
        stream: Stream,
        beat_clock_shared: BeatClockShared,
        last_scene: Option<String>,
        recorder_sink: RecorderSink,
        midi_sink_shared: MidiSinkShared,
        sr: u32,
    ) -> Self {
        let mut midi = MidiEngine::new();
        midi.list_ports();
        let patch_library = Self::load_patch_library();
        let scene_files = Self::list_scene_files();
        let scene_preview_patch_names = if scene_files.is_empty() {
            std::array::from_fn(|_| "—".to_string())
        } else {
            Self::scene_preview_for_name(&scene_files[0])
        };
        Self {
            engine,
            control,
            _stream: stream,
            midi,
            active_track: 0,
            beat_clock_shared,
            held_midi: std::array::from_fn(|_| std::collections::HashSet::new()),
            piano_octave: 4,
            arp_bpm: [120.0; TRACK_COUNT],
            arp_mode: [0; TRACK_COUNT],
            arp_division: [1; TRACK_COUNT],
            arp_oct: [1; TRACK_COUNT],
            arp_gate: [0.7; TRACK_COUNT],
            walker_bpm: [120.0; TRACK_COUNT],
            walker_scale: [0; TRACK_COUNT],
            walker_root: [60; TRACK_COUNT],
            walker_oct: [2; TRACK_COUNT],
            walker_div: [1; TRACK_COUNT],
            walker_gate: [0.6; TRACK_COUNT],
            arp_enabled_ui: [false; TRACK_COUNT],
            arp_hold_ui: [false; TRACK_COUNT],
            walker_enabled_ui: [false; TRACK_COUNT],
            transport_sync: SyncTransport::new(120),
            gen_mode_ui: [0; TRACK_COUNT],
            euclidean_hits_ui: [4; TRACK_COUNT],
            euclidean_steps_ui: [8; TRACK_COUNT],
            euclidean_root_ui: [60; TRACK_COUNT],
            euclidean_rotation_ui: [0; TRACK_COUNT],
            prob_table_tension_ui: [1.0; TRACK_COUNT],
            markov_launchpad_cache: [[0u8; LAUNCHPAD_COLS]; TRACK_COUNT],
            markov_launchpad_col_cache: 0,
            markov_root_ui: 60,
            markov_scale_ui: 1, // Minor
            markov_density_ui: 0.5,
            markov_mood_ui: {
                let mut w = [0.0f32; N_MOODS];
                w[0] = 1.0; // 100% Calm
                w
            },
            markov_bars_per_phrase_ui: 4,
            markov_voice_density_ui: [0.0; TRACK_COUNT],
            markov_voice_role_ui: [
                VoiceRole::Bass as u8,
                VoiceRole::Pad as u8,
                VoiceRole::Melody as u8,
                VoiceRole::Texture as u8,
            ],
            markov_voice_enabled_ui: [true; TRACK_COUNT],
            markov_chord_attraction_ui: 0.5,
            markov_bass_lock_ui: true,
            markov_dissonance_resolve_ui: true,
            markov_dissonance_threshold_ui: 1,
            markov_register_drift_ui: 0.2,
            markov_clock_div_ui: 4,
            markov_heatmap_tab: 0,
            markov_shared_cache: None,
            markov_seq_len_ui: 1,
            markov_seq_roots_ui: [60u8; 8],
            markov_seq_scales_ui: [0u8; 8],
            markov_seq_phrases_ui: [4u8; 8],
            timeline: ambient_engine::Timeline::inactive(),
            timeline_last_epoch: 0,
            timeline_status: None,
            shimmer_on: false,
            shimmer_mix: 0.4,
            shimmer_amt: 0.5,
            shimmer_size: 0.6,
            shimmer_damp: 0.5,
            shimmer_pitch: 1,
            crystal_on: false,
            crystal_mix: 0.35,
            crystal_grain_ms: 120.0,
            crystal_scatter: 0.25,
            crystal_feedback: 0.35,
            crystal_delay_ms: 260.0,
            crystal_pitch: 2,
            macro_ui_values: [0.0; MACRO_COUNT],
            macro_set_ui: MacroSetKind::AmbientCore,
            scene_name: "ambient_01".to_string(),
            scene_bpm: 120,
            scene_key: 0,
            scene_scale_minor: false,
            scene_status: String::new(),
            scene_files,
            scene_selected: 0,
            scene_preview_patch_names,
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
            pending_patch_load: std::collections::VecDeque::new(),
            pending_scene_save: None,
            pending_scene_load_path: last_scene.map(|name| format!("scenes/{name}.json")),
            pending_scene_load: None,
            swing_ui: 0.0,
            lfo_enabled_ui: [false; TRACK_COUNT],
            lfo_depth_ui: [0.0; TRACK_COUNT],
            lfo_dest_ui: [1; TRACK_COUNT],  // Filter
            lfo_shape_ui: [0; TRACK_COUNT], // Sin
            lfo_sync_ui: [false; TRACK_COUNT],
            lfo_division_ui: [3; TRACK_COUNT], // Eighth
            lfo_rate_ui: [2.0; TRACK_COUNT],
            motif_lock_ui: [false; TRACK_COUNT],
            motif_length_ui: [16; TRACK_COUNT],
            crystal_delay_sync_ui: false,
            crystal_delay_division_ui: 8, // DottedEighth
            recorder_sink,
            rec_status: String::new(),
            midi_sink_shared,
            midi_recorder: None,
            midi_rec_status: String::new(),
            sample_rate: sr,
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
                let _ = self
                    .control
                    .try_send(ControlEvent::ArpRestart { track: ti as u8 });
            }
            if batch.walker[ti] {
                let _ = self
                    .control
                    .try_send(ControlEvent::WalkerRestart { track: ti as u8 });
            }
        }
    }

    /// Auto-load ambient patches for each Markov voice role on mode switch.
    /// Preferred patches per role; falls back to any match in the category.
    fn auto_load_ambient_patches(&mut self) {
        // (track_index, preferred_name, fallback_category)
        let assignments: [(usize, &str, &str); TRACK_COUNT] = [
            (0, "Ambient Bass", "Ambient"),
            (1, "Ambient Pad", "Ambient"),
            (2, "Eno Space", "Ambient"),
            (3, "Ambient Texture", "Ambient"),
        ];
        for (track, preferred, fallback_cat) in assignments {
            // Try preferred name first, then fallback category
            let idx = self
                .patch_library
                .iter()
                .position(|p| p.name == preferred)
                .or_else(|| {
                    self.patch_library
                        .iter()
                        .position(|p| p.category == fallback_cat)
                });
            if let Some(idx) = idx {
                self.track_patch_choice[track] = idx;
                self.pending_patch_load.push_back((track, idx));
            }
        }
    }

    fn sync_transport_now(&mut self) {
        let batch = self.transport_sync.sync_now();
        self.dispatch_restarts(batch);
    }

    fn schedule_or_restart_arp(&mut self, track: usize) {
        if self.transport_sync.schedule_or_restart_arp(track) {
            let _ = self
                .control
                .try_send(ControlEvent::ArpRestart { track: track as u8 });
        }
    }

    fn schedule_or_restart_walker(&mut self, track: usize) {
        if self.transport_sync.schedule_or_restart_walker(track) {
            let _ = self
                .control
                .try_send(ControlEvent::WalkerRestart { track: track as u8 });
        }
    }

    fn tick_transport_sync(&mut self) {
        let batch = self.transport_sync.tick();
        self.dispatch_restarts(batch);
    }
}

// ---------------------------------------------------------------------------
// Probability matrix heatmap
// ---------------------------------------------------------------------------

/// Draw an N×N probability matrix as a heatmap with row/col labels.
/// `mat` is a slice of rows, each a slice of N probabilities.
/// `accent` is the tint color used at maximum probability.
fn draw_heatmap(
    ui: &mut egui::Ui,
    mat: &[Vec<f32>],
    row_labels: &[&str],
    col_labels: &[&str],
    n: usize,
    accent: egui::Color32,
) {
    let cell = 28.0f32;
    let label_w = 52.0f32;
    let label_h = 16.0f32;
    let gap = 2.0f32;
    let total_w = label_w + n as f32 * (cell + gap) - gap;
    let total_h = label_h + n as f32 * (cell + gap) - gap;

    let (resp, painter) = ui.allocate_painter(egui::vec2(total_w, total_h), egui::Sense::hover());
    let origin = resp.rect.min;

    // Column labels across the top
    for j in 0..n {
        let x = origin.x + label_w + j as f32 * (cell + gap) + cell * 0.5;
        let y = origin.y + label_h * 0.5;
        painter.text(
            egui::pos2(x, y),
            egui::Align2::CENTER_CENTER,
            col_labels[j],
            egui::FontId::proportional(10.0),
            egui::Color32::from_gray(160),
        );
    }

    // Rows
    for i in 0..n {
        let row_y = origin.y + label_h + i as f32 * (cell + gap);

        // Row label
        painter.text(
            egui::pos2(origin.x + label_w - 4.0, row_y + cell * 0.5),
            egui::Align2::RIGHT_CENTER,
            row_labels[i],
            egui::FontId::proportional(10.0),
            egui::Color32::from_gray(180),
        );

        for j in 0..n {
            let p = mat[i][j].clamp(0.0, 1.0);
            // Color: lerp from dark background to accent
            let bg = egui::Color32::from_gray(18);
            let r = (bg.r() as f32 + (accent.r() as f32 - bg.r() as f32) * p) as u8;
            let g = (bg.g() as f32 + (accent.g() as f32 - bg.g() as f32) * p) as u8;
            let b = (bg.b() as f32 + (accent.b() as f32 - bg.b() as f32) * p) as u8;
            let color = egui::Color32::from_rgb(r, g, b);

            let x = origin.x + label_w + j as f32 * (cell + gap);
            let rect = egui::Rect::from_min_size(egui::pos2(x, row_y), egui::vec2(cell, cell));
            painter.rect_filled(rect, 3.0, color);

            // Value text for cells above ~8% (smaller ones are too cluttered)
            if p > 0.07 {
                let luma = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
                let txt_color = if luma > 100.0 {
                    egui::Color32::BLACK
                } else {
                    egui::Color32::from_gray(200)
                };
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{:.0}", p * 100.0),
                    egui::FontId::proportional(9.0),
                    txt_color,
                );
            }

            // Tooltip on hover
            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                if rect.contains(pos) {
                    egui::show_tooltip_at_pointer(
                        ui.ctx(),
                        ui.layer_id(),
                        egui::Id::new("hm_tip").with(i).with(j),
                        |ui| {
                            ui.label(format!(
                                "{} → {}: {:.1}%",
                                row_labels[i],
                                col_labels[j],
                                p * 100.0
                            ));
                        },
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Launchpad helpers
// ---------------------------------------------------------------------------

fn voice_role_color(role: VoiceRole) -> egui::Color32 {
    match role {
        VoiceRole::Bass => egui::Color32::from_rgb(255, 160, 60),
        VoiceRole::Pad => egui::Color32::from_rgb(80, 160, 255),
        VoiceRole::Melody => egui::Color32::from_rgb(60, 220, 120),
        VoiceRole::Texture => egui::Color32::from_rgb(200, 100, 255),
        VoiceRole::Pulse => egui::Color32::from_rgb(255, 80, 80),
    }
}

fn launchpad_cell_color(
    state: RhythmicState,
    base: egui::Color32,
    is_current: bool,
) -> egui::Color32 {
    let dim = |c: egui::Color32, factor: f32| -> egui::Color32 {
        egui::Color32::from_rgb(
            (c.r() as f32 * factor) as u8,
            (c.g() as f32 * factor) as u8,
            (c.b() as f32 * factor) as u8,
        )
    };
    let col = match state {
        RhythmicState::Rest => egui::Color32::from_gray(20),
        RhythmicState::Hold => egui::Color32::from_gray(60),
        RhythmicState::Single => base,
        RhythmicState::Double => egui::Color32::WHITE,
        RhythmicState::Accent => dim(base, 1.4f32.min(2.0)),
    };
    // Brighten the current column slightly to show playhead
    if is_current {
        egui::Color32::from_rgb(
            col.r().saturating_add(30),
            col.g().saturating_add(30),
            col.b().saturating_add(30),
        )
    } else {
        col
    }
}

// Keyboard note mapping (same as the_synth)
const KEY_MAP: &[(egui::Key, i32)] = &[
    (egui::Key::A, 0),
    (egui::Key::W, 1),
    (egui::Key::S, 2),
    (egui::Key::E, 3),
    (egui::Key::D, 4),
    (egui::Key::F, 5),
    (egui::Key::T, 6),
    (egui::Key::G, 7),
    (egui::Key::Y, 8),
    (egui::Key::H, 9),
    (egui::Key::U, 10),
    (egui::Key::J, 11),
    (egui::Key::K, 12),
];

impl eframe::App for AmbientBoxApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if !self.scene_name.is_empty() {
            storage.set_string("last_scene", self.scene_name.clone());
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick_midi();
        self.tick_transport_sync();

        egui::CentralPanel::default().show(ctx, |ui| {

            // ── Engine read-sync ──────────────────────────────────────────────
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

                // Sync Markov UI state from engine (only needed when mode is active)
                {
                    let ms = &eng.markov_shared;
                    self.markov_root_ui = ms.root();
                    self.markov_scale_ui = ms.scale.load(Ordering::Relaxed);
                    self.markov_density_ui = ms.density();
                    self.markov_bars_per_phrase_ui = ms.bars_per_phrase();
                    for i in 0..TRACK_COUNT {
                        self.markov_voice_density_ui[i] = ms.voice_density[i].value();
                        self.markov_voice_role_ui[i] = ms.roles[i].load(Ordering::Relaxed);
                        self.markov_voice_enabled_ui[i] = ms.voice_enabled(i);
                    }
                    for i in 0..N_MOODS {
                        self.markov_mood_ui[i] = ms.mood.weight(i);
                    }
                    self.markov_chord_attraction_ui = ms.chord_attraction.value();
                    self.markov_bass_lock_ui = ms.bass_lock.load(Ordering::Relaxed);
                    self.markov_dissonance_resolve_ui = ms.dissonance_resolve.load(Ordering::Relaxed);
                    self.markov_dissonance_threshold_ui = ms.dissonance_threshold.load(Ordering::Relaxed);
                    self.markov_register_drift_ui = ms.register_drift.value();
                    // Snapshot launchpad into UI-owned cache so the grid renders every frame
                    self.markov_launchpad_col_cache = ms.launchpad_col.load(Ordering::Relaxed) % LAUNCHPAD_COLS;
                    for row in 0..TRACK_COUNT {
                        if let Some(pad_row) = ms.launchpad.get(row) {
                            for col in 0..LAUNCHPAD_COLS {
                                self.markov_launchpad_cache[row][col] = pad_row[col].load(Ordering::Relaxed);
                            }
                        }
                    }
                    // ── Timeline: poll phrase_epoch and advance ──
                    let epoch = ms.phrase_epoch.load(Ordering::Relaxed);
                    if self.timeline.active && epoch != self.timeline_last_epoch {
                        let delta = epoch.wrapping_sub(self.timeline_last_epoch);
                        for _ in 0..delta {
                            self.timeline.on_phrase_boundary();
                        }
                        self.timeline.apply_to_shared(ms);
                        // Apply effects targets to UI fields so the existing
                        // UI→engine write path picks them up.
                        let state = self.timeline.interpolated_state();
                        self.shimmer_mix = state.shimmer_mix;
                        self.shimmer_amt = state.shimmer_amount;
                        self.shimmer_size = state.shimmer_size;
                        self.crystal_mix = state.crystal_mix;
                        self.crystal_feedback = state.crystal_feedback;
                        self.crystal_delay_ms = state.crystal_delay_ms;
                        // Also sync the markov UI fields so knobs reflect timeline.
                        for i in 0..N_MOODS {
                            self.markov_mood_ui[i] = state.mood[i];
                        }
                        self.markov_density_ui = state.density;
                        self.markov_root_ui = state.root;
                        self.markov_scale_ui = state.scale;
                        self.markov_bars_per_phrase_ui = state.bars_per_phrase;
                        for i in 0..TRACK_COUNT.min(4) {
                            self.markov_voice_enabled_ui[i] = state.voice_enabled[i];
                        }
                        self.timeline_last_epoch = epoch;
                    }
                    self.timeline_status = Some(self.timeline.status());
                }

                // Sync generative UI state from engine (euclidean/prob params only — gen_mode_ui is UI-owned)
                let ecfg = &eng.euclidean_configs[ti];
                self.euclidean_hits_ui[ti] = ecfg.hits.load(Ordering::Relaxed);
                self.euclidean_steps_ui[ti] = ecfg.steps.load(Ordering::Relaxed);
                self.euclidean_root_ui[ti] = ecfg.root.load(Ordering::Relaxed);
                self.euclidean_rotation_ui[ti] = ecfg.rotation.load(Ordering::Relaxed);
                self.prob_table_tension_ui[ti] = eng.prob_table_configs[ti].tension.value();

                for i in 0..TRACK_COUNT {
                    let engine_patch_path = eng.track_patch_paths[i].clone();
                    if self.track_patch_last_synced_path[i] != engine_patch_path {
                        if let Some(idx) = self.patch_library.iter().position(|p| p.path == engine_patch_path) {
                            self.track_patch_choice[i] = idx;
                        }
                        self.track_patch_last_synced_path[i] = engine_patch_path;
                    }
                }
                // Cache a clone of markov_shared for heatmap rendering outside the lock
                self.markov_shared_cache = Some(eng.markov_shared.clone());
            }

            let ti = self.active_track;
            let markov_active = GenerativeMode::from_u8(self.gen_mode_ui[0]) == GenerativeMode::Markov;
            let mut trigger_arp_restart: Option<usize> = None;
            let mut trigger_walker_restart: Option<usize> = None;
            let mut pending_scene_save: Option<(String, Scene, String)> = None;

            // ── Transport bar ─────────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.heading("Ambient Box");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let _ = synth_ui::knob(ui, "Master", &mut self.master_vol_ui, 0.0, 1.0);
                });
            });
            ui.horizontal(|ui| {
                let btn = if self.transport_sync.playing { "⏹ Stop" } else { "▶ Play" };
                if ui.button(btn).clicked() {
                    let new_playing = !self.transport_sync.playing;
                    let due = self.transport_sync.set_playing(new_playing);
                    self.beat_clock_shared.set_playing(new_playing);
                    self.dispatch_restarts(due);
                }
                ui.label("BPM:");
                ui.add(egui::Slider::new(&mut self.transport_sync.bpm, 40..=300));
                ui.label("Swing:").on_hover_text("Swing amount: 0 = straight, 50 = full triplet swing. Delays every even subdivision.");
                ui.add(egui::Slider::new(&mut self.swing_ui, 0.0f32..=50.0).suffix("%"));
                if ui.checkbox(&mut self.transport_sync.clock_sync_enabled, "Clock Sync").changed() {
                    self.transport_sync.set_clock_sync(self.transport_sync.clock_sync_enabled);
                    if self.transport_sync.clock_sync_enabled { self.sync_transport_now(); }
                }
                ui.add_enabled_ui(self.transport_sync.clock_sync_enabled, |ui| {
                    ui.checkbox(&mut self.transport_sync.bar_quantize_start, "Bar Quantize");
                });
                if ui.button("SYNC NOW").clicked() { self.sync_transport_now(); }
            });

            // ── Mode toggle ───────────────────────────────────────────────────
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Mode:");
                if ui.selectable_label(!markov_active,
                    egui::RichText::new("MANUAL").color(if !markov_active { egui::Color32::WHITE } else { egui::Color32::DARK_GRAY }).strong()
                ).clicked() && markov_active {
                    for t in 0..TRACK_COUNT { self.gen_mode_ui[t] = GenerativeMode::Off as u8; }
                }
                if ui.selectable_label(markov_active,
                    egui::RichText::new("MARKOV").color(if markov_active { egui::Color32::from_rgb(200, 140, 255) } else { egui::Color32::DARK_GRAY }).strong()
                ).clicked() && !markov_active {
                    for t in 0..TRACK_COUNT { self.gen_mode_ui[t] = GenerativeMode::Markov as u8; }
                    self.auto_load_ambient_patches();
                }
            });
            ui.separator();

            // ── MARKOV content ────────────────────────────────────────────────
            if markov_active {
                let note_names = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];
                ui.label(egui::RichText::new("MARKOV ENGINE").strong().color(egui::Color32::from_rgb(200, 140, 255)));
                ui.horizontal(|ui| {
                    ui.label("Key:").on_hover_text("Root note of the generative scale. All voices stay within this key.");
                    egui::ComboBox::from_id_salt("markov_root")
                        .selected_text(note_names[(self.markov_root_ui % 12) as usize])
                        .show_ui(ui, |ui| {
                            for (i, &name) in note_names.iter().enumerate() {
                                let midi = 48 + i as u8;
                                ui.selectable_value(&mut self.markov_root_ui, midi, name);
                            }
                        });
                    ui.label("Scale:").on_hover_text("Scale mode used to constrain note choices. Major = brighter, Minor = darker/moodier.");
                    egui::ComboBox::from_id_salt("markov_scale")
                        .selected_text(MarkovScale::LABELS[self.markov_scale_ui as usize % MarkovScale::LABELS.len()])
                        .show_ui(ui, |ui| {
                            for (i, &lbl) in MarkovScale::LABELS.iter().enumerate() {
                                ui.selectable_value(&mut self.markov_scale_ui, i as u8, lbl);
                            }
                        });
                    ui.label("Density:").on_hover_text("Global note density: 0 = almost silent, 1 = voices fire on nearly every subdivision. Each voice role has its own divisor on top of this.");
                    ui.add(egui::Slider::new(&mut self.markov_density_ui, 0.0f32..=1.0).step_by(0.01));
                    ui.label("Phrase:").on_hover_text("How many bars before the harmonic chain advances to the next chord. Longer phrases = slower harmonic movement.");
                    ui.add(egui::Slider::new(&mut self.markov_bars_per_phrase_ui, 2u32..=16));
                    ui.label("bars");
                });
                ui.horizontal(|ui| {
                    ui.label("Mood:").on_hover_text("Blend of 6 probability matrices. Each matrix encodes different chord progressions and rhythmic tendencies. Values are normalised — raising one lowers the relative weight of others.");
                    let mood_labels = ["Calm", "Tense", "Dark", "Euphoric", "Cosmic", "Gravity"];
                    let mood_colors = [
                        egui::Color32::from_rgb(100, 200, 255),
                        egui::Color32::from_rgb(255, 100, 100),
                        egui::Color32::from_rgb(120, 80, 200),
                        egui::Color32::from_rgb(255, 220, 60),
                        egui::Color32::from_rgb(180, 230, 255),
                        egui::Color32::from_rgb(160, 140, 220),
                    ];
                    let mood_tips = [
                        "Calm: Tonic-centred movement (I–IV–I), stepwise melody, sparse rhythm. Peaceful and resolved.",
                        "Tense: Unresolved progressions (ii°–V–i), chromatic tension, denser rhythm. Anxious, suspenseful.",
                        "Dark: Minor modal movement (i–VI–VII), low melody register, sustained notes. Brooding, heavy.",
                        "Euphoric: Bright major movement (I–V–vi–IV), high register, flowing density. Uplifting, open.",
                        "Cosmic: Plagal IV↔I drift, very sparse events, stepwise motion. Vast, weightless, Eno-inspired.",
                        "Gravity: vi–IV–I–V with deceptive V→vi, narrow range, abrupt rests. Slow pull, inevitable.",
                    ];
                    for i in 0..N_MOODS {
                        let lbl = mood_labels.get(i).copied().unwrap_or("?");
                        ui.colored_label(mood_colors[i % mood_colors.len()], lbl)
                            .on_hover_text(mood_tips[i % mood_tips.len()]);
                        ui.add(egui::Slider::new(&mut self.markov_mood_ui[i], 0.0f32..=1.0)
                            .step_by(0.01).clamping(egui::SliderClamping::Always));
                    }
                    let sum: f32 = self.markov_mood_ui.iter().sum();
                    if sum > 0.001 {
                        ui.label(egui::RichText::new(format!("Σ={:.2}", sum)).small().weak());
                    }
                });
                // ── Voice behaviour controls ──────────────────────────────────
                ui.horizontal(|ui| {
                    ui.label("Harmony:").on_hover_text("Chord-tone attraction: how strongly each voice is biased toward notes that belong to the current chord. 0 = free chromatic movement, 1 = always snaps to chord tones.");
                    ui.add(egui::Slider::new(&mut self.markov_chord_attraction_ui, 0.0f32..=1.0)
                        .step_by(0.01).clamping(egui::SliderClamping::Always));
                    ui.checkbox(&mut self.markov_bass_lock_ui, "Bass lock")
                        .on_hover_text("When enabled, the bass voice is forced to the root note on every bar downbeat, keeping the harmonic foundation grounded.");
                    ui.separator();
                    let resolve_color = if self.markov_dissonance_resolve_ui {
                        egui::Color32::from_rgb(100, 220, 140)
                    } else {
                        egui::Color32::GRAY
                    };
                    if ui.selectable_label(self.markov_dissonance_resolve_ui,
                        egui::RichText::new("Resolve").color(resolve_color)
                    ).clicked() {
                        self.markov_dissonance_resolve_ui = !self.markov_dissonance_resolve_ui;
                    }
                    ui.label("").on_hover_text("Post-pass dissonance resolution: when two voices play simultaneously, intervals matching the selected threshold are detected and one voice is nudged to a consonant neighbour.");
                    ui.add_enabled_ui(self.markov_dissonance_resolve_ui, |ui| {
                        for (val, lbl, tip) in [
                            (0u8, "2nds",     "Resolve semitone clashes (minor/major 2nds). Mildest correction."),
                            (1u8, "+tritone", "Also resolve tritones (augmented 4th). Adds to 2nd resolution."),
                            (2u8, "+7ths",    "Also resolve major 7th intervals. Most aggressive correction."),
                        ] {
                            if ui.selectable_label(self.markov_dissonance_threshold_ui == val, lbl)
                                .on_hover_text(tip)
                                .clicked()
                            {
                                self.markov_dissonance_threshold_ui = val;
                            }
                        }
                    });
                    ui.separator();
                    ui.label("Drift:").on_hover_text("Register drift: probability per phrase that a voice shifts its octave up or down. 0 = voices stay in their initial octave, 1 = voices wander freely across registers.");
                    ui.add(egui::Slider::new(&mut self.markov_register_drift_ui, 0.0f32..=1.0)
                        .step_by(0.01).clamping(egui::SliderClamping::Always));
                    ui.separator();
                    ui.label("Step:").on_hover_text("Clock division: how many 16th-note ticks between Markov steps. 1=16th note, 2=8th, 4=quarter beat (default), 8=half note. Lower = faster, more notes; higher = slower, more spacious.");
                    for (val, lbl) in [(1u8,"1/16"),(2u8,"1/8"),(4u8,"1/4"),(8u8,"1/2")] {
                        if ui.selectable_label(self.markov_clock_div_ui == val, lbl).clicked() {
                            self.markov_clock_div_ui = val;
                        }
                    }
                });
                // ── Harmonic sequence ─────────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.label("Chord seq:").on_hover_text("Harmonic sequence: up to 8 chord slots that cycle in a loop. Each slot has its own key, scale, and phrase duration. Use 1 slot for a static key (legacy behaviour).");
                    // Slot count selector
                    for n in 1usize..=8 {
                        if ui.selectable_label(self.markov_seq_len_ui == n, format!("{n}")).clicked() {
                            self.markov_seq_len_ui = n;
                            // When shrinking to 1, sync back to the global key/scale pickers
                            if n == 1 {
                                self.markov_seq_roots_ui[0] = self.markov_root_ui;
                                self.markov_seq_scales_ui[0] = self.markov_scale_ui;
                            }
                        }
                    }
                    ui.label("slots");
                });
                if self.markov_seq_len_ui > 1 {
                    for i in 0..self.markov_seq_len_ui {
                        ui.horizontal(|ui| {
                            ui.label(format!("  #{}", i + 1));
                            egui::ComboBox::from_id_salt(format!("seq_root_{i}"))
                                .selected_text(note_names[(self.markov_seq_roots_ui[i] % 12) as usize])
                                .width(48.0)
                                .show_ui(ui, |ui| {
                                    for (j, &name) in note_names.iter().enumerate() {
                                        ui.selectable_value(&mut self.markov_seq_roots_ui[i], 48 + j as u8, name);
                                    }
                                });
                            egui::ComboBox::from_id_salt(format!("seq_scale_{i}"))
                                .selected_text(MarkovScale::LABELS[self.markov_seq_scales_ui[i] as usize % MarkovScale::LABELS.len()])
                                .width(72.0)
                                .show_ui(ui, |ui| {
                                    for (j, &lbl) in MarkovScale::LABELS.iter().enumerate() {
                                        ui.selectable_value(&mut self.markov_seq_scales_ui[i], j as u8, lbl);
                                    }
                                });
                            ui.label("×").on_hover_text("Number of phrases this chord lasts before advancing to the next slot.");
                            ui.add(egui::DragValue::new(&mut self.markov_seq_phrases_ui[i]).range(1u8..=16));
                            ui.label("phrases");
                        });
                    }
                }
                ui.separator();
                // Per-voice rows with inline patch
                for i in 0..TRACK_COUNT {
                    let role_color = voice_role_color(VoiceRole::from_u8(self.markov_voice_role_ui[i]));
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("V{}", i + 1)).color(role_color).strong())
                            .on_hover_text("Voice channel. Each voice runs its own Markov chain independently.");
                        ui.checkbox(&mut self.markov_voice_enabled_ui[i], "")
                            .on_hover_text("Enable or mute this voice. Disabled voices produce no notes but keep their state.");
                        ui.label("Role:").on_hover_text("Voice role determines note range, rhythm divisor, and chord-attraction strength. Bass = low + locked, Pad = mid + slow, Melody = high + free, Texture = sparse noise layer.");
                        egui::ComboBox::from_id_salt(format!("markov_role_{i}"))
                            .selected_text(VoiceRole::LABELS[self.markov_voice_role_ui[i] as usize % VoiceRole::LABELS.len()])
                            .show_ui(ui, |ui| {
                                for (j, &lbl) in VoiceRole::LABELS.iter().enumerate() {
                                    ui.selectable_value(&mut self.markov_voice_role_ui[i], j as u8, lbl);
                                }
                            });
                        ui.label("Patch:").on_hover_text("Synthesizer patch loaded on this voice. Choose from library then click Load to apply.");
                        if self.patch_library.is_empty() {
                            ui.label(egui::RichText::new("—").weak());
                        } else {
                            if self.track_patch_choice[i] >= self.patch_library.len() {
                                self.track_patch_choice[i] = 0;
                            }
                            let sel = &self.patch_library[self.track_patch_choice[i]];
                            egui::ComboBox::from_id_salt(format!("markov_patch_{i}"))
                                .selected_text(format!("{} / {}", sel.category, sel.name))
                                .show_ui(ui, |ui| {
                                    for (j, p) in self.patch_library.iter().enumerate() {
                                        ui.selectable_value(&mut self.track_patch_choice[i], j, format!("{} / {}", p.category, p.name));
                                    }
                                });
                            if ui.small_button("Load").clicked() {
                                self.pending_patch_load.push_back((i, self.track_patch_choice[i]));
                            }
                        }
                        ui.label("Dens:").on_hover_text("Per-voice density override. Multiplies the global density for this voice only. 0 = use global, 1 = always fire at max rate.");
                        ui.add(egui::Slider::new(&mut self.markov_voice_density_ui[i], 0.0f32..=1.0)
                            .step_by(0.01).clamping(egui::SliderClamping::Always));
                        let _ = synth_ui::knob(ui, "Vol", &mut self.track_vol_ui[i], 0.0, 1.0);
                        let _ = synth_ui::knob(ui, "Shim", &mut self.track_shimmer_send_ui[i], 0.0, 1.0);
                        let _ = synth_ui::knob(ui, "Crys", &mut self.track_crystal_send_ui[i], 0.0, 1.0);

                        ui.separator();

                        // ── LFO ──────────────────────────────────────────────
                        ui.label(egui::RichText::new("LFO").strong()).on_hover_text("Low-frequency oscillator. Modulates pitch, filter, or amplitude at the chosen rate.");
                        ui.checkbox(&mut self.lfo_enabled_ui[i], "").on_hover_text("Enable LFO for this voice.");
                        ui.add_enabled_ui(self.lfo_enabled_ui[i], |ui| {
                            egui::ComboBox::from_id_salt(format!("lfo_dest_{i}"))
                                .selected_text(["Pitch","Filter","Amp"][self.lfo_dest_ui[i] as usize % 3])
                                .width(54.0)
                                .show_ui(ui, |ui| {
                                    for (j, lbl) in ["Pitch","Filter","Amp"].iter().enumerate() {
                                        ui.selectable_value(&mut self.lfo_dest_ui[i], j as u8, *lbl);
                                    }
                                });
                            egui::ComboBox::from_id_salt(format!("lfo_shape_{i}"))
                                .selected_text(["Sin","Tri","Saw"][self.lfo_shape_ui[i] as usize % 3])
                                .width(44.0)
                                .show_ui(ui, |ui| {
                                    for (j, lbl) in ["Sin","Tri","Saw"].iter().enumerate() {
                                        ui.selectable_value(&mut self.lfo_shape_ui[i], j as u8, *lbl);
                                    }
                                });
                            let _ = synth_ui::knob(ui, "Depth", &mut self.lfo_depth_ui[i], 0.0, 1.0);
                            // Sync toggle
                            let sync_lbl = if self.lfo_sync_ui[i] { "SYNC" } else { "FREE" };
                            if ui.small_button(sync_lbl).on_hover_text("Toggle between free Hz rate and BPM-synced division.").clicked() {
                                self.lfo_sync_ui[i] = !self.lfo_sync_ui[i];
                            }
                            if self.lfo_sync_ui[i] {
                                egui::ComboBox::from_id_salt(format!("lfo_div_{i}"))
                                    .selected_text(synth_common::ClockDivision::from_u8(self.lfo_division_ui[i]).label())
                                    .width(52.0)
                                    .show_ui(ui, |ui| {
                                        for div in synth_common::ClockDivision::ALL {
                                            ui.selectable_value(&mut self.lfo_division_ui[i], div.to_u8(), div.label());
                                        }
                                    });
                            } else {
                                ui.add(egui::Slider::new(&mut self.lfo_rate_ui[i], 0.1f32..=20.0)
                                    .suffix(" Hz").step_by(0.1));
                            }
                        });

                        ui.separator();

                        // ── Motif lock ────────────────────────────────────────
                        ui.label(egui::RichText::new("MOTIF").strong())
                            .on_hover_text("Records this voice's rhythmic pattern for N steps then loops it. Melodic chain stays generative.");
                        let motif_active = if let Ok(eng_r) = self.engine.try_lock() {
                            eng_r.markov_shared.motif_active[i].load(Ordering::Relaxed)
                        } else { false };
                        let lock_col = if motif_active {
                            egui::Color32::from_rgb(80, 220, 80)   // green = replaying
                        } else if self.motif_lock_ui[i] {
                            egui::Color32::from_rgb(220, 180, 60)  // yellow = capturing
                        } else {
                            egui::Color32::GRAY
                        };
                        let lock_lbl = egui::RichText::new(
                            if self.motif_lock_ui[i] { "LOCK" } else { "FREE" }
                        ).color(lock_col).strong();
                        if ui.button(lock_lbl)
                            .on_hover_text("Enable: engine records pattern then loops it. Disable: back to Markov.")
                            .clicked()
                        {
                            self.motif_lock_ui[i] = !self.motif_lock_ui[i];
                        }
                        ui.label("Len:").on_hover_text("Steps to capture before looping (4/8/16/32).");
                        egui::ComboBox::from_id_salt(format!("motif_len_{i}"))
                            .selected_text(format!("{}", self.motif_length_ui[i]))
                            .width(42.0)
                            .show_ui(ui, |ui| {
                                for &len in &[4u8, 8, 16, 32] {
                                    ui.selectable_value(&mut self.motif_length_ui[i], len, format!("{len}"));
                                }
                            });
                    });
                }
                // Launchpad grid — rendered from UI-owned cache, no lock needed
                ui.separator();
                {
                    let label_w = 52.0f32; // left margin for voice labels
                    let cell_w  = 18.0f32;
                    let cell_h  = 18.0f32;
                    let gap     =  2.0f32;
                    let grid_w  = label_w + (cell_w + gap) * LAUNCHPAD_COLS as f32;
                    let grid_h  = (cell_h + gap) * TRACK_COUNT as f32;
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(grid_w, grid_h), egui::Sense::hover());
                    let painter = ui.painter_at(rect);

                    let role_labels = ["Bass", "Pad", "Melody", "Tex"];
                    let current_col = self.markov_launchpad_col_cache;

                    for row in 0..TRACK_COUNT {
                        let role = VoiceRole::from_u8(self.markov_voice_role_ui[row]);
                        let base_color = voice_role_color(role);
                        let enabled = self.markov_voice_enabled_ui[row];
                        let row_y = rect.top() + row as f32 * (cell_h + gap);

                        // ── Voice label on the left ────────────────────────────
                        let label = format!("V{}  {}", row + 1,
                            role_labels[self.markov_voice_role_ui[row] as usize % role_labels.len()]);
                        let label_color = if enabled { base_color } else { egui::Color32::from_gray(50) };
                        painter.text(
                            egui::pos2(rect.left() + 2.0, row_y + cell_h * 0.5),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(10.0),
                            label_color,
                        );

                        // ── Pre-pass: find note runs (attack + trailing holds) ─
                        // A run starts at any Single/Double/Accent and extends through
                        // consecutive Hold cells. We draw the whole run as one long pill.
                        let row_data = &self.markov_launchpad_cache[row];
                        let mut drawn = [false; LAUNCHPAD_COLS];
                        let mut col = 0;
                        while col < LAUNCHPAD_COLS {
                            let state = RhythmicState::from_usize(row_data[col] as usize);
                            match state {
                                RhythmicState::Single | RhythmicState::Double | RhythmicState::Accent => {
                                    // Find how many consecutive Holds follow
                                    let mut run_len = 1usize;
                                    while col + run_len < LAUNCHPAD_COLS
                                        && RhythmicState::from_usize(row_data[col + run_len] as usize)
                                            == RhythmicState::Hold
                                    {
                                        run_len += 1;
                                    }
                                    // Draw one wide pill covering the entire note
                                    let x = rect.left() + label_w + col as f32 * (cell_w + gap);
                                    let w = run_len as f32 * (cell_w + gap) - gap;
                                    let attack_color = match state {
                                        RhythmicState::Accent => egui::Color32::WHITE,
                                        RhythmicState::Double => egui::Color32::from_rgb(
                                            base_color.r().saturating_add(80),
                                            base_color.g().saturating_add(80),
                                            base_color.b().saturating_add(80),
                                        ),
                                        _ => base_color,
                                    };
                                    let note_color = if enabled { attack_color } else {
                                        egui::Color32::from_gray(35)
                                    };
                                    // Attack head: bright, slightly taller
                                    let head_rect = egui::Rect::from_min_size(
                                        egui::pos2(x, row_y + 1.0),
                                        egui::vec2(cell_w, cell_h - 2.0),
                                    );
                                    painter.rect_filled(head_rect, 3.0, note_color);
                                    // Tail (hold portion): same color but shorter height
                                    if run_len > 1 {
                                        let tail_x = x + cell_w + gap;
                                        let tail_w = (run_len - 1) as f32 * (cell_w + gap) - gap;
                                        let tail_color = egui::Color32::from_rgba_premultiplied(
                                            note_color.r(), note_color.g(), note_color.b(), 140,
                                        );
                                        let tail_rect = egui::Rect::from_min_size(
                                            egui::pos2(tail_x, row_y + cell_h * 0.3),
                                            egui::vec2(tail_w, cell_h * 0.4),
                                        );
                                        painter.rect_filled(tail_rect, 2.0, tail_color);
                                    }
                                    for k in 0..run_len { drawn[col + k] = true; }
                                    col += run_len;
                                }
                                _ => { col += 1; }
                            }
                        }
                        // ── Draw remaining cells (Rest + orphan Hold) ────────────
                        for col in 0..LAUNCHPAD_COLS {
                            if drawn[col] { continue; }
                            let state = RhythmicState::from_usize(row_data[col] as usize);
                            let cell_color = match state {
                                RhythmicState::Rest => egui::Color32::from_gray(18),
                                RhythmicState::Hold => egui::Color32::from_gray(40), // orphan hold
                                _ => egui::Color32::from_gray(18),
                            };
                            let x = rect.left() + label_w + col as f32 * (cell_w + gap);
                            painter.rect_filled(
                                egui::Rect::from_min_size(egui::pos2(x, row_y), egui::vec2(cell_w, cell_h)),
                                2.0, cell_color,
                            );
                        }
                    }

                    // ── Playhead: vertical line over current column ───────────
                    let ph_x = rect.left() + label_w
                        + current_col as f32 * (cell_w + gap)
                        + cell_w * 0.5;
                    painter.line_segment(
                        [egui::pos2(ph_x, rect.top()), egui::pos2(ph_x, rect.bottom())],
                        egui::Stroke::new(1.5, egui::Color32::from_rgba_premultiplied(255, 255, 255, 80)),
                    );
                }
            }

            // ── Timeline visualization ────────────────────────────────────────
            if markov_active {
                // Show timeline UI if the scene has timeline sections (even if disabled).
                if !self.timeline.sections.is_empty() {
                    {
                        ui.separator();
                        ui.horizontal(|ui| {
                            // On/Off toggle.
                            let label = if self.timeline.active { "Timeline ON" } else { "Timeline OFF" };
                            let color = if self.timeline.active {
                                egui::Color32::from_rgb(0, 200, 140)
                            } else {
                                egui::Color32::GRAY
                            };
                            if ui.button(egui::RichText::new(label).strong().color(color)).clicked() {
                                self.timeline.active = !self.timeline.active;
                            }

                            if self.timeline.active {
                                if let Some(ref status) = self.timeline_status {
                                    ui.label(format!(
                                        "  {}  ({}/{})",
                                        status.section_name,
                                        status.phrase_in_sect,
                                        status.section_phrases,
                                    ));
                                    if status.in_transition {
                                        ui.label(
                                            egui::RichText::new(" CROSSFADE")
                                                .small()
                                                .color(egui::Color32::from_rgb(255, 200, 80)),
                                        );
                                    }
                                    ui.label(format!(
                                        "  [{}/{}]",
                                        status.cursor + 1,
                                        status.section_count,
                                    ));
                                }
                                if self.timeline.loop_mode {
                                    ui.label(
                                        egui::RichText::new(" ⟳")
                                            .color(egui::Color32::from_rgb(120, 200, 255)),
                                    );
                                }
                            }
                        });

                        // Section bar: proportional blocks for each section.
                        let status_ref = self.timeline_status.as_ref();
                        let tl_active = self.timeline.active;
                        let total_phrases = self.timeline.total_phrases().max(1) as f32;
                        let bar_width = ui.available_width().min(600.0);
                        let bar_height = 22.0;
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(bar_width, bar_height),
                            egui::Sense::hover(),
                        );
                        let painter = ui.painter_at(rect);
                        let bg = if tl_active { egui::Color32::from_gray(25) } else { egui::Color32::from_gray(15) };
                        painter.rect_filled(rect, 3.0, bg);

                        let section_colors = [
                            egui::Color32::from_rgb(60, 100, 160),
                            egui::Color32::from_rgb(80, 140, 100),
                            egui::Color32::from_rgb(160, 120, 60),
                            egui::Color32::from_rgb(140, 70, 100),
                            egui::Color32::from_rgb(90, 80, 150),
                            egui::Color32::from_rgb(60, 140, 140),
                            egui::Color32::from_rgb(150, 100, 70),
                            egui::Color32::from_rgb(100, 130, 80),
                        ];

                        let mut x = rect.left();
                        for (i, section) in self.timeline.sections.iter().enumerate() {
                            let w = (section.phrases as f32 / total_phrases) * bar_width;
                            let is_current = tl_active && status_ref.map_or(false, |s| i == s.cursor);
                            let base_color = section_colors[i % section_colors.len()];
                            // Dim sections when timeline is off.
                            let base_color = if !tl_active {
                                egui::Color32::from_rgb(
                                    base_color.r() / 2,
                                    base_color.g() / 2,
                                    base_color.b() / 2,
                                )
                            } else {
                                base_color
                            };
                            let color = if is_current {
                                egui::Color32::from_rgb(
                                    (base_color.r() as u16 + 60).min(255) as u8,
                                    (base_color.g() as u16 + 60).min(255) as u8,
                                    (base_color.b() as u16 + 60).min(255) as u8,
                                )
                            } else {
                                base_color
                            };

                            let section_rect = egui::Rect::from_min_size(
                                egui::pos2(x, rect.top()),
                                egui::vec2(w, bar_height),
                            );
                            painter.rect_filled(section_rect, 2.0, color);

                            // Section name label.
                            if w > 30.0 {
                                let text_color = if tl_active { egui::Color32::WHITE } else { egui::Color32::from_gray(120) };
                                painter.text(
                                    section_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    &section.name,
                                    egui::FontId::proportional(10.0),
                                    text_color,
                                );
                            }

                            // Progress + playhead (only when active).
                            if let Some(ref status) = status_ref {
                                if is_current && status.section_phrases > 0 {
                                    let progress_w = w * status.section_progress;
                                    let progress_rect = egui::Rect::from_min_size(
                                        egui::pos2(x, rect.top()),
                                        egui::vec2(progress_w, bar_height),
                                    );
                                    painter.rect_filled(
                                        progress_rect,
                                        2.0,
                                        egui::Color32::from_rgba_premultiplied(255, 255, 255, 30),
                                    );
                                    let ph_x = x + progress_w;
                                    painter.line_segment(
                                        [egui::pos2(ph_x, rect.top()), egui::pos2(ph_x, rect.bottom())],
                                        egui::Stroke::new(2.0, egui::Color32::WHITE),
                                    );
                                }

                                // Transition region indicator.
                                if is_current && section.transition_phrases > 0 {
                                    let trans_w = (section.transition_phrases as f32
                                        / section.phrases.max(1) as f32)
                                        * w;
                                    let trans_rect = egui::Rect::from_min_size(
                                        egui::pos2(x, rect.bottom() - 3.0),
                                        egui::vec2(trans_w, 3.0),
                                    );
                                    painter.rect_filled(
                                        trans_rect,
                                        1.0,
                                        egui::Color32::from_rgb(255, 200, 80),
                                    );
                                }
                            }

                            x += w;
                        }
                    }
                }
            }

            // ── Probability matrix heatmaps ───────────────────────────────────
            if markov_active {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Matrices:").strong());
                    for (i, lbl) in ["Harmonic", "Rhythmic", "Melodic"].iter().enumerate() {
                        let sel = self.markov_heatmap_tab == i;
                        let color = if sel { egui::Color32::from_rgb(0, 200, 140) } else { egui::Color32::GRAY };
                        if ui.button(egui::RichText::new(*lbl).color(color)).clicked() {
                            self.markov_heatmap_tab = i;
                        }
                    }
                    ui.label(egui::RichText::new("(from→to)").small().weak());
                });
                if let Some(ms) = &self.markov_shared_cache {
                    let moods = [&MOOD_CALM, &MOOD_TENSE, &MOOD_DARK, &MOOD_EUPHORIC, &MOOD_COSMIC, &MOOD_GRAVITY];
                    match self.markov_heatmap_tab {
                        0 => {
                            // Harmonic: 7×7
                            let raw = ms.mood.blend_harmonic(&moods);
                            let mat: Vec<Vec<f32>> = raw.iter().map(|r| r[..HARMONIC_STATES].to_vec()).collect();
                            draw_heatmap(ui, &mat, HarmonicFunction::LABELS, HarmonicFunction::LABELS,
                                         HARMONIC_STATES, egui::Color32::from_rgb(60, 120, 220));
                        }
                        1 => {
                            // Rhythmic: 5×5
                            let raw = ms.mood.blend_rhythmic(&moods);
                            let mat: Vec<Vec<f32>> = raw.iter().map(|r| r[..RHYTHMIC_STATES].to_vec()).collect();
                            draw_heatmap(ui, &mat, RhythmicState::LABELS, RhythmicState::LABELS,
                                         RHYTHMIC_STATES, egui::Color32::from_rgb(220, 140, 40));
                        }
                        _ => {
                            // Melodic: 7×7
                            let raw = ms.mood.blend_melodic(&moods);
                            let mat: Vec<Vec<f32>> = raw.iter().map(|r| r[..MELODIC_STATES].to_vec()).collect();
                            let deg_labels: &[&str] = &["1", "2", "3", "4", "5", "6", "7"];
                            draw_heatmap(ui, &mat, deg_labels, deg_labels,
                                         MELODIC_STATES, egui::Color32::from_rgb(160, 60, 220));
                        }
                    }
                }
            }

            // ── MANUAL content ────────────────────────────────────────────────
            if !markov_active {
                // Track selector
                ui.horizontal(|ui| {
                    ui.label("Track:");
                    for i in 0..TRACK_COUNT {
                        let selected = self.active_track == i;
                        let color = if selected { egui::Color32::from_rgb(0, 200, 140) } else { egui::Color32::GRAY };
                        if ui.button(egui::RichText::new(format!("Track {}", i + 1)).color(color).strong()).clicked() {
                            let prev: Vec<u8> = self.held_midi[self.active_track].drain().collect();
                            for m in prev { self.push_note_off(m); }
                            self.active_track = i;
                        }
                    }
                });
                // Patch slot
                ui.horizontal(|ui| {
                    ui.label("Patch:");
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
                                    ui.selectable_value(&mut self.track_patch_choice[ti], i, format!("{} / {}", p.category, p.name));
                                }
                            });
                        if ui.button("Load").clicked() {
                            self.pending_patch_load.push_back((ti, self.track_patch_choice[ti]));
                        }
                    }
                });
                // Track basic controls
                ui.horizontal(|ui| {
                    let _ = synth_ui::knob(ui, "Volume", &mut self.track_vol_ui[ti], 0.0, 1.0);
                    let _ = synth_ui::knob(ui, "Cutoff", &mut self.track_cutoff_ui[ti], 80.0, 18000.0);
                    let _ = synth_ui::knob(ui, "Resonance", &mut self.track_resonance_ui[ti], 0.1, 10.0);
                    let _ = synth_ui::knob(ui, "Shimmer", &mut self.track_shimmer_send_ui[ti], 0.0, 1.0);
                    let _ = synth_ui::knob(ui, "Crystal", &mut self.track_crystal_send_ui[ti], 0.0, 1.0);
                });
                ui.separator();
                // ARP + Walker
                ui.columns(2, |cols| {
                    let arp_on = self.arp_enabled_ui[ti];
                    cols[0].horizontal(|ui| {
                        let lbl = egui::RichText::new("ARP").strong()
                            .color(if arp_on { egui::Color32::from_rgb(0,220,160) } else { egui::Color32::GRAY });
                        if ui.button(lbl).clicked() {
                            self.arp_enabled_ui[ti] = !arp_on;
                            if !self.arp_enabled_ui[ti] {
                                let _ = self.control.try_send(ControlEvent::ChordHold { track: ti as u8, notes: Vec::new() });
                            }
                        }
                        if ui.button("RST").clicked() {
                            let _ = self.control.try_send(ControlEvent::ArpRestart { track: ti as u8 });
                        }
                        let hold = self.arp_hold_ui[ti];
                        let hl = egui::RichText::new("HOLD").color(if hold { egui::Color32::from_rgb(255,200,0) } else { egui::Color32::GRAY });
                        if ui.button(hl).clicked() {
                            self.arp_hold_ui[ti] = !hold;
                            if !self.arp_hold_ui[ti] {
                                let _ = self.control.try_send(ControlEvent::ChordHold { track: ti as u8, notes: Vec::new() });
                            }
                        }
                    });
                    cols[0].add_enabled_ui(arp_on, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("BPM:");
                            if self.transport_sync.clock_sync_enabled { self.arp_bpm[ti] = self.transport_sync.bpm_f32(); }
                            ui.add_enabled_ui(!self.transport_sync.clock_sync_enabled, |ui| {
                                ui.add(egui::Slider::new(&mut self.arp_bpm[ti], 20.0..=300.0));
                            });
                            if self.transport_sync.clock_sync_enabled { ui.label(egui::RichText::new("SYNC").small().weak()); }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Div:");
                            for (i, &lbl) in ClockDiv::LABELS.iter().enumerate() {
                                if ui.selectable_label(self.arp_division[ti] == i as u8, lbl).clicked() { self.arp_division[ti] = i as u8; }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Mode:");
                            for (i, &lbl) in ArpMode::LABELS.iter().enumerate() {
                                if ui.selectable_label(self.arp_mode[ti] == i as u8, lbl).clicked() { self.arp_mode[ti] = i as u8; }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Oct:");
                            for oct in 1u8..=4 {
                                if ui.selectable_label(self.arp_oct[ti] == oct, oct.to_string()).clicked() { self.arp_oct[ti] = oct; }
                            }
                            ui.label(" Gate:");
                            ui.add(egui::Slider::new(&mut self.arp_gate[ti], 0.05..=1.0));
                        });
                    });
                    let walk_on = self.walker_enabled_ui[ti];
                    cols[1].horizontal(|ui| {
                        let lbl = egui::RichText::new("WALKER").strong()
                            .color(if walk_on { egui::Color32::from_rgb(100,180,255) } else { egui::Color32::GRAY });
                        if ui.button(lbl).clicked() { self.walker_enabled_ui[ti] = !walk_on; }
                        if ui.button("RST").clicked() {
                            let _ = self.control.try_send(ControlEvent::WalkerRestart { track: ti as u8 });
                        }
                    });
                    cols[1].add_enabled_ui(walk_on, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("BPM:");
                            if self.transport_sync.clock_sync_enabled { self.walker_bpm[ti] = self.transport_sync.bpm_f32(); }
                            ui.add_enabled_ui(!self.transport_sync.clock_sync_enabled, |ui| {
                                ui.add(egui::Slider::new(&mut self.walker_bpm[ti], 20.0..=300.0));
                            });
                            if self.transport_sync.clock_sync_enabled { ui.label(egui::RichText::new("SYNC").small().weak()); }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Div:");
                            for (i, &lbl) in ClockDiv::LABELS.iter().enumerate() {
                                if ui.selectable_label(self.walker_div[ti] == i as u8, lbl).clicked() { self.walker_div[ti] = i as u8; }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Scale:");
                            for (i, &lbl) in Scale::LABELS.iter().enumerate() {
                                if ui.selectable_label(self.walker_scale[ti] == i as u8, lbl).clicked() { self.walker_scale[ti] = i as u8; }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Root:");
                            ui.add(egui::Slider::new(&mut self.walker_root[ti], 36u8..=84));
                            ui.label(" Oct:");
                            for oct in 1u8..=3 {
                                if ui.selectable_label(self.walker_oct[ti] == oct, oct.to_string()).clicked() { self.walker_oct[ti] = oct; }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Gate:");
                            ui.add(egui::Slider::new(&mut self.walker_gate[ti], 0.05..=1.0));
                        });
                    });
                });
                // Generative sub-modes
                ui.separator();
                ui.label(egui::RichText::new("GENERATIVE").strong());
                ui.horizontal(|ui| {
                    ui.label("Mode:");
                    let sub_modes: &[(u8, &str, egui::Color32)] = &[
                        (GenerativeMode::Off as u8,       "Off",        egui::Color32::GRAY),
                        (GenerativeMode::Euclidean as u8, "Euclidean",  egui::Color32::from_rgb(255, 160, 60)),
                        (GenerativeMode::ProbTable as u8, "Prob.Table", egui::Color32::from_rgb(160, 220, 255)),
                        (GenerativeMode::ScaleWalk as u8, "ScaleWalk",  egui::Color32::from_rgb(180, 255, 180)),
                    ];
                    for &(val, lbl, color) in sub_modes {
                        let active = self.gen_mode_ui[ti] == val;
                        let text = egui::RichText::new(lbl).color(if active { color } else { egui::Color32::GRAY });
                        if ui.button(text).clicked() { self.gen_mode_ui[ti] = val; }
                    }
                });
                let mode = GenerativeMode::from_u8(self.gen_mode_ui[ti]);
                if mode == GenerativeMode::Euclidean {
                    ui.horizontal(|ui| {
                        ui.label("Hits:");
                        ui.add(egui::Slider::new(&mut self.euclidean_hits_ui[ti], 1u8..=self.euclidean_steps_ui[ti]));
                        ui.label("Steps:");
                        ui.add(egui::Slider::new(&mut self.euclidean_steps_ui[ti], 1u8..=32));
                        ui.label("Root:");
                        ui.add(egui::Slider::new(&mut self.euclidean_root_ui[ti], 24u8..=96));
                        ui.label("Rot:");
                        ui.add(egui::Slider::new(&mut self.euclidean_rotation_ui[ti], 0u8..=31));
                    });
                }
                if mode == GenerativeMode::ProbTable {
                    ui.horizontal(|ui| {
                        ui.label("Tension:");
                        ui.add(egui::Slider::new(&mut self.prob_table_tension_ui[ti], 0.0f32..=2.0).step_by(0.01));
                        ui.label(egui::RichText::new("(0=silent  1=as-set  2=always)").small().weak());
                    });
                }
                ui.separator();
            } // end manual

            // ── Effects (always visible) ──────────────────────────────────────
            ui.horizontal(|ui| {
                let col = egui::Color32::from_rgb(120, 200, 255);
                let lbl = egui::RichText::new("SHIMMER").strong()
                    .color(if self.shimmer_on { col } else { egui::Color32::GRAY });
                if ui.button(lbl).clicked() { self.shimmer_on = !self.shimmer_on; }
                if synth_ui::knob(ui, "Mix", &mut self.shimmer_mix, 0.0, 1.0) {}
                if synth_ui::knob(ui, "Shim", &mut self.shimmer_amt, 0.0, 1.0) {}
                if synth_ui::knob(ui, "Size", &mut self.shimmer_size, 0.0, 1.0) {}
                if synth_ui::knob(ui, "Damp", &mut self.shimmer_damp, 0.0, 1.0) {}
                ui.label("Pitch:");
                for (i, lbl) in ["0", "+12", "+24"].iter().enumerate() {
                    if ui.selectable_label(self.shimmer_pitch == i as u8, *lbl).clicked() { self.shimmer_pitch = i as u8; }
                }
            });
            ui.horizontal(|ui| {
                let col = egui::Color32::from_rgb(255, 170, 90);
                let lbl = egui::RichText::new("CRYSTAL").strong()
                    .color(if self.crystal_on { col } else { egui::Color32::GRAY });
                if ui.button(lbl).clicked() { self.crystal_on = !self.crystal_on; }
                if synth_ui::knob(ui, "Mix", &mut self.crystal_mix, 0.0, 1.0) {}
                if synth_ui::knob(ui, "Grain", &mut self.crystal_grain_ms, 10.0, 400.0) {}
                if synth_ui::knob(ui, "Scatter", &mut self.crystal_scatter, 0.0, 1.0) {}
                // Delay — free knob or BPM-synced division
                let sync_lbl = if self.crystal_delay_sync_ui { "SYNC" } else { "FREE" };
                if ui.small_button(sync_lbl).on_hover_text("Toggle between free delay time and BPM-synced note division.").clicked() {
                    self.crystal_delay_sync_ui = !self.crystal_delay_sync_ui;
                }
                if self.crystal_delay_sync_ui {
                    egui::ComboBox::from_id_salt("crystal_delay_div")
                        .selected_text(synth_common::ClockDivision::from_u8(self.crystal_delay_division_ui).label())
                        .width(56.0)
                        .show_ui(ui, |ui| {
                            for div in synth_common::ClockDivision::ALL {
                                ui.selectable_value(&mut self.crystal_delay_division_ui, div.to_u8(), div.label());
                            }
                        });
                } else {
                    if synth_ui::knob(ui, "Delay", &mut self.crystal_delay_ms, 20.0, 1200.0) {}
                }
                if synth_ui::knob(ui, "Feedback", &mut self.crystal_feedback, 0.0, 0.95) {}
                ui.label("Pitch:");
                for (i, lbl) in ["0.5x", "1x", "2x", "4x"].iter().enumerate() {
                    if ui.selectable_label(self.crystal_pitch == i as u8, *lbl).clicked() { self.crystal_pitch = i as u8; }
                }
            });
            ui.separator();

            // ── Macros ────────────────────────────────────────────────────────
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

            // ── Scene ─────────────────────────────────────────────────────────
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
                    self.pending_scene_save = Some(PendingSceneSave {
                        path: scene_path.clone(),
                        name: self.scene_name.trim().to_string(),
                        bpm: self.scene_bpm,
                        key: self.scene_key,
                        scale_minor: self.scene_scale_minor,
                    });
                }
                if ui.button("Load Scene").clicked() {
                    self.pending_scene_load_path = Some(scene_path.clone());
                }
                if ui.button("Refresh").clicked() {
                    self.scene_files = Self::list_scene_files();
                    if self.scene_selected >= self.scene_files.len() { self.scene_selected = 0; }
                    self.scene_preview_patch_names = if self.scene_files.is_empty() {
                        std::array::from_fn(|_| "—".to_string())
                    } else {
                        Self::scene_preview_for_name(&self.scene_files[self.scene_selected])
                    };
                }
                if !self.scene_files.is_empty() {
                    let prev = self.scene_selected;
                    egui::ComboBox::from_id_salt("scene_file_list")
                        .selected_text(self.scene_files[self.scene_selected].clone())
                        .show_ui(ui, |ui| {
                            for (i, name) in self.scene_files.iter().enumerate() {
                                ui.selectable_value(&mut self.scene_selected, i, name.clone());
                            }
                        });
                    if self.scene_selected != prev {
                        self.scene_preview_patch_names = Self::scene_preview_for_name(&self.scene_files[self.scene_selected]);
                    }
                    if ui.button("Load Selected").clicked() {
                        let name = self.scene_files[self.scene_selected].clone();
                        self.pending_scene_load_path = Some(format!("scenes/{name}.json"));
                    }
                }
                if !self.scene_status.is_empty() {
                    ui.label(egui::RichText::new(&self.scene_status).small());
                }
            });
            if !self.scene_files.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Scene Slots:").small().weak());
                    for i in 0..TRACK_COUNT {
                        ui.label(egui::RichText::new(format!("T{} {}", i + 1, self.scene_preview_patch_names[i])).small());
                    }
                });
            }
            // ── Recording ────────────────────────────────────────────────────
            ui.horizontal(|ui| {
                let is_recording = self.recorder_sink.lock().map(|g| g.is_some()).unwrap_or(false);
                if is_recording {
                    if ui.button(egui::RichText::new("■ Stop Recording").color(egui::Color32::RED)).clicked() {
                        if let Ok(mut guard) = self.recorder_sink.lock() {
                            if let Some(rec) = guard.take() {
                                let path = rec.path.clone();
                                match rec.stop() {
                                    Ok(()) => self.rec_status = format!("Saved: {path}"),
                                    Err(e) => self.rec_status = format!("Record error: {e}"),
                                }
                            }
                        }
                    }
                } else if ui.button("● Record WAV").clicked() {
                    let default_path = {
                        let name = if self.scene_name.trim().is_empty() { "recording" } else { self.scene_name.trim() };
                        format!("recordings/{name}.wav")
                    };
                    let path = rfd::FileDialog::new()
                        .set_title("Save recording as")
                        .set_file_name(std::path::Path::new(&default_path).file_name().and_then(|n| n.to_str()).unwrap_or("recording.wav"))
                        .add_filter("WAV audio", &["wav"])
                        .save_file()
                        .map(|p| p.to_string_lossy().to_string());
                    if let Some(path) = path {
                        if let Some(parent) = std::path::Path::new(&path).parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        match recorder::Recorder::start(path.clone(), self.sample_rate) {
                            Ok(rec) => {
                                if let Ok(mut guard) = self.recorder_sink.lock() {
                                    *guard = Some(rec);
                                }
                                self.rec_status = format!("Recording → {path}");
                            }
                            Err(e) => self.rec_status = format!("Record start failed: {e}"),
                        }
                    }
                }
                if !self.rec_status.is_empty() {
                    ui.label(egui::RichText::new(&self.rec_status).small());
                }

                ui.separator();

                // MIDI recording
                let midi_recording = self.midi_recorder.is_some();
                if midi_recording {
                    if ui.button(egui::RichText::new("■ Stop MIDI Rec").color(egui::Color32::from_rgb(255, 140, 0))).clicked() {
                        // Pull the sink out of the shared slot first so the audio thread stops sending.
                        if let Ok(mut guard) = self.midi_sink_shared.lock() {
                            *guard = None;
                        }
                        if let Some(rec) = self.midi_recorder.take() {
                            let path = rec.path.clone();
                            match rec.stop() {
                                Ok(()) => self.midi_rec_status = format!("MIDI saved: {path}"),
                                Err(e) => self.midi_rec_status = format!("MIDI error: {e}"),
                            }
                        }
                    }
                } else if ui.button("● Record MIDI").clicked() {
                    let default_name = if self.scene_name.trim().is_empty() { "recording" } else { self.scene_name.trim() };
                    let path = rfd::FileDialog::new()
                        .set_title("Save MIDI recording as")
                        .set_file_name(&format!("{default_name}.mid"))
                        .add_filter("MIDI file", &["mid"])
                        .save_file()
                        .map(|p| p.to_string_lossy().to_string());
                    if let Some(path) = path {
                        if let Some(parent) = std::path::Path::new(&path).parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let bpm = self.beat_clock_shared.bpm() as f32;
                        match midi_recorder::MidiRecorder::start(path.clone(), self.sample_rate, bpm, TRACK_COUNT) {
                            Ok((rec, sink)) => {
                                if let Ok(mut guard) = self.midi_sink_shared.lock() {
                                    *guard = Some(sink);
                                }
                                self.midi_recorder = Some(rec);
                                self.midi_rec_status = format!("MIDI recording → {path}");
                            }
                            Err(e) => self.midi_rec_status = format!("MIDI start failed: {e}"),
                        }
                    }
                }
                if !self.midi_rec_status.is_empty() {
                    ui.label(egui::RichText::new(&self.midi_rec_status).small());
                }
            });

            if self.pending_scene_load.is_none() {
                if let Some(path) = self.pending_scene_load_path.take() {
                    match load_scene_json(&path) {
                        Ok(scene) => self.pending_scene_load = Some((path, scene)),
                        Err(e) => self.scene_status = format!("Load failed: {e}"),
                    }
                }
            }

            // ── Engine write-back ─────────────────────────────────────────────
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
                    self.beat_clock_shared.set_bpm(bpm);
                }
                self.beat_clock_shared.set_swing(self.swing_ui / 100.0);

                // Write generative config back to engine (Markov is global — propagate to all tracks)
                for t in 0..TRACK_COUNT {
                    eng.generative_modes[t].store(self.gen_mode_ui[t], Ordering::Relaxed);
                }

                // Write Markov shared config
                {
                    let ms = &eng.markov_shared;
                    ms.root.store(self.markov_root_ui, Ordering::Relaxed);
                    ms.scale.store(self.markov_scale_ui, Ordering::Relaxed);
                    ms.density.set(self.markov_density_ui);
                    ms.bars_per_phrase.store(self.markov_bars_per_phrase_ui, Ordering::Relaxed);
                    for i in 0..TRACK_COUNT {
                        ms.voice_density[i].set(self.markov_voice_density_ui[i]);
                        ms.roles[i].store(self.markov_voice_role_ui[i], Ordering::Relaxed);
                        ms.voice_enabled[i].store(self.markov_voice_enabled_ui[i], Ordering::Relaxed);
                        // LFO
                        let track = &eng.tracks[i];
                        let depth = if self.lfo_enabled_ui[i] { self.lfo_depth_ui[i] } else { 0.0 };
                        track.lfo_depth.set(depth);
                        track.lfo_dest.store(self.lfo_dest_ui[i], Ordering::Relaxed);
                        track.lfo_shape.store(self.lfo_shape_ui[i], Ordering::Relaxed);
                        track.lfo_sync.store(self.lfo_sync_ui[i] as u8, Ordering::Relaxed);
                        track.lfo_division.store(self.lfo_division_ui[i], Ordering::Relaxed);
                        if !self.lfo_sync_ui[i] {
                            track.lfo_rate.set(self.lfo_rate_ui[i]);
                        }
                        // Motif lock
                        ms.motif_lock[i].store(self.motif_lock_ui[i], Ordering::Relaxed);
                        ms.motif_length[i].store(self.motif_length_ui[i], Ordering::Relaxed);
                    }
                    // Normalize and write mood blend
                    let total: f32 = self.markov_mood_ui.iter().sum();
                    let norm = if total > 0.001 { total } else { 1.0 };
                    let normalized: [f32; N_MOODS] = std::array::from_fn(|i| self.markov_mood_ui[i] / norm);
                    ms.mood.set(&normalized);
                    ms.chord_attraction.set(self.markov_chord_attraction_ui);
                    ms.bass_lock.store(self.markov_bass_lock_ui, Ordering::Relaxed);
                    ms.dissonance_resolve.store(self.markov_dissonance_resolve_ui, Ordering::Relaxed);
                    ms.dissonance_threshold.store(self.markov_dissonance_threshold_ui, Ordering::Relaxed);
                    ms.register_drift.set(self.markov_register_drift_ui);
                    ms.clock_div.store(self.markov_clock_div_ui.max(1), Ordering::Relaxed);
                    // Write harmonic sequence
                    let seq_len = self.markov_seq_len_ui.clamp(1, 8);
                    ms.seq_len.store(seq_len as u8, Ordering::Relaxed);
                    for i in 0..seq_len {
                        ms.seq_roots[i].store(self.markov_seq_roots_ui[i], Ordering::Relaxed);
                        ms.seq_scales[i].store(self.markov_seq_scales_ui[i], Ordering::Relaxed);
                        ms.seq_phrases[i].store(self.markov_seq_phrases_ui[i].max(1), Ordering::Relaxed);
                    }
                    // If static mode (len=1), keep root/scale in sync with sequence slot 0
                    if seq_len == 1 {
                        ms.seq_roots[0].store(self.markov_root_ui, Ordering::Relaxed);
                        ms.seq_scales[0].store(self.markov_scale_ui, Ordering::Relaxed);
                    }
                }
                // Per-voice track properties (volume, sends) — written for all tracks in Markov mode
                for i in 0..TRACK_COUNT {
                    eng.tracks[i].track_vol.set(self.track_vol_ui[i].clamp(0.0, 1.0));
                    eng.tracks[i].shimmer_send.set(self.track_shimmer_send_ui[i].clamp(0.0, 1.0));
                    eng.tracks[i].crystal_send.set(self.track_crystal_send_ui[i].clamp(0.0, 1.0));
                }
                let ecfg = &eng.euclidean_configs[ti];
                ecfg.hits.store(self.euclidean_hits_ui[ti], Ordering::Relaxed);
                ecfg.steps.store(self.euclidean_steps_ui[ti], Ordering::Relaxed);
                ecfg.root.store(self.euclidean_root_ui[ti], Ordering::Relaxed);
                ecfg.rotation.store(self.euclidean_rotation_ui[ti], Ordering::Relaxed);
                eng.prob_table_configs[ti].tension.set(self.prob_table_tension_ui[ti]);

                eng.shimmer.shimmer.set(self.shimmer_amt);
                eng.shimmer.size.set(self.shimmer_size);
                eng.shimmer.damp.set(self.shimmer_damp);
                eng.shimmer.pitch.store(self.shimmer_pitch, Ordering::Relaxed);
                eng.shimmer.mix.set(if self.shimmer_on {
                    self.shimmer_mix.clamp(0.0, 1.0)
                } else { 0.0 });

                eng.crystal.grain_ms.set(self.crystal_grain_ms);
                eng.crystal.scatter.set(self.crystal_scatter);
                // Delay: BPM-synced or free
                if self.crystal_delay_sync_ui {
                    let bpm = self.beat_clock_shared.bpm();
                    let div = synth_common::ClockDivision::from_u8(self.crystal_delay_division_ui);
                    eng.crystal.delay_ms.set(div.seconds(bpm) * 1000.0);
                } else {
                    eng.crystal.delay_ms.set(self.crystal_delay_ms);
                }
                eng.crystal.feedback.set(self.crystal_feedback);
                eng.crystal.pitch.store(self.crystal_pitch, Ordering::Relaxed);
                eng.crystal.mix.set(if self.crystal_on {
                    self.crystal_mix.clamp(0.0, 1.0)
                } else { 0.0 });

                if let Some((track_idx, patch_idx)) = self.pending_patch_load.pop_front() {
                    if patch_idx < self.patch_library.len() {
                        let p = &self.patch_library[patch_idx];
                        eng.apply_patch_to_track(track_idx, p.path.clone(), &p.patch);
                        self.scene_status = format!("Loaded patch '{}' on Track {}", p.name, track_idx + 1);
                    }
                }

                if let Some(req) = self.pending_scene_save.take() {
                    let scale = if req.scale_minor { "minor" } else { "major" };
                    let mut scene = eng.capture_scene(
                        req.name.clone(),
                        req.bpm,
                        req.key,
                        scale,
                    );
                    // Include Markov state when in Markov mode
                    let mode = GenerativeMode::from_u8(self.gen_mode_ui[0]);
                    if mode == GenerativeMode::Markov {
                        let mut ms = eng.capture_markov_scene(GenerativeMode::Markov as u8);
                        // Persist timeline sections if timeline is active.
                        if self.timeline.active && !self.timeline.sections.is_empty() {
                            ms.timeline = Some(self.timeline.sections.clone());
                            ms.timeline_loop = self.timeline.loop_mode;
                        }
                        scene.markov = Some(ms);
                    }
                    pending_scene_save = Some((req.path, scene, req.name));
                }

                if let Some((path, scene)) = self.pending_scene_load.take() {
                    self.scene_name = scene.name.clone();
                    self.scene_bpm = scene.bpm;
                    self.scene_key = scene.key % 12;
                    self.scene_scale_minor = scene.scale.eq_ignore_ascii_case("minor");
                    // Sync Markov UI state from scene before applying
                    if let Some(ms) = &scene.markov {
                        self.markov_root_ui = ms.root;
                        self.markov_scale_ui = ms.scale;
                        self.markov_density_ui = ms.density;
                        self.markov_bars_per_phrase_ui = ms.bars_per_phrase;
                        for i in 0..TRACK_COUNT {
                            if let Some(&r) = ms.voice_roles.get(i) { self.markov_voice_role_ui[i] = r; }
                            if let Some(&d) = ms.voice_densities.get(i) { self.markov_voice_density_ui[i] = d; }
                            if let Some(&e) = ms.voice_enabled.get(i) { self.markov_voice_enabled_ui[i] = e; }
                        }
                        for i in 0..N_MOODS {
                            if let Some(&w) = ms.mood.get(i) { self.markov_mood_ui[i] = w; }
                        }
                        self.markov_chord_attraction_ui = ms.chord_attraction;
                        self.markov_bass_lock_ui = ms.bass_lock;
                        self.markov_dissonance_resolve_ui = ms.dissonance_resolve;
                        self.markov_dissonance_threshold_ui = ms.dissonance_threshold;
                        self.markov_register_drift_ui = ms.register_drift;
                        self.markov_clock_div_ui = ms.clock_div;
                        let seq = &ms.harmonic_seq;
                        self.markov_seq_len_ui = seq.len().clamp(1, 8);
                        for (i, slot) in seq.iter().take(8).enumerate() {
                            self.markov_seq_roots_ui[i] = slot.root;
                            self.markov_seq_scales_ui[i] = slot.scale;
                            self.markov_seq_phrases_ui[i] = slot.phrases;
                        }
                        // Sync static root/scale from slot 0 if sequence present
                        if !seq.is_empty() {
                            self.markov_root_ui = seq[0].root;
                            self.markov_scale_ui = seq[0].scale;
                        }
                        for t in 0..TRACK_COUNT { self.gen_mode_ui[t] = ms.generative_mode; }

                        // ── Create Timeline from scene data ──
                        if let Some(ref sections) = ms.timeline {
                            if !sections.is_empty() {
                                // Build base state from scene's markov config + scene globals for effects.
                                let mut base = ambient_engine::ResolvedState::snapshot_from_shared(&eng.markov_shared);
                                base.shimmer_mix = scene.global.shimmer_mix;
                                base.shimmer_amount = scene.global.shimmer_amount;
                                base.shimmer_size = scene.global.shimmer_size;
                                base.crystal_mix = scene.global.crystal_mix;
                                base.crystal_feedback = scene.global.crystal_feedback;
                                base.crystal_delay_ms = scene.global.crystal_delay_ms;
                                self.timeline = ambient_engine::Timeline::new(
                                    sections.clone(),
                                    ms.timeline_loop,
                                    base,
                                );
                                // Reset epoch tracking so we don't process stale phrase events.
                                self.timeline_last_epoch = eng.markov_shared.phrase_epoch.load(Ordering::Relaxed);
                            } else {
                                self.timeline = ambient_engine::Timeline::inactive();
                            }
                        } else {
                            self.timeline = ambient_engine::Timeline::inactive();
                        }
                    }
                    eng.apply_scene(&scene);
                    // Sync per-voice UI from restored scene values so the
                    // UI→engine write path doesn't overwrite them each frame.
                    for i in 0..TRACK_COUNT {
                        self.track_vol_ui[i]          = eng.tracks[i].track_vol.value();
                        self.track_shimmer_send_ui[i] = eng.tracks[i].shimmer_send.value();
                        self.track_crystal_send_ui[i] = eng.tracks[i].crystal_send.value();
                        // LFO — read back from TrackState so knobs match the patch
                        let lfo_depth = eng.tracks[i].lfo_depth.value();
                        self.lfo_depth_ui[i]    = lfo_depth;
                        self.lfo_enabled_ui[i]  = lfo_depth > 0.0;
                        self.lfo_dest_ui[i]     = eng.tracks[i].lfo_dest.load(Ordering::Relaxed);
                        self.lfo_shape_ui[i]    = eng.tracks[i].lfo_shape.load(Ordering::Relaxed);
                        self.lfo_rate_ui[i]     = eng.tracks[i].lfo_rate.value();
                        self.lfo_sync_ui[i]     = eng.tracks[i].lfo_sync.load(Ordering::Relaxed) != 0;
                        self.lfo_division_ui[i] = eng.tracks[i].lfo_division.load(Ordering::Relaxed);
                    }
                    // Auto-enable effect buses when the scene uses them
                    if scene.global.shimmer_mix > 0.0 { self.shimmer_on = true; }
                    if scene.global.crystal_mix > 0.0 { self.crystal_on = true; }
                    self.shimmer_mix         = scene.global.shimmer_mix;
                    self.shimmer_amt         = scene.global.shimmer_amount;
                    self.shimmer_size        = scene.global.shimmer_size;
                    self.crystal_mix         = scene.global.crystal_mix;
                    self.crystal_feedback    = scene.global.crystal_feedback;
                    self.crystal_delay_ms    = scene.global.crystal_delay_ms;
                    self.scene_status = format!("Loaded {path}");
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

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::Sample;

    #[test]
    fn fallback_buffer_decays_silence_f32() {
        let mut data = vec![0.0f32; 4];
        let mut last_out_l = 1.0f32;
        let mut last_out_r = -1.0f32;

        fill_buffer_with_silence(&mut data, 2, &mut last_out_l, &mut last_out_r);

        assert_eq!(data[0], 1.0);
        assert_eq!(data[1], -1.0);
        assert!((data[2] - 0.9995).abs() < 1e-6);
        assert!((data[3] + 0.9995).abs() < 1e-6);
        assert!(data[2].abs() < data[0].abs());
        assert!(data[3].abs() < data[1].abs());
        let expected = 1.0f32 * 0.9995 * 0.9995;
        assert!((last_out_l - expected).abs() < 1e-6);
        assert!((last_out_r + expected).abs() < 1e-6);
    }

    #[test]
    fn fallback_buffer_writes_stereo_i16() {
        let mut data = vec![0i16; 4];
        let mut last_out_l = 0.5f32;
        let mut last_out_r = -0.5f32;

        fill_buffer_with_silence(&mut data, 2, &mut last_out_l, &mut last_out_r);

        let left = i16::from_sample(0.5);
        let right = i16::from_sample(-0.5);
        assert_eq!(data[0], left);
        assert_eq!(data[1], right);
        assert!(data[2].abs() < data[0].abs());
        assert!(data[3].abs() < data[1].abs());
        let expected = 0.5f32 * 0.9995 * 0.9995;
        assert!((last_out_l - expected).abs() < 1e-6);
        assert!((last_out_r + expected).abs() < 1e-6);
    }
}
