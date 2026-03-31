use std::any::Any;
use std::path::Path;
use std::time::Instant;

use bytemuck::Pod;
use glam::Mat4;
use vzglyd_slide::{
    DrawSource, DrawSpec, FilterMode, Limits, PipelineKind, RuntimeMeshSet, RuntimeOverlay,
    SceneSpace, ScreenVertex, ShaderSources, SlideSpec, StaticMesh, TextureDesc, TextureFormat,
    WorldLighting, WorldVertex, WrapMode,
};
use wgpu::util::DeviceExt;

use crate::render_context::{
    HEIGHT, OffscreenTarget, PipelineDesc, RenderContext, ScenePipelines, WIDTH,
    custom_shader_source,
};
use crate::scene_utils::{
    WorldUniforms, build_fps_text_with_mode, melbourne_clock_seconds, sample_camera,
};
use crate::shader_validation::{
    ShaderContract, default_imported_scene_shader_source, validate_slide_shader_body,
};
use crate::slide_loader;
use crate::slide_loader::LoadError;
use crate::slide_manifest::SlideManifest;

const WORLD_CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const WORLD_FOG_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ScreenUniforms {
    time: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

pub(crate) struct ScreenSlide {
    spec: SlideSpec<ScreenVertex>,
    runtime: Option<slide_loader::SlideRuntime>,
    background_scene: Option<slide_loader::ScreenBackgroundScene>,
}

pub(crate) struct WorldSlide {
    spec: SlideSpec<WorldVertex>,
    runtime: Option<slide_loader::SlideRuntime>,
    shader_source_hint: Option<slide_loader::ShaderSourceHint>,
}

pub(crate) enum LoadedSlide {
    Screen(ScreenSlide),
    World(WorldSlide),
}

impl LoadedSlide {
    pub fn validate(&self) -> Result<(), vzglyd_slide::SpecError> {
        match self {
            LoadedSlide::Screen(screen) => screen.spec.validate(),
            LoadedSlide::World(world) => world.spec.validate(),
        }
    }
}

fn set_runtime_active(runtime: &mut Option<slide_loader::SlideRuntime>, active: bool) {
    if let Some(runtime) = runtime {
        runtime.set_active(active);
    }
}

struct MeshBuffers {
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    index_count: u32,
}

struct DynamicMeshBuffers {
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    vertex_budget: u32,
    index_budget: u32,
    current_index_count: u32,
}

struct OverlayRuntime {
    buffers: DynamicMeshBuffers,
    limits: Limits,
}

struct ScreenBackdropRuntime {
    draw_plan: Vec<DrawSpec>,
    pipelines: ScenePipelines,
    static_meshes: Vec<MeshBuffers>,
    bind_group: wgpu::BindGroup,
    uniform_buf: wgpu::Buffer,
    camera_path: Option<vzglyd_slide::CameraPath>,
    lighting: WorldLighting,
}

struct ScreenRuntime {
    runtime: Option<slide_loader::SlideRuntime>,
    draw_plan: Vec<DrawSpec>,
    pipelines: ScenePipelines,
    static_meshes: Vec<MeshBuffers>,
    bind_group: wgpu::BindGroup,
    uniform_buf: wgpu::Buffer,
    overlay: Option<OverlayRuntime>,
    backdrop: Option<ScreenBackdropRuntime>,
}

enum DynamicUpdater {
    FpsText { mode: f32 },
    Passive,
}

fn pack_ambient_light(lighting: &WorldLighting) -> [f32; 4] {
    let intensity = lighting.ambient_intensity.max(0.0);
    [
        lighting.ambient_color[0] * intensity,
        lighting.ambient_color[1] * intensity,
        lighting.ambient_color[2] * intensity,
        0.0,
    ]
}

fn pack_main_light_dir(lighting: &WorldLighting) -> [f32; 4] {
    let Some(light) = lighting.directional_light else {
        return [0.0, 1.0, 0.0, 0.0];
    };

    let dir = glam::Vec3::from_array(light.direction).normalize_or_zero();
    if dir.length_squared() == 0.0 {
        [0.0, 1.0, 0.0, 0.0]
    } else {
        [dir.x, dir.y, dir.z, 1.0]
    }
}

fn pack_main_light_color(lighting: &WorldLighting) -> [f32; 4] {
    let Some(light) = lighting.directional_light else {
        return [0.0, 0.0, 0.0, 0.0];
    };

    let intensity = light.intensity.max(0.0);
    [
        light.color[0] * intensity,
        light.color[1] * intensity,
        light.color[2] * intensity,
        0.0,
    ]
}

impl ScreenBackdropRuntime {
    fn new(
        ctx: &RenderContext,
        scene: slide_loader::ScreenBackgroundScene,
    ) -> Result<Self, String> {
        let spec = scene.spec;
        let shader_source_hint = scene.shader_source_hint;
        let font_desc = spec
            .textures
            .first()
            .expect("screen backdrop font texture missing");
        let secondary_desc = spec.textures.get(1).unwrap_or(font_desc);
        let material_a_desc = spec.textures.get(2).unwrap_or(secondary_desc);
        let material_b_desc = spec.textures.get(3).unwrap_or(material_a_desc);

        let font_view = upload_texture_from_spec(&ctx.device, &ctx.queue, font_desc);
        let secondary_view = upload_texture_from_spec(&ctx.device, &ctx.queue, secondary_desc);
        let material_a_view = upload_texture_from_spec(&ctx.device, &ctx.queue, material_a_desc);
        let material_b_view = upload_texture_from_spec(&ctx.device, &ctx.queue, material_b_desc);
        let font_sampler = sampler_from_spec(&ctx.device, font_desc);
        let secondary_sampler = sampler_from_spec(&ctx.device, secondary_desc);

        let uniform_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen_backdrop_uniforms"),
            size: std::mem::size_of::<WorldUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (bind_group_layout, bind_group) = make_world_bind_group(
            &ctx.device,
            &uniform_buf,
            &font_view,
            &secondary_view,
            &material_a_view,
            &material_b_view,
            &font_sampler,
            &secondary_sampler,
        );

        let shader_source = resolve_slide_shader_source(
            "screen_backdrop_shader",
            spec.shaders.as_ref(),
            ShaderContract::World3D,
            shader_source_hint,
        )?;
        let pipelines = catch_pipeline_creation("screen backdrop", || {
            ScenePipelines::create(ctx, &spec.draws, &bind_group_layout, |kind| {
                let label = match kind {
                    PipelineKind::Opaque => "screen_backdrop_pipeline_opaque",
                    PipelineKind::Transparent => "screen_backdrop_pipeline_transparent",
                };
                PipelineDesc::for_kind(
                    label,
                    &shader_source,
                    "vs_main",
                    "fs_main",
                    WorldVertex::desc(),
                    Some(wgpu::Face::Back),
                    kind,
                )
            })
        })?;

        Ok(Self {
            draw_plan: spec.draws,
            pipelines,
            static_meshes: spec
                .static_meshes
                .iter()
                .map(|mesh| create_static_mesh_buffers(&ctx.device, mesh))
                .collect(),
            bind_group,
            uniform_buf,
            camera_path: spec.camera_path,
            lighting: spec.lighting.unwrap_or_default(),
        })
    }

    fn write_uniforms(&self, ctx: &RenderContext, elapsed: f32) {
        let (eye, target, up, fov_y) = sample_camera(&self.camera_path, elapsed, 0.0, 0.0);
        let view = Mat4::look_at_rh(eye, target, up);
        let proj = Mat4::perspective_rh(
            fov_y.to_radians(),
            WIDTH as f32 / HEIGHT as f32,
            0.15,
            180.0,
        );
        let uniforms = WorldUniforms {
            view_proj: (proj * view).to_cols_array_2d(),
            cam_pos: eye.to_array(),
            time: elapsed,
            fog_color: WORLD_FOG_COLOR,
            fog_start: 18.0,
            fog_end: 75.0,
            clock_seconds: melbourne_clock_seconds(),
            _pad: 0.0,
            ambient_light: pack_ambient_light(&self.lighting),
            main_light_dir: pack_main_light_dir(&self.lighting),
            main_light_color: pack_main_light_color(&self.lighting),
        };
        ctx.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.bind_group, &[]);
        for draw in &self.draw_plan {
            if let DrawSource::Static(index) = draw.source {
                if let Some(mesh) = self.static_meshes.get(index) {
                    pass.set_pipeline(self.pipelines.get(draw.pipeline));
                    pass.set_vertex_buffer(0, mesh.vertex.slice(..));
                    pass.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(draw.index_range.clone(), 0, 0..1);
                }
            }
        }
    }
}

struct WorldRuntime {
    runtime: Option<slide_loader::SlideRuntime>,
    draw_plan: Vec<DrawSpec>,
    pipelines: ScenePipelines,
    static_meshes: Vec<MeshBuffers>,
    dynamic_meshes: Vec<DynamicMeshBuffers>,
    dynamic_updaters: Vec<DynamicUpdater>,
    bind_group: wgpu::BindGroup,
    uniform_buf: wgpu::Buffer,
    camera_path: Option<vzglyd_slide::CameraPath>,
    lighting: WorldLighting,
    smoke_focus: (f32, f32),
}

enum SceneRuntime {
    Screen(ScreenRuntime),
    World(WorldRuntime),
}

pub(crate) struct SlideRenderer {
    runtime: SceneRuntime,
    offscreen_target: OffscreenTarget,
    elapsed_secs: f32,
    last_frame_time: Option<Instant>,
    frame_deltas: [f32; 30],
    frame_delta_idx: usize,
}

struct FrameMetrics {
    elapsed: f32,
    dt: f32,
    fps: u32,
}

fn catch_pipeline_creation<T, F>(label: &str, build: F) -> Result<T, String>
where
    F: FnOnce() -> T,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(build)).map_err(|payload| {
        format!(
            "{label} pipeline creation failed: {}",
            describe_panic_payload(payload)
        )
    })
}

fn describe_panic_payload(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else {
        "unknown panic payload".to_string()
    }
}

fn resolve_slide_shader_source(
    label: &str,
    shaders: Option<&ShaderSources>,
    contract: ShaderContract,
    shader_source_hint: Option<slide_loader::ShaderSourceHint>,
) -> Result<String, String> {
    match custom_shader_source(shaders) {
        Some(shader_source) => {
            validate_slide_shader_body(label, shader_source, contract, "vs_main", "fs_main")
                .map_err(|error| format!("{label} validation failed:\n{}", error.diagnostic()))
        }
        None => match (contract, shader_source_hint) {
            (ShaderContract::World3D, Some(slide_loader::ShaderSourceHint::DefaultWorldScene)) => {
                default_imported_scene_shader_source()
                    .map_err(|error| format!("{label} validation failed:\n{}", error.diagnostic()))
            }
            _ => Err(format!("{label} is missing custom shaders")),
        },
    }
}

impl SlideRenderer {
    pub(crate) fn new(ctx: &RenderContext, slide: LoadedSlide) -> Result<Self, String> {
        let runtime = match slide {
            LoadedSlide::Screen(screen) => SceneRuntime::Screen(ScreenRuntime::new(ctx, screen)?),
            LoadedSlide::World(spec) => SceneRuntime::World(WorldRuntime::new(ctx, spec)?),
        };

        Ok(Self {
            runtime,
            offscreen_target: ctx.create_offscreen_target(),
            elapsed_secs: 0.0,
            last_frame_time: None,
            frame_deltas: [0.016; 30],
            frame_delta_idx: 0,
        })
    }

    pub(crate) fn warm_up(&mut self, ctx: &RenderContext) {
        let render_commands = self.encode_to_own_target(
            ctx,
            FrameMetrics {
                elapsed: 0.0,
                dt: 0.0,
                fps: 60,
            },
        );
        ctx.queue.submit([render_commands]);
        self.park();
    }

    pub(crate) fn park(&mut self) {
        match &mut self.runtime {
            SceneRuntime::Screen(screen) => set_runtime_active(&mut screen.runtime, false),
            SceneRuntime::World(world) => set_runtime_active(&mut world.runtime, false),
        }
        self.last_frame_time = None;
    }

    pub(crate) fn render(&mut self, ctx: &RenderContext) -> Result<(), wgpu::SurfaceError> {
        match &mut self.runtime {
            SceneRuntime::Screen(screen) => set_runtime_active(&mut screen.runtime, true),
            SceneRuntime::World(world) => set_runtime_active(&mut world.runtime, true),
        }
        let frame_metrics = self.advance_frame_metrics();
        self.prepare_frame(ctx, &frame_metrics);
        let render_commands = self.encode_to_own_target(ctx, frame_metrics);
        let blit = ctx.blit_to_surface(&self.offscreen_target)?;
        ctx.queue.submit([render_commands, blit.command_buffer]);
        blit.frame.present();
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn render_to_target(
        &mut self,
        ctx: &RenderContext,
        target: &OffscreenTarget,
    ) -> wgpu::CommandBuffer {
        match &mut self.runtime {
            SceneRuntime::Screen(screen) => set_runtime_active(&mut screen.runtime, true),
            SceneRuntime::World(world) => set_runtime_active(&mut world.runtime, true),
        }
        let frame_metrics = self.advance_frame_metrics();
        self.prepare_frame(ctx, &frame_metrics);
        self.encode_to_target(ctx, target, frame_metrics)
    }

    fn advance_frame_metrics(&mut self) -> FrameMetrics {
        let now = Instant::now();
        let dt = self
            .last_frame_time
            .map(|last_frame_time| now.duration_since(last_frame_time).as_secs_f32().min(0.5))
            .unwrap_or(0.0);
        self.last_frame_time = Some(now);
        if dt > 0.0 {
            self.frame_deltas[self.frame_delta_idx] = dt;
            self.frame_delta_idx = (self.frame_delta_idx + 1) % self.frame_deltas.len();
        }
        self.elapsed_secs += dt;
        let avg_dt: f32 = self.frame_deltas.iter().sum::<f32>() / self.frame_deltas.len() as f32;
        let fps = (1.0 / avg_dt.max(0.001)) as u32;

        FrameMetrics {
            elapsed: self.elapsed_secs,
            dt,
            fps,
        }
    }

    fn prepare_frame(&mut self, ctx: &RenderContext, frame_metrics: &FrameMetrics) {
        match &mut self.runtime {
            SceneRuntime::Screen(runtime) => runtime.prepare_frame(ctx, frame_metrics.dt),
            SceneRuntime::World(runtime) => runtime.prepare_frame(ctx, frame_metrics.dt),
        }
    }

    fn encode_to_target(
        &mut self,
        ctx: &RenderContext,
        target: &OffscreenTarget,
        frame_metrics: FrameMetrics,
    ) -> wgpu::CommandBuffer {
        match &mut self.runtime {
            SceneRuntime::Screen(runtime) => runtime.encode(
                ctx,
                &target.color_view,
                &target.depth_view,
                frame_metrics.elapsed,
            ),
            SceneRuntime::World(runtime) => runtime.encode(
                ctx,
                &target.color_view,
                &target.depth_view,
                frame_metrics.elapsed,
                frame_metrics.fps,
            ),
        }
    }

    fn encode_to_own_target(
        &mut self,
        ctx: &RenderContext,
        frame_metrics: FrameMetrics,
    ) -> wgpu::CommandBuffer {
        let target = &self.offscreen_target;
        match &mut self.runtime {
            SceneRuntime::Screen(runtime) => runtime.encode(
                ctx,
                &target.color_view,
                &target.depth_view,
                frame_metrics.elapsed,
            ),
            SceneRuntime::World(runtime) => runtime.encode(
                ctx,
                &target.color_view,
                &target.depth_view,
                frame_metrics.elapsed,
                frame_metrics.fps,
            ),
        }
    }
}

impl ScreenRuntime {
    fn new(ctx: &RenderContext, slide: ScreenSlide) -> Result<Self, String> {
        let ScreenSlide {
            spec,
            runtime,
            background_scene,
        } = slide;
        let tex_desc = spec.textures.first().expect("screen slide texture missing");

        let tex_view = upload_texture_from_spec(&ctx.device, &ctx.queue, tex_desc);
        let detail_texture = spec
            .textures
            .get(1)
            .map(|desc| upload_texture_from_spec(&ctx.device, &ctx.queue, desc));
        let lookup_texture = spec
            .textures
            .get(2)
            .map(|desc| upload_texture_from_spec(&ctx.device, &ctx.queue, desc));
        let sampler = sampler_from_spec(&ctx.device, tex_desc);
        let font_texture = spec
            .font
            .as_ref()
            .map(|font| upload_font_atlas(&ctx.device, &ctx.queue, font));
        let font_sampler = spec.font.as_ref().map(|_| make_font_sampler(&ctx.device));
        let font_view = font_texture.as_ref().unwrap_or(&tex_view);
        let detail_view = detail_texture.as_ref().unwrap_or(&tex_view);
        let lookup_view = lookup_texture.as_ref().unwrap_or(detail_view);
        let font_sampler_ref = font_sampler.as_ref().unwrap_or(&sampler);

        let uniform_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen_uniforms"),
            size: std::mem::size_of::<ScreenUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (bind_group_layout, bind_group) = make_screen_bind_group(
            &ctx.device,
            &tex_view,
            font_view,
            detail_view,
            lookup_view,
            &sampler,
            font_sampler_ref,
            &uniform_buf,
        );

        let shader_source = resolve_slide_shader_source(
            "screen_slide_shader",
            spec.shaders.as_ref(),
            ShaderContract::Screen2D,
            None,
        )?;
        let pipelines = catch_pipeline_creation("screen slide", || {
            ScenePipelines::create(ctx, &spec.draws, &bind_group_layout, |kind| {
                let label = match kind {
                    PipelineKind::Opaque => "screen_pipeline_opaque",
                    PipelineKind::Transparent => "screen_pipeline_transparent",
                };
                PipelineDesc::for_kind(
                    label,
                    &shader_source,
                    "vs_main",
                    "fs_main",
                    ScreenVertex::desc(),
                    Some(wgpu::Face::Back),
                    kind,
                )
            })
        })?;

        let static_meshes = spec
            .static_meshes
            .iter()
            .map(|mesh| create_static_mesh_buffers(&ctx.device, mesh))
            .collect();
        let overlay = make_overlay_runtime(
            ctx,
            &spec,
            runtime
                .as_ref()
                .is_some_and(slide_loader::SlideRuntime::has_overlay),
        );
        let backdrop = background_scene
            .map(|scene| ScreenBackdropRuntime::new(ctx, scene))
            .transpose()?;

        Ok(Self {
            runtime,
            draw_plan: spec.draws,
            pipelines,
            static_meshes,
            bind_group,
            uniform_buf,
            overlay,
            backdrop,
        })
    }

    fn prepare_frame(&mut self, ctx: &RenderContext, dt: f32) {
        let update_result = match &mut self.runtime {
            Some(runtime) => match runtime.update(dt) {
                Ok(result) => result,
                Err(error) => {
                    log::error!("screen slide update failed: {error}");
                    slide_loader::SLIDE_UPDATE_NO_CHANGE
                }
            },
            None => slide_loader::SLIDE_UPDATE_NO_CHANGE,
        };

        if update_result == slide_loader::SLIDE_UPDATE_MESHES_UPDATED {
            if let (Some(runtime), Some(overlay)) = (&mut self.runtime, &mut self.overlay) {
                match runtime.read_overlay::<ScreenVertex>() {
                    Ok(Some(updated_overlay)) => {
                        if overlay.validate(&updated_overlay) {
                            overlay.apply(ctx, &updated_overlay);
                        } else {
                            log::warn!(
                                "screen slide overlay update exceeded declared runtime limits"
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        log::error!("screen slide overlay read failed: {error}");
                    }
                }
            }
        } else if update_result != slide_loader::SLIDE_UPDATE_NO_CHANGE {
            log::warn!(
                "screen slide returned unsupported vzglyd_update code {update_result}; expected 0 or 1"
            );
        }
    }

    fn encode(
        &mut self,
        ctx: &RenderContext,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        elapsed: f32,
    ) -> wgpu::CommandBuffer {
        let uniform = ScreenUniforms {
            time: elapsed,
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
        };
        ctx.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniform));

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("screen_target_encoder"),
            });

        if let Some(backdrop) = &self.backdrop {
            backdrop.write_uniforms(ctx, elapsed);
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("screen_backdrop_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(WORLD_CLEAR_COLOR),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
                backdrop.draw(&mut pass);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("screen_overlay_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: color_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
                self.draw_screen_pass(&mut pass);
            }
        } else {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("screen_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.01,
                            g: 0.02,
                            b: 0.04,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            self.draw_screen_pass(&mut pass);
        }

        encoder.finish()
    }

    fn draw_screen_pass(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.bind_group, &[]);
        for draw in &self.draw_plan {
            pass.set_pipeline(self.pipelines.get(draw.pipeline));
            if let DrawSource::Static(index) = draw.source {
                if let Some(mesh) = self.static_meshes.get(index) {
                    pass.set_vertex_buffer(0, mesh.vertex.slice(..));
                    pass.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(draw.index_range.clone(), 0, 0..1);
                }
            }
        }

        if let Some(overlay) = &self.overlay {
            if overlay.buffers.current_index_count > 0 {
                pass.set_pipeline(self.pipelines.first());
                pass.set_vertex_buffer(0, overlay.buffers.vertex.slice(..));
                pass.set_index_buffer(overlay.buffers.index.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..overlay.buffers.current_index_count, 0, 0..1);
            }
        }
    }
}

impl OverlayRuntime {
    fn validate(&self, overlay: &RuntimeOverlay<ScreenVertex>) -> bool {
        overlay.vertices.len() <= self.limits.max_vertices as usize
            && overlay.indices.len() <= self.limits.max_indices as usize
    }

    fn apply(&mut self, ctx: &RenderContext, overlay: &RuntimeOverlay<ScreenVertex>) {
        if overlay.vertices.is_empty() || overlay.indices.is_empty() {
            self.buffers.current_index_count = 0;
            return;
        }

        let max_vertices = self.buffers.vertex_budget as usize;
        let max_indices = self.buffers.index_budget as usize;
        let used_vertices = overlay.vertices.len().min(max_vertices);
        let used_indices = overlay.indices.len().min(max_indices);
        ctx.queue.write_buffer(
            &self.buffers.vertex,
            0,
            bytemuck::cast_slice(&overlay.vertices[..used_vertices]),
        );
        ctx.queue.write_buffer(
            &self.buffers.index,
            0,
            bytemuck::cast_slice(&overlay.indices[..used_indices]),
        );
        self.buffers.current_index_count = used_indices as u32;
    }
}

impl WorldRuntime {
    fn new(ctx: &RenderContext, slide: WorldSlide) -> Result<Self, String> {
        let WorldSlide {
            spec,
            runtime,
            shader_source_hint,
        } = slide;
        let font_desc = spec
            .textures
            .first()
            .expect("world slide font texture missing");
        let secondary_desc = spec.textures.get(1).unwrap_or(font_desc);
        let material_a_desc = spec.textures.get(2).unwrap_or(secondary_desc);
        let material_b_desc = spec.textures.get(3).unwrap_or(material_a_desc);

        let font_view = upload_texture_from_spec(&ctx.device, &ctx.queue, font_desc);
        let secondary_view = upload_texture_from_spec(&ctx.device, &ctx.queue, secondary_desc);
        let material_a_view = upload_texture_from_spec(&ctx.device, &ctx.queue, material_a_desc);
        let material_b_view = upload_texture_from_spec(&ctx.device, &ctx.queue, material_b_desc);
        let font_sampler = sampler_from_spec(&ctx.device, font_desc);
        let secondary_sampler = sampler_from_spec(&ctx.device, secondary_desc);

        let uniform_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("world_uniforms"),
            size: std::mem::size_of::<WorldUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (bind_group_layout, bind_group) = make_world_bind_group(
            &ctx.device,
            &uniform_buf,
            &font_view,
            &secondary_view,
            &material_a_view,
            &material_b_view,
            &font_sampler,
            &secondary_sampler,
        );

        let shader_source = resolve_slide_shader_source(
            "world_slide_shader",
            spec.shaders.as_ref(),
            ShaderContract::World3D,
            shader_source_hint,
        )?;
        let pipelines = catch_pipeline_creation("world slide", || {
            ScenePipelines::create(ctx, &spec.draws, &bind_group_layout, |kind| {
                let label = match kind {
                    PipelineKind::Opaque => "world_pipeline_opaque",
                    PipelineKind::Transparent => "world_pipeline_transparent",
                };
                PipelineDesc::for_kind(
                    label,
                    &shader_source,
                    "vs_main",
                    "fs_main",
                    WorldVertex::desc(),
                    Some(wgpu::Face::Back),
                    kind,
                )
            })
        })?;

        let static_meshes = spec
            .static_meshes
            .iter()
            .map(|mesh| create_static_mesh_buffers(&ctx.device, mesh))
            .collect();
        let dynamic_meshes = spec
            .dynamic_meshes
            .iter()
            .map(|mesh| DynamicMeshBuffers {
                vertex: ctx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(mesh.label.as_str()),
                    size: (mesh.max_vertices as usize * std::mem::size_of::<WorldVertex>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                index: ctx
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(format!("{}_indices", mesh.label).as_str()),
                        contents: bytemuck::cast_slice(&mesh.indices),
                        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    }),
                vertex_budget: mesh.max_vertices,
                index_budget: mesh.indices.len() as u32,
                current_index_count: 0,
            })
            .collect::<Vec<_>>();

        let smoke_focus = spec
            .camera_path
            .as_ref()
            .and_then(|path| path.keyframes.first())
            .map(|frame| (frame.target[0], frame.target[2]))
            .unwrap_or((0.0, 18.0));
        let lighting = spec.lighting.clone().unwrap_or_default();
        let dynamic_updaters = spec
            .dynamic_meshes
            .iter()
            .map(|mesh| {
                let label = mesh.label.to_ascii_lowercase();
                if label.contains("fps") {
                    DynamicUpdater::FpsText {
                        mode: if label.contains("mode3") {
                            3.0
                        } else if label.contains("mode4") {
                            4.0
                        } else {
                            4.0
                        },
                    }
                } else {
                    DynamicUpdater::Passive
                }
            })
            .collect();

        let mut runtime = Self {
            runtime,
            draw_plan: spec.draws,
            pipelines,
            static_meshes,
            dynamic_meshes,
            dynamic_updaters,
            bind_group,
            uniform_buf,
            camera_path: spec.camera_path,
            lighting,
            smoke_focus,
        };
        if let Some(slide_runtime) = &mut runtime.runtime {
            if slide_runtime.has_dynamic_meshes() {
                match slide_runtime.read_dynamic_meshes::<WorldVertex>() {
                    Ok(Some(meshes)) => {
                        apply_runtime_world_meshes(ctx, &mut runtime.dynamic_meshes, &meshes);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        log::error!("world slide initial dynamic mesh read failed: {error}");
                    }
                }
            }
        }
        Ok(runtime)
    }

    fn prepare_frame(&mut self, ctx: &RenderContext, dt: f32) {
        let Some(runtime) = &mut self.runtime else {
            return;
        };

        match runtime.update(dt) {
            Ok(slide_loader::SLIDE_UPDATE_NO_CHANGE) => {}
            Ok(slide_loader::SLIDE_UPDATE_MESHES_UPDATED) => {
                match runtime.read_dynamic_meshes::<WorldVertex>() {
                    Ok(Some(meshes)) => {
                        apply_runtime_world_meshes(ctx, &mut self.dynamic_meshes, &meshes);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        log::error!("world slide dynamic mesh read failed: {error}");
                    }
                }
            }
            Ok(other) => {
                log::warn!(
                    "world slide returned unsupported vzglyd_update code {other}; expected 0 or 1"
                );
            }
            Err(error) => {
                log::error!("world slide update failed: {error}");
            }
        }
    }

    fn encode(
        &mut self,
        ctx: &RenderContext,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        elapsed: f32,
        fps: u32,
    ) -> wgpu::CommandBuffer {
        let (eye, target, up, fov_y) = sample_camera(
            &self.camera_path,
            elapsed,
            self.smoke_focus.0,
            self.smoke_focus.1,
        );
        let view = Mat4::look_at_rh(eye, target, up);
        let proj = Mat4::perspective_rh(
            fov_y.to_radians(),
            WIDTH as f32 / HEIGHT as f32,
            0.15,
            180.0,
        );
        let view_proj = (proj * view).to_cols_array_2d();

        let forward = (target - eye).normalize();
        let cam_right = forward.cross(up).normalize_or_zero();
        let _cam_up = cam_right.cross(forward).normalize_or_zero();
        let uniforms = WorldUniforms {
            view_proj,
            cam_pos: eye.to_array(),
            time: elapsed,
            fog_color: WORLD_FOG_COLOR,
            fog_start: 18.0,
            fog_end: 75.0,
            clock_seconds: melbourne_clock_seconds(),
            _pad: 0.0,
            ambient_light: pack_ambient_light(&self.lighting),
            main_light_dir: pack_main_light_dir(&self.lighting),
            main_light_color: pack_main_light_color(&self.lighting),
        };
        ctx.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        for (buffers, updater) in self
            .dynamic_meshes
            .iter_mut()
            .zip(self.dynamic_updaters.iter_mut())
        {
            match updater {
                DynamicUpdater::FpsText { mode } => {
                    let mut verts = Vec::new();
                    let mut indices = Vec::new();
                    build_fps_text_with_mode(&mut verts, &mut indices, fps, *mode);
                    let requested_quads = (indices.len() / 6) as u32;
                    let allowed_quads = (buffers.vertex_budget / 4).min(buffers.index_budget / 6);
                    let used_quads = requested_quads.min(allowed_quads);
                    let used_vertices = (used_quads as usize) * 4;
                    verts.truncate(used_vertices);
                    ctx.queue
                        .write_buffer(&buffers.vertex, 0, bytemuck::cast_slice(&verts));
                    buffers.current_index_count = used_quads * 6;
                }
                DynamicUpdater::Passive => {}
            }
        }

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("world_target_encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(WORLD_CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            pass.set_bind_group(0, &self.bind_group, &[]);
            for draw in &self.draw_plan {
                pass.set_pipeline(self.pipelines.get(draw.pipeline));
                match draw.source {
                    DrawSource::Static(index) => {
                        if let Some(mesh) = self.static_meshes.get(index) {
                            debug_assert!(draw.index_range.end <= mesh.index_count);
                            pass.set_vertex_buffer(0, mesh.vertex.slice(..));
                            pass.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint16);
                            pass.draw_indexed(draw.index_range.clone(), 0, 0..1);
                        }
                    }
                    DrawSource::Dynamic(index) => {
                        if let Some(mesh) = self.dynamic_meshes.get(index) {
                            let available = mesh
                                .current_index_count
                                .saturating_sub(draw.index_range.start);
                            let requested = draw.index_range.end - draw.index_range.start;
                            let count = available.min(requested);
                            if count > 0 {
                                pass.set_vertex_buffer(0, mesh.vertex.slice(..));
                                pass.set_index_buffer(
                                    mesh.index.slice(..),
                                    wgpu::IndexFormat::Uint16,
                                );
                                pass.draw_indexed(
                                    draw.index_range.start..draw.index_range.start + count,
                                    0,
                                    0..1,
                                );
                            }
                        }
                    }
                }
            }
        }

        encoder.finish()
    }
}

fn apply_runtime_world_meshes(
    ctx: &RenderContext,
    buffers: &mut [DynamicMeshBuffers],
    meshes: &RuntimeMeshSet<WorldVertex>,
) {
    for mesh in &meshes.meshes {
        let Some(buffer) = buffers.get_mut(mesh.mesh_index as usize) else {
            log::warn!(
                "world slide runtime updated unknown dynamic mesh index {}",
                mesh.mesh_index
            );
            continue;
        };
        let used_vertices = mesh.vertices.len().min(buffer.vertex_budget as usize);
        ctx.queue.write_buffer(
            &buffer.vertex,
            0,
            bytemuck::cast_slice(&mesh.vertices[..used_vertices]),
        );
        buffer.current_index_count = mesh.index_count.min(buffer.index_budget);
    }
}

pub(crate) fn load_wasm_slide_from_bytes(
    bytes: &[u8],
) -> Result<(LoadedSlide, Option<SlideManifest>), LoadError> {
    let extracted = slide_loader::extract_embedded_bytes_to_cache(bytes)?;
    load_wasm_slide(&extracted.to_string_lossy(), None)
}

pub(crate) fn load_wasm_slide(
    path: &str,
    params_bytes: Option<&[u8]>,
) -> Result<(LoadedSlide, Option<SlideManifest>), LoadError> {
    if let Ok((slide, manifest)) = load_screen_wasm_slide(path, params_bytes) {
        if let LoadedSlide::Screen(screen) = &slide {
            if screen.spec.scene_space == SceneSpace::Screen2D && screen.spec.validate().is_ok() {
                return Ok((slide, Some(manifest)));
            }
        }
    }

    let (loaded, manifest) = load_spec_with_manifest::<WorldVertex>(path, params_bytes)?;
    Ok((
        LoadedSlide::World(WorldSlide {
            spec: loaded.spec,
            runtime: loaded.runtime,
            shader_source_hint: loaded.shader_source_hint,
        }),
        Some(manifest),
    ))
}

fn load_screen_wasm_slide(
    path: &str,
    params_bytes: Option<&[u8]>,
) -> Result<(LoadedSlide, SlideManifest), LoadError> {
    let (loaded, manifest) = load_spec_with_manifest::<ScreenVertex>(path, params_bytes)?;
    Ok((
        LoadedSlide::Screen(ScreenSlide {
            spec: loaded.spec,
            runtime: loaded.runtime,
            background_scene: loaded.screen_background_scene,
        }),
        manifest,
    ))
}

fn load_spec_with_manifest<V>(
    path: &str,
    params_bytes: Option<&[u8]>,
) -> Result<(slide_loader::LoadedSpec<V>, SlideManifest), LoadError>
where
    V: slide_loader::PackageMeshVertex,
{
    if Path::new(path).extension().and_then(|ext| ext.to_str())
        == Some(slide_loader::PACKAGE_ARCHIVE_EXTENSION)
    {
        slide_loader::load_slide_from_archive(path, params_bytes)
    } else {
        slide_loader::load_slide_from_wasm(path, params_bytes)
    }
}

fn make_overlay_runtime(
    ctx: &RenderContext,
    spec: &SlideSpec<ScreenVertex>,
    has_runtime_overlay: bool,
) -> Option<OverlayRuntime> {
    if spec.overlay.is_none() && !has_runtime_overlay {
        return None;
    }

    let mut overlay = OverlayRuntime {
        buffers: DynamicMeshBuffers {
            vertex: ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("overlay_vertex_buffer"),
                size: (spec.limits.max_vertices as usize * std::mem::size_of::<ScreenVertex>())
                    as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            index: ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("overlay_index_buffer"),
                size: (spec.limits.max_indices as usize * std::mem::size_of::<u16>()) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            vertex_budget: spec.limits.max_vertices,
            index_budget: spec.limits.max_indices,
            current_index_count: 0,
        },
        limits: spec.limits,
    };
    if let Some(static_overlay) = spec.overlay.as_ref() {
        overlay.apply(ctx, static_overlay);
    }
    Some(overlay)
}

fn create_static_mesh_buffers<V: Pod>(device: &wgpu::Device, mesh: &StaticMesh<V>) -> MeshBuffers {
    MeshBuffers {
        vertex: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(format!("{}_vertex", mesh.label).as_str()),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        index: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(format!("{}_index", mesh.label).as_str()),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
        index_count: mesh.indices.len() as u32,
    }
}

fn texture_format_to_wgpu(fmt: TextureFormat) -> wgpu::TextureFormat {
    match fmt {
        TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
    }
}

fn wrap_to_wgpu(wrap: WrapMode) -> wgpu::AddressMode {
    match wrap {
        WrapMode::Repeat => wgpu::AddressMode::Repeat,
        WrapMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
    }
}

fn filter_to_wgpu(filter: FilterMode) -> wgpu::FilterMode {
    match filter {
        FilterMode::Nearest => wgpu::FilterMode::Nearest,
        FilterMode::Linear => wgpu::FilterMode::Linear,
    }
}

fn upload_texture_from_spec(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    desc: &TextureDesc,
) -> wgpu::TextureView {
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(desc.label.as_str()),
            size: wgpu::Extent3d {
                width: desc.width,
                height: desc.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format_to_wgpu(desc.format),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &desc.data,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn sampler_from_spec(device: &wgpu::Device, desc: &TextureDesc) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(desc.label.as_str()),
        address_mode_u: wrap_to_wgpu(desc.wrap_u),
        address_mode_v: wrap_to_wgpu(desc.wrap_v),
        address_mode_w: wrap_to_wgpu(desc.wrap_w),
        mag_filter: filter_to_wgpu(desc.mag_filter),
        min_filter: filter_to_wgpu(desc.min_filter),
        mipmap_filter: filter_to_wgpu(desc.mip_filter),
        ..Default::default()
    })
}

fn upload_font_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &vzglyd_slide::FontAtlas,
) -> wgpu::TextureView {
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("font_atlas"),
            size: wgpu::Extent3d {
                width: atlas.width,
                height: atlas.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &atlas.pixels,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn make_font_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("font_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    })
}

fn make_screen_bind_group(
    device: &wgpu::Device,
    tex_view: &wgpu::TextureView,
    font_view: &wgpu::TextureView,
    detail_view: &wgpu::TextureView,
    lookup_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    font_sampler: &wgpu::Sampler,
    uniform_buf: &wgpu::Buffer,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("screen_bgl"),
        entries: &[
            bgl_texture(0, wgpu::ShaderStages::FRAGMENT),
            bgl_texture(1, wgpu::ShaderStages::FRAGMENT),
            bgl_texture(2, wgpu::ShaderStages::FRAGMENT),
            bgl_texture(3, wgpu::ShaderStages::FRAGMENT),
            bgl_sampler(4, wgpu::ShaderStages::FRAGMENT),
            bgl_sampler(5, wgpu::ShaderStages::FRAGMENT),
            bgl_uniform(6, wgpu::ShaderStages::FRAGMENT),
        ],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("screen_bg"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(tex_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(font_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(detail_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(lookup_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(font_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: uniform_buf.as_entire_binding(),
            },
        ],
    });
    (layout, bind_group)
}

fn make_world_bind_group(
    device: &wgpu::Device,
    uniform_buf: &wgpu::Buffer,
    font_view: &wgpu::TextureView,
    secondary_view: &wgpu::TextureView,
    material_a_view: &wgpu::TextureView,
    material_b_view: &wgpu::TextureView,
    font_sampler: &wgpu::Sampler,
    secondary_sampler: &wgpu::Sampler,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("world_bgl"),
        entries: &[
            bgl_uniform(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
            bgl_texture(1, wgpu::ShaderStages::FRAGMENT),
            bgl_texture(2, wgpu::ShaderStages::FRAGMENT),
            bgl_texture(3, wgpu::ShaderStages::FRAGMENT),
            bgl_texture(4, wgpu::ShaderStages::FRAGMENT),
            bgl_sampler(5, wgpu::ShaderStages::FRAGMENT),
            bgl_sampler(6, wgpu::ShaderStages::FRAGMENT),
        ],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("world_bg"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(font_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(secondary_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(material_a_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(material_b_view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(font_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(secondary_sampler),
            },
        ],
    });
    (layout, bind_group)
}

fn bgl_uniform(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn bgl_texture(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
        },
        count: None,
    }
}

fn bgl_sampler(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_context::DEPTH_FORMAT;

    #[test]
    fn screen_vertex_struct_is_40_bytes() {
        assert_eq!(std::mem::size_of::<ScreenVertex>(), 40);
    }

    #[test]
    fn screen_uniform_struct_is_16_bytes() {
        assert_eq!(std::mem::size_of::<ScreenUniforms>(), 16);
    }

    #[test]
    fn invalid_custom_shader_is_fatal_at_renderer_boundary() {
        let shaders = ShaderSources {
            vertex_wgsl: None,
            fragment_wgsl: Some(
                r#"
@vertex
fn vs_main(in: VzglydVertexInput) -> VzglydVertexOutput {
    var out: VzglydVertexOutput;
    out.clip_pos = vec4<f32>(in.position, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    out.mode = in.mode;
    return out;
}

@group(1) @binding(0) var bad_tex: texture_2d<f32>;

@fragment
fn fs_main(in: VzglydVertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
"#
                .into(),
            ),
        };

        let error = resolve_slide_shader_source(
            "screen_slide_shader",
            Some(&shaders),
            ShaderContract::Screen2D,
            None,
        )
        .expect_err("invalid custom shaders should reject the slide");

        assert!(error.contains("validation failed"));
        assert!(error.contains("may only use bind group 0"));
    }

    #[test]
    fn slides_without_custom_shaders_are_rejected() {
        let error = resolve_slide_shader_source(
            "screen_slide_shader",
            None,
            ShaderContract::Screen2D,
            None,
        )
        .expect_err("slides without custom shaders should be rejected");

        assert!(error.contains("missing custom shaders"));
    }

    #[test]
    fn default_imported_scene_shader_is_available_without_custom_wgsl() {
        let shader = resolve_slide_shader_source(
            "world_slide_shader",
            None,
            ShaderContract::World3D,
            Some(slide_loader::ShaderSourceHint::DefaultWorldScene),
        )
        .expect("imported world scenes should use the built-in shader");

        assert!(shader.contains("@group(0) @binding(4) var t_material_b"));
        assert!(shader.contains("textureSample(t_material_a"));
        assert!(!shader.contains("textureSample(t_noise"));
        assert!(!shader.contains("floor(diff * 3.0"));
        assert!(shader.contains("fn fs_main"));
    }

    #[test]
    fn depth_format_is_float() {
        assert_eq!(DEPTH_FORMAT, wgpu::TextureFormat::Depth32Float);
    }

    #[test]
    fn static_mesh_buffers_preserve_index_count() {
        let mesh = StaticMesh {
            label: "mesh".into(),
            vertices: vec![ScreenVertex {
                position: [0.0, 0.0, 0.0],
                tex_coords: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
                mode: 0.0,
            }],
            indices: vec![0],
        };
        assert_eq!(mesh.indices.len() as u32, 1);
    }
}
