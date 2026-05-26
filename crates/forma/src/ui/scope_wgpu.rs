use bytemuck::{Pod, Zeroable};
use eframe::egui::epaint::PaintCallbackInfo;
use eframe::egui_wgpu;
use egui_wgpu::wgpu;
use wgpu::util::DeviceExt;

// ── Visualization mode ────────────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum VizMode {
    #[default]
    Scope,
    Harmonograph,
    Voronoi,
    Spectrum,
    Envelope,
    Spectrogram,
    SpectrogramV,
}

// ── Harmonograph uniform params (48 bytes, 3 × vec4) ──────────────────────────
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct HarmParams {
    pub freqs: [f32; 4],  // pendulum frequencies f1..f4
    pub phases: [f32; 4], // phase offsets p1..p4
    pub damping: f32,
    pub t_max: f32,
    pub viewport: [f32; 2], // (width, height) px — needed for pixel-accurate ribbon width
}

// ── Voronoi uniform params (272 bytes = 16 × vec4 + 1 × vec4) ────────────────
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct VorParams {
    pub seeds: [[f32; 4]; 16], // seeds[i].xy = UV position (0..1)
    pub num_seeds: u32,
    pub beat_pulse: f32,
    pub tex_w: f32,
    pub tex_h: f32,
}

// ── WGSL: render waveform vertices into offscreen texture ─────────────────────
const WAVEFORM_SHADER: &str = r#"
@vertex
fn vs_main(@location(0) pos: vec2<f32>) -> @builtin(position) vec4<f32> {
    return vec4<f32>(pos.x, pos.y, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // Full-intensity phosphor — the CRT pass handles bloom/glow
    return vec4<f32>(0.13, 1.0, 0.55, 1.0);
}
"#;

// ── WGSL: harmonograph triangle-strip ribbon (same style as waveform) ────────
const HARMONOGRAPH_SHADER: &str = r#"
const N_STEPS: u32      = 3000u;
const RIBBON_HALF_W: f32 = 3.0;   // half-width in pixels (matches waveform)

struct HarmParams {
    freqs:    vec4<f32>,
    phases:   vec4<f32>,
    damping:  f32,
    t_max:    f32,
    viewport: vec2<f32>,  // (width, height) in pixels
}

struct VertOut {
    @builtin(position) pos:   vec4<f32>,
    @location(0)       alpha: f32,
}

@group(0) @binding(0) var<uniform> h: HarmParams;

fn harm_pos(t: f32) -> vec2<f32> {
    let d = exp(-h.damping * t);
    let x = (sin(h.freqs.x * t + h.phases.x) + 0.3 * sin(h.freqs.y * t + h.phases.y)) * d * 0.9;
    let y = (sin(h.freqs.z * t + h.phases.z) + 0.3 * sin(h.freqs.w * t + h.phases.w)) * d * 0.9;
    return vec2<f32>(x, y);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertOut {
    let pt_idx = vi / 2u;
    let side   = vi % 2u;

    let dt = h.t_max / f32(N_STEPS - 1u);
    let t  = f32(pt_idx) * dt;
    let p0 = harm_pos(t);

    // Central-difference tangent (forward/backward at endpoints)
    var tangent: vec2<f32>;
    if pt_idx == 0u {
        tangent = harm_pos(dt) - p0;
    } else if pt_idx >= N_STEPS - 1u {
        tangent = p0 - harm_pos(t - dt);
    } else {
        tangent = harm_pos(t + dt) - harm_pos(t - dt);
    }

    // Work in pixel space so the ribbon width is truly pixel-accurate
    let aspect   = h.viewport.x / h.viewport.y;
    let tang_px  = vec2<f32>(tangent.x * aspect, tangent.y);
    let tang_norm = normalize(tang_px + vec2<f32>(1e-6, 0.0));

    // Perpendicular (rotated 90°)
    let perp_px = vec2<f32>(-tang_norm.y, tang_norm.x);

    // Variable half-width: wider on flat segments, narrower on steep (matches waveform)
    let steepness  = abs(tang_norm.y);
    let half_w_px  = RIBBON_HALF_W - steepness * 1.8;
    let half_w_ndc = half_w_px * 2.0 / h.viewport.y;

    // Back to NDC
    let perp_ndc = vec2<f32>(perp_px.x / aspect, perp_px.y) * half_w_ndc;
    let sign = select(-1.0, 1.0, side == 0u);

    var out: VertOut;
    out.pos   = vec4<f32>(p0 + perp_ndc * sign, 0.0, 1.0);
    out.alpha = exp(-h.damping * t); // fade ribbon as pendulum decays
    return out;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    return vec4<f32>(0.13, 1.0, 0.55, in.alpha);
}
"#;

// ── WGSL: voronoi distance-field (fullscreen, GPU-computed) ───────────────────
const VORONOI_SHADER: &str = r#"
struct VertOut {
    @builtin(position) pos: vec4<f32>,
    @location(0)       uv:  vec2<f32>,
}

struct VorParams {
    seeds:      array<vec4<f32>, 16>,
    num_seeds:  u32,
    beat_pulse: f32,
    tex_w:      f32,
    tex_h:      f32,
}

@group(0) @binding(0) var<uniform> v: VorParams;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uv = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var o: VertOut;
    o.pos = vec4<f32>(p[vi], 0.0, 1.0);
    o.uv  = uv[vi];
    return o;
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    if in.uv.x < 0.0 || in.uv.x > 1.0 || in.uv.y < 0.0 || in.uv.y > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let aspect = v.tex_w / v.tex_h;
    let p = vec2<f32>((in.uv.x - 0.5) * aspect, in.uv.y - 0.5);

    // Silence: single seed → draw a radial glow dot
    if v.num_seeds <= 1u {
        let sp = vec2<f32>((v.seeds[0].x - 0.5) * aspect, v.seeds[0].y - 0.5);
        let d  = length(p - sp);
        let g  = smoothstep(0.06, 0.0, d);
        return vec4<f32>(0.10 * g, 0.65 * g, 0.42 * g, 1.0);
    }

    var d1 = 1e9;
    var d2 = 1e9;

    for (var i = 0u; i < 16u; i++) {
        if i >= v.num_seeds { break; }
        let sp = vec2<f32>((v.seeds[i].x - 0.5) * aspect, v.seeds[i].y - 0.5);
        let d  = length(p - sp);
        if d < d1 { d2 = d1; d1 = d; } else if d < d2 { d2 = d; }
    }

    // Thinner edge, lower luminosity so overlapping cells stay readable
    let edge   = d2 - d1;
    let edge_w = 0.008 + v.beat_pulse * 0.004;
    let glow   = smoothstep(edge_w + 0.010, 0.0, edge);

    return vec4<f32>(0.08 * glow, 0.52 * glow, 0.36 * glow, 1.0);
}
"#;

// ── WGSL: CRT post-process — barrel + scanlines + bloom ──────────────────────
const CRT_SHADER: &str = r#"
struct VertOut {
    @builtin(position) pos: vec4<f32>,
    @location(0)       uv:  vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertOut {
    // Full-screen triangle (covers the quad without a vertex buffer)
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uv = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var o: VertOut;
    o.pos = vec4<f32>(p[vi], 0.0, 1.0);
    o.uv  = uv[vi];
    return o;
}

struct Params {
    resolution:  vec2<f32>,
    bloom_scale: f32,
    bypass:      f32,  // 1.0 = output texture directly, skipping barrel/bloom
}

@group(0) @binding(0) var t_scope:  texture_2d<f32>;
@group(0) @binding(1) var s_scope:  sampler;
@group(0) @binding(2) var<uniform> p: Params;

// Barrel distortion — pulls edges inward like a CRT glass screen
fn barrel(uv: vec2<f32>, strength: f32) -> vec2<f32> {
    let d  = uv - vec2<f32>(0.5);
    let r2 = dot(d, d);
    return uv + d * (r2 * strength);
}

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    // Bypass mode: output texture directly (no barrel, no bloom) — used for spectrogram.
    if p.bypass > 0.5 {
        return textureSample(t_scope, s_scope, in.uv);
    }

    let uv = barrel(in.uv, 0.18);

    // Anything outside the barrel maps to solid black (curved-screen border)
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let px = 1.0 / p.resolution;

    // ── Bloom: two-radius additive pass for a thick phosphor glow ─────────────
    // Inner ring (r=3): tight bright core that gives the line apparent thickness
    let r1: f32 = 3.0;
    var inner = textureSample(t_scope, s_scope, uv).rgb                        * 0.65;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>( r1,  0.0) * px).rgb * 0.20;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r1,  0.0) * px).rgb * 0.20;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>(0.0,  r1)  * px).rgb * 0.20;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>(0.0, -r1)  * px).rgb * 0.20;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>( r1,  r1)  * px).rgb * 0.10;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r1,  r1)  * px).rgb * 0.10;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>( r1, -r1)  * px).rgb * 0.10;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r1, -r1)  * px).rgb * 0.10;

    // Outer ring (r=10): wide soft halo that gives the phosphor bloom
    let r2: f32 = 10.0;
    var outer = textureSample(t_scope, s_scope, uv + vec2<f32>( r2,  0.0) * px).rgb * 0.18;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r2,  0.0) * px).rgb * 0.18;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>(0.0,  r2)  * px).rgb * 0.18;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>(0.0, -r2)  * px).rgb * 0.18;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>( r2,  r2)  * px).rgb * 0.08;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r2,  r2)  * px).rgb * 0.08;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>( r2, -r2)  * px).rgb * 0.08;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r2, -r2)  * px).rgb * 0.08;

    var col = inner + outer * (0.85 * p.bloom_scale);

    // ── Scanlines: sinusoidal brightness modulation per raster line ───────────
    let scan = sin(uv.y * p.resolution.y * 3.14159265) * 0.5 + 0.5;
    col *= mix(0.88, 1.0, scan);

    // ── Vignette: darken corners to match CRT glass curvature ────────────────
    let d2 = in.uv - vec2<f32>(0.5);
    col *= max(1.0 - dot(d2, d2) * 1.8, 0.0);

    return vec4<f32>(col, 1.0);
}
"#;

// ── WGSL: spectrogram — reads R8Unorm ring-buffer texture, applies phosphor colormap ──
const SPECTROGRAM_SHADER: &str = r#"
struct SgrParams {
    write_col : u32,
    sgr_cols  : u32,
    sgr_rows  : u32,
    vertical  : u32,   // 0 = X=time Y=freq, 1 = X=freq Y=time(bottom→top)
    tex_w     : f32,
    tex_h     : f32,
    _pad2     : vec2<f32>,
}

@group(0) @binding(0) var sgr_data : texture_2d<f32>;
@group(0) @binding(1) var<uniform> sgr : SgrParams;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    return vec4<f32>(p[vi], 0.0, 1.0);
}

fn phosphor_heat(v: f32) -> vec3<f32> {
    let c = clamp(v, 0.0, 1.0);
    if c < 0.35 {
        let t = c / 0.35;
        return mix(vec3<f32>(0.016, 0.047, 0.035), vec3<f32>(0.086, 0.439, 0.216), vec3<f32>(t, t, t));
    } else if c < 0.65 {
        let t = (c - 0.35) / 0.30;
        return mix(vec3<f32>(0.086, 0.439, 0.216), vec3<f32>(0.125, 0.753, 0.447), vec3<f32>(t, t, t));
    } else if c < 0.88 {
        let t = (c - 0.65) / 0.23;
        return mix(vec3<f32>(0.125, 0.753, 0.447), vec3<f32>(0.282, 1.000, 0.761), vec3<f32>(t, t, t));
    } else {
        let t = (c - 0.88) / 0.12;
        return mix(vec3<f32>(0.282, 1.000, 0.761), vec3<f32>(1.000, 1.000, 1.000), vec3<f32>(t, t, t));
    }
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // Padding matches draw_sgr_labels() in scope.rs
    let pad_l: f32 = 38.0;
    let pad_b: f32 = 18.0;
    let pad_t: f32 =  6.0;
    let pad_r: f32 =  6.0;

    let ix = pos.x - pad_l;
    let iy = pos.y - pad_t;
    let iw = sgr.tex_w - pad_l - pad_r;
    let ih = sgr.tex_h - pad_t - pad_b;

    // Background outside the data area
    if ix < 0.0 || iy < 0.0 || ix >= iw || iy >= ih {
        return vec4<f32>(0.016, 0.047, 0.035, 1.0);
    }

    let frac_x = ix / iw;
    // frac_y=0 = top of data area, frac_y=1 = bottom.
    let frac_y = iy / ih;

    // Texture layout: width = sgr_rows (freq), height = sgr_cols (time).
    var freq_idx: u32;
    var time_col: u32;

    if sgr.vertical == 0u {
        // Horizontal: X = time (left=oldest, right=newest), Y = freq (top=high, bottom=low).
        time_col = u32(frac_x * f32(sgr.sgr_cols)) % sgr.sgr_cols;
        freq_idx = u32((1.0 - frac_y) * f32(sgr.sgr_rows));
    } else {
        // Vertical: X = freq (left=low, right=high), Y = time (bottom=oldest, top=newest).
        freq_idx = u32(frac_x * f32(sgr.sgr_rows));
        // frac_y=0(top)=newest, frac_y=1(bottom)=oldest → invert
        let yi = min(u32(frac_y * f32(sgr.sgr_cols)), sgr.sgr_cols - 1u);
        time_col = sgr.sgr_cols - 1u - yi;
    }

    let ring_row = (sgr.write_col + time_col) % sgr.sgr_cols;
    let fi       = clamp(freq_idx, 0u, sgr.sgr_rows - 1u);

    let amp = textureLoad(sgr_data, vec2<i32>(i32(fi), i32(ring_row)), 0).r;
    return vec4<f32>(phosphor_heat(amp), 1.0);
}
"#;

// ── Uniform buffer layouts (must match WGSL structs, 16-byte aligned) ─────────
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CrtParams {
    resolution: [f32; 2],
    bloom_scale: f32,
    /// 1.0 = bypass barrel/bloom and output the texture directly (used for spectrogram)
    bypass: f32,
}

// ── Spectrogram ring-buffer constants ─────────────────────────────────────────
/// Number of frequency bins (= texture width). Must be a multiple of 256 for
/// write_texture alignment when height=1.
pub const SGR_ROWS: u32 = 256;
/// Number of time columns kept in the ring buffer (= texture height).
pub const SGR_COLS: u32 = 512;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SgrParams {
    pub write_col: u32, // oldest-data ring position (next to be overwritten)
    pub sgr_cols: u32,
    pub sgr_rows: u32,
    /// 0 = horizontal (X=time, Y=freq), 1 = vertical (X=freq, Y=time bottom→top)
    pub vertical: u32,
    pub tex_w: f32,
    pub tex_h: f32,
    pub _pad2: [f32; 2],
}

// ── Persistent GPU resources (stored in CallbackResources across frames) ──────
pub struct ScopeGpuResources {
    // Offscreen texture the visualizer renders into
    tex: wgpu::Texture,
    tex_view: wgpu::TextureView,
    tex_size: (u32, u32),

    // Pipeline 1: waveform lines → offscreen texture
    waveform_pipeline: wgpu::RenderPipeline,
    vertex_buf: wgpu::Buffer,

    // Pipeline 2: CRT post-process → surface
    crt_pipeline: wgpu::RenderPipeline,
    crt_bind_group: wgpu::BindGroup,
    crt_bind_group_layout: wgpu::BindGroupLayout,
    params_buf: wgpu::Buffer,
    sampler: wgpu::Sampler,

    // Pipeline 3: harmonograph line strip → offscreen texture
    harm_pipeline: wgpu::RenderPipeline,
    harm_params_buf: wgpu::Buffer,
    harm_bind_group: wgpu::BindGroup,
    harm_bgl: wgpu::BindGroupLayout,

    // Pipeline 4: voronoi fullscreen → offscreen texture
    vor_pipeline: wgpu::RenderPipeline,
    vor_params_buf: wgpu::Buffer,
    vor_bind_group: wgpu::BindGroup,
    vor_bgl: wgpu::BindGroupLayout,

    // Pipeline 5: spectrogram (ring-buffer texture → phosphor colormap)
    sgr_pipeline: wgpu::RenderPipeline,
    sgr_tex: wgpu::Texture,
    sgr_params_buf: wgpu::Buffer,
    sgr_bind_group: wgpu::BindGroup,
    sgr_bgl: wgpu::BindGroupLayout,
    /// Next ring-buffer row to write into sgr_tex (wraps mod SGR_COLS).
    sgr_write_col: u32,
}

const MAX_VERTS: u64 = 16384;
const TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const HARM_N_STEPS: u32 = 3000;

impl ScopeGpuResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        // ── Sampler ───────────────────────────────────────────────────────────
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scope_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // ── CRT params uniform buffer ─────────────────────────────────────────
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("crt_params"),
            contents: bytemuck::bytes_of(&CrtParams {
                resolution: [512.0, 256.0],
                bloom_scale: 1.0,
                bypass: 0.0,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // ── Waveform vertex buffer (pre-allocated, updated each frame) ────────
        let vertex_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope_verts"),
            size: MAX_VERTS * 8, // 2 × f32 per vertex
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Harm params uniform buffer ────────────────────────────────────────
        let harm_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("harm_params"),
            contents: bytemuck::bytes_of(&HarmParams {
                freqs: [1.0, 2.0, 3.0, 1.0],
                phases: [0.0; 4],
                damping: 0.025,
                t_max: 400.0,
                viewport: [512.0, 256.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // ── Vor params uniform buffer ─────────────────────────────────────────
        let vor_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vor_params"),
            contents: bytemuck::bytes_of(&VorParams {
                seeds: [[0.0; 4]; 16],
                num_seeds: 1,
                beat_pulse: 0.0,
                tex_w: 512.0,
                tex_h: 256.0,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // ── Initial offscreen texture ─────────────────────────────────────────
        let (tex, tex_view) = Self::make_texture(device, 512, 256);

        // ── CRT bind group layout ─────────────────────────────────────────────
        let crt_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("crt_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let crt_bind_group = Self::make_crt_bind_group(
            device,
            &crt_bind_group_layout,
            &tex_view,
            &sampler,
            &params_buf,
        );

        // ── Single-uniform BGLs for harm and vor ──────────────────────────────
        let harm_bgl = Self::make_uniform_bgl(device, "harm_bgl");
        let vor_bgl = Self::make_uniform_bgl(device, "vor_bgl");

        let harm_bind_group = Self::make_uniform_bg(device, &harm_bgl, &harm_params_buf, "harm_bg");
        let vor_bind_group = Self::make_uniform_bg(device, &vor_bgl, &vor_params_buf, "vor_bg");

        // ── Waveform pipeline ─────────────────────────────────────────────────
        let wv_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("waveform_shader"),
            source: wgpu::ShaderSource::Wgsl(WAVEFORM_SHADER.into()),
        });
        let wv_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wv_layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let waveform_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("waveform_pipeline"),
            layout: Some(&wv_layout),
            vertex: wgpu::VertexState {
                module: &wv_shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 0,
                        format: wgpu::VertexFormat::Float32x2,
                    }],
                }],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &wv_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TEX_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        // ── Harmonograph pipeline (line strip, no vertex buffer) ──────────────
        let harm_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("harm_shader"),
            source: wgpu::ShaderSource::Wgsl(HARMONOGRAPH_SHADER.into()),
        });
        let harm_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("harm_layout"),
            bind_group_layouts: &[Some(&harm_bgl)],
            immediate_size: 0,
        });
        let harm_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("harm_pipeline"),
            layout: Some(&harm_layout),
            vertex: wgpu::VertexState {
                module: &harm_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &harm_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TEX_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        // ── Voronoi pipeline (fullscreen triangle, no vertex buffer) ──────────
        let vor_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vor_shader"),
            source: wgpu::ShaderSource::Wgsl(VORONOI_SHADER.into()),
        });
        let vor_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vor_layout"),
            bind_group_layouts: &[Some(&vor_bgl)],
            immediate_size: 0,
        });
        let vor_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vor_pipeline"),
            layout: Some(&vor_layout),
            vertex: wgpu::VertexState {
                module: &vor_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &vor_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TEX_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        // ── CRT pipeline ──────────────────────────────────────────────────────
        let crt_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("crt_shader"),
            source: wgpu::ShaderSource::Wgsl(CRT_SHADER.into()),
        });
        let crt_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("crt_layout"),
            bind_group_layouts: &[Some(&crt_bind_group_layout)],
            immediate_size: 0,
        });
        let crt_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("crt_pipeline"),
            layout: Some(&crt_layout),
            vertex: wgpu::VertexState {
                module: &crt_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &crt_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        // ── Spectrogram ring-buffer texture ──────────────────────────────────
        // Layout: width = SGR_ROWS (freq axis), height = SGR_COLS (time axis).
        // Each row = one spectrogram time-column, written via queue.write_texture.
        let sgr_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sgr_ring_tex"),
            size: wgpu::Extent3d {
                width: SGR_ROWS,
                height: SGR_COLS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sgr_tex_view = sgr_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let sgr_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sgr_params"),
            contents: bytemuck::bytes_of(&SgrParams {
                write_col: 0,
                sgr_cols: SGR_COLS,
                sgr_rows: SGR_ROWS,
                vertical: 0,
                tex_w: 512.0,
                tex_h: 256.0,
                _pad2: [0.0; 2],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let sgr_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sgr_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let sgr_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sgr_bg"),
            layout: &sgr_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&sgr_tex_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sgr_params_buf.as_entire_binding(),
                },
            ],
        });

        let sgr_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sgr_shader"),
            source: wgpu::ShaderSource::Wgsl(SPECTROGRAM_SHADER.into()),
        });
        let sgr_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sgr_layout"),
            bind_group_layouts: &[Some(&sgr_bgl)],
            immediate_size: 0,
        });
        let sgr_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sgr_pipeline"),
            layout: Some(&sgr_layout),
            vertex: wgpu::VertexState {
                module: &sgr_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sgr_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TEX_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            tex,
            tex_view,
            tex_size: (512, 256),
            waveform_pipeline,
            vertex_buf,
            crt_pipeline,
            crt_bind_group,
            crt_bind_group_layout,
            params_buf,
            sampler,
            harm_pipeline,
            harm_params_buf,
            harm_bind_group,
            harm_bgl,
            vor_pipeline,
            vor_params_buf,
            vor_bind_group,
            vor_bgl,
            sgr_pipeline,
            sgr_tex,
            sgr_params_buf,
            sgr_bind_group,
            sgr_bgl,
            sgr_write_col: 0,
        }
    }

    fn make_texture(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scope_tex"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEX_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        (tex, view)
    }

    fn make_crt_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("crt_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        })
    }

    fn make_uniform_bgl(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        })
    }

    fn make_uniform_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buf: &wgpu::Buffer,
        label: &str,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buf.as_entire_binding(),
            }],
        })
    }

    /// Recreate the offscreen texture and CRT bind group when the panel size changes.
    fn resize(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        let (tex, tex_view) = Self::make_texture(device, w, h);
        self.tex = tex;
        self.tex_view = tex_view;
        self.tex_size = (w, h);
        self.crt_bind_group = Self::make_crt_bind_group(
            device,
            &self.crt_bind_group_layout,
            &self.tex_view,
            &self.sampler,
            &self.params_buf,
        );
    }
}

// ── Per-frame callback data ───────────────────────────────────────────────────
pub struct ScopeCallback {
    pub samples: Vec<f32>,
    pub x_scale: f32,
    pub y_scale: f32,
    pub viewport_size: (u32, u32),
    pub viz_mode: VizMode,
    pub harm_params: HarmParams,
    pub vor_params: VorParams,
    /// SGR_ROWS amplitude values in [0,1] for one spectrogram column (Spectrogram mode only).
    pub sgr_bins: Option<Vec<f32>>,
}

impl ScopeCallback {
    /// Map samples → triangle-strip ribbon vertices for the waveform pipeline.
    fn build_vertices(&self) -> Vec<[f32; 2]> {
        let n = ((self.samples.len() as f32 / self.x_scale) as usize)
            .clamp(2, self.samples.len())
            .min(MAX_VERTS as usize / 2);

        let (w, h) = (self.viewport_size.0 as f32, self.viewport_size.1 as f32);

        let pts: Vec<(f32, f32)> = self.samples[..n]
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let x = i as f32 / (n - 1).max(1) as f32 * w;
                let y = (0.5 - (s * self.y_scale).clamp(-1.0, 1.0) * 0.5) * h;
                (x, y)
            })
            .collect();

        let mut verts = Vec::with_capacity(n * 2);
        for i in 0..n {
            let (dx, dy) = if i == 0 {
                (pts[1].0 - pts[0].0, pts[1].1 - pts[0].1)
            } else if i == n - 1 {
                (pts[n - 1].0 - pts[n - 2].0, pts[n - 1].1 - pts[n - 2].1)
            } else {
                (pts[i + 1].0 - pts[i - 1].0, pts[i + 1].1 - pts[i - 1].1)
            };
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let (ndx, ndy) = (dx / len, dy / len);
            let (px, py) = (-ndy, ndx);
            let steepness = ndy.abs();
            let half_w = 3.0 - steepness * 1.8;

            let top = [
                (pts[i].0 + px * half_w) / w * 2.0 - 1.0,
                1.0 - (pts[i].1 + py * half_w) / h * 2.0,
            ];
            let bot = [
                (pts[i].0 - px * half_w) / w * 2.0 - 1.0,
                1.0 - (pts[i].1 - py * half_w) / h * 2.0,
            ];
            verts.push(top);
            verts.push(bot);
        }
        verts
    }
}

impl egui_wgpu::CallbackTrait for ScopeCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let res = resources.get_mut::<ScopeGpuResources>().unwrap();
        let (w, h) = self.viewport_size;
        if w == 0 || h == 0 {
            return vec![];
        }

        // Resize offscreen texture if the panel changed size
        if res.tex_size != (w, h) {
            res.resize(device, w, h);
        }

        // Update CRT uniforms
        let (bloom_scale, bypass) = match self.viz_mode {
            VizMode::Scope => (1.0f32, 0.0f32),
            VizMode::Harmonograph => (2.8, 0.0),
            VizMode::Voronoi => (0.2, 0.0),
            // Spectrogram modes: bypass CRT barrel/bloom for accurate color rendering.
            VizMode::Spectrogram | VizMode::SpectrogramV => (0.0, 1.0),
            // Spectrum and Envelope are CPU-drawn; the wgpu pass is a no-op.
            VizMode::Spectrum | VizMode::Envelope => (0.0, 0.0),
        };
        queue.write_buffer(
            &res.params_buf,
            0,
            bytemuck::bytes_of(&CrtParams {
                resolution: [w as f32, h as f32],
                bloom_scale,
                bypass,
            }),
        );

        match self.viz_mode {
            VizMode::Scope => {
                let verts = self.build_vertices();
                if verts.len() < 2 {
                    return vec![];
                }
                queue.write_buffer(&res.vertex_buf, 0, bytemuck::cast_slice(&verts));

                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("scope_waveform_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &res.tex_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&res.waveform_pipeline);
                pass.set_vertex_buffer(0, res.vertex_buf.slice(..));
                pass.draw(0..verts.len() as u32, 0..1);
            }

            VizMode::Harmonograph => {
                queue.write_buffer(
                    &res.harm_params_buf,
                    0,
                    bytemuck::bytes_of(&self.harm_params),
                );

                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("harm_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &res.tex_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&res.harm_pipeline);
                pass.set_bind_group(0, &res.harm_bind_group, &[]);
                pass.draw(0..HARM_N_STEPS * 2, 0..1); // 2 verts per point (ribbon)
            }

            VizMode::Voronoi => {
                let vor = VorParams {
                    tex_w: w as f32,
                    tex_h: h as f32,
                    ..self.vor_params
                };
                queue.write_buffer(&res.vor_params_buf, 0, bytemuck::bytes_of(&vor));

                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("vor_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &res.tex_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&res.vor_pipeline);
                pass.set_bind_group(0, &res.vor_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }

            VizMode::Spectrogram | VizMode::SpectrogramV => {
                let is_vertical = self.viz_mode == VizMode::SpectrogramV;
                // Upload new column into the ring-buffer texture (one row write = 256 bytes).
                if let Some(bins) = &self.sgr_bins {
                    let col_bytes: Vec<u8> = bins
                        .iter()
                        .map(|&v| (v.clamp(0.0, 1.0) * 255.0) as u8)
                        .collect();
                    let write_row = res.sgr_write_col % SGR_COLS;
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &res.sgr_tex,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: 0,
                                y: write_row,
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        &col_bytes,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            // height=1 → no stride alignment requirement; None = tightly packed
                            bytes_per_row: None,
                            rows_per_image: None,
                        },
                        wgpu::Extent3d {
                            width: SGR_ROWS,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                    );
                    res.sgr_write_col = res.sgr_write_col.wrapping_add(1);
                }
                // Update sgr params with current ring position and texture dimensions.
                queue.write_buffer(
                    &res.sgr_params_buf,
                    0,
                    bytemuck::bytes_of(&SgrParams {
                        write_col: res.sgr_write_col % SGR_COLS,
                        sgr_cols: SGR_COLS,
                        sgr_rows: SGR_ROWS,
                        vertical: is_vertical as u32,
                        tex_w: w as f32,
                        tex_h: h as f32,
                        _pad2: [0.0; 2],
                    }),
                );
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("sgr_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &res.tex_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&res.sgr_pipeline);
                pass.set_bind_group(0, &res.sgr_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }

            // Spectrum and Envelope are CPU-drawn; nothing to do in the wgpu pass.
            VizMode::Spectrum | VizMode::Envelope => {}
        }

        vec![]
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let res = resources.get::<ScopeGpuResources>().unwrap();
        // CRT post-process: full-screen triangle over the scope rect
        pass.set_pipeline(&res.crt_pipeline);
        pass.set_bind_group(0, &res.crt_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
