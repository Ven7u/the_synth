use ambient_engine::{
    load_scene_json, AmbientEngine, Scene, SceneGlobal, MACRO_COUNT, TRACK_COUNT,
    BeatClock, BeatClockShared,
    MarkovEngine, GenerativeMode,
    EuclideanGen, ProbTableGen,
};
use bevy::prelude::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample, Stream};
use fundsp::prelude::midi_hz;
use fundsp::audiounit::AudioUnit;
use fundsp::prelude32::{shared, BlockRateAdapter, Shared};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use synth_control::{make_control_channel, ControlEvent, ControlReceiver, ControlSender, ParamId};
use synth_engine::arp::{ArpState, ScaleWalker};
use synth_engine::audio::{build_synth_graph, AudioState, VOICE_COUNT as SYNTH_VOICE_COUNT};

/// How a `SceneTransition` blends from the current scene to the target.
///
/// | Mode | Volume dip | Sound during transition |
/// |------|-----------|------------------------|
/// | `LinearFade` | V-shaped, reaches silence at midpoint | Audible dip |
/// | `EqualPowerFade` | Cosine curve, no perceived loudness dip | Smooth crossfade |
/// | `ParamMorph` | None — parameters lerp continuously | Seamless morph |
/// | `MorphWithFade` | Short equal-power dip only at patch-swap point | Best of both |
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SceneTransitionMode {
    /// Linear gain V-shape: fades to silence at midpoint, then back up.
    LinearFade,
    /// Equal-power cosine crossfade: no perceived loudness dip.
    #[default]
    EqualPowerFade,
    /// No volume change: continuous parameters (shimmer, crystal, sends, vol, cutoff)
    /// interpolate smoothly; patch parameters are swapped silently at the midpoint.
    ParamMorph,
    /// Parameter morph with a brief (~150 ms) equal-power dip at the midpoint
    /// to mask any waveform discontinuity from the patch swap.
    MorphWithFade,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynthBackendKind {
    Ambient,
    Mono,
}

#[derive(Resource, Clone)]
pub struct SynthBevyConfig {
    pub backend: SynthBackendKind,
    pub sample_rate_hz: f64,
    pub control_capacity: usize,
    pub scene_dir: String,
    pub initial_bpm: f32,
    /// Default transition mode used when `SynthEvent::SceneTransition` does not specify one.
    pub default_transition_mode: SceneTransitionMode,
}

impl Default for SynthBevyConfig {
    fn default() -> Self {
        Self {
            backend: SynthBackendKind::Ambient,
            sample_rate_hz: 44_100.0,
            control_capacity: 1024,
            scene_dir: "scenes".to_string(),
            initial_bpm: 120.0,
            default_transition_mode: SceneTransitionMode::EqualPowerFade,
        }
    }
}

#[derive(Resource, Clone)]
pub struct SynthParam {
    pub macro_values: [Shared; MACRO_COUNT],
    // Per-track handles (populated for Ambient backend; fallback shared(x) for Mono)
    pub track_vol: [Shared; TRACK_COUNT],
    pub track_cutoff: [Shared; TRACK_COUNT],
    pub track_shimmer_send: [Shared; TRACK_COUNT],
    pub track_crystal_send: [Shared; TRACK_COUNT],
    // Global bus handles
    pub shimmer_mix: Shared,
    pub shimmer_size: Shared,
    pub shimmer_amount: Shared,
    pub crystal_mix: Shared,
    pub crystal_feedback: Shared,
    pub master_vol: Shared,
    /// Phrase epoch — incremented by the audio thread on each phrase boundary.
    pub phrase_epoch: Arc<std::sync::atomic::AtomicUsize>,
    /// Bar epoch — incremented by the audio thread on each bar boundary.
    pub bar_epoch: Arc<std::sync::atomic::AtomicUsize>,
    /// Subdivision epoch — incremented on each 16th-note subdivision.
    pub subdivision_epoch: Arc<std::sync::atomic::AtomicUsize>,
    /// Set to `true` to force an immediate timeline section advance.
    /// Cleared automatically by the bridge system after processing.
    pub force_timeline_advance: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for SynthParam {
    fn default() -> Self {
        Self {
            macro_values: std::array::from_fn(|_| shared(0.0)),
            track_vol: std::array::from_fn(|_| shared(0.7)),
            track_cutoff: std::array::from_fn(|_| shared(3000.0)),
            track_shimmer_send: std::array::from_fn(|_| shared(0.0)),
            track_crystal_send: std::array::from_fn(|_| shared(0.0)),
            shimmer_mix: shared(0.0),
            shimmer_size: shared(0.6),
            shimmer_amount: shared(0.0),
            crystal_mix: shared(0.0),
            crystal_feedback: shared(0.35),
            master_vol: shared(0.7),
            phrase_epoch: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            bar_epoch: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            subdivision_epoch: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            force_timeline_advance: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

impl SynthParam {
    pub fn set_macro(&self, index: usize, value: f32) {
        if index < MACRO_COUNT {
            self.macro_values[index].set(value.clamp(0.0, 1.0));
        }
    }

    pub fn macro_value(&self, index: usize) -> f32 {
        if index < MACRO_COUNT {
            self.macro_values[index].value()
        } else {
            0.0
        }
    }
}

pub enum SynthBackendRuntime {
    Ambient(Arc<Mutex<AmbientEngine>>),
    Mono(Arc<AudioState>),
}

/// Snapshot of the continuous engine parameters at the moment a transition starts.
/// Used by ParamMorph / MorphWithFade to interpolate from old → new values.
#[derive(Clone)]
struct SceneSnapshot {
    global: SceneGlobal,
    track_vol: [f32; TRACK_COUNT],
    track_cutoff: [f32; TRACK_COUNT],
    track_shimmer_send: [f32; TRACK_COUNT],
    track_crystal_send: [f32; TRACK_COUNT],
}

#[derive(Clone)]
struct SceneTransitionPlan {
    scene: Scene,
    mode: SceneTransitionMode,
    total_frames: u32,
    frames_left: u32,
    applied_target: bool,
    /// Captured at transition start for morph modes.
    from_snapshot: Option<SceneSnapshot>,
}

#[derive(Resource)]
pub struct SynthBevyRuntime {
    pub backend: SynthBackendRuntime,
    pub control_tx: ControlSender,
    scene_transition: Option<Arc<Mutex<Option<SceneTransitionPlan>>>>,
    /// Shared clock used by the ambient audio thread's Markov engine.
    /// Writing BPM here syncs to the audio thread lock-free.
    pub ambient_beat_clock: Option<BeatClockShared>,
}

pub struct SynthAudioStream(pub Stream);

#[derive(Clone)]
enum PendingEngineAction {
    SetMacro { index: usize, value: f32 },
    ApplyScene(Scene),
    StartTransition { scene: Scene, frames: u32, mode: SceneTransitionMode },
}

#[derive(Resource, Default)]
struct PendingEngineActions {
    queue: VecDeque<PendingEngineAction>,
}

#[derive(Resource, Default)]
pub struct SynthTempo {
    pub bpm: f32,
}

#[derive(Message, Debug, Clone)]
pub enum SynthEvent {
    NoteOn { track: u8, pitch: u8, velocity: u8 },
    NoteOff { track: u8, pitch: u8 },
    ChordHold { track: u8, notes: Vec<u8> },
    SetMacro { index: u8, value: f32 },
    SetParam { track: u8, param: ParamId, value: f32 },
    SceneLoad { name: String },
    /// Transition to a named scene over `frames` samples using `mode`.
    /// `mode: None` falls back to `SynthBevyConfig::default_transition_mode`.
    SceneTransition { name: String, frames: u32, mode: Option<SceneTransitionMode> },
    Tempo { bpm: f32 },
    /// Immediately advance the timeline to the next section, ignoring remaining phrase count.
    /// The Bevy timeline resource (if any) handles the actual state update each frame.
    TimelineAdvance,
}

pub struct SynthPlugin;

impl Plugin for SynthPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SynthBevyConfig>()
            .init_resource::<SynthTempo>()
            .init_resource::<PendingEngineActions>()
            .add_message::<SynthEvent>()
            .add_systems(Startup, setup_synth_runtime)
            .add_systems(PostUpdate, (bevy_bridge_system, process_pending_engine_actions).chain());

        #[cfg(feature = "inspector")]
        app.add_plugins(inspector::SynthInspectorPlugin);
    }
}

fn setup_synth_runtime(world: &mut World) {
    let cfg = world.resource::<SynthBevyConfig>().clone();
    let mut tempo = world.resource_mut::<SynthTempo>();
    let actual_sr = detect_output_sample_rate().unwrap_or(cfg.sample_rate_hz);
    let (control_tx, control_rx) = make_control_channel(cfg.control_capacity);

    let (backend, synth_param, stream_res, scene_transition, ambient_beat_clock) = match cfg.backend {
        SynthBackendKind::Ambient => {
            let eng = Arc::new(Mutex::new(AmbientEngine::new(actual_sr)));
            let transition = Arc::new(Mutex::new(None));
            let synth_param = if let Ok(guard) = eng.lock() {
                SynthParam {
                    macro_values: std::array::from_fn(|i| guard.macro_values[i].clone()),
                    track_vol: std::array::from_fn(|i| guard.tracks[i].track_vol.clone()),
                    track_cutoff: std::array::from_fn(|i| guard.tracks[i].cutoff.clone()),
                    track_shimmer_send: std::array::from_fn(|i| guard.tracks[i].shimmer_send.clone()),
                    track_crystal_send: std::array::from_fn(|i| guard.tracks[i].crystal_send.clone()),
                    shimmer_mix: guard.shimmer.mix.clone(),
                    shimmer_size: guard.shimmer.size.clone(),
                    shimmer_amount: guard.shimmer.shimmer.clone(),
                    crystal_mix: guard.crystal.mix.clone(),
                    crystal_feedback: guard.crystal.feedback.clone(),
                    master_vol: guard.master_vol.clone(),
                    phrase_epoch: Arc::clone(&guard.markov_shared.phrase_epoch),
                    bar_epoch: Arc::clone(&guard.markov_shared.bar_epoch),
                    subdivision_epoch: Arc::clone(&guard.markov_shared.subdivision_epoch),
                    force_timeline_advance: Arc::clone(&guard.markov_shared.force_timeline_advance),
                }
            } else {
                SynthParam::default()
            };
            let stream_res = build_ambient_stream(
                Arc::clone(&eng),
                Arc::clone(&transition),
                control_rx,
                actual_sr,
                cfg.initial_bpm,
            );
            let (stream_and_clock, ambient_clock) = match stream_res {
                Ok((s, c)) => (Ok(s), Some(c)),
                Err(e) => (Err(e), None),
            };
            (
                SynthBackendRuntime::Ambient(eng),
                synth_param,
                stream_and_clock,
                Some(transition),
                ambient_clock,
            )
        }
        SynthBackendKind::Mono => {
            let state = Arc::new(AudioState::new());
            let stream_res = build_mono_stream(Arc::clone(&state), control_rx, actual_sr);
            (SynthBackendRuntime::Mono(state), SynthParam::default(), stream_res, None, None)
        }
    };

    let stream_opt = match stream_res {
        Ok(s) => {
            if let Err(e) = s.play() {
                warn!("synth-bevy: failed to start audio stream: {e}");
                None
            } else {
                Some(s)
            }
        }
        Err(e) => {
            warn!("synth-bevy: failed to build audio stream: {e}");
            None
        }
    };

    tempo.bpm = cfg.initial_bpm.max(1.0);
    drop(tempo);
    world.insert_resource(synth_param);
    world.insert_resource(SynthBevyRuntime {
        backend,
        control_tx,
        scene_transition,
        ambient_beat_clock,
    });
    if let Some(stream) = stream_opt {
        world.insert_non_send_resource(SynthAudioStream(stream));
    }
}

fn bevy_bridge_system(
    mut events: MessageReader<SynthEvent>,
    cfg: Res<SynthBevyConfig>,
    runtime: Option<Res<SynthBevyRuntime>>,
    params: Option<Res<SynthParam>>,
    mut pending: ResMut<PendingEngineActions>,
    mut tempo: ResMut<SynthTempo>,
) {
    let Some(runtime) = runtime else { return; };
    let Some(params) = params else { return; };

    for ev in events.read() {
        match ev {
            SynthEvent::NoteOn { track, pitch, velocity } => {
                let _ = runtime.control_tx.try_send(ControlEvent::NoteOn {
                    pitch: *pitch,
                    velocity: *velocity,
                    track: *track,
                });
            }
            SynthEvent::NoteOff { track, pitch } => {
                let _ = runtime.control_tx.try_send(ControlEvent::NoteOff {
                    pitch: *pitch,
                    track: *track,
                });
            }
            SynthEvent::ChordHold { track, notes } => {
                let _ = runtime.control_tx.try_send(ControlEvent::ChordHold {
                    track: *track,
                    notes: notes.clone(),
                });
            }
            SynthEvent::SetMacro { index, value } => {
                let idx = *index as usize;
                params.set_macro(idx, *value);
                if matches!(runtime.backend, SynthBackendRuntime::Ambient(_)) {
                    pending.queue.push_back(PendingEngineAction::SetMacro {
                        index: idx,
                        value: *value,
                    });
                }
            }
            SynthEvent::SetParam { track: _, param, value } => {
                let _ = runtime.control_tx.try_send(ControlEvent::SetParam {
                    param: *param,
                    value: *value,
                });
            }
            SynthEvent::SceneLoad { name } => {
                if matches!(runtime.backend, SynthBackendRuntime::Ambient(_)) {
                    let path = scene_path(&cfg.scene_dir, name);
                    match load_scene_json(&path) {
                        Ok(scene) => {
                            pending.queue.push_back(PendingEngineAction::ApplyScene(scene));
                        }
                        Err(e) => error!("synth-bevy: scene load failed '{}': {e}", path),
                    }
                }
            }
            SynthEvent::SceneTransition { name, frames, mode } => {
                if matches!(runtime.backend, SynthBackendRuntime::Ambient(_)) {
                    let path = scene_path(&cfg.scene_dir, name);
                    match load_scene_json(&path) {
                        Ok(scene) => {
                            let resolved_mode = mode.unwrap_or(cfg.default_transition_mode);
                            pending.queue.push_back(PendingEngineAction::StartTransition {
                                scene,
                                frames: (*frames).max(1),
                                mode: resolved_mode,
                            });
                        }
                        Err(e) => warn!("synth-bevy: scene transition load failed '{}': {e}", path),
                    }
                }
            }
            SynthEvent::Tempo { bpm } => {
                tempo.bpm = bpm.max(1.0);
            }
            SynthEvent::TimelineAdvance => {
                params.force_timeline_advance
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

fn process_pending_engine_actions(
    runtime: Option<Res<SynthBevyRuntime>>,
    mut pending: ResMut<PendingEngineActions>,
) {
    let Some(runtime) = runtime else { return; };
    let SynthBackendRuntime::Ambient(eng) = &runtime.backend else { return; };
    let Ok(mut guard) = eng.try_lock() else { return; };

    let mut iterations = 0usize;
    while iterations < 64 {
        let Some(action) = pending.queue.pop_front() else { break; };
        iterations += 1;
        match action {
            PendingEngineAction::SetMacro { index, value } => {
                guard.set_macro_value(index, value);
            }
            PendingEngineAction::ApplyScene(scene) => {
                if let Some(transition) = &runtime.scene_transition {
                    if let Ok(mut slot) = transition.try_lock() {
                        *slot = None;
                    } else {
                        pending.queue.push_front(PendingEngineAction::ApplyScene(scene));
                        break;
                    }
                }
                let scene_bpm = scene.bpm as f32;
                guard.apply_scene(&scene);
                // Sync BPM to the Markov beat clock so the audio thread plays at the right tempo.
                if let Some(clk) = &runtime.ambient_beat_clock {
                    clk.set_bpm(scene_bpm);
                }
            }
            PendingEngineAction::StartTransition { scene, frames, mode } => {
                if let Some(transition) = &runtime.scene_transition {
                    if let Ok(mut slot) = transition.try_lock() {
                        // Capture current state for morph modes.
                        let from_snapshot = match mode {
                            SceneTransitionMode::ParamMorph | SceneTransitionMode::MorphWithFade => {
                                Some(SceneSnapshot {
                                    global: SceneGlobal {
                                        master_vol: guard.master_vol.value(),
                                        shimmer_mix: guard.shimmer.mix.value(),
                                        shimmer_amount: guard.shimmer.shimmer.value(),
                                        shimmer_size: guard.shimmer.size.value(),
                                        shimmer_damp: guard.shimmer.damp.value(),
                                        shimmer_pitch: guard.shimmer.pitch.load(std::sync::atomic::Ordering::Relaxed),
                                        crystal_mix: guard.crystal.mix.value(),
                                        crystal_grain_ms: guard.crystal.grain_ms.value(),
                                        crystal_scatter: guard.crystal.scatter.value(),
                                        crystal_feedback: guard.crystal.feedback.value(),
                                        crystal_delay_ms: guard.crystal.delay_ms.value(),
                                        crystal_pitch: guard.crystal.pitch.load(std::sync::atomic::Ordering::Relaxed),
                                    },
                                    track_vol: std::array::from_fn(|i| guard.tracks[i].track_vol.value()),
                                    track_cutoff: std::array::from_fn(|i| guard.tracks[i].cutoff.value()),
                                    track_shimmer_send: std::array::from_fn(|i| guard.tracks[i].shimmer_send.value()),
                                    track_crystal_send: std::array::from_fn(|i| guard.tracks[i].crystal_send.value()),
                                })
                            }
                            _ => None,
                        };
                        *slot = Some(SceneTransitionPlan {
                            scene,
                            mode,
                            total_frames: frames,
                            frames_left: frames,
                            applied_target: false,
                            from_snapshot,
                        });
                    } else {
                        pending.queue.push_front(PendingEngineAction::StartTransition { scene, frames, mode });
                        break;
                    }
                } else {
                    guard.apply_scene(&scene);
                }
            }
        }
    }
}

fn scene_path(scene_dir: &str, name: &str) -> String {
    if name.ends_with(".json") {
        format!("{scene_dir}/{name}")
    } else {
        format!("{scene_dir}/{name}.json")
    }
}

fn detect_output_sample_rate() -> Option<f64> {
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let config = device.default_output_config().ok()?;
    Some(config.sample_rate().0 as f64)
}

fn build_ambient_stream(
    engine: Arc<Mutex<AmbientEngine>>,
    transition_plan: Arc<Mutex<Option<SceneTransitionPlan>>>,
    rx: ControlReceiver,
    sr: f64,
    initial_bpm: f32,
) -> anyhow::Result<(Stream, BeatClockShared)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No output device"))?;
    let config = device.default_output_config()?;

    match config.sample_format() {
        cpal::SampleFormat::F32 => {
            make_ambient_stream::<f32>(&device, &config.into(), engine, transition_plan, rx, sr, initial_bpm)
        }
        cpal::SampleFormat::I16 => {
            make_ambient_stream::<i16>(&device, &config.into(), engine, transition_plan, rx, sr, initial_bpm)
        }
        cpal::SampleFormat::U16 => {
            make_ambient_stream::<u16>(&device, &config.into(), engine, transition_plan, rx, sr, initial_bpm)
        }
        _ => anyhow::bail!("Unsupported sample format"),
    }
}

fn make_ambient_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    engine: Arc<Mutex<AmbientEngine>>,
    transition_plan: Arc<Mutex<Option<SceneTransitionPlan>>>,
    rx: ControlReceiver,
    sr: f64,
    initial_bpm: f32,
) -> anyhow::Result<(Stream, BeatClockShared)>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut voice_notes: [[Option<u8>; synth_engine::VOICE_COUNT]; TRACK_COUNT] =
        [[None; synth_engine::VOICE_COUNT]; TRACK_COUNT];
    let mut steal_idx: [usize; TRACK_COUNT] = [0; TRACK_COUNT];
    let mut lfo_phases: [f32; TRACK_COUNT] = [0.0; TRACK_COUNT];
    let mut arp_states: [ArpState; TRACK_COUNT] = std::array::from_fn(|_| ArpState::new());
    let mut walker_states: [ScaleWalker; TRACK_COUNT] = std::array::from_fn(|_| ScaleWalker::new());

    // Markov generative engine — drives note events from the audio thread.
    let mut markov_engine = MarkovEngine::new(TRACK_COUNT, 0xABE_BEEF_C0DE_1234_u64);
    let mut markov_subdiv_counter: u8 = 0;
    let mut euclidean_states: [EuclideanGen; TRACK_COUNT] = std::array::from_fn(|_| EuclideanGen::new());
    let mut prob_table_states: [ProbTableGen; TRACK_COUNT] = std::array::from_fn(|i| ProbTableGen::new(0xDEAD_BEEF ^ i as u64));
    let beat_clock_shared = BeatClockShared::new(initial_bpm);
    let beat_clock_shared_for_runtime = beat_clock_shared.clone();
    beat_clock_shared.set_playing(true);
    let mut beat_clock = BeatClock::default();
    let _was_playing = true; // Bevy ambient auto-plays

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let frames = data.len() / channels;
            let Ok(mut eng) = engine.try_lock() else {
                let z = T::from_sample(0.0_f32);
                for smp in data.iter_mut() {
                    *smp = z;
                }
                return;
            };
            let mut transition_guard = transition_plan.try_lock().ok();

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

            while let Ok(ev) = rx.try_recv() {
                match ev {
                    ControlEvent::NoteOn { pitch, velocity: _, track } => {
                        let ti = track as usize % TRACK_COUNT;
                        if eng.arp_configs[ti].enabled.load(std::sync::atomic::Ordering::Relaxed) {
                            arp_states[ti].note_on(pitch);
                        } else {
                            let slot = voice_notes[ti]
                                .iter()
                                .position(|n| n.is_none())
                                .unwrap_or_else(|| {
                                    let s = steal_idx[ti] % synth_engine::VOICE_COUNT;
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
                        if eng.arp_configs[ti].enabled.load(std::sync::atomic::Ordering::Relaxed) {
                            let hold = eng.arp_configs[ti].hold.load(std::sync::atomic::Ordering::Relaxed);
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
                        if let Some(p) = arp_states[ti].restart() {
                            for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                                if *note == Some(p) {
                                    eng.tracks[ti].voice_gates[slot].set(0.0);
                                    break;
                                }
                            }
                        }
                    }
                    ControlEvent::WalkerRestart { track } => {
                        let ti = track as usize % TRACK_COUNT;
                        if let Some(p) = walker_states[ti].restart() {
                            for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                                if *note == Some(p) {
                                    eng.tracks[ti].voice_gates[slot].set(0.0);
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            for ti in 0..TRACK_COUNT {
                for ev in [
                    arp_states[ti].tick(&eng.arp_configs[ti], frames, sr),
                    walker_states[ti].tick(&eng.walker_configs[ti], frames, sr),
                ] {
                    if let Some(p) = ev.note_off {
                        for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                            if *note == Some(p) {
                                eng.tracks[ti].voice_gates[slot].set(0.0);
                                break;
                            }
                        }
                    }
                    if let Some(p) = ev.note_on {
                        let slot = voice_notes[ti]
                            .iter()
                            .position(|n| n.is_none())
                            .unwrap_or_else(|| {
                                let s = steal_idx[ti] % synth_engine::VOICE_COUNT;
                                steal_idx[ti] += 1;
                                s
                            });
                        voice_notes[ti][slot] = Some(p);
                        eng.tracks[ti].voice_freq_targets[slot].set(midi_hz(p as f64) as f32);
                        eng.tracks[ti].voice_gates[slot].set(1.0);
                    }
                }
            }

            // ── Markov / generative tick (once per buffer) ───────────────────
            {
                let beat_ev = beat_clock.tick(frames, sr, &beat_clock_shared);
                let markov_active = GenerativeMode::from_u8(
                    eng.generative_modes[0].load(std::sync::atomic::Ordering::Relaxed),
                ) == GenerativeMode::Markov;

                if beat_ev.bar {
                    eng.markov_shared.bar_epoch.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    for ti in 0..TRACK_COUNT {
                        euclidean_states[ti].reset();
                        prob_table_states[ti].reset();
                    }
                    if markov_active {
                        markov_engine.on_bar(&eng.markov_shared);
                    }
                }
                if beat_ev.subdivision {
                    eng.markov_shared.subdivision_epoch.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if markov_active {
                        markov_subdiv_counter += 1;
                        let clock_div = eng.markov_shared.clock_div();
                        let step_now = markov_subdiv_counter >= clock_div;
                        if step_now { markov_subdiv_counter = 0; }

                        let voice_events = if step_now {
                            markov_engine.on_subdivision(&eng.markov_shared)
                        } else {
                            vec![Default::default(); TRACK_COUNT]
                        };

                        for (ti, vev) in voice_events.iter().enumerate().take(TRACK_COUNT) {
                            if let Some(pitch) = vev.note_off {
                                for (slot, note) in voice_notes[ti].iter_mut().enumerate() {
                                    if *note == Some(pitch) {
                                        eng.tracks[ti].voice_gates[slot].set(0.0);
                                        break;
                                    }
                                }
                            }
                            if let Some(pitch) = vev.note_on {
                                if !voice_notes[ti].iter().enumerate().any(|(s, &n)| {
                                    n == Some(pitch) && eng.tracks[ti].voice_gates[s].value() > 0.5
                                }) {
                                    let slot = voice_notes[ti].iter().position(|&n| n == Some(pitch))
                                        .or_else(|| voice_notes[ti].iter().position(|n| n.is_none()))
                                        .or_else(|| (0..synth_engine::VOICE_COUNT).find(|&s| eng.tracks[ti].voice_gates[s].value() < 0.5))
                                        .unwrap_or_else(|| {
                                            let s = steal_idx[ti] % synth_engine::VOICE_COUNT;
                                            steal_idx[ti] += 1;
                                            s
                                        });
                                    voice_notes[ti][slot] = Some(pitch);
                                    eng.tracks[ti].voice_freq_targets[slot].set(midi_hz(pitch as f64) as f32);
                                    let vel = if vev.accent { 1.0f32 } else { 0.75 };
                                    eng.tracks[ti].voice_gates[slot].set(vel);
                                }
                            }
                        }
                    } else {
                        for ti in 0..TRACK_COUNT {
                            let mode = GenerativeMode::from_u8(
                                eng.generative_modes[ti].load(std::sync::atomic::Ordering::Relaxed),
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
                                let slot = voice_notes[ti].iter().position(|n| n.is_none())
                                    .unwrap_or_else(|| {
                                        let s = steal_idx[ti] % synth_engine::VOICE_COUNT;
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

            eng.tick_glide(frames);
            for frame in data.chunks_mut(channels) {
                for ti in 0..TRACK_COUNT {
                    let lfo_rate = eng.tracks[ti].lfo_rate.value();
                    lfo_phases[ti] = (lfo_phases[ti] + lfo_rate / sr as f32).fract();
                    eng.tick_lfo_sample(ti, lfo_phases[ti]);
                }
                let (mut l, mut r) = eng.get_stereo();
                if let Some(slot) = transition_guard.as_mut() {
                    let mut clear = false;
                    if let Some(plan) = slot.as_mut() {
                        let done = plan.total_frames.saturating_sub(plan.frames_left);
                        let progress = done as f32 / plan.total_frames as f32;

                        match plan.mode {
                            SceneTransitionMode::LinearFade => {
                                // V-shape: linear fade to silence at midpoint, then back up.
                                if !plan.applied_target && progress >= 0.5 {
                                    eng.apply_scene(&plan.scene);
                                    plan.applied_target = true;
                                }
                                let gain = if progress < 0.5 {
                                    1.0 - progress * 2.0
                                } else {
                                    (progress - 0.5) * 2.0
                                }.clamp(0.0, 1.0);
                                l *= gain;
                                r *= gain;
                            }
                            SceneTransitionMode::EqualPowerFade => {
                                // Cosine crossfade: constant perceived loudness, no dip.
                                if !plan.applied_target && progress >= 0.5 {
                                    eng.apply_scene(&plan.scene);
                                    plan.applied_target = true;
                                }
                                let angle = if progress < 0.5 {
                                    progress * 2.0
                                } else {
                                    1.0 - (progress - 0.5) * 2.0
                                } * std::f32::consts::FRAC_PI_2;
                                let gain = angle.cos();
                                l *= gain;
                                r *= gain;
                            }
                            SceneTransitionMode::ParamMorph => {
                                // No volume change. Lerp continuous params each sample.
                                // Patches (waveform/ADSR) swap at midpoint; no audible click
                                // because the parameter evolution masks it.
                                if !plan.applied_target && progress >= 0.5 {
                                    // Apply full scene to pick up patch/ADSR; then immediately
                                    // re-apply the target continuous params to avoid a jump.
                                    eng.apply_scene(&plan.scene);
                                    plan.applied_target = true;
                                }
                                if let Some(from) = &plan.from_snapshot {
                                    let t = progress.min(1.0);
                                    let lerp = |a: f32, b: f32| a + (b - a) * t;
                                    let to = &plan.scene.global;
                                    eng.shimmer.mix.set(lerp(from.global.shimmer_mix, to.shimmer_mix));
                                    eng.shimmer.shimmer.set(lerp(from.global.shimmer_amount, to.shimmer_amount));
                                    eng.shimmer.size.set(lerp(from.global.shimmer_size, to.shimmer_size));
                                    eng.shimmer.damp.set(lerp(from.global.shimmer_damp, to.shimmer_damp));
                                    eng.crystal.mix.set(lerp(from.global.crystal_mix, to.crystal_mix));
                                    eng.crystal.grain_ms.set(lerp(from.global.crystal_grain_ms, to.crystal_grain_ms));
                                    eng.crystal.scatter.set(lerp(from.global.crystal_scatter, to.crystal_scatter));
                                    eng.crystal.feedback.set(lerp(from.global.crystal_feedback, to.crystal_feedback));
                                    eng.crystal.delay_ms.set(lerp(from.global.crystal_delay_ms, to.crystal_delay_ms));
                                    eng.master_vol.set(lerp(from.global.master_vol, to.master_vol));
                                    for ti in 0..TRACK_COUNT {
                                        let tt = &plan.scene.tracks[ti];
                                        eng.tracks[ti].track_vol.set(lerp(from.track_vol[ti], tt.volume));
                                        eng.tracks[ti].cutoff.set(lerp(from.track_cutoff[ti], tt.cutoff));
                                        eng.tracks[ti].shimmer_send.set(lerp(from.track_shimmer_send[ti], tt.shimmer_send));
                                        eng.tracks[ti].crystal_send.set(lerp(from.track_crystal_send[ti], tt.crystal_send));
                                    }
                                }
                            }
                            SceneTransitionMode::MorphWithFade => {
                                // Morph continuous params throughout.
                                // Add a short equal-power dip around midpoint (±5% of total)
                                // to mask the patch/waveform discontinuity.
                                const DIP_HALF: f32 = 0.05;
                                let in_dip = (progress - 0.5).abs() < DIP_HALF;
                                let dip_gain = if in_dip {
                                    let t = ((progress - 0.5).abs() / DIP_HALF) * std::f32::consts::FRAC_PI_2;
                                    t.sin() // 0 at midpoint, 1 at edges of dip window
                                } else {
                                    1.0
                                };
                                l *= dip_gain;
                                r *= dip_gain;

                                if !plan.applied_target && progress >= 0.5 {
                                    eng.apply_scene(&plan.scene);
                                    plan.applied_target = true;
                                }
                                if let Some(from) = &plan.from_snapshot {
                                    let t = progress.min(1.0);
                                    let lerp = |a: f32, b: f32| a + (b - a) * t;
                                    let to = &plan.scene.global;
                                    eng.shimmer.mix.set(lerp(from.global.shimmer_mix, to.shimmer_mix));
                                    eng.shimmer.shimmer.set(lerp(from.global.shimmer_amount, to.shimmer_amount));
                                    eng.shimmer.size.set(lerp(from.global.shimmer_size, to.shimmer_size));
                                    eng.shimmer.damp.set(lerp(from.global.shimmer_damp, to.shimmer_damp));
                                    eng.crystal.mix.set(lerp(from.global.crystal_mix, to.crystal_mix));
                                    eng.crystal.grain_ms.set(lerp(from.global.crystal_grain_ms, to.crystal_grain_ms));
                                    eng.crystal.scatter.set(lerp(from.global.crystal_scatter, to.crystal_scatter));
                                    eng.crystal.feedback.set(lerp(from.global.crystal_feedback, to.crystal_feedback));
                                    eng.crystal.delay_ms.set(lerp(from.global.crystal_delay_ms, to.crystal_delay_ms));
                                    eng.master_vol.set(lerp(from.global.master_vol, to.master_vol));
                                    for ti in 0..TRACK_COUNT {
                                        let tt = &plan.scene.tracks[ti];
                                        eng.tracks[ti].track_vol.set(lerp(from.track_vol[ti], tt.volume));
                                        eng.tracks[ti].cutoff.set(lerp(from.track_cutoff[ti], tt.cutoff));
                                        eng.tracks[ti].shimmer_send.set(lerp(from.track_shimmer_send[ti], tt.shimmer_send));
                                        eng.tracks[ti].crystal_send.set(lerp(from.track_crystal_send[ti], tt.crystal_send));
                                    }
                                }
                            }
                        }

                        plan.frames_left = plan.frames_left.saturating_sub(1);
                        if plan.frames_left == 0 {
                            if !plan.applied_target {
                                eng.apply_scene(&plan.scene);
                            }
                            clear = true;
                        }
                    }
                    if clear {
                        **slot = None;
                    }
                }

                let left = T::from_sample(l);
                let right = T::from_sample(r);
                for (i, smp) in frame.iter_mut().enumerate() {
                    *smp = if i & 1 == 0 { left } else { right };
                }
            }
        },
        |err| error!("synth-bevy ambient audio error: {err}"),
        None,
    )?;
    Ok((stream, beat_clock_shared_for_runtime))
}

fn build_mono_stream(state: Arc<AudioState>, rx: ControlReceiver, sr: f64) -> anyhow::Result<Stream> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No output device"))?;
    let config = device.default_output_config()?;
    match config.sample_format() {
        cpal::SampleFormat::F32 => make_mono_stream::<f32>(&device, &config.into(), state, rx, sr),
        cpal::SampleFormat::I16 => make_mono_stream::<i16>(&device, &config.into(), state, rx, sr),
        cpal::SampleFormat::U16 => make_mono_stream::<u16>(&device, &config.into(), state, rx, sr),
        _ => anyhow::bail!("Unsupported sample format"),
    }
}

fn make_mono_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: Arc<AudioState>,
    rx: ControlReceiver,
    sr: f64,
) -> anyhow::Result<Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut graph = BlockRateAdapter::new(build_synth_graph(&state, sr));
    let mut lfo_phase: f32 = 0.0;
    let mut smoothed_freqs: Vec<f32> = vec![440.0; SYNTH_VOICE_COUNT];
    let attack_coeff = (-1.0_f64 / (0.0001 * sr)).exp() as f32;
    let release_coeff = (-1.0_f64 / (0.05 * sr)).exp() as f32;
    let mut env_l: f32 = 0.0;
    let mut env_r: f32 = 0.0;
    let mut voice_notes: [Option<u8>; SYNTH_VOICE_COUNT] = [None; SYNTH_VOICE_COUNT];
    let mut steal_idx: usize = 0;
    let mut arp = ArpState::new();
    let mut walker = ScaleWalker::new();

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for (slot, note) in voice_notes.iter_mut().enumerate() {
                if note.is_some()
                    && state.voice_gates[slot].value() < 0.5
                    && state.amp_cursors[slot].value() < 0.5
                {
                    *note = None;
                }
            }

            while let Ok(ev) = rx.try_recv() {
                match ev {
                    ControlEvent::NoteOn { pitch, .. } => {
                        if state.arp.enabled.load(std::sync::atomic::Ordering::Relaxed) {
                            arp.note_on(pitch);
                        } else {
                            let slot = voice_notes.iter().position(|n| n.is_none()).unwrap_or_else(|| {
                                let s = steal_idx % SYNTH_VOICE_COUNT;
                                steal_idx += 1;
                                s
                            });
                            voice_notes[slot] = Some(pitch);
                            state.voice_freq_targets[slot].set(midi_hz(pitch as f64) as f32);
                            state.voice_gates[slot].set(1.0);
                        }
                    }
                    ControlEvent::NoteOff { pitch, .. } => {
                        if state.arp.enabled.load(std::sync::atomic::Ordering::Relaxed) {
                            let hold = state.arp.hold.load(std::sync::atomic::Ordering::Relaxed);
                            arp.note_off(pitch, hold);
                        } else {
                            for (slot, note) in voice_notes.iter_mut().enumerate() {
                                if *note == Some(pitch) {
                                    state.voice_gates[slot].set(0.0);
                                    break;
                                }
                            }
                        }
                    }
                    ControlEvent::SetParam { param, value } => match param {
                        ParamId::FilterCutoff => state.cutoff.set(value),
                        ParamId::FilterResonance => state.resonance.set(value),
                        ParamId::LfoDepth => state.lfo_depth.set(value),
                        ParamId::MasterVolume => state.master_vol.set(value),
                        ParamId::LfoPitchMult => state.lfo_pitch_mult.set(value),
                    },
                    ControlEvent::ChordHold { notes, .. } => arp.set_chord(&notes),
                    ControlEvent::ArpRestart { .. } => {
                        if let Some(p) = arp.restart() {
                            for (slot, note) in voice_notes.iter_mut().enumerate() {
                                if *note == Some(p) {
                                    state.voice_gates[slot].set(0.0);
                                    break;
                                }
                            }
                        }
                    }
                    ControlEvent::WalkerRestart { .. } => {
                        if let Some(p) = walker.restart() {
                            for (slot, note) in voice_notes.iter_mut().enumerate() {
                                if *note == Some(p) {
                                    state.voice_gates[slot].set(0.0);
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            let frames = data.len() / channels;
            let arp_ev = arp.tick(&state.arp, frames, sr);
            let walk_ev = walker.tick(&state.walker, frames, sr);
            for ev in [arp_ev, walk_ev] {
                if let Some(p) = ev.note_off {
                    for (slot, note) in voice_notes.iter_mut().enumerate() {
                        if *note == Some(p) {
                            state.voice_gates[slot].set(0.0);
                            break;
                        }
                    }
                }
                if let Some(p) = ev.note_on {
                    let slot = voice_notes.iter().position(|n| n.is_none()).unwrap_or_else(|| {
                        let s = steal_idx % SYNTH_VOICE_COUNT;
                        steal_idx += 1;
                        s
                    });
                    voice_notes[slot] = Some(p);
                    state.voice_freq_targets[slot].set(midi_hz(p as f64) as f32);
                    state.voice_gates[slot].set(1.0);
                }
            }

            let sr_f = sr as f32;
            let lfo_rate = state.lfo_rate.value();
            let lfo_depth = state.lfo_depth.value();
            let lfo_shape = state.lfo_shape.load(std::sync::atomic::Ordering::Relaxed);
            let lfo_dest = state.lfo_dest.load(std::sync::atomic::Ordering::Relaxed);
            let lfo_dt = lfo_rate / sr_f;
            let base_cutoff = state.cutoff.value().clamp(80.0, 18000.0);

            let glide_time = state.glide_time.value();
            for vi in 0..SYNTH_VOICE_COUNT {
                let target = state.voice_freq_targets[vi].value();
                if glide_time < 0.001 {
                    smoothed_freqs[vi] = target;
                } else {
                    let coeff = (-(frames as f32) / (glide_time * sr_f)).exp();
                    smoothed_freqs[vi] = coeff * smoothed_freqs[vi] + (1.0 - coeff) * target;
                }
                state.voice_freqs[vi].set(smoothed_freqs[vi]);
            }

            let limiter_on = state.limiter_enabled.load(std::sync::atomic::Ordering::Relaxed);
            let threshold = state.limiter_threshold.value();

            for frame in data.chunks_mut(channels) {
                lfo_phase += lfo_dt;
                if lfo_phase >= 1.0 {
                    lfo_phase -= 1.0;
                }
                let lfo_raw = match lfo_shape {
                    1 => {
                        if lfo_phase < 0.5 {
                            4.0 * lfo_phase - 1.0
                        } else {
                            3.0 - 4.0 * lfo_phase
                        }
                    }
                    2 => 2.0 * lfo_phase - 1.0,
                    _ => (lfo_phase * std::f32::consts::TAU).sin(),
                };
                let lfo_out = lfo_raw * lfo_depth;
                let lfo_amp = match lfo_dest {
                    2 => 1.0 - lfo_depth * (1.0 - lfo_raw) * 0.5,
                    _ => 1.0,
                };
                match lfo_dest {
                    0 => {
                        state.lfo_pitch_mult.set(2_f32.powf(lfo_out * 2.0 / 12.0));
                        state.effective_cutoff.set(base_cutoff);
                    }
                    2 => {
                        state.lfo_pitch_mult.set(1.0);
                        state.effective_cutoff.set(base_cutoff);
                    }
                    _ => {
                        state.lfo_pitch_mult.set(1.0);
                        state.effective_cutoff
                            .set((base_cutoff + lfo_out * base_cutoff * 0.5).clamp(80.0, 18000.0));
                    }
                }

                let (mut l, mut r) = graph.get_stereo();
                if limiter_on {
                    let abs_l = l.abs();
                    let abs_r = r.abs();
                    env_l = if abs_l > env_l {
                        attack_coeff * env_l + (1.0 - attack_coeff) * abs_l
                    } else {
                        release_coeff * env_l + (1.0 - release_coeff) * abs_l
                    };
                    env_r = if abs_r > env_r {
                        attack_coeff * env_r + (1.0 - attack_coeff) * abs_r
                    } else {
                        release_coeff * env_r + (1.0 - release_coeff) * abs_r
                    };
                    if env_l > threshold {
                        l *= threshold / env_l;
                    }
                    if env_r > threshold {
                        r *= threshold / env_r;
                    }
                }
                let l = l.tanh() * lfo_amp;
                let r = r.tanh() * lfo_amp;
                let left = T::from_sample(l);
                let right = T::from_sample(r);
                for (i, smp) in frame.iter_mut().enumerate() {
                    *smp = if i & 1 == 0 { left } else { right };
                }
            }
        },
        |err| error!("synth-bevy mono audio error: {err}"),
        None,
    )?;
    Ok(stream)
}

#[cfg(feature = "inspector")]
mod inspector {
    use super::{SynthParam, SynthTempo, ACTIVE_MACRO_KNOBS, MACRO_COUNT, TRACK_COUNT};
    use bevy::prelude::*;
    use bevy_egui::{egui, EguiContexts, EguiPlugin};

    pub struct SynthInspectorPlugin;

    impl Plugin for SynthInspectorPlugin {
        fn build(&self, app: &mut App) {
            app.add_plugins(EguiPlugin)
                .add_systems(Update, draw_synth_inspector);
        }
    }

    fn draw_synth_inspector(
        mut contexts: EguiContexts,
        params: Option<Res<SynthParam>>,
        tempo: Option<Res<SynthTempo>>,
    ) {
        let Some(params) = params else { return; };
        let Some(tempo) = tempo else { return; };

        egui::Window::new("Synth Inspector")
            .default_width(300.0)
            .show(contexts.ctx_mut(), |ui| {
                ui.label(format!("Tempo: {:.1} BPM", tempo.bpm));

                ui.separator();
                egui::CollapsingHeader::new("Macros").default_open(true).show(ui, |ui| {
                    for i in 0..ACTIVE_MACRO_KNOBS {
                        let mut v = params.macro_value(i);
                        if ui
                            .add(egui::Slider::new(&mut v, 0.0..=1.0).text(format!("Macro {}", i + 1)))
                            .changed()
                        {
                            params.set_macro(i, v);
                        }
                    }
                });

                ui.separator();
                egui::CollapsingHeader::new("Global Bus").default_open(false).show(ui, |ui| {
                    {
                        let mut v = params.master_vol.value();
                        if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).text("Master Vol")).changed() {
                            params.master_vol.set(v);
                        }
                    }
                    ui.label("Shimmer");
                    {
                        let mut v = params.shimmer_mix.value();
                        if ui.add(egui::Slider::new(&mut v, 0.0..=0.80).text("Mix")).changed() {
                            params.shimmer_mix.set(v);
                        }
                    }
                    {
                        let mut v = params.shimmer_size.value();
                        if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).text("Size")).changed() {
                            params.shimmer_size.set(v);
                        }
                    }
                    {
                        let mut v = params.shimmer_amount.value();
                        if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).text("Amount")).changed() {
                            params.shimmer_amount.set(v);
                        }
                    }
                    ui.label("Crystal");
                    {
                        let mut v = params.crystal_mix.value();
                        if ui.add(egui::Slider::new(&mut v, 0.0..=0.45).text("Mix")).changed() {
                            params.crystal_mix.set(v);
                        }
                    }
                    {
                        let mut v = params.crystal_feedback.value();
                        if ui.add(egui::Slider::new(&mut v, 0.0..=0.65).text("Feedback")).changed() {
                            params.crystal_feedback.set(v);
                        }
                    }
                });

                ui.separator();
                egui::CollapsingHeader::new("Tracks").default_open(false).show(ui, |ui| {
                    for i in 0..TRACK_COUNT {
                        egui::CollapsingHeader::new(format!("Track {}", i + 1)).show(ui, |ui| {
                            {
                                let mut v = params.track_vol[i].value();
                                if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).text("Volume")).changed() {
                                    params.track_vol[i].set(v);
                                }
                            }
                            {
                                let mut v = params.track_cutoff[i].value();
                                if ui.add(egui::Slider::new(&mut v, 80.0..=18_000.0).text("Cutoff").logarithmic(true)).changed() {
                                    params.track_cutoff[i].set(v);
                                }
                            }
                            {
                                let mut v = params.track_shimmer_send[i].value();
                                if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).text("Shimmer Send")).changed() {
                                    params.track_shimmer_send[i].set(v);
                                }
                            }
                            {
                                let mut v = params.track_crystal_send[i].value();
                                if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).text("Crystal Send")).changed() {
                                    params.track_crystal_send[i].set(v);
                                }
                            }
                        });
                    }
                });
            });
    }
}
