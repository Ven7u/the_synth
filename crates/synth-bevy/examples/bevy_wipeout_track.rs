//! WipEout-style endless track — Bevy 0.18 + custom WGSL shader.
//!
//! Visual features:
//!   - Fullscreen procedural track shader with Mode-7 perspective projection.
//!   - Glowing side rails, lane dividers, dashed centre line.
//!   - Translucent side walls / barriers for corridor feel.
//!   - Atmospheric horizon glow and volumetric fog.
//!   - Beat-reactive uniforms: bar boundary spikes `beat_pulse`.
//!
//! Audio:
//!   - Scene is configurable via `SCENE_NAME` constant below.
//!
//! Run:
//!   cargo run -p synth-bevy --example bevy_wipeout_track --features bevy --release

use std::sync::atomic::Ordering;

use bevy::{
    post_process::bloom::{Bloom, BloomCompositeMode},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
    sprite_render::{Material2d, Material2dPlugin},
    window::WindowResolution,
};
use synth_bevy::{SceneTransitionMode, SynthBackendKind, SynthBevyConfig, SynthEvent, SynthPlugin};

// ── Config ────────────────────────────────────────────────────────────────────
// Change SCENE_NAME to any filename (without .json) inside your `scenes/` dir.

const SCENE_NAME: &str = "Says";

// Window
const WIN_W: u32 = 1280;
const WIN_H: u32 = 800;

// Track quad covers the full screen — large enough for any resolution.
const TRACK_SIZE: f32 = 4000.0;

// ── Palette ───────────────────────────────────────────────────────────────────

/// Colour palette for the track visuals. Tweak freely.
#[derive(Resource, Clone)]
struct TrackConfig {
    /// HDR emissive color for lane lines and centre glow.
    zone_color: Vec4,
    /// Horizon fog color — should match or complement the window clear color.
    fog_color:  Vec4,
    /// Background / clear color.
    background: Color,
    /// Base forward speed (units/sec scrolled in shader).
    base_speed: f32,
}

impl Default for TrackConfig {
    fn default() -> Self {
        Self {
            // Retro synthwave — hot pink grid on dark purple
            zone_color: Vec4::new(3.20, 0.10, 2.40, 1.0),
            fog_color:  Vec4::new(0.018, 0.0, 0.045, 1.0),
            background: Color::srgb(0.010, 0.0, 0.028),
            base_speed: 1.0,
        }
    }
}

// ── Materials ─────────────────────────────────────────────────────────────────

/// GPU uniform block for the track shader (group 2, binding 0).
/// std140 layout — must match `TrackMaterial` struct in track.wgsl exactly.
#[derive(ShaderType, Clone, Default)]
struct TrackUniforms {
    time:       f32,
    speed:      f32,
    beat_pulse: f32,
    _pad:       f32,
    zone_color: Vec4,
    fog_color:  Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct TrackMaterial {
    #[uniform(0)]
    pub uniforms: TrackUniforms,
}

impl Material2d for TrackMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/track.wgsl".into()
    }
}

// ── Components ────────────────────────────────────────────────────────────────

#[derive(Component)]
struct TrackQuad {
    material: Handle<TrackMaterial>,
}

// ── State ─────────────────────────────────────────────────────────────────────

#[derive(Resource, Default)]
struct VisualState {
    /// Decays from 1.0 → 0.0 after each bar hit.
    beat_pulse: f32,
    /// Smoothed forward speed fed to shaders.
    speed:      f32,
    /// Tracks last bar epoch to detect new bars.
    last_bar:   usize,
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let cfg = TrackConfig::default();

    App::new()
        .insert_resource(ClearColor(cfg.background))
        .insert_resource(cfg)
        .insert_resource(VisualState { speed: 1.0, ..default() })
        .insert_resource(SynthBevyConfig {
            backend: SynthBackendKind::Ambient,
            sample_rate_hz: 44_100.0,
            control_capacity: 1024,
            scene_dir: "scenes".to_string(),
            initial_bpm: 60.0,
            default_transition_mode: SceneTransitionMode::EqualPowerFade,
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("WipEout Track — {SCENE_NAME}"),
                resolution: WindowResolution::new(WIN_W, WIN_H),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(SynthPlugin)
        .add_plugins(Material2dPlugin::<TrackMaterial>::default())
        .add_systems(Startup, setup)
        .add_systems(Update, (
            beat_sync_system,
            update_uniforms_system,
        ).chain())
        .run();
}

// ── Setup ─────────────────────────────────────────────────────────────────────

fn setup(
    mut commands:       Commands,
    mut meshes:         ResMut<Assets<Mesh>>,
    mut track_mats:     ResMut<Assets<TrackMaterial>>,
    cfg:                Res<TrackConfig>,
    mut synth:          MessageWriter<SynthEvent>,
) {
    // Camera with bloom — high intensity to let rail glow spill
    commands.spawn((
        Camera2d,
        Bloom {
            intensity: 0.65,
            low_frequency_boost: 0.75,
            low_frequency_boost_curvature: 0.45,
            high_pass_frequency: 0.70,
            composite_mode: BloomCompositeMode::EnergyConserving,
            ..default()
        },
    ));

    // ── Track fullscreen quad ─────────────────────────────────────────────────
    let track_mat = track_mats.add(TrackMaterial {
        uniforms: TrackUniforms {
            time:       0.0,
            speed:      cfg.base_speed,
            beat_pulse: 0.0,
            _pad:       0.0,
            zone_color: cfg.zone_color,
            fog_color:  cfg.fog_color,
        },
    });
    commands.spawn((
        Mesh2d(meshes.add(Rectangle::new(TRACK_SIZE, TRACK_SIZE))),
        MeshMaterial2d(track_mat.clone()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        TrackQuad { material: track_mat },
    ));

    // ── Load audio scene ─────────────────────────────────────────────────────
    synth.write(SynthEvent::SceneLoad { name: SCENE_NAME.to_string() });
}

// ── Beat sync ─────────────────────────────────────────────────────────────────

fn beat_sync_system(
    synth_params: Option<Res<synth_bevy::SynthParam>>,
    mut state:    ResMut<VisualState>,
    time:         Res<Time>,
) {
    let dt = time.delta_secs();

    // Decay beat pulse
    state.beat_pulse = (state.beat_pulse - dt * 3.5).max(0.0);

    let Some(params) = synth_params else { return };

    let bar = params.bar_epoch.load(Ordering::Relaxed);
    if bar != state.last_bar {
        state.last_bar  = bar;
        state.beat_pulse = 1.0;
    }
}

// ── Uniform update ────────────────────────────────────────────────────────────

fn update_uniforms_system(
    time:       Res<Time>,
    state:      Res<VisualState>,
    cfg:        Res<TrackConfig>,
    track_q:    Query<&TrackQuad>,
    mut track_mats: ResMut<Assets<TrackMaterial>>,
) {
    let t  = time.elapsed_secs();
    let sp = state.speed * cfg.base_speed;
    let bp = state.beat_pulse;

    for quad in &track_q {
        if let Some(mat) = track_mats.get_mut(&quad.material) {
            mat.uniforms.time       = t;
            mat.uniforms.speed      = sp;
            mat.uniforms.beat_pulse = bp;
            mat.uniforms.zone_color = cfg.zone_color;
            mat.uniforms.fog_color  = cfg.fog_color;
        }
    }
}
