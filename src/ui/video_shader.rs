//! Vídeo na GPU (plano 2.3 / B5): um widget `shader` que mantém **uma textura por vídeo** e a atualiza no
//! lugar a cada quadro novo.
//!
//! Por que não `iced::widget::image`: o `image` pede um `Handle` novo por quadro. No iced 0.13 isso já
//! custava uma alocação de 8 MiB por quadro 1080p; no 0.14 pior, porque imagens maiores que 2 MiB são
//! carregadas por uma thread e **não são desenhadas até terminarem**, o que faz o vídeo piscar. Aqui a
//! textura é a mesma durante toda a vida do vídeo e só recebe os bytes novos (`queue.write_texture`).
//!
//! O quadro entra com *letterbox* (mantém a proporção, como `ContentFit::Contain`); o que sobra fica
//! com o fundo do contêiner (preto). Sem quadro ainda, nada é desenhado.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use bytes::Bytes;
use iced::widget::shader::{self, Viewport, wgpu};
use iced::{Rectangle, mouse};

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

/// Um quadro RGBA pronto para subir à GPU.
#[derive(Debug, Clone)]
pub struct Frame {
    pub rgba: Bytes,
    pub width: u32,
    pub height: u32,
    /// Muda a cada quadro novo; é o que diz se a textura precisa ser reescrita.
    pub generation: u64,
}

impl Frame {
    /// O quadro é coerente (largura × altura × 4 bytes)?
    pub fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.rgba.len() == self.width as usize * self.height as usize * 4
    }
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

/// O que fica na GPU de cada vídeo.
struct VideoGpu {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    size: (u32, u32),
    generation: u64,
    /// Retângulo atual em coordenadas normalizadas (e o recorte), para o `render`.
    ndc: [f32; 4],
    alive: Weak<()>,
}

/// Pipeline compartilhado por todos os vídeos + as texturas de cada um.
struct VideoPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    videos: HashMap<u64, VideoGpu>,
}

const SHADER: &str = r#"
struct Rect { v: vec4<f32> };
@group(0) @binding(0) var<uniform> rect: Rect;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

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
    // rect = (esquerda, topo, direita, base) normalizados; uv.y = 0 é o topo do quadro.
    out.position = vec4<f32>(mix(rect.v.x, rect.v.z, c.x), mix(rect.v.y, rect.v.w, c.y), 0.0, 1.0);
    out.uv = c;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(tex, samp, in.uv).rgb, 1.0);
}
"#;

impl VideoPipeline {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rrv video shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rrv video layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rrv video pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rrv video pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: "fs_main",
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
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rrv video sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            videos: HashMap::new(),
        }
    }

    /// Uma textura nova (`width × height`) e o que a liga ao shader.
    fn make_video(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        alive: Weak<()>,
    ) -> VideoGpu {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rrv video texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rrv video rect"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rrv video bind group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        VideoGpu {
            texture,
            bind_group,
            uniform,
            size: (width, height),
            generation: u64::MAX,
            ndc: [0.0; 4],
            alive,
        }
    }
}

impl shader::Primitive for VideoPrimitive {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        storage: &mut shader::Storage,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        if !storage.has::<VideoPipeline>() {
            storage.store(VideoPipeline::new(device, format));
        }
        let Some(state) = storage.get_mut::<VideoPipeline>() else {
            return;
        };
        // Libera as texturas dos vídeos que deixaram de existir (ex.: o player de gravações).
        state.videos.retain(|_, v| v.alive.strong_count() > 0);

        let Some(frame) = &self.frame else {
            return;
        };

        let recreate = state
            .videos
            .get(&self.id)
            .is_none_or(|v| v.size != (frame.width, frame.height));
        if recreate {
            let gpu = state.make_video(device, frame.width, frame.height, self.alive.clone());
            let _ = state.videos.insert(self.id, gpu);
        }
        let Some(video) = state.videos.get_mut(&self.id) else {
            return;
        };

        if video.generation != frame.generation {
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &video.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.rgba,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * frame.width),
                    rows_per_image: Some(frame.height),
                },
                wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
            );
            video.generation = frame.generation;
        }

        // Onde o quadro cai, em pixels físicos, e daí em coordenadas normalizadas.
        let scale = viewport.scale_factor() as f32;
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
        if ndc != video.ndc {
            video.ndc = ndc;
            let bytes: Vec<u8> = ndc.iter().flat_map(|f| f.to_ne_bytes()).collect();
            queue.write_buffer(&video.uniform, 0, &bytes);
        }
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        storage: &shader::Storage,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(state) = storage.get::<VideoPipeline>() else {
            return;
        };
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
        pass.set_pipeline(&state.pipeline);
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
            rgba: Bytes::from(vec![0u8; 4 * 2 * 3]),
            width: 2,
            height: 3,
            generation: 1,
        };
        assert!(ok.is_valid());
        let short = Frame {
            rgba: Bytes::from(vec![0u8; 5]),
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
    }

    fn gpu() -> Option<Gpu> {
        let instance = wgpu::Instance::default();
        let adapter = iced::futures::executor::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions {
                // WGPU_POWER_PREF=high roda os testes na GPU discreta
                power_preference: wgpu::util::power_preference_from_env()
                    .unwrap_or(wgpu::PowerPreference::LowPower),
                force_fallback_adapter: false,
                compatible_surface: None,
            }),
        )?;
        eprintln!("adaptador de teste: {:?}", adapter.get_info().name);
        let (device, queue) = iced::futures::executor::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor::default(), None),
        )
        .ok()?;
        Some(Gpu { device, queue })
    }

    fn frame(width: u32, height: u32, generation: u64, px: &[[u8; 4]]) -> Frame {
        assert_eq!(px.len(), (width * height) as usize);
        Frame {
            rgba: Bytes::from(px.iter().flatten().copied().collect::<Vec<u8>>()),
            width,
            height,
            generation,
        }
    }

    /// Desenha `primitive` sobre um alvo preto de `SIDE`×`SIDE` e devolve os pixels RGBA.
    fn draw(
        gpu: &Gpu,
        storage: &mut shader::Storage,
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
        primitive.prepare(&gpu.device, &gpu.queue, FORMAT, storage, &bounds, &viewport);

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
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
            &mut encoder,
            storage,
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
            wgpu::ImageCopyTexture {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &readback,
                layout: wgpu::ImageDataLayout {
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
        gpu.device.poll(wgpu::Maintain::Wait);
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
        let mut storage = shader::Storage::default();
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
        let mut storage = shader::Storage::default();
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
        let mut storage = shader::Storage::default();
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
        assert_eq!(
            storage.get::<VideoPipeline>().unwrap().videos.len(),
            1,
            "uma textura só"
        );
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
        let mut storage = shader::Storage::default();
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
        assert_eq!(
            storage.get::<VideoPipeline>().unwrap().videos[&1].size,
            (2, 2)
        );
    }

    #[test]
    fn two_videos_do_not_share_a_texture_and_a_dead_one_is_collected() {
        let Some(gpu) = gpu() else {
            eprintln!("sem adaptador wgpu: teste pulado");
            return;
        };
        let (a, b) = (new_alive_token(), new_alive_token());
        let mut storage = shader::Storage::default();
        let red = primitive(1, Some(frame(1, 1, 1, &[[255, 0, 0, 255]])), &a);
        let blue = primitive(2, Some(frame(1, 1, 1, &[[0, 0, 255, 255]])), &b);
        let img = draw(&gpu, &mut storage, &red, FULL);
        assert!(near(pixel(&img, 32, 32), [255, 0, 0], 2));
        let img = draw(&gpu, &mut storage, &blue, FULL);
        assert!(
            near(pixel(&img, 32, 32), [0, 0, 255], 2),
            "o segundo vídeo tem a sua textura"
        );
        assert_eq!(storage.get::<VideoPipeline>().unwrap().videos.len(), 2);
        // o primeiro widget morre; o próximo quadro de qualquer vídeo recolhe a textura dele
        drop(a);
        let _ = draw(&gpu, &mut storage, &blue, FULL);
        let videos = &storage.get::<VideoPipeline>().unwrap().videos;
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
        let mut storage = shader::Storage::default();
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
        let mut storage = shader::Storage::default();
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

    #[test]
    fn every_video_gets_its_own_id() {
        let a = next_video_id();
        let b = next_video_id();
        assert_ne!(a, b);
    }
}
