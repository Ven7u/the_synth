use bevy::prelude::*;
use synth_bevy::{SynthBackendKind, SynthBevyConfig, SynthEvent, SynthPlugin};
use synth_control::ParamId;

#[derive(Component)]
struct Player;

fn main() {
    App::new()
        .insert_resource(SynthBevyConfig {
            backend: SynthBackendKind::Mono,
            sample_rate_hz: 44_100.0,
            control_capacity: 1024,
            scene_dir: "scenes".to_string(),
            initial_bpm: 120.0,
            ..Default::default()
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Bevy Mono Playground".to_string(),
                resolution: (980u32, 620u32).into(),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .add_plugins(SynthPlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, (move_player, map_player_to_synth, keyboard_notes))
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);
    commands.spawn((
        Sprite::from_color(Color::srgb(0.9, 0.55, 0.2), Vec2::new(26.0, 26.0)),
        Transform::from_xyz(0.0, 0.0, 1.0),
        Player,
    ));
}

fn move_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window>,
    mut q: Query<&mut Transform, With<Player>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(mut tf) = q.single_mut() else {
        return;
    };

    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::ArrowLeft) || keys.pressed(KeyCode::KeyA) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) || keys.pressed(KeyCode::KeyD) {
        dir.x += 1.0;
    }
    if keys.pressed(KeyCode::ArrowUp) || keys.pressed(KeyCode::KeyW) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::ArrowDown) || keys.pressed(KeyCode::KeyS) {
        dir.y -= 1.0;
    }
    let speed = 280.0;
    let delta = dir.normalize_or_zero() * speed * time.delta_secs();
    tf.translation.x += delta.x;
    tf.translation.y += delta.y;

    let half_w = window.width() * 0.5 - 20.0;
    let half_h = window.height() * 0.5 - 20.0;
    tf.translation.x = tf.translation.x.clamp(-half_w, half_w);
    tf.translation.y = tf.translation.y.clamp(-half_h, half_h);
}

fn map_player_to_synth(
    windows: Query<&Window>,
    q: Query<&Transform, With<Player>>,
    mut synth: MessageWriter<SynthEvent>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(tf) = q.single() else {
        return;
    };
    let x_norm = ((tf.translation.x / (window.width() * 0.5)) * 0.5 + 0.5).clamp(0.0, 1.0);
    let y_norm = ((tf.translation.y / (window.height() * 0.5)) * 0.5 + 0.5).clamp(0.0, 1.0);

    let cutoff = 80.0 * (18_000.0_f32 / 80.0).powf(x_norm);
    let resonance = 0.1 + y_norm * (1.2 - 0.1);
    let lfo_depth = (1.0 - y_norm) * 0.5;

    synth.write(SynthEvent::SetParam {
        track: 0,
        param: ParamId::FilterCutoff,
        value: cutoff,
    });
    synth.write(SynthEvent::SetParam {
        track: 0,
        param: ParamId::FilterResonance,
        value: resonance,
    });
    synth.write(SynthEvent::SetParam {
        track: 0,
        param: ParamId::LfoDepth,
        value: lfo_depth,
    });
}

fn keyboard_notes(keys: Res<ButtonInput<KeyCode>>, mut synth: MessageWriter<SynthEvent>) {
    // One-octave row: Z S X D C V G B H N J M
    let key_map: &[(KeyCode, u8)] = &[
        (KeyCode::KeyZ, 48),
        (KeyCode::KeyS, 49),
        (KeyCode::KeyX, 50),
        (KeyCode::KeyD, 51),
        (KeyCode::KeyC, 52),
        (KeyCode::KeyV, 53),
        (KeyCode::KeyG, 54),
        (KeyCode::KeyB, 55),
        (KeyCode::KeyH, 56),
        (KeyCode::KeyN, 57),
        (KeyCode::KeyJ, 58),
        (KeyCode::KeyM, 59),
    ];

    for (k, pitch) in key_map {
        if keys.just_pressed(*k) {
            synth.write(SynthEvent::NoteOn {
                track: 0,
                pitch: *pitch,
                velocity: 110,
            });
        }
        if keys.just_released(*k) {
            synth.write(SynthEvent::NoteOff {
                track: 0,
                pitch: *pitch,
            });
        }
    }
}
