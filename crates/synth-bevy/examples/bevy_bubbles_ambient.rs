use ambient_engine::{save_scene_json, AmbientPatch, MacroSetKind, Scene, SceneGlobal, SceneTrack};
use bevy::{
    core_pipeline::bloom::Bloom,
    prelude::*,
    sprite::{ColorMaterial, MeshMaterial2d},
};
use synth_bevy::{SceneTransitionMode, SynthBackendKind, SynthBevyConfig, SynthEvent, SynthPlugin};
use synth_control::midi_note;

const BUBBLE_RADIUS: f32 = 18.0;
const MAX_BUBBLES: usize = 4;
const COLLISIONS_PER_STAGE: u32 = 8;

// Am pentatonic scale, D4–A5.
const AM_PENTA: [u8; 10] = [
    midi_note!(D, 4),
    midi_note!(E, 4),
    midi_note!(G, 4),
    midi_note!(A, 4),
    midi_note!(C, 5),
    midi_note!(D, 5),
    midi_note!(E, 5),
    midi_note!(G, 5),
    midi_note!(A, 5),
    midi_note!(C, 6),
];
// Bass pulse pattern — slow Am walk.
const BASS_PATTERN: [u8; 8] = [
    midi_note!(D, 2),
    midi_note!(E, 2),
    midi_note!(G, 2),
    midi_note!(D, 3),
    midi_note!(E, 3),
    midi_note!(G, 3),
    midi_note!(D, 4),
    midi_note!(E, 4),
];

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

/// One complete visual palette for a stage.
#[derive(Clone)]
struct StagePalette {
    /// Background clear colour — very dark, sets the mood.
    background: LinearRgba,
    /// Obstacle tint — dim, barely-glowing accent.
    obstacle: LinearRgba,
    /// Per-bubble HDR colours — values > 1.0 bloom.
    bubbles: [LinearRgba; 4],
}

fn lerp_linear(a: LinearRgba, b: LinearRgba, t: f32) -> LinearRgba {
    LinearRgba::new(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
        1.0,
    )
}

fn lrgb(r: f32, g: f32, b: f32) -> LinearRgba {
    LinearRgba::new(r, g, b, 1.0)
}

/// Stage 0 — **Midnight**
/// Near-black ink-blue. Cold cyan / cobalt bubbles. Silent and deep.
fn palette_midnight() -> StagePalette {
    StagePalette {
        background: lrgb(0.005, 0.006, 0.020),
        obstacle: lrgb(0.030, 0.045, 0.110),
        bubbles: [
            lrgb(0.10, 1.40, 3.00), // electric cyan
            lrgb(0.05, 0.70, 2.40), // ice blue
            lrgb(0.50, 0.15, 2.20), // indigo
            lrgb(0.08, 0.35, 2.60), // cobalt
        ],
    }
}

/// Stage 1 — **Abyss**
/// Near-black with a teal undertone. Seafoam / emerald bubbles. Alive but cold.
fn palette_abyss() -> StagePalette {
    StagePalette {
        background: lrgb(0.004, 0.014, 0.016),
        obstacle: lrgb(0.025, 0.090, 0.095),
        bubbles: [
            lrgb(0.05, 2.00, 1.80), // teal
            lrgb(0.00, 1.60, 1.20), // seafoam
            lrgb(0.00, 2.40, 0.80), // emerald
            lrgb(0.15, 1.80, 1.40), // mint
        ],
    }
}

/// Stage 2 — **Ether**
/// Near-black with a violet wash. Magenta / lavender bubbles. Ethereal, building.
fn palette_ether() -> StagePalette {
    StagePalette {
        background: lrgb(0.010, 0.005, 0.030),
        obstacle: lrgb(0.065, 0.025, 0.140),
        bubbles: [
            lrgb(1.80, 0.40, 2.80), // violet
            lrgb(2.20, 0.20, 1.80), // magenta
            lrgb(1.20, 0.35, 2.40), // lavender
            lrgb(2.00, 0.30, 1.60), // pink
        ],
    }
}

/// Stage 3 — **Ember**
/// Near-black with a crimson tinge. Amber / gold / crimson bubbles. Intense, final.
fn palette_ember() -> StagePalette {
    StagePalette {
        background: lrgb(0.018, 0.004, 0.004),
        obstacle: lrgb(0.130, 0.038, 0.025),
        bubbles: [
            lrgb(2.40, 1.00, 0.20), // amber
            lrgb(2.80, 0.20, 0.10), // crimson
            lrgb(2.60, 1.60, 0.00), // gold
            lrgb(2.20, 0.60, 0.10), // rust
        ],
    }
}

fn stage_palette(stage: usize) -> StagePalette {
    match stage {
        0 => palette_midnight(),
        1 => palette_abyss(),
        2 => palette_ether(),
        _ => palette_ember(),
    }
}

// ---------------------------------------------------------------------------
// Components / resources
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Bubble {
    id: usize,
    /// Handle kept so the palette system can tint it.
    material: Handle<ColorMaterial>,
}

#[derive(Component, Deref, DerefMut)]
struct Velocity(Vec2);

#[derive(Component, Clone, Copy)]
struct Obstacle {
    half_extents: Vec2,
}

/// Marker so the palette system can find obstacle materials.
#[derive(Component)]
struct ObstacleMaterial(Handle<ColorMaterial>);

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

/// Drives a smooth palette crossfade between two `StagePalette`s.
#[derive(Resource)]
struct PaletteState {
    from: StagePalette,
    to: StagePalette,
    /// 0.0 = fully `from`, 1.0 = fully `to`.
    progress: f32,
    /// How many seconds the crossfade takes.
    duration: f32,
}

impl PaletteState {
    fn new(palette: StagePalette) -> Self {
        Self {
            from: palette.clone(),
            to: palette,
            progress: 1.0,
            duration: 3.0,
        }
    }

    fn transition_to(&mut self, next: StagePalette, duration: f32) {
        // Snapshot current interpolated state as new `from`.
        let t = self.progress;
        self.from = StagePalette {
            background: lerp_linear(self.from.background, self.to.background, t),
            obstacle: lerp_linear(self.from.obstacle, self.to.obstacle, t),
            bubbles: std::array::from_fn(|i| {
                lerp_linear(self.from.bubbles[i], self.to.bubbles[i], t)
            }),
        };
        self.to = next;
        self.progress = 0.0;
        self.duration = duration;
    }

    fn current_obstacle(&self) -> LinearRgba {
        lerp_linear(self.from.obstacle, self.to.obstacle, self.progress)
    }
    fn current_bubble(&self, id: usize) -> LinearRgba {
        lerp_linear(
            self.from.bubbles[id % 4],
            self.to.bubbles[id % 4],
            self.progress,
        )
    }
}

// ---------------------------------------------------------------------------
// Scene building (audio)
// ---------------------------------------------------------------------------

fn load_patch(path: &str) -> AmbientPatch {
    AmbientPatch::from_file(std::path::Path::new(path)).unwrap_or_default()
}

fn build_bubble_scene(stage: usize, pad: &AmbientPatch, bell: &AmbientPatch) -> Scene {
    let shimmer_mix = (0.24 + stage as f32 * 0.09).min(0.75);
    let shimmer_amt = (0.42 + stage as f32 * 0.10).min(0.88);
    let shimmer_size = (0.68 + stage as f32 * 0.06).min(0.96);
    let crystal_mix = (0.08 + stage as f32 * 0.05).min(0.30);
    let crystal_fb = (0.22 + stage as f32 * 0.05).min(0.55);

    Scene {
        name: format!("bevy_bubbles_stage_{}", stage + 1),
        bpm: 88 + (stage as u32 * 4),
        key: 9,
        scale: "minor".to_string(),
        macro_set: MacroSetKind::AmbientCore,
        markov: None,
        tracks: std::array::from_fn(|ti| match ti {
            0 => SceneTrack {
                patch_path: "assets/patches/Ambient/Night Drift.json".to_string(),
                patch: pad.clone(),
                volume: 0.72,
                cutoff: 1600.0 + stage as f32 * 500.0,
                resonance: 0.14,
                shimmer_send: 0.58 + stage as f32 * 0.05,
                crystal_send: 0.06,
            },
            1 => SceneTrack {
                patch_path: "assets/patches/Keys/Crystal Bell.json".to_string(),
                patch: bell.clone(),
                volume: 0.62,
                cutoff: 9000.0,
                resonance: 0.18,
                shimmer_send: 0.22,
                crystal_send: 0.40 + stage as f32 * 0.05,
            },
            _ => SceneTrack {
                patch_path: String::new(),
                patch: AmbientPatch::default(),
                volume: 0.0,
                cutoff: 3000.0,
                resonance: 0.3,
                shimmer_send: 0.0,
                crystal_send: 0.0,
            },
        }),
        macros: Vec::new(),
        global: SceneGlobal {
            master_vol: 0.62,
            shimmer_mix,
            shimmer_amount: shimmer_amt,
            shimmer_size,
            shimmer_damp: 0.40,
            shimmer_pitch: 1,
            crystal_mix,
            crystal_grain_ms: 85.0,
            crystal_scatter: 0.32,
            crystal_feedback: crystal_fb,
            crystal_delay_ms: 210.0,
            crystal_pitch: 2,
        },
    }
}

fn ensure_demo_scenes() {
    let pad = load_patch("assets/patches/Ambient/Night Drift.json");
    let bell = load_patch("assets/patches/Keys/Crystal Bell.json");
    let _ = std::fs::create_dir_all("scenes");
    for stage in 0..4 {
        let scene = build_bubble_scene(stage, &pad, &bell);
        let _ = save_scene_json(format!("scenes/{}.json", scene.name), &scene);
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let initial_palette = stage_palette(0);
    let bg = initial_palette.background;

    App::new()
        .insert_resource(ClearColor(Color::LinearRgba(bg)))
        .insert_resource(SynthBevyConfig {
            backend: SynthBackendKind::Ambient,
            sample_rate_hz: 44_100.0,
            control_capacity: 1024,
            scene_dir: "scenes".to_string(),
            initial_bpm: 88.0,
            default_transition_mode: SceneTransitionMode::EqualPowerFade,
        })
        .insert_resource(PaletteState::new(initial_palette))
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
                palette_transition_system,
                ambient_pulse_system,
                pending_note_off_system,
            ),
        )
        .run();
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    palette: Res<PaletteState>,
    mut synth: EventWriter<SynthEvent>,
) {
    commands.spawn((
        Camera2d,
        Camera {
            hdr: true,
            ..default()
        },
        Bloom {
            intensity: 0.28,
            low_frequency_boost: 0.50,
            low_frequency_boost_curvature: 0.50,
            high_pass_frequency: 1.0,
            ..default()
        },
    ));

    ensure_demo_scenes();
    let scene_names: Vec<String> = (0..4)
        .map(|i| format!("bevy_bubbles_stage_{}", i + 1))
        .collect();
    commands.insert_resource(StageState {
        stage: 0,
        collisions: 0,
        scene_names: scene_names.clone(),
    });
    commands.insert_resource(PendingNoteOffs::default());

    spawn_bubble(&mut commands, &mut meshes, &mut materials, &palette, 0);

    let obs_color = Color::LinearRgba(palette.current_obstacle());
    spawn_obstacle(
        &mut commands,
        &mut meshes,
        &mut materials,
        obs_color,
        Vec2::new(240.0, 28.0),
        Vec3::new(0.0, 70.0, 0.0),
        Vec2::new(120.0, 14.0),
    );
    spawn_obstacle(
        &mut commands,
        &mut meshes,
        &mut materials,
        obs_color,
        Vec2::new(160.0, 28.0),
        Vec3::new(-220.0, -120.0, 0.0),
        Vec2::new(80.0, 14.0),
    );
    spawn_obstacle(
        &mut commands,
        &mut meshes,
        &mut materials,
        obs_color,
        Vec2::new(140.0, 28.0),
        Vec3::new(260.0, -200.0, 0.0),
        Vec2::new(70.0, 14.0),
    );

    spawn_obstacle(
        &mut commands,
        &mut meshes,
        &mut materials,
        obs_color,
        Vec2::new(28.0, 28.0),
        Vec3::new(160.0, -100.0, 0.0),
        Vec2::new(14.0, 14.0),
    );

    spawn_obstacle(
        &mut commands,
        &mut meshes,
        &mut materials,
        obs_color,
        Vec2::new(100.0, 64.0),
        Vec3::new(20.0, 220.0, 0.0),
        Vec2::new(50.0, 32.0),
    );

    synth.send(SynthEvent::SceneLoad {
        name: scene_names[0].clone(),
    });
    for &pitch in &[midi_note!(A, 2), midi_note!(E, 3), midi_note!(A, 3)] {
        synth.send(SynthEvent::NoteOn {
            track: 0,
            pitch,
            velocity: 78,
        });
    }
}

fn spawn_obstacle(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    color: Color,
    size: Vec2,
    translation: Vec3,
    half_extents: Vec2,
) {
    let handle = materials.add(ColorMaterial::from_color(color));
    commands.spawn((
        Mesh2d(meshes.add(Rectangle::new(size.x, size.y))),
        MeshMaterial2d(handle.clone()),
        Transform::from_translation(translation),
        Obstacle { half_extents },
        ObstacleMaterial(handle),
    ));
}

fn spawn_bubble(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    palette: &PaletteState,
    id: usize,
) {
    let speed = 120.0 + id as f32 * 22.0;
    let dir = Vec2::new(1.0 - id as f32 * 0.12, 0.8 - id as f32 * 0.08).normalize_or_zero();
    let color = Color::LinearRgba(palette.current_bubble(id));
    let handle = materials.add(ColorMaterial::from_color(color));
    commands.spawn((
        Mesh2d(meshes.add(Circle::new(BUBBLE_RADIUS))),
        MeshMaterial2d(handle.clone()),
        Transform::from_translation(Vec3::new(-320.0 + id as f32 * 45.0, 120.0, 1.0)),
        Bubble {
            id,
            material: handle,
        },
        Velocity(dir * speed),
    ));
}

// ---------------------------------------------------------------------------
// Palette transition system
// ---------------------------------------------------------------------------

fn palette_transition_system(
    time: Res<Time>,
    mut palette: ResMut<PaletteState>,
    mut clear_color: ResMut<ClearColor>,
    mut color_assets: ResMut<Assets<ColorMaterial>>,
    bubbles: Query<&Bubble>,
    obstacles: Query<&ObstacleMaterial>,
) {
    if palette.progress >= 1.0 {
        return;
    }
    palette.progress = (palette.progress + time.delta_secs() / palette.duration).min(1.0);

    // Ease in-out: smooth step.
    let t = {
        let p = palette.progress;
        p * p * (3.0 - 2.0 * p)
    };

    // Background.
    let bg = lerp_linear(palette.from.background, palette.to.background, t);
    clear_color.0 = Color::LinearRgba(bg);

    // Obstacles — all share the same target colour.
    let obs = Color::LinearRgba(lerp_linear(palette.from.obstacle, palette.to.obstacle, t));
    for ObstacleMaterial(handle) in &obstacles {
        if let Some(mat) = color_assets.get_mut(handle) {
            mat.color = obs;
        }
    }

    // Bubbles — each gets its own colour from the palette.
    for bubble in &bubbles {
        let col = Color::LinearRgba(lerp_linear(
            palette.from.bubbles[bubble.id % 4],
            palette.to.bubbles[bubble.id % 4],
            t,
        ));
        if let Some(mat) = color_assets.get_mut(&bubble.material) {
            mat.color = col;
        }
    }
}

// ---------------------------------------------------------------------------
// Game systems
// ---------------------------------------------------------------------------

fn bubble_motion_system(
    time: Res<Time>,
    windows: Query<&Window>,
    obstacles: Query<(&Transform, &Obstacle), Without<Bubble>>,
    mut bubbles: Query<(&mut Transform, &mut Velocity, &Bubble), Without<Obstacle>>,
    mut stage: ResMut<StageState>,
    mut pending_offs: ResMut<PendingNoteOffs>,
    mut synth: EventWriter<SynthEvent>,
) {
    let Ok(window) = windows.get_single() else {
        return;
    };
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
                    vel.x *= -1.0;
                    tf.translation.x += overlap_x * d.x.signum(); // Sposta fuori
                } else {
                    vel.y *= -1.0;
                    tf.translation.y += overlap_y * d.y.signum(); // Sposta fuori
                }
                hit = true;
            }
        }

        if hit {
            stage.collisions += 1;
            let idx = (bubble.id * 3 + stage.collisions as usize) % AM_PENTA.len();
            let pitch = AM_PENTA[idx];
            synth.send(SynthEvent::NoteOn {
                track: 1,
                pitch,
                velocity: 95,
            });
            pending_offs.0.push(PendingNoteOff {
                track: 1,
                pitch,
                seconds_left: 0.5,
            });
            let tension = (stage.collisions as f32
                / (COLLISIONS_PER_STAGE as f32 * (stage.stage + 1) as f32))
                .clamp(0.0, 1.0);
            synth.send(SynthEvent::SetMacro {
                index: 0,
                value: tension,
            });
        }
    }
}

fn stage_progression_system(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut stage: ResMut<StageState>,
    mut palette: ResMut<PaletteState>,
    bubbles: Query<&Bubble>,
    mut synth: EventWriter<SynthEvent>,
) {
    if stage.collisions < COLLISIONS_PER_STAGE * (stage.stage + 1) as u32 {
        return;
    }
    stage.collisions = 0;

    if stage.stage + 1 < stage.scene_names.len() {
        stage.stage += 1;
    }

    let current_count = bubbles.iter().count();
    if current_count < MAX_BUBBLES {
        spawn_bubble(
            &mut commands,
            &mut meshes,
            &mut materials,
            &palette,
            current_count,
        );
        // Trigger visual palette transition — same 3-second duration.
        palette.transition_to(stage_palette(stage.stage), 3.0);

        // Trigger audio scene transition.
        synth.send(SynthEvent::SceneTransition {
            name: stage.scene_names[stage.stage].clone(),
            frames: 44_100 * 100,
            mode: None,
        });
    }
}

#[derive(Resource)]
struct PulseTimer(Timer);

fn ambient_pulse_system(
    time: Res<Time>,
    mut local_timer: Local<Option<PulseTimer>>,
    mut pulse_step: Local<usize>,
    mut pending_offs: ResMut<PendingNoteOffs>,
    mut synth: EventWriter<SynthEvent>,
) {
    if local_timer.is_none() {
        *local_timer = Some(PulseTimer(Timer::from_seconds(3.5, TimerMode::Repeating)));
    }
    let Some(t) = local_timer.as_mut() else {
        return;
    };
    if t.0.tick(time.delta()).just_finished() {
        let pitch = BASS_PATTERN[*pulse_step % BASS_PATTERN.len()];
        *pulse_step += 1;
        synth.send(SynthEvent::NoteOn {
            track: 0,
            pitch,
            velocity: 72,
        });
        pending_offs.0.push(PendingNoteOff {
            track: 0,
            pitch,
            seconds_left: 1.8,
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
