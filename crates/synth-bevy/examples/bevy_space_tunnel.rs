//! Space Tunnel — Bevy-controlled ambient timeline example.
//!
//! Visual features:
//!   - 220 stars as line segments with perspective depth rush.
//!   - Star width and trail length scale with proximity (thin/short far, thick/long near).
//!   - Slow continuous tunnel rotation — the field drifts.
//!   - 4 gate rings that fly through the tunnel and pulse on every bar boundary.
//!   - Per-section nebula glow that breathes on every 16th-note subdivision.
//!   - Full-screen white flash on section advance.
//!   - Speed burst on advance — spike then smooth settle to new section speed.
//!   - Palette crossfades over 8 seconds between sections.
//!
//! Audio:
//!   - Loads scenes/Inception.json (4-section Markov timeline).
//!   - Bevy controls when to advance sections via SynthEvent::TimelineAdvance.
//!   - bar_epoch and subdivision_epoch drive visual sync.
//!
//! Run:
//!   cargo run -p synth-bevy --example bevy_space_tunnel --features bevy --release

use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::atomic::Ordering;

use bevy::{
    core_pipeline::bloom::Bloom,
    prelude::*,
    sprite::{ColorMaterial, MeshMaterial2d},
};
use synth_bevy::{SceneTransitionMode, SynthBackendKind, SynthBevyConfig, SynthEvent,
    SynthParam, SynthPlugin};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const STAR_COUNT:  usize = 220;
const RING_COUNT:  usize = 4;
const Z_FAR:       f32   = 1600.0;
const Z_NEAR:      f32   = 20.0;
const TUNNEL_RADIUS: f32 = 900.0;
const BASE_SPEED:  f32   = 180.0;
const SCENE_NAME:  &str  = "Inception";
/// Seconds before Bevy force-advances to the next section.
const SECTION_ADVANCE_SECS: [f32; 4] = [28.0, 38.0, 42.0, 9999.0];
/// Speed spike multiplier applied on section advance, then decays back.
const ADVANCE_SPEED_SPIKE: f32 = 3.5;

// ---------------------------------------------------------------------------
// Palettes
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Palette {
    background: LinearRgba,
    star_inner:  LinearRgba,
    star_outer:  LinearRgba,
    ring_color:  LinearRgba,
    nebula:      LinearRgba,
}

fn lrgb(r: f32, g: f32, b: f32) -> LinearRgba { LinearRgba::new(r, g, b, 1.0) }

fn lerp_color(a: LinearRgba, b: LinearRgba, t: f32) -> LinearRgba {
    LinearRgba::new(
        a.red   + (b.red   - a.red)   * t,
        a.green + (b.green - a.green) * t,
        a.blue  + (b.blue  - a.blue)  * t,
        a.alpha + (b.alpha - a.alpha) * t,
    )
}

fn palette_sedation() -> Palette {
    Palette {
        background: lrgb(0.003, 0.004, 0.018),
        star_inner:  lrgb(0.20, 0.55, 2.80),
        star_outer:  lrgb(0.04, 0.10, 0.40),
        ring_color:  lrgb(0.10, 0.30, 1.80),
        nebula:      lrgb(0.05, 0.10, 0.60),
    }
}

fn palette_first_level() -> Palette {
    Palette {
        background: lrgb(0.006, 0.004, 0.028),
        star_inner:  lrgb(1.00, 0.40, 3.20),
        star_outer:  lrgb(0.08, 0.04, 0.55),
        ring_color:  lrgb(0.60, 0.20, 2.50),
        nebula:      lrgb(0.15, 0.05, 0.80),
    }
}

fn palette_second_level() -> Palette {
    Palette {
        background: lrgb(0.010, 0.003, 0.035),
        star_inner:  lrgb(2.40, 0.30, 3.00),
        star_outer:  lrgb(0.20, 0.05, 0.60),
        ring_color:  lrgb(1.80, 0.10, 2.80),
        nebula:      lrgb(0.30, 0.05, 1.00),
    }
}

fn palette_limbo() -> Palette {
    Palette {
        background: lrgb(0.025, 0.020, 0.045),
        star_inner:  lrgb(6.00, 5.60, 7.20),
        star_outer:  lrgb(0.40, 0.35, 0.60),
        ring_color:  lrgb(4.00, 3.60, 5.00),
        nebula:      lrgb(0.60, 0.55, 0.80),
    }
}

fn section_palette(idx: usize) -> Palette {
    match idx {
        0 => palette_sedation(),
        1 => palette_first_level(),
        2 => palette_second_level(),
        _ => palette_limbo(),
    }
}

fn section_speed_mult(idx: usize) -> f32 {
    [0.30, 0.65, 1.20, 2.20][idx.min(3)]
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Star {
    angle: f32,
    phase: f32,
    z:     f32,
    material: Handle<ColorMaterial>,
}

/// One of the gate rings that fly through the tunnel.
#[derive(Component)]
struct Ring {
    z: f32,
    /// Base angle for slow personal spin.
    spin: f32,
    material: Handle<ColorMaterial>,
    /// Scale target for bar-pulse animation (1.0 = rest).
    pulse_scale: f32,
}

/// Full-screen white flash spawned on section advance.
#[derive(Component)]
struct FlashOverlay {
    alpha: f32,
}

/// Single nebula glow behind everything.
#[derive(Component)]
struct Nebula {
    material: Handle<ColorMaterial>,
}

fn xorshift(x: &mut u32) -> f32 {
    *x ^= *x << 13;
    *x ^= *x >> 17;
    *x ^= *x << 5;
    *x as f32 / u32::MAX as f32
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

#[derive(Resource)]
struct TunnelState {
    section:          usize,
    section_elapsed:  f32,
    speed_smooth:     f32,
    /// Global rotation offset applied to all star angles (radians).
    rotation:         f32,
    last_epoch:       usize,
    last_bar_epoch:   usize,
    last_subdiv_epoch: usize,
}

impl Default for TunnelState {
    fn default() -> Self {
        Self {
            section:          0,
            section_elapsed:  0.0,
            speed_smooth:     BASE_SPEED * section_speed_mult(0),
            rotation:         0.0,
            last_epoch:       0,
            last_bar_epoch:   0,
            last_subdiv_epoch: 0,
        }
    }
}

#[derive(Resource)]
struct PaletteFade {
    from:     Palette,
    to:       Palette,
    progress: f32,
    duration: f32,
}

impl PaletteFade {
    fn new(p: Palette) -> Self {
        Self { from: p, to: p, progress: 1.0, duration: 6.0 }
    }

    fn start(&mut self, next: Palette, dur: f32) {
        let t = smooth_step(self.progress);
        self.from = lerp_palette(self.from, self.to, t);
        self.to = next;
        self.progress = 0.0;
        self.duration = dur;
    }

    fn current(&self) -> Palette {
        lerp_palette(self.from, self.to, smooth_step(self.progress))
    }
}

fn smooth_step(t: f32) -> f32 { t * t * (3.0 - 2.0 * t) }

fn lerp_palette(a: Palette, b: Palette, t: f32) -> Palette {
    Palette {
        background: lerp_color(a.background, b.background, t),
        star_inner:  lerp_color(a.star_inner,  b.star_inner,  t),
        star_outer:  lerp_color(a.star_outer,  b.star_outer,  t),
        ring_color:  lerp_color(a.ring_color,  b.ring_color,  t),
        nebula:      lerp_color(a.nebula,       b.nebula,       t),
    }
}

#[derive(Resource)]
struct TunnelTimeline {
    section_names: Vec<String>,
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let initial_palette = palette_sedation();

    App::new()
        .insert_resource(ClearColor(Color::LinearRgba(initial_palette.background)))
        .insert_resource(SynthBevyConfig {
            backend: SynthBackendKind::Ambient,
            sample_rate_hz: 44_100.0,
            control_capacity: 1024,
            scene_dir: "scenes".to_string(),
            initial_bpm: 60.0,
            default_transition_mode: SceneTransitionMode::EqualPowerFade,
        })
        .insert_resource(TunnelState::default())
        .insert_resource(PaletteFade::new(initial_palette))
        .insert_resource(TunnelTimeline {
            section_names: vec![
                "Sedation".to_string(),
                "First Level".to_string(),
                "Second Level".to_string(),
                "Limbo".to_string(),
            ],
        })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Space Tunnel — Inception".to_string(),
                resolution: (1200.0, 750.0).into(),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .add_plugins(SynthPlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, (
            beat_sync_system,
            tunnel_motion_system,
            ring_system,
            nebula_system,
            flash_system,
            palette_fade_system,
            timeline_conductor_system,
        ).chain())
        .run();
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

fn setup(
    mut commands: Commands,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    palette_fade:  Res<PaletteFade>,
    mut synth:     EventWriter<SynthEvent>,
) {
    commands.spawn((
        Camera2d,
        Camera { hdr: true, ..default() },
        Bloom {
            intensity: 0.40,
            low_frequency_boost: 0.65,
            low_frequency_boost_curvature: 0.55,
            high_pass_frequency: 0.85,
            composite_mode: BloomCompositeMode::EnergyConserving,
            ..default()
        },
    ));

    let palette = palette_fade.current();

    // ── Nebula — large soft circle behind everything ─────────────────────────
    {
        let neb_color = LinearRgba::new(
            palette.nebula.red,
            palette.nebula.green,
            palette.nebula.blue,
            0.18,
        );
        let mat = materials.add(ColorMaterial::from_color(Color::LinearRgba(neb_color)));
        commands.spawn((
            Mesh2d(meshes.add(Circle::new(480.0))),
            MeshMaterial2d(mat.clone()),
            Transform::from_xyz(0.0, 0.0, -1.0),
            Nebula { material: mat },
        ));
    }

    // ── Stars ─────────────────────────────────────────────────────────────────
    let mut rng: u32 = 0xDEAD_CAFE;
    for i in 0..STAR_COUNT {
        let angle = (i as f32 / STAR_COUNT as f32) * TAU + xorshift(&mut rng) * 0.10;
        let z     = xorshift(&mut rng) * Z_FAR;
        let phase = xorshift(&mut rng);

        let depth_t = z / Z_FAR;
        let color = lerp_color(palette.star_inner, palette.star_outer, depth_t);
        let mat = materials.add(ColorMaterial::from_color(Color::LinearRgba(color)));

        let nearness = 1.0 - depth_t;
        let trail = nearness * nearness * 20.0 + 2.0;
        let width = nearness * nearness * 3.0 + 1.0;
        let mesh = meshes.add(Rectangle::new(width, trail));

        let (sx, sy) = star_screen_pos(angle, z, 0.0);
        commands.spawn((
            Mesh2d(mesh),
            MeshMaterial2d(mat.clone()),
            Transform::from_xyz(sx, sy, 0.0)
                .with_rotation(Quat::from_rotation_z(angle + FRAC_PI_2)),
            Star { angle, phase, z, material: mat },
        ));
    }

    // ── Gate rings ────────────────────────────────────────────────────────────
    let ring_z_positions = [400.0f32, 700.0, 1000.0, 1300.0];
    for (i, &z) in ring_z_positions.iter().enumerate() {
        let spin = i as f32 * TAU / RING_COUNT as f32;
        let mat = materials.add(ColorMaterial::from_color(
            Color::LinearRgba(LinearRgba::new(
                palette.ring_color.red,
                palette.ring_color.green,
                palette.ring_color.blue,
                0.7,
            ))
        ));
        let r = ring_radius_at_z(z);
        commands.spawn((
            Mesh2d(meshes.add(Annulus::new(r - 2.0, r + 2.0))),
            MeshMaterial2d(mat.clone()),
            Transform::from_xyz(0.0, 0.0, 0.5),
            Ring { z, spin, material: mat, pulse_scale: 1.0 },
        ));
    }

    synth.send(SynthEvent::SceneLoad { name: SCENE_NAME.to_string() });
}

fn star_screen_pos(angle: f32, z: f32, rotation: f32) -> (f32, f32) {
    let a = angle + rotation;
    let p = Z_NEAR / z.max(Z_NEAR);
    let r = TUNNEL_RADIUS * p;
    (a.cos() * r, a.sin() * r)
}

/// Ring outer radius at a given Z depth.
fn ring_radius_at_z(z: f32) -> f32 {
    let p = Z_NEAR / z.max(Z_NEAR);
    TUNNEL_RADIUS * p
}

// ---------------------------------------------------------------------------
// Beat sync — detect bar and subdivision edges, drive ring pulse and nebula
// ---------------------------------------------------------------------------

fn beat_sync_system(
    params:    Option<Res<SynthParam>>,
    mut tunnel: ResMut<TunnelState>,
    mut rings:  Query<&mut Ring>,
) {
    let Some(params) = params else { return; };

    let bar_epoch   = params.bar_epoch.load(Ordering::Relaxed);
    let subdiv_epoch = params.subdivision_epoch.load(Ordering::Relaxed);

    // Bar boundary — pulse all rings
    if bar_epoch != tunnel.last_bar_epoch {
        tunnel.last_bar_epoch = bar_epoch;
        for mut ring in &mut rings {
            ring.pulse_scale = 1.25; // will decay in ring_system
        }
    }

    tunnel.last_subdiv_epoch = subdiv_epoch;
}

// ---------------------------------------------------------------------------
// Star motion
// ---------------------------------------------------------------------------

fn tunnel_motion_system(
    time:          Res<Time>,
    mut tunnel:    ResMut<TunnelState>,
    palette_fade:  Res<PaletteFade>,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut stars:     Query<(&mut Transform, &mut Star, &Mesh2d)>,
) {
    let dt    = time.delta_secs();
    let speed = tunnel.speed_smooth;
    let palette = palette_fade.current();

    // Slow global rotation — speeds up slightly with section
    let rot_speed = 0.003 + tunnel.section as f32 * 0.002;
    tunnel.rotation += rot_speed * dt;

    for (mut tf, mut star, mesh_h) in &mut stars {
        star.z -= speed * dt;
        if star.z < Z_NEAR {
            star.z = Z_FAR + (star.z - Z_NEAR);
            star.angle += (star.phase - 0.5) * 0.015;
        }

        let (sx, sy) = star_screen_pos(star.angle, star.z, tunnel.rotation);
        tf.translation.x = sx;
        tf.translation.y = sy;
        tf.rotation = Quat::from_rotation_z(star.angle + tunnel.rotation + FRAC_PI_2);

        let depth_t  = ((star.z - Z_NEAR) / (Z_FAR - Z_NEAR)).clamp(0.0, 1.0);
        let nearness = 1.0 - depth_t;

        // Trail and width both scale quadratically with nearness
        let trail = nearness * nearness * 80.0 + 2.0;
        let width = nearness * nearness * 5.0 + 1.0;
        if let Some(mesh) = meshes.get_mut(&mesh_h.0) {
            *mesh = Rectangle::new(width, trail).into();
        }

        let color = lerp_color(palette.star_inner, palette.star_outer, depth_t);
        if let Some(mat) = materials.get_mut(&star.material) {
            mat.color = Color::LinearRgba(color);
        }
    }
}

// ---------------------------------------------------------------------------
// Ring system
// ---------------------------------------------------------------------------

fn ring_system(
    time:          Res<Time>,
    tunnel:        Res<TunnelState>,
    palette_fade:  Res<PaletteFade>,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut rings:     Query<(&mut Transform, &mut Ring, &Mesh2d)>,
) {
    let dt      = time.delta_secs();
    let speed   = tunnel.speed_smooth;
    let palette = palette_fade.current();

    for (mut tf, mut ring, mesh_h) in &mut rings {
        ring.z -= speed * dt;
        if ring.z < Z_NEAR {
            ring.z = Z_FAR + (ring.z - Z_NEAR);
        }

        // Slow personal spin
        ring.spin += dt * 0.08;

        // Pulse decay back toward 1.0
        ring.pulse_scale = 1.0 + (ring.pulse_scale - 1.0) * (1.0 - dt * 8.0).max(0.0);

        let r = ring_radius_at_z(ring.z);
        let scale = ring.pulse_scale;
        tf.translation.z = 0.5;
        tf.rotation = Quat::from_rotation_z(ring.spin);
        tf.scale = Vec3::splat(scale);

        // Rebuild ring mesh to match depth
        if let Some(mesh) = meshes.get_mut(&mesh_h.0) {
            let thickness = (2.0 * Z_NEAR / ring.z.max(Z_NEAR)).max(1.0);
            *mesh = Annulus::new((r - thickness).max(0.1), r + thickness).into();
        }

        // Colour + alpha by depth
        let depth_t = ((ring.z - Z_NEAR) / (Z_FAR - Z_NEAR)).clamp(0.0, 1.0);
        let alpha = (1.0 - depth_t) * 0.85 * ring.pulse_scale.min(1.5);
        let c = palette.ring_color;
        if let Some(mat) = materials.get_mut(&ring.material) {
            mat.color = Color::LinearRgba(LinearRgba::new(c.red, c.green, c.blue, alpha));
        }
    }
}

// ---------------------------------------------------------------------------
// Nebula — breathes on subdivisions
// ---------------------------------------------------------------------------

fn nebula_system(
    params:        Option<Res<SynthParam>>,
    tunnel:        Res<TunnelState>,
    palette_fade:  Res<PaletteFade>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    nebula_q:      Query<&Nebula>,
    time:          Res<Time>,
) {
    let Some(params) = params else { return; };
    let subdiv = params.subdivision_epoch.load(Ordering::Relaxed);
    let palette = palette_fade.current();

    // Slow sine breath + a quick kick on each subdivision
    let beat_t = (subdiv as f32 * 0.25 + time.elapsed_secs() * 0.4).sin() * 0.5 + 0.5;
    let alpha = 0.10 + beat_t * 0.12 * (1.0 + tunnel.section as f32 * 0.15);

    for neb in &nebula_q {
        if let Some(mat) = materials.get_mut(&neb.material) {
            let c = palette.nebula;
            mat.color = Color::LinearRgba(LinearRgba::new(c.red, c.green, c.blue, alpha));
        }
    }
}

// ---------------------------------------------------------------------------
// Flash overlay — fades out quickly after section advance
// ---------------------------------------------------------------------------

fn flash_system(
    mut commands:  Commands,
    time:          Res<Time>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut flashes:   Query<(Entity, &mut FlashOverlay, &MeshMaterial2d<ColorMaterial>)>,
) {
    let dt = time.delta_secs();
    for (entity, mut flash, mat_h) in &mut flashes {
        flash.alpha -= dt * 3.5; // ~285ms fade
        if flash.alpha <= 0.0 {
            commands.entity(entity).despawn();
        } else if let Some(mat) = materials.get_mut(&mat_h.0) {
            mat.color = Color::LinearRgba(LinearRgba::new(1.0, 1.0, 1.0, flash.alpha));
        }
    }
}

fn spawn_flash(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<ColorMaterial>) {
    let mat = materials.add(ColorMaterial::from_color(
        Color::LinearRgba(LinearRgba::new(1.0, 1.0, 1.0, 1.0)),
    ));
    commands.spawn((
        Mesh2d(meshes.add(Rectangle::new(3000.0, 3000.0))),
        MeshMaterial2d(mat),
        Transform::from_xyz(0.0, 0.0, 10.0),
        FlashOverlay { alpha: 1.0 },
    ));
}

// ---------------------------------------------------------------------------
// Palette fade
// ---------------------------------------------------------------------------

fn palette_fade_system(
    time: Res<Time>,
    mut fade: ResMut<PaletteFade>,
    mut clear_color: ResMut<ClearColor>,
) {
    if fade.progress >= 1.0 { return; }
    fade.progress = (fade.progress + time.delta_secs() / fade.duration).min(1.0);
    clear_color.0 = Color::LinearRgba(fade.current().background);
}

// ---------------------------------------------------------------------------
// Timeline conductor — Bevy decides when to advance
// ---------------------------------------------------------------------------

fn timeline_conductor_system(
    time:          Res<Time>,
    mut tunnel:    ResMut<TunnelState>,
    mut fade:      ResMut<PaletteFade>,
    tl:            Res<TunnelTimeline>,
    params:        Option<Res<SynthParam>>,
    mut commands:  Commands,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut synth:     EventWriter<SynthEvent>,
) {
    let dt = time.delta_secs();
    let Some(params) = params else { return; };

    // ── Speed update ─────────────────────────────────────────────────────────
    let target_speed = BASE_SPEED * section_speed_mult(tunnel.section);
    let rate = if target_speed > tunnel.speed_smooth { 6.0 } else { 3.0 };
    tunnel.speed_smooth += (target_speed - tunnel.speed_smooth) * (rate * dt).min(1.0);

    // ── Phrase epoch ─────────────────────────────────────────────────────────
    let epoch = params.phrase_epoch.load(Ordering::Relaxed);
    tunnel.last_epoch = epoch;
    tunnel.section_elapsed += dt;

    // ── Section advance decision ──────────────────────────────────────────────
    let last_section = tl.section_names.len().saturating_sub(1);
    if tunnel.section < last_section {
        let time_up = tunnel.section_elapsed >= SECTION_ADVANCE_SECS[tunnel.section];
        let speed_frac = tunnel.speed_smooth / (BASE_SPEED * section_speed_mult(tunnel.section));
        let visual_saturated = speed_frac > 0.97 && tunnel.section_elapsed > 12.0;

        if time_up || visual_saturated {
            let next = tunnel.section + 1;
            info!("[tunnel] {} → {}", tl.section_names[tunnel.section],
                  tl.section_names.get(next).map(|s| s.as_str()).unwrap_or("end"));

            synth.send(SynthEvent::TimelineAdvance);
            fade.start(section_palette(next), 8.0);
            spawn_flash(&mut commands, &mut meshes, &mut materials);

            // Speed spike — smoother will decay it back to target
            tunnel.speed_smooth = BASE_SPEED * section_speed_mult(tunnel.section) * ADVANCE_SPEED_SPIKE;

            tunnel.section = next;
            tunnel.section_elapsed = 0.0;
        }
    }
}
