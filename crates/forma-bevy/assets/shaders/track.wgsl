// Retro-synthwave endless track
//
// Sky:   dark gradient + mirrored perspective grid (matrix look) + horizon glow
// Floor: scrolling grid on track; heightfield-raymarched polygon terrain on sides
//
// UV convention: y=0 at top, y=1 at bottom.

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

// Returns vec2(sharp_core, soft_halo) for a grid line at v.
fn grid_glow(v: f32) -> vec2<f32> {
    let d     = abs(fract(v + 0.5) - 0.5);
    let fw    = fwidth(v);
    let sharp = 1.0 - smoothstep(0.0, fw, d);
    let halo  = exp(-d / max(fw * 6.0, 0.0001));
    return vec2<f32>(sharp, halo);
}

fn grid_line(v: f32) -> f32 { return grid_glow(v).x; }

// ── Procedural terrain height ─────────────────────────────────────────────────

fn hash2(ix: f32, iz: f32) -> f32 {
    return fract(sin(ix * 127.1 + iz * 311.7) * 43758.5453);
}

// Height at integer grid vertex — pow sharpens peaks: most cells stay low, few spike
fn vertex_height(ix: f32, iz: f32) -> f32 {
    return pow(hash2(ix, iz), 1.8) * 5.5;
}

// Bilinear interpolation between 4 surrounding vertices
fn terrain_h(wx: f32, wz: f32) -> f32 {
    let cell_x = 0.5;
    let cell_z = 1.2;
    let ix  = floor(wx / cell_x);
    let iz  = floor(wz / cell_z);
    let fx  = fract(wx / cell_x);
    let fz  = fract(wz / cell_z);
    let h00 = vertex_height(ix,       iz      );
    let h10 = vertex_height(ix + 1.0, iz      );
    let h01 = vertex_height(ix,       iz + 1.0);
    let h11 = vertex_height(ix + 1.0, iz + 1.0);
    return mix(mix(h00, h10, fx), mix(h01, h11, fx), fz);
}

// Terrain height only on sides (zero on track)
fn side_terrain_h(wx: f32, wz: f32, track_hw: f32) -> f32 {
    let side = clamp((abs(wx) - track_hw) * 3.0, 0.0, 1.0);
    return terrain_h(wx, wz) * side;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let sy = uv.y;
    let sx = uv.x;
    let cx = sx - 0.5;

    let horizon  = 0.42;
    let pulse    = 1.0 + mat.beat_pulse * 0.70;
    let z        = mat.zone_color.rgb;
    let f        = mat.fog_color.rgb;
    let track_hw = 0.76;

    // ── SKY — mirrored grid + gradient ───────────────────────────────────────
    if sy < horizon {
        let t = sy / horizon;   // 0=top  1=horizon

        var sky = mix(vec3<f32>(0.008, 0.0, 0.028), f * 0.55, t * t * t);

        let mirror_sy = 2.0 * horizon - sy;
        let depth_sky = (mirror_sy - horizon) / (1.0 - horizon);
        let safe_sky  = max(depth_sky, 0.0015);
        let wx_sky    = cx / safe_sky;
        let wz_sky    = 1.0 / safe_sky;

        let hz_sky = grid_line(wz_sky * 0.55);
        let vt_sky = grid_line(wx_sky * 2.5);
        let sky_grid_fade = t * (1.0 - exp(-depth_sky * 3.0));
        sky += z * max(hz_sky, vt_sky) * sky_grid_fade * 1.4 * pulse;

        sky += z * exp(-abs(sy - horizon) * 28.0) * (1.0 + mat.beat_pulse * 0.5);

        let adx = cx * 1.6;
        let ady = sy - horizon;
        sky += z * exp(-(adx * adx + ady * ady) * 2.5) * 0.28 * t;

        return vec4<f32>(sky, 1.0);
    }

    // ── FLOOR ─────────────────────────────────────────────────────────────────
    let depth  = (sy - horizon) / (1.0 - horizon);
    let safe_d = max(depth, 0.0015);
    let scroll = mat.time * mat.speed * 2.2;
    let world_x = cx / safe_d;
    let world_z = 1.0 / safe_d + scroll;

    let fog_t   = (1.0 - depth) * (1.0 - depth);
    let abs_wx  = abs(world_x);
    let on_track = 1.0 - smooth_step(clamp((abs_wx - track_hw) * 5.0, 0.0, 1.0));
    let on_side  = 1.0 - on_track;

    // ── Flat track grid (always computed, blended in later) ───────────────────
    let hz_flat = grid_glow(world_z * 0.55);
    let vt_flat = grid_glow(world_x * 2.5);
    let dramp_flat = depth * (0.4 + depth * 1.8);
    let bright_flat = mix(0.60, 1.40, on_track);
    let sharp_flat  = max(hz_flat.x, vt_flat.x) * dramp_flat * bright_flat;
    let halo_flat   = max(hz_flat.y, vt_flat.y) * dramp_flat * bright_flat * 0.40;
    let cross_flat  = hz_flat.x * vt_flat.x * dramp_flat * 2.5;

    var color = vec3<f32>(0.008, 0.0, 0.022) * (0.7 + on_track * 0.7);
    color += z * (sharp_flat + halo_flat + cross_flat) * pulse;

    // ── Heightfield raymarching for side terrain ──────────────────────────────
    // For a pixel at screen depth D (= depth), terrain at closer depth d > D
    // occludes the pixel if its normalised height h >= 1 - D/d.
    // March d from just past D toward 1.0 (camera near-plane).
    let MAX_H   = 5.5;
    let STEPS   = 16;

    if on_side > 0.05 {
        var hit_found = false;
        var hit_wx    = world_x;
        var hit_wz    = world_z;
        var hit_d     = depth;
        var hit_hn    = 0.0;   // normalised height at hit

        for (var i = 0; i < STEPS; i++) {
            // Linearly interpolate d from depth+step to 1.0
            let d      = depth + (1.0 - depth) * (f32(i) + 1.0) / f32(STEPS);
            let d_safe = max(d, 0.001);
            let m_wx   = cx / d_safe;
            let m_wz   = 1.0 / d_safe + scroll;

            let raw_h  = side_terrain_h(m_wx, m_wz, track_hw * 0.85);
            let h_norm = raw_h / MAX_H;

            // Hit condition: terrain top projects to screen y >= our pixel
            if h_norm >= 1.0 - depth / d_safe {
                hit_found = true;
                hit_wx    = m_wx;
                hit_wz    = m_wz;
                hit_d     = d_safe;
                hit_hn    = h_norm;
                break;
            }
        }

        if hit_found {
            // ── Face shading via central differences ──────────────────────────
            let eps   = 0.10;
            let sl_x  = (side_terrain_h(hit_wx + eps, hit_wz, track_hw * 0.85)
                       - side_terrain_h(hit_wx - eps, hit_wz, track_hw * 0.85))
                       / (2.0 * eps * MAX_H);
            let sl_z  = (side_terrain_h(hit_wx, hit_wz + eps, track_hw * 0.85)
                       - side_terrain_h(hit_wx, hit_wz - eps, track_hw * 0.85))
                       / (2.0 * eps * MAX_H);

            let nrm    = normalize(vec3<f32>(-sl_x, 1.0, -sl_z));
            let light  = normalize(vec3<f32>(0.4, 1.0, 0.3));
            let diffuse = clamp(dot(nrm, light), 0.0, 1.0);
            // Boost contrast so faces read clearly
            let face   = 0.20 + 0.80 * diffuse;

            // ── Wireframe grid on terrain surface ─────────────────────────────
            let hz   = grid_glow(hit_wz * 0.55);
            let vt   = grid_glow(hit_wx * 2.5);
            let dramp = hit_d * (0.4 + hit_d * 1.8);
            let sharp = max(hz.x, vt.x) * dramp;
            let halo  = max(hz.y, vt.y) * dramp * 0.40;
            let cross = hz.x * vt.x * dramp * 2.5;

            // ── Terrain pixel color ───────────────────────────────────────────
            var t_col = vec3<f32>(0.006, 0.0, 0.018) * face;
            // Grid lines on the terrain face
            t_col += z * (sharp + halo + cross) * pulse * face;
            // Peak-height glow (tallest peaks are brightest)
            t_col += z * hit_hn * hit_d * 0.35 * face;

            // Fog
            let t_fog = (1.0 - hit_d) * (1.0 - hit_d);
            t_col = mix(t_col, f * 0.30, t_fog * 0.93);

            // Blend terrain over flat color weighted by on_side
            color = mix(color, t_col, on_side);
        }
        // If no hit, on_side pixels keep the flat grid color (looks like low ground)
    }

    // ── Side rails ────────────────────────────────────────────────────────────
    let rail_l = exp(-abs(world_x + track_hw) * 85.0) * depth;
    let rail_r = exp(-abs(world_x - track_hw) * 85.0) * depth;
    let glow_l = exp(-abs(world_x + track_hw) * 18.0) * depth * 0.4;
    let glow_r = exp(-abs(world_x - track_hw) * 18.0) * depth * 0.4;
    color += z * (rail_l + rail_r + glow_l + glow_r) * 5.0 * pulse;

    // ── Dashed centre line ────────────────────────────────────────────────────
    let dash_on = step(0.45, fract(world_z * 0.22));
    color += (z + vec3<f32>(0.1, 0.1, 0.35))
           * exp(-abs(world_x) * 80.0) * dash_on * depth * 1.5 * pulse;

    // ── Horizon reflection ────────────────────────────────────────────────────
    color += z * exp(-depth * 6.0) * exp(-abs(cx) * 3.0) * 0.22;

    // ── Fog + vignette ────────────────────────────────────────────────────────
    color  = mix(color, f * 0.30, fog_t * 0.93);
    let vx = (sx - 0.5) * 2.0;
    let vy = (sy - 0.5) * 2.0;
    color *= clamp(1.0 - (vx * vx + vy * vy) * 0.25, 0.0, 1.0);

    return vec4<f32>(color, 1.0);
}
