use ambient_engine::{load_scene_json, AmbientEngine, MACRO_COUNT};
use bevy::prelude::*;
use fundsp::prelude32::Shared;
use std::sync::{Arc, Mutex};
use synth_control::{make_control_channel, ControlEvent, ControlReceiver, ControlSender, ParamId};

#[derive(Resource, Clone)]
pub struct SynthBevyConfig {
    pub sample_rate_hz: f64,
    pub control_capacity: usize,
    pub scene_dir: String,
    pub initial_bpm: f32,
}

impl Default for SynthBevyConfig {
    fn default() -> Self {
        Self {
            sample_rate_hz: 44_100.0,
            control_capacity: 1024,
            scene_dir: "scenes".to_string(),
            initial_bpm: 120.0,
        }
    }
}

#[derive(Resource, Clone)]
pub struct SynthParam {
    pub macro_values: [Shared; MACRO_COUNT],
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

#[derive(Resource)]
pub struct SynthRuntime {
    pub engine: Arc<Mutex<AmbientEngine>>,
    pub control_tx: ControlSender,
    pub control_rx: ControlReceiver,
}

#[derive(Resource, Default)]
pub struct SynthTempo {
    pub bpm: f32,
}

#[derive(Event, Debug, Clone)]
pub enum SynthEvent {
    NoteOn { track: u8, pitch: u8, velocity: u8 },
    NoteOff { track: u8, pitch: u8 },
    ChordHold { track: u8, notes: Vec<u8> },
    SetMacro { index: u8, value: f32 },
    SetParam { track: u8, param: ParamId, value: f32 },
    SceneLoad { name: String },
    SceneTransition { name: String, frames: u32 },
    Tempo { bpm: f32 },
}

pub struct SynthPlugin;

impl Plugin for SynthPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SynthBevyConfig>()
            .init_resource::<SynthTempo>()
            .add_event::<SynthEvent>()
            .add_systems(Startup, setup_synth_runtime)
            .add_systems(PostUpdate, bevy_bridge_system);

        #[cfg(feature = "inspector")]
        app.add_plugins(inspector::SynthInspectorPlugin);
    }
}

fn setup_synth_runtime(mut commands: Commands, cfg: Res<SynthBevyConfig>, mut tempo: ResMut<SynthTempo>) {
    let engine_inner = AmbientEngine::new(cfg.sample_rate_hz);
    let macro_values = std::array::from_fn(|i| engine_inner.macro_values[i].clone());
    let engine = Arc::new(Mutex::new(engine_inner));
    let (control_tx, control_rx) = make_control_channel(cfg.control_capacity);

    tempo.bpm = cfg.initial_bpm;
    commands.insert_resource(SynthParam { macro_values });
    commands.insert_resource(SynthRuntime {
        engine,
        control_tx,
        control_rx,
    });
}

fn bevy_bridge_system(
    mut events: EventReader<SynthEvent>,
    cfg: Res<SynthBevyConfig>,
    runtime: Option<Res<SynthRuntime>>,
    params: Option<Res<SynthParam>>,
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
                if let Ok(eng) = runtime.engine.try_lock() {
                    eng.set_macro_value(idx, *value);
                }
            }
            SynthEvent::SetParam { track: _, param, value } => {
                // Ambient bridge currently forwards named params to the control queue.
                // Track-scoped direct parameter writes can be added on top of Macro routing.
                let _ = runtime.control_tx.try_send(ControlEvent::SetParam {
                    param: *param,
                    value: *value,
                });
            }
            SynthEvent::SceneLoad { name } => {
                let path = scene_path(&cfg.scene_dir, name);
                match load_scene_json(&path) {
                    Ok(scene) => {
                        if let Ok(mut eng) = runtime.engine.try_lock() {
                            eng.apply_scene(&scene);
                        } else {
                            warn!("synth-bevy: engine busy, skipped scene load '{}'", name);
                        }
                    }
                    Err(e) => warn!("synth-bevy: scene load failed '{}': {e}", path),
                }
            }
            SynthEvent::SceneTransition { name, frames } => {
                // Phase-7 scaffold: use immediate scene load now; true crossfade in a follow-up.
                debug!(
                    "synth-bevy: SceneTransition requested (name='{}', frames={}), using immediate load",
                    name, frames
                );
                let path = scene_path(&cfg.scene_dir, name);
                match load_scene_json(&path) {
                    Ok(scene) => {
                        if let Ok(mut eng) = runtime.engine.try_lock() {
                            eng.apply_scene(&scene);
                        } else {
                            warn!("synth-bevy: engine busy, skipped scene transition '{}'", name);
                        }
                    }
                    Err(e) => warn!("synth-bevy: scene transition load failed '{}': {e}", path),
                }
            }
            SynthEvent::Tempo { bpm } => {
                tempo.bpm = bpm.max(1.0);
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

#[cfg(feature = "inspector")]
mod inspector {
    use super::{SynthParam, SynthTempo, MACRO_COUNT};
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
        egui::Window::new("Synth Inspector").show(contexts.ctx_mut(), |ui| {
            ui.label(format!("Tempo: {:.1} BPM", tempo.bpm));
            for i in 0..MACRO_COUNT {
                let mut v = params.macro_value(i);
                if ui
                    .add(egui::Slider::new(&mut v, 0.0..=1.0).text(format!("Macro {}", i + 1)))
                    .changed()
                {
                    params.set_macro(i, v);
                }
            }
        });
    }
}
