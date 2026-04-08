use ambient_engine::{save_scene_json, scene_from_single_patch, AmbientPatch};
use bevy::prelude::*;
use synth_bevy::{SynthBackendKind, SynthBevyConfig, SynthEvent, SynthPlugin};

const BUBBLE_RADIUS: f32 = 18.0;
const MAX_BUBBLES: usize = 4;
const COLLISIONS_PER_STAGE: u32 = 8;

#[derive(Component)]
struct Bubble {
    id: usize,
}

#[derive(Component, Deref, DerefMut)]
struct Velocity(Vec2);

#[derive(Component, Clone, Copy)]
struct Obstacle {
    half_extents: Vec2,
}

#[derive(Resource)]
struct StageState {
    stage: usize,
    collisions: u32,
    scene_names: Vec<String>,
}

#[derive(Resource, Default)]
struct PendingNoteOffs(Vec<PendingNoteOff>);

struct PendingNoteOff {
    track: u8,
    pitch: u8,
    seconds_left: f32,
}

fn main() {
    App::new()
        .insert_resource(SynthBevyConfig {
            backend: SynthBackendKind::Ambient,
            sample_rate_hz: 44_100.0,
            control_capacity: 1024,
            scene_dir: "scenes".to_string(),
            initial_bpm: 96.0,
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Bevy Bubbles Ambient".to_string(),
                resolution: (1100.0, 700.0).into(),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .add_plugins(SynthPlugin)
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                bubble_motion_system,
                stage_progression_system,
                ambient_pulse_system,
                pending_note_off_system,
            ),
        )
        .run();
}

fn setup(mut commands: Commands, mut synth: EventWriter<SynthEvent>) {
    commands.spawn(Camera2d);

    ensure_demo_scenes();
    let scene_names = vec![
        "bevy_bubbles_stage_1".to_string(),
        "bevy_bubbles_stage_2".to_string(),
        "bevy_bubbles_stage_3".to_string(),
        "bevy_bubbles_stage_4".to_string(),
    ];
    commands.insert_resource(StageState {
        stage: 0,
        collisions: 0,
        scene_names: scene_names.clone(),
    });
    commands.insert_resource(PendingNoteOffs::default());

    spawn_bubble(&mut commands, 0);

    commands.spawn((
        Sprite::from_color(Color::srgba(0.35, 0.45, 0.52, 0.6), Vec2::new(240.0, 30.0)),
        Transform::from_translation(Vec3::new(0.0, 70.0, 0.0)),
        Obstacle {
            half_extents: Vec2::new(120.0, 15.0),
        },
    ));
    commands.spawn((
        Sprite::from_color(Color::srgba(0.35, 0.45, 0.52, 0.6), Vec2::new(160.0, 30.0)),
        Transform::from_translation(Vec3::new(-220.0, -120.0, 0.0)),
        Obstacle {
            half_extents: Vec2::new(80.0, 15.0),
        },
    ));
    commands.spawn((
        Sprite::from_color(Color::srgba(0.35, 0.45, 0.52, 0.6), Vec2::new(140.0, 30.0)),
        Transform::from_translation(Vec3::new(260.0, -200.0, 0.0)),
        Obstacle {
            half_extents: Vec2::new(70.0, 15.0),
        },
    ));

    synth.send(SynthEvent::SceneLoad {
        name: scene_names[0].clone(),
    });
    synth.send(SynthEvent::ChordHold {
        track: 0,
        notes: vec![48, 55, 60, 67],
    });
}

fn ensure_demo_scenes() {
    let patch = AmbientPatch::default();
    let _ = std::fs::create_dir_all("scenes");

    for stage in 0..4 {
        let name = format!("bevy_bubbles_stage_{}", stage + 1);
        let mut scene = scene_from_single_patch(
            "assets/patches/Ambient/Eno Space.json",
            &patch,
            name.clone(),
            90 + (stage as u32 * 8),
            0,
            "minor",
        );
        scene.global.master_vol = 0.58;
        scene.global.shimmer_mix = 0.18 + stage as f32 * 0.10;
        scene.global.shimmer_amount = 0.10 + stage as f32 * 0.08;
        scene.global.shimmer_size = 0.70 + stage as f32 * 0.07;
        scene.global.crystal_mix = stage as f32 * 0.06;
        scene.global.crystal_feedback = 0.20 + stage as f32 * 0.05;
        scene.tracks[0].volume = 0.9;
        let path = format!("scenes/{name}.json");
        let _ = save_scene_json(path, &scene);
    }
}

fn spawn_bubble(commands: &mut Commands, id: usize) {
    let speed = 120.0 + id as f32 * 22.0;
    let dir = Vec2::new(1.0 - id as f32 * 0.12, 0.8 - id as f32 * 0.08).normalize_or_zero();
    commands.spawn((
        Sprite::from_color(
            Color::srgb(0.45 + id as f32 * 0.08, 0.70, 1.0 - id as f32 * 0.12),
            Vec2::splat(BUBBLE_RADIUS * 2.0),
        ),
        Transform::from_translation(Vec3::new(-320.0 + id as f32 * 45.0, 120.0, 1.0)),
        Bubble { id },
        Velocity(dir * speed),
    ));
}

fn bubble_motion_system(
    time: Res<Time>,
    windows: Query<&Window>,
    obstacles: Query<(&Transform, &Obstacle), Without<Bubble>>,
    mut bubbles: Query<(&mut Transform, &mut Velocity, &Bubble), Without<Obstacle>>,
    mut stage: ResMut<StageState>,
    mut pending_offs: ResMut<PendingNoteOffs>,
    mut synth: EventWriter<SynthEvent>,
) {
    let Ok(window) = windows.get_single() else { return; };
    let half_w = window.width() * 0.5;
    let half_h = window.height() * 0.5;

    for (mut tf, mut vel, bubble) in &mut bubbles {
        tf.translation.x += vel.x * time.delta_secs();
        tf.translation.y += vel.y * time.delta_secs();

        let mut hit = false;
        if tf.translation.x < -half_w + BUBBLE_RADIUS {
            tf.translation.x = -half_w + BUBBLE_RADIUS;
            vel.x = vel.x.abs();
            hit = true;
        } else if tf.translation.x > half_w - BUBBLE_RADIUS {
            tf.translation.x = half_w - BUBBLE_RADIUS;
            vel.x = -vel.x.abs();
            hit = true;
        }
        if tf.translation.y < -half_h + BUBBLE_RADIUS {
            tf.translation.y = -half_h + BUBBLE_RADIUS;
            vel.y = vel.y.abs();
            hit = true;
        } else if tf.translation.y > half_h - BUBBLE_RADIUS {
            tf.translation.y = half_h - BUBBLE_RADIUS;
            vel.y = -vel.y.abs();
            hit = true;
        }

        let p = tf.translation.truncate();
        for (otf, obs) in &obstacles {
            let c = otf.translation.truncate();
            let d = p - c;
            let overlap_x = obs.half_extents.x + BUBBLE_RADIUS - d.x.abs();
            let overlap_y = obs.half_extents.y + BUBBLE_RADIUS - d.y.abs();
            if overlap_x > 0.0 && overlap_y > 0.0 {
                if overlap_x < overlap_y {
                    vel.x = -vel.x;
                } else {
                    vel.y = -vel.y;
                }
                hit = true;
            }
        }

        if hit {
            stage.collisions += 1;
            let pitch = (48 + bubble.id as u8 * 5 + (stage.collisions % 12) as u8).min(84);
            synth.send(SynthEvent::NoteOn {
                track: 1,
                pitch,
                velocity: 100,
            });
            pending_offs.0.push(PendingNoteOff {
                track: 1,
                pitch,
                seconds_left: 0.11,
            });
            let tension = (stage.collisions as f32 / COLLISIONS_PER_STAGE as f32).clamp(0.0, 1.0);
            synth.send(SynthEvent::SetMacro {
                index: 0,
                value: tension,
            });
        }
    }
}

fn stage_progression_system(
    mut commands: Commands,
    mut stage: ResMut<StageState>,
    bubbles: Query<&Bubble>,
    mut synth: EventWriter<SynthEvent>,
) {
    if stage.collisions < COLLISIONS_PER_STAGE {
        return;
    }
    stage.collisions = 0;

    if stage.stage + 1 < stage.scene_names.len() {
        stage.stage += 1;
    }
    let next_scene = stage.scene_names[stage.stage].clone();
    synth.send(SynthEvent::SceneTransition {
        name: next_scene,
        frames: 44_100 * 2,
    });

    let current_count = bubbles.iter().count();
    if current_count < MAX_BUBBLES {
        spawn_bubble(&mut commands, current_count);
    }
}

#[derive(Resource)]
struct PulseTimer(Timer);

fn ambient_pulse_system(
    time: Res<Time>,
    mut local_timer: Local<Option<PulseTimer>>,
    mut pending_offs: ResMut<PendingNoteOffs>,
    mut synth: EventWriter<SynthEvent>,
) {
    if local_timer.is_none() {
        *local_timer = Some(PulseTimer(Timer::from_seconds(2.2, TimerMode::Repeating)));
    }
    let Some(t) = local_timer.as_mut() else { return; };
    if t.0.tick(time.delta()).just_finished() {
        synth.send(SynthEvent::NoteOn {
            track: 0,
            pitch: 52,
            velocity: 70,
        });
        pending_offs.0.push(PendingNoteOff {
            track: 0,
            pitch: 52,
            seconds_left: 0.28,
        });
    }
}

fn pending_note_off_system(
    time: Res<Time>,
    mut pending_offs: ResMut<PendingNoteOffs>,
    mut synth: EventWriter<SynthEvent>,
) {
    let dt = time.delta_secs();
    for n in &mut pending_offs.0 {
        n.seconds_left -= dt;
    }
    let mut i = 0;
    while i < pending_offs.0.len() {
        if pending_offs.0[i].seconds_left <= 0.0 {
            let n = pending_offs.0.swap_remove(i);
            synth.send(SynthEvent::NoteOff {
                track: n.track,
                pitch: n.pitch,
            });
        } else {
            i += 1;
        }
    }
}
