//! Vídeo na GPU (plano 2.3 / B5): um widget `shader` que mantém **uma textura por vídeo** e a atualiza no
//! lugar a cada quadro novo.
//!
//! Por que não `iced::widget::image`: o `image` pede um `Handle` novo por quadro. No iced 0.13 isso já
//! custava uma alocação de 8 MiB por quadro 1080p; no 0.14 pior, porque imagens maiores que 2 MiB são
//! carregadas por uma thread e **não são desenhadas até terminarem**, o que faz o vídeo piscar. Aqui a
//! textura é a mesma durante toda a vida do vídeo e só recebe os bytes novos (`queue.write_texture`).
//!
//! Dois formatos de quadro: **RGBA** (4 bytes por pixel, o caminho antigo) e **NV12** (1,5 byte por pixel,
//! o que os decodificadores de hardware entregam): o NV12 sobe como duas texturas (Y em `R8`, UV em
//! `Rg8`) e o fragment shader faz o YUV → RGB, então a CPU não converte nem copia 8 MiB por quadro.
//!
//! O quadro entra com *letterbox* (mantém a proporção, como `ContentFit::Contain`); o que sobra fica
//! com o fundo do contêiner (preto). Sem quadro ainda, nada é desenhado.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use bytes::Bytes;
use iced::wgpu;
use iced::widget::shader::{self, Viewport};
use iced::{Rectangle, mouse};

use crate::domain::yuv::{self, YuvFormat};
use crate::engine::bridge::PixelFormat;

/// Identificador único de um vídeo na GPU (um por `VideoWidget`).
pub fn next_video_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// O retângulo (x, y, largura, altura) em que um quadro `img_w × img_h` cabe dentro de
/// `bounds_w × bounds_h` mantendo a proporção, centralizado. Tudo na mesma unidade.
pub fn fit_rect(bounds_w: f32, bounds_h: f32, img_w: f32, img_h: f32) -> (f32, f32, f32, f32) {
    if bounds_w <= 0.0 || bounds_h <= 0.0 || img_w <= 0.0 || img_h <= 0.0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let scale = (bounds_w / img_w).min(bounds_h / img_h);
    let (w, h) = (img_w * scale, img_h * scale);
    ((bounds_w - w) / 2.0, (bounds_h - h) / 2.0, w, h)
}

/// Converte um retângulo em pixels físicos (origem no canto superior esquerdo da janela) para
/// coordenadas normalizadas da GPU: `[esquerda, topo, direita, base]`, com y para cima.
pub fn ndc_rect(x: f32, y: f32, w: f32, h: f32, target_w: f32, target_h: f32) -> [f32; 4] {
    let tw = target_w.max(1.0);
    let th = target_h.max(1.0);
    [
        x / tw * 2.0 - 1.0,
        1.0 - y / th * 2.0,
        (x + w) / tw * 2.0 - 1.0,
        1.0 - (y + h) / th * 2.0,
    ]
}

/// Um quadro pronto para subir à GPU.
#[derive(Debug, Clone)]
pub struct Frame {
    pub pixels: Bytes,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    /// Muda a cada quadro novo; é o que diz se a textura precisa ser reescrita.
    pub generation: u64,
}

impl Frame {
    /// O quadro é coerente com o seu formato e tamanho?
    pub fn is_valid(&self) -> bool {
        if self.width == 0 || self.height == 0 {
            return false;
        }
        let expected = match self.format {
            PixelFormat::Rgba => Some(self.width as usize * self.height as usize * 4),
            PixelFormat::Nv12(_) => yuv::nv12_len(self.width, self.height),
        };
        expected == Some(self.pixels.len())
    }

    fn kind(&self) -> Kind {
        match self.format {
            PixelFormat::Rgba => Kind::Rgba,
            PixelFormat::Nv12(_) => Kind::Nv12,
        }
    }
}

/// Qual conjunto de texturas e qual pipeline um vídeo usa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rgba,
    Nv12,
}

/// O programa do widget: lê o quadro mais novo e o entrega ao primitivo.
pub struct VideoProgram {
    pub id: u64,
    pub frame: Option<Frame>,
    /// Vivo enquanto o `VideoWidget` existe: quando morre, a textura é liberada.
    pub alive: Weak<()>,
}

impl<Message> shader::Program<Message> for VideoProgram {
    type State = ();
    type Primitive = VideoPrimitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> VideoPrimitive {
        VideoPrimitive {
            id: self.id,
            frame: self.frame.clone().filter(Frame::is_valid),
            alive: self.alive.clone(),
        }
    }
}

#[derive(Debug)]
pub struct VideoPrimitive {
    id: u64,
    frame: Option<Frame>,
    alive: Weak<()>,
}

/// As texturas de um vídeo.
enum Planes {
    Rgba(wgpu::Texture),
    Nv12 { y: wgpu::Texture, uv: wgpu::Texture },
}

/// O que fica na GPU de cada vídeo.
pub struct VideoGpu {
    planes: Planes,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    size: (u32, u32),
    generation: u64,
    /// O que está escrito no uniform (retângulo normalizado + parâmetros), para só reescrever se mudar.
    uniform_data: [f32; 8],
    alive: Weak<()>,
}

impl VideoGpu {
    fn kind(&self) -> Kind {
        match self.planes {
            Planes::Rgba(_) => Kind::Rgba,
            Planes::Nv12 { .. } => Kind::Nv12,
        }
    }
}

/// Um pipeline (shader + layout) por formato de quadro.
pub struct Variant {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

/// Pipelines compartilhados por todos os vídeos + as texturas de cada um.
pub struct VideoPipeline {
    rgba: Variant,
    nv12: Variant,
    sampler: wgpu::Sampler,
    /// O alvo é sRGB? Então o NV12 (que sai gama-codificado do YUV→RGB) é linearizado antes de escrever.
    srgb_target: bool,
    videos: HashMap<u64, VideoGpu>,
}

/// Uniform: `rect` = (esquerda, topo, direita, base) normalizados; `p` = (kr, kb, faixa total?, alvo sRGB?).
const COMMON: &str = r#"
struct Params { rect: vec4<f32>, p: vec4<f32> };
@group(0) @binding(0) var<uniform> u: Params;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[i];
    var out: VsOut;
    // uv.y = 0 é o topo do quadro.
    out.position = vec4<f32>(mix(u.rect.x, u.rect.z, c.x), mix(u.rect.y, u.rect.w, c.y), 0.0, 1.0);
    out.uv = c;
    return out;
}
"#;

const RGBA_FS: &str = r#"
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(tex, samp, in.uv).rgb, 1.0);
}
"#;

const NV12_FS: &str = r#"
@group(0) @binding(1) var tex_y: texture_2d<f32>;
@group(0) @binding(2) var tex_uv: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let kr = u.p.x;
    let kb = u.p.y;
    let kg = 1.0 - kr - kb;
    var y = textureSample(tex_y, samp, in.uv).r * 255.0;
    var cc = textureSample(tex_uv, samp, in.uv).rg * 255.0 - vec2<f32>(128.0);
    if (u.p.z < 0.5) {
        y = (y - 16.0) * (255.0 / 219.0);
        cc = cc * (255.0 / 224.0);
    }
    let r = y + 2.0 * (1.0 - kr) * cc.y;
    let b = y + 2.0 * (1.0 - kb) * cc.x;
    let g = (y - kr * r - kb * b) / kg;
    var rgb = clamp(vec3<f32>(r, g, b) / 255.0, vec3<f32>(0.0), vec3<f32>(1.0));
    if (u.p.w > 0.5) {
        rgb = to_linear(rgb);
    }
    return vec4<f32>(rgb, 1.0);
}
"#;

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn uniform_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

impl Variant {
    fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        label: &str,
        fragment: &str,
        entries: &[wgpu::BindGroupLayoutEntry],
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(format!("{COMMON}{fragment}").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self { pipeline, layout }
    }
}

fn plane_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rrv video texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn write_plane(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    data: &[u8],
    bytes_per_row: u32,
    width: u32,
    height: u32,
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

impl shader::Pipeline for VideoPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self::build(device, format)
    }
}

impl VideoPipeline {
    fn build(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let rgba = Variant::new(
            device,
            format,
            "rrv video rgba",
            RGBA_FS,
            &[uniform_entry(), texture_entry(1), sampler_entry(2)],
        );
        let nv12 = Variant::new(
            device,
            format,
            "rrv video nv12",
            NV12_FS,
            &[
                uniform_entry(),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rrv video sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            rgba,
            nv12,
            sampler,
            srgb_target: format.is_srgb(),
            videos: HashMap::new(),
        }
    }

    fn variant(&self, kind: Kind) -> &Variant {
        match kind {
            Kind::Rgba => &self.rgba,
            Kind::Nv12 => &self.nv12,
        }
    }

    /// As texturas novas (`width × height`) e o que as liga ao shader.
    fn make_video(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        kind: Kind,
        alive: Weak<()>,
    ) -> VideoGpu {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rrv video uniform"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let (planes, bind_group) = match kind {
            Kind::Rgba => {
                let texture =
                    plane_texture(device, width, height, wgpu::TextureFormat::Rgba8UnormSrgb);
                let v = view(&texture);
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("rrv video bind group"),
                    layout: &self.rgba.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&v),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                (Planes::Rgba(texture), bind_group)
            }
            Kind::Nv12 => {
                let y = plane_texture(device, width, height, wgpu::TextureFormat::R8Unorm);
                let uv = plane_texture(
                    device,
                    width.div_ceil(2),
                    height.div_ceil(2),
                    wgpu::TextureFormat::Rg8Unorm,
                );
                let (yv, uvv) = (view(&y), view(&uv));
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("rrv video bind group"),
                    layout: &self.nv12.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&yv),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&uvv),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                (Planes::Nv12 { y, uv }, bind_group)
            }
        };
        VideoGpu {
            planes,
            bind_group,
            uniform,
            size: (width, height),
            generation: u64::MAX,
            uniform_data: [f32::NAN; 8],
            alive,
        }
    }
}

/// `(kr, kb, faixa total, alvo sRGB)` que o shader NV12 lê.
fn yuv_params(fmt: YuvFormat, srgb_target: bool) -> [f32; 4] {
    let (kr, kb) = fmt.matrix.weights();
    [
        kr,
        kb,
        f32::from(u8::from(fmt.full_range)),
        f32::from(u8::from(srgb_target)),
    ]
}

impl shader::Primitive for VideoPrimitive {
    type Pipeline = VideoPipeline;

    fn prepare(
        &self,
        state: &mut VideoPipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        // Libera as texturas dos vídeos que deixaram de existir (ex.: o player de gravações).
        state.videos.retain(|_, v| v.alive.strong_count() > 0);

        let Some(frame) = &self.frame else {
            return;
        };

        let recreate = state
            .videos
            .get(&self.id)
            .is_none_or(|v| v.size != (frame.width, frame.height) || v.kind() != frame.kind());
        if recreate {
            let gpu = state.make_video(
                device,
                frame.width,
                frame.height,
                frame.kind(),
                self.alive.clone(),
            );
            let _ = state.videos.insert(self.id, gpu);
        }
        let srgb_target = state.srgb_target;
        let Some(video) = state.videos.get_mut(&self.id) else {
            return;
        };

        if video.generation != frame.generation {
            match &video.planes {
                Planes::Rgba(texture) => write_plane(
                    queue,
                    texture,
                    &frame.pixels,
                    4 * frame.width,
                    frame.width,
                    frame.height,
                ),
                Planes::Nv12 { y, uv } => {
                    let (w, h) = (frame.width, frame.height);
                    let (y_data, uv_data) = frame.pixels.split_at(w as usize * h as usize);
                    write_plane(queue, y, y_data, w, w, h);
                    write_plane(
                        queue,
                        uv,
                        uv_data,
                        w.div_ceil(2) * 2,
                        w.div_ceil(2),
                        h.div_ceil(2),
                    );
                }
            }
            video.generation = frame.generation;
        }

        // Onde o quadro cai, em pixels físicos, e daí em coordenadas normalizadas.
        let scale = viewport.scale_factor();
        let (fx, fy, fw, fh) = fit_rect(
            bounds.width * scale,
            bounds.height * scale,
            frame.width as f32,
            frame.height as f32,
        );
        let size = viewport.physical_size();
        let ndc = ndc_rect(
            bounds.x * scale + fx,
            bounds.y * scale + fy,
            fw,
            fh,
            size.width as f32,
            size.height as f32,
        );
        let params = match frame.format {
            PixelFormat::Rgba => [0.0; 4],
            PixelFormat::Nv12(fmt) => yuv_params(fmt, srgb_target),
        };
        let mut data = [0.0f32; 8];
        data[..4].copy_from_slice(&ndc);
        data[4..].copy_from_slice(&params);
        // `NAN != NAN`: o primeiro quadro sempre escreve.
        if data != video.uniform_data {
            video.uniform_data = data;
            let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_ne_bytes()).collect();
            queue.write_buffer(&video.uniform, 0, &bytes);
        }
    }

    fn render(
        &self,
        state: &VideoPipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(video) = state.videos.get(&self.id) else {
            return;
        };
        if self.frame.is_none() || clip_bounds.width == 0 || clip_bounds.height == 0 {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rrv video pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_scissor_rect(
            clip_bounds.x,
            clip_bounds.y,
            clip_bounds.width,
            clip_bounds.height,
        );
        pass.set_pipeline(&state.variant(video.kind()).pipeline);
        pass.set_bind_group(0, &video.bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
}

/// Um token que vive enquanto o `VideoWidget` vive; o primitivo guarda um `Weak` dele.
pub fn new_alive_token() -> Arc<()> {
    Arc::new(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_frame_in_a_square_box_is_letterboxed_vertically() {
        // 16:9 em 100×100 → 100 de largura, 56,25 de altura, centralizado em y
        let (x, y, w, h) = fit_rect(100.0, 100.0, 1920.0, 1080.0);
        assert_eq!((x, w), (0.0, 100.0));
        assert!(
            (h - 56.25).abs() < 1e-3 && (y - 21.875).abs() < 1e-3,
            "{y} {h}"
        );
    }

    #[test]
    fn a_tall_box_pillarboxes_a_wide_frame_and_a_wide_box_a_tall_one() {
        let (x, y, w, h) = fit_rect(200.0, 100.0, 100.0, 100.0);
        assert_eq!((y, h), (0.0, 100.0));
        assert_eq!(
            (x, w),
            (50.0, 100.0),
            "quadrado em caixa larga: barras dos lados"
        );
        let (x, _, w, _) = fit_rect(100.0, 100.0, 50.0, 100.0);
        assert_eq!((x, w), (25.0, 50.0));
    }

    #[test]
    fn the_frame_never_exceeds_the_box_and_degenerate_sizes_draw_nothing() {
        for (bw, bh, iw, ih) in [(300.0, 200.0, 1280.0, 720.0), (50.0, 400.0, 640.0, 480.0)] {
            let (x, y, w, h) = fit_rect(bw, bh, iw, ih);
            assert!(x >= 0.0 && y >= 0.0 && x + w <= bw + 1e-3 && y + h <= bh + 1e-3);
            assert!((w / h - iw / ih).abs() < 1e-3, "mantém a proporção");
        }
        assert_eq!(fit_rect(0.0, 100.0, 10.0, 10.0), (0.0, 0.0, 0.0, 0.0));
        assert_eq!(fit_rect(100.0, 100.0, 0.0, 10.0), (0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn pixel_rects_map_to_normalised_device_coordinates() {
        // a janela inteira
        assert_eq!(
            ndc_rect(0.0, 0.0, 800.0, 600.0, 800.0, 600.0),
            [-1.0, 1.0, 1.0, -1.0]
        );
        // o quarto superior esquerdo
        assert_eq!(
            ndc_rect(0.0, 0.0, 400.0, 300.0, 800.0, 600.0),
            [-1.0, 1.0, 0.0, 0.0]
        );
        // o quarto inferior direito
        assert_eq!(
            ndc_rect(400.0, 300.0, 400.0, 300.0, 800.0, 600.0),
            [0.0, 0.0, 1.0, -1.0]
        );
        // alvo de tamanho zero não divide por zero
        assert!(
            ndc_rect(0.0, 0.0, 1.0, 1.0, 0.0, 0.0)
                .iter()
                .all(|v| v.is_finite())
        );
    }

    #[test]
    fn a_frame_must_match_its_size_to_be_drawn() {
        let ok = Frame {
            pixels: Bytes::from(vec![0u8; 4 * 2 * 3]),
            width: 2,
            height: 3,
            format: PixelFormat::Rgba,
            generation: 1,
        };
        assert!(ok.is_valid());
        let short = Frame {
            pixels: Bytes::from(vec![0u8; 5]),
            ..ok.clone()
        };
        assert!(!short.is_valid(), "bytes a menos: não vai para a GPU");
        let empty = Frame { width: 0, ..ok };
        assert!(!empty.is_valid());
    }

    // ── Renderização de verdade, fora da tela: um dispositivo wgpu, uma textura de 64×64 e os pixels
    // lidos de volta. Pula (com aviso) se a máquina não tem nenhum adaptador.

    use iced::Size;
    use iced::widget::shader::Primitive as _;

    const SIDE: u32 = 64;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    struct Gpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
        /// Os testes de GPU rodam um por vez: criar várias instâncias wgpu/EGL ao mesmo tempo, em
        /// threads diferentes, derruba a inicialização do EGL de vez em quando (`egl.rs`).
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    fn gpu() -> Option<Gpu> {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let instance = wgpu::Instance::default();
        let adapter = iced::futures::executor::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                // WGPU_POWER_PREF=high roda os testes na GPU discreta
                power_preference:
                    wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::LowPower),
                force_fallback_adapter: false,
                compatible_surface: None,
            },
        ))
        .ok()?;
        eprintln!("adaptador de teste: {:?}", adapter.get_info().name);
        let (device, queue) = iced::futures::executor::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor::default()),
        )
        .ok()?;
        Some(Gpu {
            device,
            queue,
            _serial: serial,
        })
    }

    fn frame(width: u32, height: u32, generation: u64, px: &[[u8; 4]]) -> Frame {
        assert_eq!(px.len(), (width * height) as usize);
        Frame {
            pixels: Bytes::from(px.iter().flatten().copied().collect::<Vec<u8>>()),
            width,
            height,
            format: PixelFormat::Rgba,
            generation,
        }
    }

    /// Desenha `primitive` sobre um alvo preto de `SIDE`×`SIDE` e devolve os pixels RGBA.
    fn draw(
        gpu: &Gpu,
        pipeline: &mut VideoPipeline,
        primitive: &VideoPrimitive,
        bounds: Rectangle,
    ) -> Vec<u8> {
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let viewport = Viewport::with_physical_size(Size::new(SIDE, SIDE), 1.0);
        primitive.prepare(pipeline, &gpu.device, &gpu.queue, &bounds, &viewport);

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        }));
        primitive.render(
            pipeline,
            &mut encoder,
            &view,
            &Rectangle {
                x: 0,
                y: 0,
                width: SIDE,
                height: SIDE,
            },
        );

        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIDE * SIDE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIDE * 4), // 256: já alinhado
                    rows_per_image: Some(SIDE),
                },
            },
            wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
        );
        let _ = gpu.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("mapear o buffer"));
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().to_vec();
        readback.unmap();
        data
    }

    fn pixel(img: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIDE + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    fn near(a: [u8; 4], b: [u8; 3], tol: i32) -> bool {
        (0..3).all(|i| (i32::from(a[i]) - i32::from(b[i])).abs() <= tol)
    }

    fn primitive(id: u64, frame: Option<Frame>, alive: &Arc<()>) -> VideoPrimitive {
        VideoPrimitive {
            id,
            frame,
            alive: Arc::downgrade(alive),
        }
    }

    const FULL: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: SIDE as f32,
        height: SIDE as f32,
    };

    #[test]
    fn the_frame_is_drawn_letterboxed_with_exact_colours() {
        let Some(gpu) = gpu() else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        let alive = new_alive_token();
        let mut storage = <VideoPipeline as shader::Pipeline>::new(&gpu.device, &gpu.queue, FORMAT);
        // 2×1 (proporção 2:1) numa caixa 64×64: ocupa y de 16 a 48; esquerda vermelha, direita azul
        let p = primitive(
            1,
            Some(frame(2, 1, 1, &[[255, 0, 0, 255], [0, 0, 255, 255]])),
            &alive,
        );
        let img = draw(&gpu, &mut storage, &p, FULL);
        assert!(
            near(pixel(&img, 4, 32), [255, 0, 0], 2),
            "esquerda vermelha: {:?}",
            pixel(&img, 4, 32)
        );
        assert!(
            near(pixel(&img, 60, 32), [0, 0, 255], 2),
            "direita azul: {:?}",
            pixel(&img, 60, 32)
        );
        // as barras de cima e de baixo não são tocadas: ficam pretas
        assert!(
            near(pixel(&img, 32, 4), [0, 0, 0], 0),
            "barra de cima: {:?}",
            pixel(&img, 32, 4)
        );
        assert!(
            near(pixel(&img, 32, 60), [0, 0, 0], 0),
            "barra de baixo: {:?}",
            pixel(&img, 32, 60)
        );
    }

    #[test]
    fn grey_levels_survive_the_gpu_round_trip() {
        let Some(gpu) = gpu() else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        let alive = new_alive_token();
        let mut storage = <VideoPipeline as shader::Pipeline>::new(&gpu.device, &gpu.queue, FORMAT);
        // sem isto o sRGB seria aplicado duas vezes (o defeito do iced 0.14 na NVIDIA): 100/150/200 têm
        // de sair 100/150/200, não mais claros
        let p = primitive(1, Some(frame(1, 1, 1, &[[100, 150, 200, 255]])), &alive);
        let img = draw(&gpu, &mut storage, &p, FULL);
        assert!(
            near(pixel(&img, 32, 32), [100, 150, 200], 2),
            "{:?}",
            pixel(&img, 32, 32)
        );
    }

    #[test]
    fn a_new_generation_updates_the_same_texture_in_place() {
        let Some(gpu) = gpu() else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        let alive = new_alive_token();
        let mut storage = <VideoPipeline as shader::Pipeline>::new(&gpu.device, &gpu.queue, FORMAT);
        let p1 = primitive(1, Some(frame(1, 1, 1, &[[255, 0, 0, 255]])), &alive);
        let img = draw(&gpu, &mut storage, &p1, FULL);
        assert!(near(pixel(&img, 32, 32), [255, 0, 0], 2));
        // quadro novo (geração 2): a mesma textura recebe os bytes novos
        let p2 = primitive(1, Some(frame(1, 1, 2, &[[0, 255, 0, 255]])), &alive);
        let img = draw(&gpu, &mut storage, &p2, FULL);
        assert!(
            near(pixel(&img, 32, 32), [0, 255, 0], 2),
            "{:?}",
            pixel(&img, 32, 32)
        );
        assert_eq!(storage.videos.len(), 1, "uma textura só");
        // a mesma geração de novo: continua o mesmo desenho
        let img = draw(&gpu, &mut storage, &p2, FULL);
        assert!(near(pixel(&img, 32, 32), [0, 255, 0], 2));
    }

    #[test]
    fn a_resolution_change_recreates_the_texture() {
        let Some(gpu) = gpu() else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        let alive = new_alive_token();
        let mut storage = <VideoPipeline as shader::Pipeline>::new(&gpu.device, &gpu.queue, FORMAT);
        let _ = draw(
            &gpu,
            &mut storage,
            &primitive(1, Some(frame(1, 1, 1, &[[255, 0, 0, 255]])), &alive),
            FULL,
        );
        // a câmera trocou de 1×1 para 2×2 (ex.: sub-stream → principal)
        let big = frame(2, 2, 2, &[[0, 0, 255, 255]; 4]);
        let img = draw(&gpu, &mut storage, &primitive(1, Some(big), &alive), FULL);
        assert!(
            near(pixel(&img, 32, 32), [0, 0, 255], 2),
            "{:?}",
            pixel(&img, 32, 32)
        );
        assert_eq!(storage.videos[&1].size, (2, 2));
    }

    #[test]
    fn two_videos_do_not_share_a_texture_and_a_dead_one_is_collected() {
        let Some(gpu) = gpu() else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        let (a, b) = (new_alive_token(), new_alive_token());
        let mut storage = <VideoPipeline as shader::Pipeline>::new(&gpu.device, &gpu.queue, FORMAT);
        let red = primitive(1, Some(frame(1, 1, 1, &[[255, 0, 0, 255]])), &a);
        let blue = primitive(2, Some(frame(1, 1, 1, &[[0, 0, 255, 255]])), &b);
        let img = draw(&gpu, &mut storage, &red, FULL);
        assert!(near(pixel(&img, 32, 32), [255, 0, 0], 2));
        let img = draw(&gpu, &mut storage, &blue, FULL);
        assert!(
            near(pixel(&img, 32, 32), [0, 0, 255], 2),
            "o segundo vídeo tem a sua textura"
        );
        assert_eq!(storage.videos.len(), 2);
        // o primeiro widget morre; o próximo quadro de qualquer vídeo recolhe a textura dele
        drop(a);
        let _ = draw(&gpu, &mut storage, &blue, FULL);
        let videos = &storage.videos;
        assert_eq!(videos.len(), 1, "a textura do vídeo morto foi liberada");
        assert!(videos.contains_key(&2));
    }

    #[test]
    fn no_frame_yet_draws_nothing() {
        let Some(gpu) = gpu() else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        let alive = new_alive_token();
        let mut storage = <VideoPipeline as shader::Pipeline>::new(&gpu.device, &gpu.queue, FORMAT);
        let img = draw(&gpu, &mut storage, &primitive(1, None, &alive), FULL);
        assert!(
            near(pixel(&img, 32, 32), [0, 0, 0], 0),
            "sem quadro, fica o fundo"
        );
    }

    #[test]
    fn a_widget_offset_inside_the_window_lands_where_its_bounds_say() {
        let Some(gpu) = gpu() else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        let alive = new_alive_token();
        let mut storage = <VideoPipeline as shader::Pipeline>::new(&gpu.device, &gpu.queue, FORMAT);
        // o widget ocupa o quadrante inferior direito (32..64); o quadro vermelho 1×1 o preenche
        let bounds = Rectangle {
            x: 32.0,
            y: 32.0,
            width: 32.0,
            height: 32.0,
        };
        let p = primitive(1, Some(frame(1, 1, 1, &[[255, 0, 0, 255]])), &alive);
        let img = draw(&gpu, &mut storage, &p, bounds);
        assert!(
            near(pixel(&img, 48, 48), [255, 0, 0], 2),
            "dentro: {:?}",
            pixel(&img, 48, 48)
        );
        assert!(
            near(pixel(&img, 8, 8), [0, 0, 0], 0),
            "fora (canto superior esquerdo)"
        );
        assert!(
            near(pixel(&img, 48, 8), [0, 0, 0], 0),
            "fora (canto superior direito)"
        );
        assert!(
            near(pixel(&img, 8, 48), [0, 0, 0], 0),
            "fora (canto inferior esquerdo)"
        );
    }

    /// Um quadro NV12 4×4: quatro blocos 2×2, cada um com a sua luma e o seu par UV.
    /// `blocks` = [(y, u, v); 4] em ordem de leitura (cima-esq, cima-dir, baixo-esq, baixo-dir).
    fn nv12_frame(blocks: [(u8, u8, u8); 4], fmt: YuvFormat, generation: u64) -> Frame {
        let mut data = Vec::new();
        for row in 0..4usize {
            for col in 0..4usize {
                data.push(blocks[(row / 2) * 2 + col / 2].0);
            }
        }
        for brow in 0..2usize {
            for bcol in 0..2usize {
                let (_, u, v) = blocks[brow * 2 + bcol];
                data.extend_from_slice(&[u, v]);
            }
        }
        Frame {
            pixels: Bytes::from(data),
            width: 4,
            height: 4,
            format: PixelFormat::Nv12(fmt),
            generation,
        }
    }

    const BLOCKS: [(u8, u8, u8); 4] = [
        (81, 90, 240),
        (145, 54, 34),
        (41, 240, 110),
        (126, 128, 128),
    ];

    /// O centro de cada bloco do quadro 4×4 numa caixa 64×64.
    /// O centro do pixel (16,5) fica 0,5 px do centro do texel de croma, e o filtro linear mistura ~1,5 % do
    /// bloco vizinho (que tem outra cor): daí a folga, que ainda pega um erro de matriz ou de faixa (dezenas).
    const TOL: i32 = 10;

    const CENTRES: [(u32, u32); 4] = [(16, 16), (48, 16), (16, 48), (48, 48)];

    fn check_nv12_against_the_cpu(fmt: YuvFormat) {
        let Some(gpu) = gpu() else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        let alive = new_alive_token();
        let mut storage = <VideoPipeline as shader::Pipeline>::new(&gpu.device, &gpu.queue, FORMAT);
        let p = primitive(1, Some(nv12_frame(BLOCKS, fmt, 1)), &alive);
        let img = draw(&gpu, &mut storage, &p, FULL);
        for (&(y, u, v), &(px, py)) in BLOCKS.iter().zip(&CENTRES) {
            let want = yuv::pixel_to_rgb(y, u, v, fmt);
            assert!(
                near(pixel(&img, px, py), want, TOL),
                "bloco em ({px},{py}) com {fmt:?}: GPU {:?}, CPU {want:?}",
                pixel(&img, px, py)
            );
        }
    }

    #[test]
    fn nv12_on_the_gpu_matches_the_cpu_conversion_bt601_limited() {
        check_nv12_against_the_cpu(YuvFormat {
            matrix: yuv::YuvMatrix::Bt601,
            full_range: false,
        });
    }

    #[test]
    fn nv12_on_the_gpu_matches_the_cpu_conversion_bt709_and_full_range() {
        check_nv12_against_the_cpu(YuvFormat {
            matrix: yuv::YuvMatrix::Bt709,
            full_range: false,
        });
        check_nv12_against_the_cpu(YuvFormat {
            matrix: yuv::YuvMatrix::Bt709,
            full_range: true,
        });
    }

    #[test]
    fn a_video_can_switch_between_rgba_and_nv12() {
        let Some(gpu) = gpu() else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        let fmt = YuvFormat {
            matrix: yuv::YuvMatrix::Bt601,
            full_range: false,
        };
        let alive = new_alive_token();
        let mut storage = <VideoPipeline as shader::Pipeline>::new(&gpu.device, &gpu.queue, FORMAT);
        let rgba = primitive(1, Some(frame(1, 1, 1, &[[0, 255, 0, 255]])), &alive);
        let img = draw(&gpu, &mut storage, &rgba, FULL);
        assert!(near(pixel(&img, 32, 32), [0, 255, 0], 2));
        // o mesmo vídeo passa a chegar em NV12 (ex.: o decodificador mudou): textura nova, mesmo id
        let nv = primitive(1, Some(nv12_frame(BLOCKS, fmt, 2)), &alive);
        let img = draw(&gpu, &mut storage, &nv, FULL);
        let want = yuv::pixel_to_rgb(81, 90, 240, fmt);
        assert!(
            near(pixel(&img, 16, 16), want, TOL),
            "{:?}",
            pixel(&img, 16, 16)
        );
        assert_eq!(storage.videos.len(), 1);
        // e volta
        let img = draw(&gpu, &mut storage, &rgba, FULL);
        assert!(near(pixel(&img, 32, 32), [0, 255, 0], 2));
    }

    #[test]
    fn an_nv12_frame_with_the_wrong_byte_count_is_not_valid() {
        let fmt = YuvFormat {
            matrix: yuv::YuvMatrix::Bt601,
            full_range: false,
        };
        let ok = nv12_frame(BLOCKS, fmt, 1);
        assert!(ok.is_valid());
        let short = Frame {
            pixels: ok.pixels.slice(..10),
            ..ok.clone()
        };
        assert!(!short.is_valid());
        // um quadro RGBA com os bytes de um NV12 não passa por RGBA
        let wrong = Frame {
            format: PixelFormat::Rgba,
            ..ok
        };
        assert!(!wrong.is_valid());
    }

    #[test]
    fn every_video_gets_its_own_id() {
        let a = next_video_id();
        let b = next_video_id();
        assert_ne!(a, b);
    }
}
