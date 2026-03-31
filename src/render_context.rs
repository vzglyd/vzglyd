use std::sync::Arc;

use vzglyd_slide::{DrawSpec, PipelineKind, ShaderSources};
use winit::dpi::PhysicalSize;
use winit::window::Window;

pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 480;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub struct PipelineDesc<'a> {
    pub label: &'a str,
    pub shader_source: &'a str,
    pub vs_entry: &'a str,
    pub fs_entry: &'a str,
    pub vertex_layout: wgpu::VertexBufferLayout<'a>,
    pub cull_mode: Option<wgpu::Face>,
    pub depth_write: bool,
    pub depth_compare: wgpu::CompareFunction,
}

impl<'a> PipelineDesc<'a> {
    pub fn for_kind(
        label: &'a str,
        shader_source: &'a str,
        vs_entry: &'a str,
        fs_entry: &'a str,
        vertex_layout: wgpu::VertexBufferLayout<'a>,
        cull_mode: Option<wgpu::Face>,
        kind: PipelineKind,
    ) -> Self {
        Self {
            label,
            shader_source,
            vs_entry,
            fs_entry,
            vertex_layout,
            cull_mode,
            depth_write: Self::depth_write_for(kind),
            depth_compare: Self::depth_compare_for(kind),
        }
    }

    pub fn depth_write_for(kind: PipelineKind) -> bool {
        matches!(kind, PipelineKind::Opaque)
    }

    pub fn depth_compare_for(kind: PipelineKind) -> wgpu::CompareFunction {
        match kind {
            PipelineKind::Opaque => wgpu::CompareFunction::Less,
            PipelineKind::Transparent => wgpu::CompareFunction::LessEqual,
        }
    }
}

pub fn custom_shader_source(shaders: Option<&ShaderSources>) -> Option<&str> {
    shaders.and_then(|sources| {
        sources
            .fragment_wgsl
            .as_deref()
            .or(sources.vertex_wgsl.as_deref())
    })
}

pub struct ScenePipelines {
    opaque: Option<wgpu::RenderPipeline>,
    transparent: Option<wgpu::RenderPipeline>,
}

impl ScenePipelines {
    pub fn create<'a, F>(
        ctx: &RenderContext,
        draw_plan: &[DrawSpec],
        bind_group_layout: &wgpu::BindGroupLayout,
        mut describe: F,
    ) -> Self
    where
        F: FnMut(PipelineKind) -> PipelineDesc<'a>,
    {
        let opaque = draw_plan
            .iter()
            .any(|draw| draw.pipeline == PipelineKind::Opaque)
            .then(|| {
                let desc = describe(PipelineKind::Opaque);
                ctx.create_pipeline(&desc, bind_group_layout)
            });
        let transparent = draw_plan
            .iter()
            .any(|draw| draw.pipeline == PipelineKind::Transparent)
            .then(|| {
                let desc = describe(PipelineKind::Transparent);
                ctx.create_pipeline(&desc, bind_group_layout)
            });

        Self {
            opaque,
            transparent,
        }
    }

    pub fn get(&self, kind: PipelineKind) -> &wgpu::RenderPipeline {
        match kind {
            PipelineKind::Opaque => self.opaque.as_ref(),
            PipelineKind::Transparent => self.transparent.as_ref(),
        }
        .expect("missing render pipeline required by draw plan")
    }

    pub fn first(&self) -> &wgpu::RenderPipeline {
        self.opaque
            .as_ref()
            .or(self.transparent.as_ref())
            .expect("scene requires at least one render pipeline")
    }
}

struct BlitResources {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

pub struct OffscreenTarget {
    #[allow(dead_code)]
    pub color_texture: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    #[allow(dead_code)]
    depth_texture: wgpu::Texture,
    pub depth_view: wgpu::TextureView,
    blit_bind_group: wgpu::BindGroup,
}

pub(crate) struct SurfaceBlit {
    pub(crate) frame: wgpu::SurfaceTexture,
    pub(crate) command_buffer: wgpu::CommandBuffer,
}

pub struct RenderContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub depth_view: wgpu::TextureView,
    pub window: Arc<Window>,
    blit: BlitResources,
}

impl RenderContext {
    pub async fn new(window: Arc<Window>) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let surface = instance
            .create_surface(Arc::clone(&window))
            .expect("failed to create wgpu surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("no suitable GPU adapter found");
        // Desktop default limits over-request on Raspberry Pi GLES drivers
        // (for example eight color attachments when only four are exposed).
        // Request the selected adapter's supported limits instead.
        let required_limits = adapter.limits();
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("vzglyd_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits,
                    memory_hints: Default::default(),
                },
                None,
            )
            .await
            .expect("failed to create wgpu device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);
        let surface_size = Self::initial_surface_size(&window);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: surface_size.width,
            height: surface_size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth_view = Self::make_depth_view(&device);
        let blit = Self::create_blit_resources(&device, format);

        Self {
            device,
            queue,
            surface,
            config,
            depth_view,
            window,
            blit,
        }
    }

    pub fn reconfigure(&mut self) {
        self.sync_surface_size_to_window();
        self.surface.configure(&self.device, &self.config);
        self.depth_view = Self::make_depth_view(&self.device);
    }

    pub fn resize_surface(&mut self, size: PhysicalSize<u32>) {
        let Some(size) = Self::usable_surface_size(size) else {
            return;
        };

        if self.config.width == size.width && self.config.height == size.height {
            return;
        }

        self.config.width = size.width;
        self.config.height = size.height;
        self.reconfigure();
    }

    pub fn create_offscreen_target(&self) -> OffscreenTarget {
        let color_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen_color"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (depth_texture, depth_view) =
            Self::make_depth_attachment(&self.device, "offscreen_depth");
        let blit_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("offscreen_blit_bind_group"),
            layout: &self.blit.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blit.sampler),
                },
            ],
        });

        OffscreenTarget {
            color_texture,
            color_view,
            depth_texture,
            depth_view,
            blit_bind_group,
        }
    }

    pub(crate) fn blit_to_surface(
        &self,
        target: &OffscreenTarget,
    ) -> Result<SurfaceBlit, wgpu::SurfaceError> {
        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("blit_frame_encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.blit.pipeline);
            pass.set_bind_group(0, &target.blit_bind_group, &[]);
            let (x, y, width, height) = self.surface_blit_rect();
            pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
            pass.set_scissor_rect(x, y, width, height);
            pass.draw(0..3, 0..1);
        }

        Ok(SurfaceBlit {
            frame,
            command_buffer: encoder.finish(),
        })
    }

    pub fn create_pipeline(
        &self,
        desc: &PipelineDesc<'_>,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(desc.label),
                source: wgpu::ShaderSource::Wgsl(desc.shader_source.into()),
            });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(desc.label),
                bind_group_layouts: &[bind_group_layout],
                push_constant_ranges: &[],
            });

        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(desc.label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(desc.vs_entry),
                    buffers: &[desc.vertex_layout.clone()],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: desc.cull_mode,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: desc.depth_write,
                    depth_compare: desc.depth_compare,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: 1,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(desc.fs_entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.config.format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview: None,
                cache: None,
            })
    }

    fn create_blit_resources(device: &wgpu::Device, format: wgpu::TextureFormat) -> BlitResources {
        let shader = device.create_shader_module(wgpu::include_wgsl!("blit.wgsl"));
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit_bgl"),
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
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit_pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview: None,
            cache: None,
        });

        BlitResources {
            pipeline,
            bind_group_layout,
            sampler,
        }
    }

    fn make_depth_view(device: &wgpu::Device) -> wgpu::TextureView {
        Self::make_depth_attachment(device, "depth").1
    }

    pub(crate) fn surface_blit_rect(&self) -> (u32, u32, u32, u32) {
        let surface_width = self.config.width.max(1);
        let surface_height = self.config.height.max(1);
        let render_width = WIDTH as u64;
        let render_height = HEIGHT as u64;
        let surface_width_u64 = surface_width as u64;
        let surface_height_u64 = surface_height as u64;

        if surface_width_u64 * render_height > surface_height_u64 * render_width {
            let width = ((surface_height_u64 * render_width) / render_height).max(1) as u32;
            let x = surface_width.saturating_sub(width) / 2;
            (x, 0, width, surface_height)
        } else {
            let height = ((surface_width_u64 * render_height) / render_width).max(1) as u32;
            let y = surface_height.saturating_sub(height) / 2;
            (0, y, surface_width, height)
        }
    }

    fn initial_surface_size(window: &Window) -> PhysicalSize<u32> {
        Self::usable_surface_size(window.inner_size()).unwrap_or(PhysicalSize::new(WIDTH, HEIGHT))
    }

    fn usable_surface_size(size: PhysicalSize<u32>) -> Option<PhysicalSize<u32>> {
        (size.width > 0 && size.height > 0).then_some(size)
    }

    fn sync_surface_size_to_window(&mut self) {
        let Some(size) = Self::usable_surface_size(self.window.inner_size()) else {
            return;
        };

        self.config.width = size.width;
        self.config.height = size.height;
    }

    fn make_depth_attachment(
        device: &wgpu::Device,
        label: &str,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        (depth, view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_pipeline_kind_uses_strict_depth() {
        assert!(PipelineDesc::depth_write_for(PipelineKind::Opaque));
        assert_eq!(
            PipelineDesc::depth_compare_for(PipelineKind::Opaque),
            wgpu::CompareFunction::Less
        );
    }

    #[test]
    fn transparent_pipeline_kind_uses_relaxed_depth() {
        assert!(!PipelineDesc::depth_write_for(PipelineKind::Transparent));
        assert_eq!(
            PipelineDesc::depth_compare_for(PipelineKind::Transparent),
            wgpu::CompareFunction::LessEqual
        );
    }

    #[test]
    fn offscreen_target_keeps_color_usage_for_sampling() {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        assert!(usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
        assert!(usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
    }

    #[test]
    fn custom_shader_source_prefers_fragment_then_vertex() {
        let shaders = ShaderSources {
            vertex_wgsl: Some("vertex".into()),
            fragment_wgsl: Some("fragment".into()),
        };
        assert_eq!(custom_shader_source(Some(&shaders)), Some("fragment"));

        let shaders = ShaderSources {
            vertex_wgsl: Some("vertex".into()),
            fragment_wgsl: None,
        };
        assert_eq!(custom_shader_source(Some(&shaders)), Some("vertex"));
        assert_eq!(custom_shader_source(None), None);
    }
}
