use std::time::{Duration, Instant};

use bytemuck::{Pod, Zeroable};

use crate::render_context::{OffscreenTarget, RenderContext, SurfaceBlit};
use crate::slide_renderer::SlideRenderer;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct TransitionUniforms {
    blend_factor: f32,
    transition_kind: u32,
    padding: [u32; 2],
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransitionKind {
    Crossfade = 0,
    WipeLeft = 1,
    WipeDown = 2,
    Dissolve = 3,
    Cut = 4,
}

impl TransitionKind {
    pub(crate) const fn uses_compositor(self) -> bool {
        !matches!(self, Self::Cut)
    }

    const fn shader_tag(self) -> u32 {
        match self {
            Self::Crossfade => 0,
            Self::WipeLeft => 1,
            Self::WipeDown => 2,
            Self::Dissolve => 3,
            Self::Cut => 0,
        }
    }
}

pub(crate) struct TransitionRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buf: wgpu::Buffer,
}

pub(crate) enum TransitionState {
    Idle,
    Blending(ActiveTransition),
}

pub(crate) struct ActiveTransition {
    kind: TransitionKind,
    outgoing_idx: usize,
    outgoing_target: OffscreenTarget,
    incoming_target: OffscreenTarget,
    bind_group: wgpu::BindGroup,
    start_time: Instant,
    duration: Duration,
}

impl Default for TransitionState {
    fn default() -> Self {
        Self::Idle
    }
}

impl TransitionState {
    pub(crate) fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }
}

impl TransitionRenderer {
    pub(crate) fn new(ctx: &RenderContext) -> Self {
        let shader = ctx
            .device
            .create_shader_module(wgpu::include_wgsl!("transition.wgsl"));
        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("transition_bgl"),
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
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
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
        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("transition_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let uniform_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("transition_uniforms"),
            size: std::mem::size_of::<TransitionUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("transition_pipeline_layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });
        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("transition_pipeline"),
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
                        format: ctx.config.format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview: None,
                cache: None,
            });

        Self {
            pipeline,
            bind_group_layout,
            sampler,
            uniform_buf,
        }
    }

    pub(crate) fn create_bind_group(
        &self,
        ctx: &RenderContext,
        outgoing_target: &OffscreenTarget,
        incoming_target: &OffscreenTarget,
    ) -> wgpu::BindGroup {
        ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transition_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&outgoing_target.color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&incoming_target.color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.uniform_buf.as_entire_binding(),
                },
            ],
        })
    }

    fn composite_to_surface(
        &self,
        ctx: &RenderContext,
        bind_group: &wgpu::BindGroup,
        kind: TransitionKind,
        blend_factor: f32,
    ) -> Result<SurfaceBlit, wgpu::SurfaceError> {
        let uniforms = TransitionUniforms {
            blend_factor,
            transition_kind: kind.shader_tag(),
            padding: [0; 2],
        };
        ctx.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        let frame = ctx.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("transition_frame_encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("transition_pass"),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            let (x, y, width, height) = ctx.surface_blit_rect();
            pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
            pass.set_scissor_rect(x, y, width, height);
            pass.draw(0..3, 0..1);
        }

        Ok(SurfaceBlit {
            frame,
            command_buffer: encoder.finish(),
        })
    }
}

impl ActiveTransition {
    pub(crate) fn new(
        ctx: &RenderContext,
        transition_renderer: &TransitionRenderer,
        kind: TransitionKind,
        outgoing_idx: usize,
        duration: Duration,
    ) -> Self {
        let outgoing_target = ctx.create_offscreen_target();
        let incoming_target = ctx.create_offscreen_target();
        let bind_group =
            transition_renderer.create_bind_group(ctx, &outgoing_target, &incoming_target);

        Self {
            kind,
            outgoing_idx,
            outgoing_target,
            incoming_target,
            bind_group,
            start_time: Instant::now(),
            duration,
        }
    }

    pub(crate) fn render(
        &mut self,
        ctx: &RenderContext,
        outgoing: &mut SlideRenderer,
        incoming: &mut SlideRenderer,
        transition_renderer: &TransitionRenderer,
    ) -> Result<bool, wgpu::SurfaceError> {
        let outgoing_commands = outgoing.render_to_target(ctx, &self.outgoing_target);
        let incoming_commands = incoming.render_to_target(ctx, &self.incoming_target);
        let composite = transition_renderer.composite_to_surface(
            ctx,
            &self.bind_group,
            self.kind,
            smoothstep(self.progress()),
        )?;
        ctx.queue.submit([
            outgoing_commands,
            incoming_commands,
            composite.command_buffer,
        ]);
        composite.frame.present();
        Ok(self.is_complete())
    }

    fn progress(&self) -> f32 {
        if self.duration.is_zero() {
            1.0
        } else {
            (self.start_time.elapsed().as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
        }
    }

    fn is_complete(&self) -> bool {
        self.progress() >= 1.0
    }

    pub(crate) fn outgoing_idx(&self) -> usize {
        self.outgoing_idx
    }
}

pub(crate) fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::{TransitionKind, TransitionUniforms, smoothstep};

    #[test]
    fn transition_uniforms_match_uniform_alignment() {
        assert_eq!(std::mem::size_of::<TransitionUniforms>(), 16);
    }

    #[test]
    fn compositor_transition_tags_are_stable() {
        assert_eq!(TransitionKind::Crossfade.shader_tag(), 0);
        assert_eq!(TransitionKind::WipeLeft.shader_tag(), 1);
        assert_eq!(TransitionKind::WipeDown.shader_tag(), 2);
        assert_eq!(TransitionKind::Dissolve.shader_tag(), 3);
    }

    #[test]
    fn cut_skips_compositor_path() {
        assert!(!TransitionKind::Cut.uses_compositor());
        assert!(TransitionKind::Crossfade.uses_compositor());
        assert!(TransitionKind::WipeLeft.uses_compositor());
        assert!(TransitionKind::WipeDown.uses_compositor());
        assert!(TransitionKind::Dissolve.uses_compositor());
    }

    #[test]
    fn smoothstep_clamps_to_unit_interval() {
        assert_eq!(smoothstep(-1.0), 0.0);
        assert_eq!(smoothstep(2.0), 1.0);
    }

    #[test]
    fn smoothstep_keeps_midpoint_fixed() {
        assert!((smoothstep(0.5) - 0.5).abs() < f32::EPSILON);
    }
}
