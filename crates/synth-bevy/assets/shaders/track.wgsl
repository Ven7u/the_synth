// Retro-synthwave endless track
//
// Sky:   deep purple gradient + large striped sun on horizon
// Floor: pixel-perfect scrolling grid (fwidth derivatives for constant line width)
//        + side rail glow at track edges
//
// UV convention (Bevy Mesh2d Rectangle): (0,0)=bottom-left (1,1)=top-right
// Internal: sy=0 top, sy=1 bottom.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct TrackMaterial {
    time:       f32,
    speed:      f32,
    beat_pulse: f32,
    _pad:       f32,
    zone_color: vec4<f32>,
    fog_color:  vec4<f32>,
}

@group(2) @binding(0) var<uniform> mat: TrackMaterial;

fn smooth_step(t: f32) -> f32 { return t * t * (3.0 - 2.0 * t); }

// Anti-aliased grid line: returns 1.0 on line, 0.0 between
// Uses screen-space derivatives so lines are always ~1.5px wide
fn grid_line(v: f32) -> f32 {
    let d = abs(fract(v + 0.5) - 0.5);
    let fw = fwidth(v) * 1.0;
    return 1.0 - smoothstep(0.0, fw, d);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let sy = uv.y;   // 0=top, 1=bottom (Bevy mesh2d UV: y=0 at top)
    let sx = uv.x;

    let horizon = 0.42;
    let pulse   = 1.0 + mat.beat_pulse * 0.70;
    let z = mat.zone_color.rgb;
    let f = mat.fog_color.rgb;

    // ── SKY ──────────────────────────────────────────────────────────────────
    if sy < horizon {
        let t = sy / horizon;   // 0=screen top, 1=horizon

        // Deep purple gradient
        var sky = mix(vec3<f32>(0.01, 0.0, 0.04), f * 0.65, t * t * t);

        // Horizon glow
        let h_dist = abs(sy - horizon);
        let h_halo = exp(-h_dist * 30.0);
        sky += z * h_halo * (0.9 + mat.beat_pulse * 0.5);

        // Outer atmospheric glow
        let atmos_dx = (sx - 0.5) * 1.6;
        let atmos_dy = sy - horizon;
        let atmos_d2 = atmos_dx * atmos_dx + atmos_dy * atmos_dy;
        sky += z * exp(-atmos_d2 * 2.5) * 0.35 * t;

        return vec4<f32>(sky, 1.0);
    }

    // ── FLOOR ─────────────────────────────────────────────────────────────────
    let depth   = (sy - horizon) / (1.0 - horizon);  // 0=horizon, 1=bottom
    let safe_d  = max(depth, 0.0015);
    let cx      = sx - 0.5;
    let world_x = cx / safe_d;
    let world_z = 1.0 / safe_d + mat.time * mat.speed * 2.2;

    let fog_t = (1.0 - depth) * (1.0 - depth);

    // Track layout
    let track_hw  = 0.76;   // half-width of the racing surface
    let abs_wx    = abs(world_x);

    // ── Floor base ────────────────────────────────────────────────────────────
    // Dark purple base, slightly brighter inside track
    var color = vec3<f32>(0.008, 0.0, 0.022) * (1.0 + step(abs_wx, track_hw) * 0.5);

    // ── Synthwave grid ────────────────────────────────────────────────────────
    // Horizontal lines (parallel to camera, scrolling forward toward viewer)
    let hz_freq = 0.55;     // spacing in world_z units
    let hz_line = grid_line(world_z * hz_freq);

    // Vertical lines (radiating toward vanishing point)
    let vt_freq = 2.5;      // spacing in world_x units
    let vt_line = grid_line(world_x * vt_freq);

    // Combined grid — pink/magenta, brighter inside track
    let grid = max(hz_line, vt_line) * depth;   // depth fades near horizon naturally
    let grid_bright = mix(0.30, 0.75, step(abs_wx, track_hw));
    color += z * grid * grid_bright * pulse;

    // ── Glowing side rails ────────────────────────────────────────────────────
    let rail_l = exp(-abs(world_x + track_hw) * 85.0) * depth;
    let rail_r = exp(-abs(world_x - track_hw) * 85.0) * depth;
    // Extra wide soft glow underneath the sharp rail
    let glow_l = exp(-abs(world_x + track_hw) * 18.0) * depth * 0.4;
    let glow_r = exp(-abs(world_x - track_hw) * 18.0) * depth * 0.4;
    color += z * (rail_l + rail_r + glow_l + glow_r) * 5.0 * pulse;

    // ── Dashed centre line ────────────────────────────────────────────────────
    let dash_on = step(0.45, fract(world_z * 0.22));
    let ctr = exp(-abs(world_x) * 80.0) * dash_on * depth;
    color += (z + vec3<f32>(0.1, 0.1, 0.35)) * ctr * 1.5 * pulse;

    // ── Horizon glow reflected on floor ──────────────────────────────────────
    // A subtle stripe where the floor meets the horizon
    let reflect = exp(-depth * 6.0) * exp(-abs(cx) * 3.0);
    color += z * reflect * 0.25;

    // ── Fog ───────────────────────────────────────────────────────────────────
    color = mix(color, f * 0.30, fog_t * 0.93);

    // ── Vignette ─────────────────────────────────────────────────────────────
    let vx  = (sx - 0.5) * 2.0;
    let vy  = (sy - 0.5) * 2.0;
    color  *= clamp(1.0 - (vx * vx + vy * vy) * 0.25, 0.0, 1.0);

    return vec4<f32>(color, 1.0);
}
