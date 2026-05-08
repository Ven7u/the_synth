use bytemuck::{Pod, Zeroable};
use eframe::egui::epaint::PaintCallbackInfo;
use eframe::egui_wgpu;
use egui_wgpu::wgpu;
use wgpu::util::DeviceExt;

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
    resolution: vec2<f32>,
    _pad0: f32,
    _pad1: f32,
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
    let uv = barrel(in.uv, 0.18);

    // Anything outside the barrel maps to solid black (curved-screen border)
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    let px = 1.0 / p.resolution;

    // ── Bloom: two-radius additive pass for a thick phosphor glow ─────────────
    // Inner ring (r=3): tight bright core that gives the line apparent thickness
    let r1: f32 = 3.0;
    var inner = textureSample(t_scope, s_scope, uv).rgb                        * 0.30;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>( r1,  0.0) * px).rgb * 0.14;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r1,  0.0) * px).rgb * 0.14;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>(0.0,  r1)  * px).rgb * 0.14;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>(0.0, -r1)  * px).rgb * 0.14;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>( r1,  r1)  * px).rgb * 0.06;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r1,  r1)  * px).rgb * 0.06;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>( r1, -r1)  * px).rgb * 0.06;
    inner    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r1, -r1)  * px).rgb * 0.06;

    // Outer ring (r=10): wide soft halo that gives the phosphor bloom
    let r2: f32 = 10.0;
    var outer = textureSample(t_scope, s_scope, uv + vec2<f32>( r2,  0.0) * px).rgb * 0.12;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r2,  0.0) * px).rgb * 0.12;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>(0.0,  r2)  * px).rgb * 0.12;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>(0.0, -r2)  * px).rgb * 0.12;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>( r2,  r2)  * px).rgb * 0.05;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r2,  r2)  * px).rgb * 0.05;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>( r2, -r2)  * px).rgb * 0.05;
    outer    += textureSample(t_scope, s_scope, uv + vec2<f32>(-r2, -r2)  * px).rgb * 0.05;

    var col = inner + outer * 0.6;

    // ── Scanlines: sinusoidal brightness modulation per raster line ───────────
    let scan = sin(uv.y * p.resolution.y * 3.14159265) * 0.5 + 0.5;
    col *= mix(0.82, 1.0, scan);

    // ── Vignette: darken corners to match CRT glass curvature ────────────────
    let d2 = in.uv - vec2<f32>(0.5);
    col *= max(1.0 - dot(d2, d2) * 1.8, 0.0);

    return vec4<f32>(col, 1.0);
}
"#;

// ── Uniform buffer layout (must match WGSL struct, 16-byte aligned) ───────────
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CrtParams {
    resolution: [f32; 2],
    _pad: [f32; 2],
}

// ── Persistent GPU resources (stored in CallbackResources across frames) ──────
pub struct ScopeGpuResources {
    // Offscreen texture the waveform is rendered into
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
}

const MAX_VERTS: u64 = 8192;
const TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

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

        // ── Params uniform buffer ─────────────────────────────────────────────
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("crt_params"),
            contents: bytemuck::bytes_of(&CrtParams {
                resolution: [512.0, 256.0],
                _pad: [0.0; 2],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // ── Vertex buffer (pre-allocated, updated each frame) ─────────────────
        let vertex_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope_verts"),
            size: MAX_VERTS * 8, // 2 × f32 per vertex
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Initial offscreen texture ─────────────────────────────────────────
        let (tex, tex_view) = Self::make_texture(device, 512, 256);

        // ── Bind group layout for CRT pass ────────────────────────────────────
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

        let crt_bind_group = Self::make_bind_group(
            device,
            &crt_bind_group_layout,
            &tex_view,
            &sampler,
            &params_buf,
        );

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
                topology: wgpu::PrimitiveTopology::LineStrip,
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

    fn make_bind_group(
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

    /// Recreate the offscreen texture and bind group when the panel size changes.
    fn resize(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        let (tex, tex_view) = Self::make_texture(device, w, h);
        self.tex = tex;
        self.tex_view = tex_view;
        self.tex_size = (w, h);
        self.crt_bind_group = Self::make_bind_group(
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
    /// Physical pixel size of the scope canvas rect.
    pub viewport_size: (u32, u32),
}

impl ScopeCallback {
    /// Map samples → clip-space vertices for the waveform pipeline.
    fn build_vertices(&self) -> Vec<[f32; 2]> {
        let n = ((self.samples.len() as f32 / self.x_scale) as usize)
            .clamp(2, self.samples.len())
            .min(MAX_VERTS as usize);
        let inv = 1.0 / (n - 1).max(1) as f32;
        self.samples[..n]
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let x = i as f32 * inv * 2.0 - 1.0; // [-1, +1]
                let y = (s * self.y_scale).clamp(-1.0, 1.0); // NDC: +1 = up
                [x, y]
            })
            .collect()
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

        // Upload waveform vertices
        let verts = self.build_vertices();
        if verts.len() < 2 {
            return vec![];
        }
        queue.write_buffer(&res.vertex_buf, 0, bytemuck::cast_slice(&verts));

        // Update CRT uniforms
        queue.write_buffer(
            &res.params_buf,
            0,
            bytemuck::bytes_of(&CrtParams {
                resolution: [w as f32, h as f32],
                _pad: [0.0; 2],
            }),
        );

        // ── Render pass: waveform lines → offscreen texture ───────────────────
        {
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
