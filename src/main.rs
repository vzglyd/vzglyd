mod date_utils;
mod playlist;
mod render_context;
mod scene_utils;
mod shader_validation;
mod slide_loader;
mod slide_manifest;
mod slide_renderer;
mod transition;

use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use render_context::{HEIGHT, RenderContext, WIDTH};
use slide_manifest::SlideManifest;
use slide_renderer::{LoadedSlide, SlideRenderer, load_wasm_slide, load_wasm_slide_from_bytes};
use transition::{ActiveTransition, TransitionKind, TransitionRenderer, TransitionState};
#[cfg(target_os = "linux")]
use winit::platform::x11::{
    ActiveEventLoopExtX11, WindowAttributesExtX11, WindowType as X11WindowType,
};
use winit::{
    application::ApplicationHandler,
    dpi::{PhysicalPosition, PhysicalSize},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowAttributes, WindowId, WindowLevel},
};

// ── Scene selection ───────────────────────────────────────────────────────────

const LOADING_SCENE_PATH: &str = "$loading";
const LOADING_SLIDE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/loading.vzglyd"));
/// Default directory scanned for `.vzglyd` slide packages when no `--slides-dir` is given.
const DEFAULT_SLIDES_DIR: &str = "slides";

/// A slide in the display schedule.
///
/// Two scenes are considered equal if their paths match; the override fields are
/// display metadata and do not affect identity comparisons used by schedule lookup.
#[derive(Clone, Debug)]
struct Scene {
    path: String,
    /// Overrides the per-manifest and engine-default display duration.
    duration_override: Option<Duration>,
    /// Overrides the manifest and engine-default transition into this slide.
    transition_in_override: Option<TransitionKind>,
    /// Overrides the manifest and engine-default transition out of this slide.
    transition_out_override: Option<TransitionKind>,
    /// Optional JSON parameters written to the slide's configure buffer before init.
    params: Option<serde_json::Value>,
}

impl PartialEq for Scene {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
impl Eq for Scene {}

struct RunConfig {
    scene: Scene,
    slides_dir: Option<String>,
    single_scene: bool,
    verbose: bool,
}

enum Command {
    Run(RunConfig),
    Pack {
        source_dir: String,
        output_path: String,
        verbose: bool,
    },
}

impl Scene {
    fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            duration_override: None,
            transition_in_override: None,
            transition_out_override: None,
            params: None,
        }
    }

    fn new_with_overrides(
        path: impl Into<String>,
        duration_override: Option<Duration>,
        transition_in_override: Option<TransitionKind>,
        transition_out_override: Option<TransitionKind>,
    ) -> Self {
        Self {
            path: path.into(),
            duration_override,
            transition_in_override,
            transition_out_override,
            params: None,
        }
    }

    fn path(&self) -> &str {
        &self.path
    }
}

fn parse_command() -> Result<Command, String> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "pack") {
        return parse_pack_command(&args[2..]);
    }
    Ok(Command::Run(parse_run_config(&args[1..])))
}

fn parse_pack_command(args: &[String]) -> Result<Command, String> {
    let source = args
        .first()
        .ok_or_else(|| "usage: vzglyd pack <slide-dir> -o <archive.vzglyd>".to_string())?;

    let mut output_path = None;
    let mut verbose = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                let Some(path) = args.get(i + 1) else {
                    return Err("missing output path after -o".into());
                };
                output_path = Some(path.clone());
                i += 2;
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            other => {
                return Err(format!(
                    "unknown pack argument '{other}'; usage: vzglyd pack <slide-dir> -o <archive.vzglyd>"
                ));
            }
        }
    }

    let output_path = output_path.ok_or_else(|| {
        "missing -o <archive.vzglyd>; usage: vzglyd pack <slide-dir> -o <archive.vzglyd>".to_string()
    })?;
    Ok(Command::Pack {
        source_dir: source.to_string(),
        output_path,
        verbose,
    })
}

fn parse_run_config(args: &[String]) -> RunConfig {
    let mut scene = Scene::new(DEFAULT_SLIDES_DIR);
    let mut slides_dir = None;
    let mut single_scene = false;
    let mut verbose = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
                continue;
            }
            "--scene" => {
                if let Some(s) = args.get(i + 1) {
                    scene = Scene::new(s.clone());
                    slides_dir = None;
                    single_scene = true;
                    i += 2;
                    continue;
                }
            }
            "--slides-dir" => {
                if let Some(dir) = args.get(i + 1) {
                    scene = Scene::new(dir.clone());
                    slides_dir = Some(dir.clone());
                    single_scene = false;
                    i += 2;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    RunConfig {
        scene,
        slides_dir,
        single_scene,
        verbose,
    }
}

fn command_verbose(command: &Command) -> bool {
    match command {
        Command::Run(run) => run.verbose,
        Command::Pack { verbose, .. } => *verbose,
    }
}

fn init_logging(verbose: bool) {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(if verbose {
            "warn"
        } else {
            "error"
        }));
    builder.filter_module("wgpu_hal::gles::egl", log::LevelFilter::Off);
    if verbose {
        builder.filter_module("vzglyd", log::LevelFilter::Info);
    }
    builder.init();
}

fn build_content_schedule(
    scene: &Scene,
    slides_dir: Option<&str>,
    single_scene: bool,
) -> Result<Vec<Scene>, String> {
    if single_scene {
        Ok(vec![scene.clone()])
    } else {
        discover_slide_schedule(slides_dir.unwrap_or(DEFAULT_SLIDES_DIR))
    }
}

fn build_bootstrap_schedule(
    scene: &Scene,
    content_schedule: &[Scene],
) -> Option<(Vec<Scene>, usize)> {
    if scene.path() == LOADING_SCENE_PATH {
        return None;
    }

    let target_idx = content_schedule
        .iter()
        .position(|scheduled| scheduled == scene)
        .unwrap_or(0)
        + 1;
    let mut schedule = Vec::with_capacity(content_schedule.len() + 1);
    schedule.push(Scene::new(LOADING_SCENE_PATH));
    schedule.extend(content_schedule.iter().cloned());
    Some((schedule, target_idx))
}

fn discover_slide_schedule(slides_dir: &str) -> Result<Vec<Scene>, String> {
    let dir = Path::new(slides_dir);

    if let Some(playlist) = playlist::load_playlist(dir)
        .map_err(|e| format!("playlist error in '{}': {e}", dir.display()))?
    {
        return build_schedule_from_playlist(dir, &playlist);
    }

    discover_slide_schedule_alphabetical(dir)
}

fn build_schedule_from_playlist(
    slides_dir: &Path,
    playlist: &playlist::Playlist,
) -> Result<Vec<Scene>, String> {
    let scenes: Vec<Scene> = playlist
        .slides
        .iter()
        .filter(|entry| entry.enabled.unwrap_or(true))
        .map(|entry| {
            let abs_path = slides_dir.join(&entry.path);
            let duration = entry
                .duration_seconds
                .or(playlist.defaults.duration_seconds)
                .map(|s| Duration::from_secs(u64::from(s)));
            let t_in = entry
                .transition_in
                .as_deref()
                .or(playlist.defaults.transition_in.as_deref())
                .map(slide_manifest::parse_transition_kind);
            let t_out = entry
                .transition_out
                .as_deref()
                .or(playlist.defaults.transition_out.as_deref())
                .map(slide_manifest::parse_transition_kind);
            let mut scene =
                Scene::new_with_overrides(abs_path.to_string_lossy(), duration, t_in, t_out);
            scene.params = entry.params.clone();
            scene
        })
        .collect();

    if scenes.is_empty() {
        return Err(format!(
            "playlist in '{}' has no enabled slides",
            slides_dir.display()
        ));
    }
    Ok(scenes)
}

fn discover_slide_schedule_alphabetical(path: &Path) -> Result<Vec<Scene>, String> {
    let entries = std::fs::read_dir(path).map_err(|error| {
        format!(
            "failed to read slides directory '{}': {error}",
            path.display()
        )
    })?;

    let mut scenes = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect slides directory '{}': {error}",
                path.display()
            )
        })?;
        let entry_path = entry.path();
        if is_slide_archive(&entry_path) || is_slide_directory(&entry_path) {
            scenes.push(Scene::new(entry_path.to_string_lossy().into_owned()));
        }
    }

    scenes.sort_by(|left, right| left.path().cmp(right.path()));
    if scenes.is_empty() {
        return Err(format!(
            "slides directory '{}' contains no .vzglyd archives or slide packages",
            path.display()
        ));
    }

    Ok(scenes)
}

fn is_slide_archive(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case(slide_loader::PACKAGE_ARCHIVE_EXTENSION))
}

fn is_slide_directory(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }

    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    let mut has_manifest = false;
    let mut has_wasm = false;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name == slide_loader::PACKAGE_MANIFEST_NAME || file_name.ends_with("_slide.json") {
            has_manifest = true;
        }
        if file_name == slide_loader::PACKAGE_WASM_NAME || file_name.ends_with("_slide.wasm") {
            has_wasm = true;
        }
    }

    has_manifest && has_wasm
}

fn scene_title(scene: &Scene) -> String {
    let path = scene.path();
    let label = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path);
    format!("VZGLYD — {label}")
}

const DEFAULT_TRANSITION: TransitionKind = TransitionKind::Crossfade;
const TRANSITION_DURATION: Duration = Duration::from_millis(600);
const DEFAULT_SLIDE_DURATION: Duration = Duration::from_secs(7);
#[cfg(target_os = "linux")]
const X11_BORDERLESS_INSET: u32 = 1;

fn build_window_attributes(event_loop: &ActiveEventLoop, scene: &Scene) -> WindowAttributes {
    let desired_size = PhysicalSize::new(WIDTH, HEIGHT);
    let mut level = WindowLevel::AlwaysOnTop;
    let mut attributes = Window::default_attributes()
        .with_title(scene_title(scene))
        .with_inner_size(desired_size)
        .with_resizable(false)
        .with_decorations(false);

    #[cfg(target_os = "linux")]
    {
        if event_loop.is_x11() {
            attributes = attributes
                .with_override_redirect(false)
                .with_x11_window_type(vec![X11WindowType::Normal]);

            if let Some((window_size, position)) =
                x11_borderless_managed_geometry(event_loop, desired_size)
            {
                log::info!(
                    "X11 borderless window matched the monitor size; using {:?} at {:?} to avoid fullscreen heuristics",
                    window_size,
                    position
                );
                attributes = attributes
                    .with_inner_size(window_size)
                    .with_position(position);
                level = WindowLevel::Normal;
            }
        }
    }

    attributes.with_window_level(level)
}

#[cfg(target_os = "linux")]
fn x11_borderless_managed_geometry(
    event_loop: &ActiveEventLoop,
    desired_size: PhysicalSize<u32>,
) -> Option<(PhysicalSize<u32>, PhysicalPosition<i32>)> {
    let monitor = event_loop.primary_monitor()?;
    let monitor_size = monitor.size();
    if desired_size.width < monitor_size.width || desired_size.height < monitor_size.height {
        return None;
    }

    let inset = X11_BORDERLESS_INSET;
    let inset_span = inset.saturating_mul(2);
    let window_size = PhysicalSize::new(
        monitor_size.width.saturating_sub(inset_span).max(1),
        monitor_size.height.saturating_sub(inset_span).max(1),
    );
    let monitor_position = monitor.position();
    let position = PhysicalPosition::new(
        monitor_position.x + inset as i32,
        monitor_position.y + inset as i32,
    );

    Some((window_size, position))
}

// ── App ───────────────────────────────────────────────────────────────────────

struct PreparedRenderer {
    renderer: SlideRenderer,
    manifest: Option<SlideManifest>,
}

struct BootstrapState {
    target_schedule_idx: usize,
    first_frame_presented: bool,
    next_load_idx: usize,
    load_receiver: Option<Receiver<Result<LoadedSlidePackage, String>>>,
}

struct LoadedSlidePackage {
    slide: LoadedSlide,
    manifest: Option<SlideManifest>,
    path: String,
}

fn prepared_pair_mut(
    renderers: &mut [PreparedRenderer],
    first_idx: usize,
    second_idx: usize,
) -> Option<(&mut PreparedRenderer, &mut PreparedRenderer)> {
    if first_idx == second_idx {
        return None;
    }

    let (lower_idx, upper_idx, swapped) = if first_idx < second_idx {
        (first_idx, second_idx, false)
    } else {
        (second_idx, first_idx, true)
    };
    let (lower, upper) = renderers.split_at_mut(upper_idx);
    let first = lower.get_mut(lower_idx)?;
    let second = upper.first_mut()?;

    Some(if swapped {
        (second, first)
    } else {
        (first, second)
    })
}

fn prepare_renderer(
    ctx: &RenderContext,
    slide: LoadedSlide,
    manifest: Option<SlideManifest>,
    label: &str,
) -> Result<PreparedRenderer, String> {
    let mut renderer = SlideRenderer::new(ctx, slide)
        .map_err(|err| format!("{label} renderer init failed: {err}"))?;
    renderer.warm_up(ctx);
    Ok(PreparedRenderer { renderer, manifest })
}

fn resolve_transition(
    outgoing_scene: Option<&Scene>,
    outgoing_manifest: Option<&SlideManifest>,
    incoming_scene: Option<&Scene>,
    incoming_manifest: Option<&SlideManifest>,
) -> (TransitionKind, Duration) {
    // Scene-level overrides (from playlist) take precedence over per-slide manifest values.
    outgoing_scene
        .and_then(|s| s.transition_out_override)
        .or_else(|| outgoing_manifest.and_then(|m| m.transition_out_kind()))
        .or_else(|| incoming_scene.and_then(|s| s.transition_in_override))
        .or_else(|| incoming_manifest.and_then(|m| m.transition_in_kind()))
        .map(|kind| (kind, TRANSITION_DURATION))
        .unwrap_or((DEFAULT_TRANSITION, TRANSITION_DURATION))
}

fn current_slide_duration(
    scene: Option<&Scene>,
    manifest: Option<&SlideManifest>,
) -> Duration {
    scene
        .and_then(|s| s.duration_override)
        .or_else(|| {
            manifest
                .and_then(|m| m.display_duration_seconds())
                .map(|s| Duration::from_secs(u64::from(s)))
        })
        .unwrap_or(DEFAULT_SLIDE_DURATION)
}

fn load_renderer_from_path(
    path: &str,
    params: Option<&serde_json::Value>,
    ctx: &RenderContext,
) -> Result<PreparedRenderer, String> {
    if path == LOADING_SCENE_PATH {
        let (slide, manifest) = load_wasm_slide_from_bytes(LOADING_SLIDE)
            .map_err(|err| format!("loading slide: {err}"))?;
        return prepare_loaded_slide(
            ctx,
            LoadedSlidePackage { slide, manifest, path: path.to_string() },
        );
    }
    let loaded = load_slide_package_from_path(path, params)?;
    prepare_loaded_slide(ctx, loaded)
}

fn load_slide_package_from_path(
    path: &str,
    params: Option<&serde_json::Value>,
) -> Result<LoadedSlidePackage, String> {
    let params_bytes: Option<Vec<u8>> = params
        .map(|v| serde_json::to_vec(v).expect("params serialization is infallible"));
    let (slide, manifest) = load_wasm_slide(path, params_bytes.as_deref())
        .map_err(|err| format!("slide '{path}' load failed: {err}"))?;
    slide
        .validate()
        .map_err(|err| format!("slide '{path}' failed validation: {err}"))?;

    if let Some(name) = manifest
        .as_ref()
        .and_then(|manifest| manifest.name.as_deref())
    {
        log::info!("loaded slide from path '{path}': {name}");
    }

    Ok(LoadedSlidePackage {
        slide,
        manifest,
        path: path.to_string(),
    })
}

fn prepare_loaded_slide(
    ctx: &RenderContext,
    loaded: LoadedSlidePackage,
) -> Result<PreparedRenderer, String> {
    prepare_renderer(ctx, loaded.slide, loaded.manifest, &loaded.path)
}

struct App {
    scene: Scene,
    slides_dir: Option<String>,
    single_scene: bool,
    render_context: Option<RenderContext>,
    renderers: Vec<PreparedRenderer>,
    transition_renderer: Option<TransitionRenderer>,
    schedule: Vec<Scene>,
    schedule_idx: usize,
    last_switch: Instant,
    transition: TransitionState,
    bootstrap: Option<BootstrapState>,
}

impl App {
    fn preload_schedule(&mut self, ctx: &RenderContext) {
        self.renderers = self
            .schedule
            .iter()
            .map(|scene| {
                load_renderer_from_path(scene.path(), scene.params.as_ref(), ctx)
                    .unwrap_or_else(|err| {
                        panic!("failed to preload slide '{}': {err}", scene.path())
                    })
            })
            .collect();
    }

    fn renderer_manifest(&self, schedule_idx: usize) -> Option<&SlideManifest> {
        self.renderers
            .get(schedule_idx)
            .and_then(|prepared| prepared.manifest.as_ref())
    }

    fn park_renderer(&mut self, schedule_idx: usize) {
        if let Some(prepared) = self.renderers.get_mut(schedule_idx) {
            prepared.renderer.park();
        }
    }

    fn finish_bootstrap(&mut self) {
        let Some(_) = self.bootstrap.take() else {
            return;
        };

        if self.schedule.is_empty() || self.renderers.is_empty() {
            return;
        }

        self.schedule.remove(0);
        self.renderers.remove(0);
        self.schedule_idx = self.schedule_idx.saturating_sub(1);
        if let Some(current_scene) = self.schedule.get(self.schedule_idx).cloned() {
            self.scene = current_scene;
            if let Some(ctx) = &self.render_context {
                ctx.window.set_title(&scene_title(&self.scene));
            }
        }
    }

    fn complete_transition(&mut self, outgoing_idx: usize) {
        self.park_renderer(outgoing_idx);
        if self.bootstrap.as_ref().is_some_and(|bootstrap| {
            outgoing_idx == 0 && self.schedule_idx == bootstrap.target_schedule_idx
        }) {
            self.finish_bootstrap();
        }
        self.transition = TransitionState::Idle;
        self.last_switch = Instant::now();
    }

    fn start_transition_to(&mut self, next_idx: usize) {
        if self.schedule.len() < 2
            || next_idx >= self.schedule.len()
            || next_idx == self.schedule_idx
        {
            self.last_switch = Instant::now();
            return;
        }

        let current_idx = self.schedule_idx;
        let next_scene = self.schedule[next_idx].clone();
        let (kind, duration) = resolve_transition(
            self.schedule.get(current_idx),
            self.renderer_manifest(current_idx),
            self.schedule.get(next_idx),
            self.renderer_manifest(next_idx),
        );

        self.schedule_idx = next_idx;
        self.scene = next_scene;
        self.render_context
            .as_ref()
            .expect("render context missing during transition start")
            .window
            .set_title(&scene_title(&self.scene));

        if kind.uses_compositor() {
            let ctx = self
                .render_context
                .as_ref()
                .expect("render context missing during transition start");
            let transition_renderer = self
                .transition_renderer
                .as_ref()
                .expect("transition renderer missing during transition start");
            self.transition = TransitionState::Blending(ActiveTransition::new(
                ctx,
                transition_renderer,
                kind,
                current_idx,
                duration,
            ));
        } else {
            self.complete_transition(current_idx);
        }
    }

    fn start_transition(&mut self) {
        if self.schedule.len() < 2 {
            self.last_switch = Instant::now();
            return;
        }

        let next_idx = (self.schedule_idx + 1) % self.schedule.len();
        self.start_transition_to(next_idx);
    }

    fn advance_bootstrap(&mut self, rendered_ok: bool) {
        let Some(first_frame_presented) = self
            .bootstrap
            .as_ref()
            .map(|bootstrap| bootstrap.first_frame_presented)
        else {
            return;
        };

        if !first_frame_presented {
            if rendered_ok {
                if let Some(bootstrap) = &mut self.bootstrap {
                    bootstrap.first_frame_presented = true;
                }
            }
            return;
        }

        if !self.transition.is_idle() {
            return;
        }

        let target_schedule_idx = self
            .bootstrap
            .as_ref()
            .map(|bootstrap| bootstrap.target_schedule_idx)
            .expect("bootstrap target missing during startup");

        let mut completed_load = None;
        if let Some(receiver) = self
            .bootstrap
            .as_ref()
            .and_then(|bootstrap| bootstrap.load_receiver.as_ref())
        {
            match receiver.try_recv() {
                Ok(result) => {
                    completed_load = Some(result);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    panic!("bootstrap slide loader thread disconnected unexpectedly");
                }
            }
        }

        if let Some(result) = completed_load {
            if let Some(bootstrap) = &mut self.bootstrap {
                bootstrap.load_receiver = None;
            }
            let loaded = result.unwrap_or_else(|err| {
                panic!("failed to load slide during startup bootstrap: {err}")
            });
            let ctx = self
                .render_context
                .as_ref()
                .expect("render context missing during bootstrap");
            let prepared = prepare_loaded_slide(ctx, loaded).unwrap_or_else(|err| {
                panic!("failed to initialize slide renderer during startup bootstrap: {err}")
            });
            self.renderers.push(prepared);
        }

        if let Some(bootstrap) = &mut self.bootstrap {
            if bootstrap.load_receiver.is_none() && bootstrap.next_load_idx < self.schedule.len() {
                let next_scene = self.schedule[bootstrap.next_load_idx].clone();
                let (tx, rx) = mpsc::channel();
                std::thread::Builder::new()
                    .name(format!("bootstrap-load-{}", bootstrap.next_load_idx))
                    .spawn(move || {
                        let _ = tx.send(load_slide_package_from_path(
                            next_scene.path(),
                            next_scene.params.as_ref(),
                        ));
                    })
                    .expect("failed to spawn bootstrap slide loader thread");
                bootstrap.next_load_idx += 1;
                bootstrap.load_receiver = Some(rx);
            }
        }

        let load_in_flight = self
            .bootstrap
            .as_ref()
            .and_then(|bootstrap| bootstrap.load_receiver.as_ref())
            .is_some();
        if self.renderers.len() == self.schedule.len() && !load_in_flight {
            self.start_transition_to(target_schedule_idx);
        }
    }

    fn render_frame(&mut self) -> Option<Result<(), wgpu::SurfaceError>> {
        let ctx = self.render_context.as_ref()?;
        let mut transition_complete = false;

        let result = match &mut self.transition {
            TransitionState::Idle => self
                .renderers
                .get_mut(self.schedule_idx)
                .map(|prepared| prepared.renderer.render(ctx)),
            TransitionState::Blending(active) => {
                let incoming = self.schedule_idx;
                let transition_renderer = self
                    .transition_renderer
                    .as_ref()
                    .expect("transition renderer missing during transition");
                let (outgoing, incoming) =
                    prepared_pair_mut(&mut self.renderers, active.outgoing_idx(), incoming)
                        .expect("transition slides missing during transition");
                Some(
                    active
                        .render(
                            ctx,
                            &mut outgoing.renderer,
                            &mut incoming.renderer,
                            transition_renderer,
                        )
                        .map(|complete| {
                            transition_complete = complete;
                        }),
                )
            }
        };

        if transition_complete {
            if let TransitionState::Blending(active) = std::mem::take(&mut self.transition) {
                self.complete_transition(active.outgoing_idx());
            }
        }

        result
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.render_context.is_some() {
            return;
        }

        let requested_scene = self.scene.clone();
        let content_schedule = match build_content_schedule(
            &requested_scene,
            self.slides_dir.as_deref(),
            self.single_scene,
        ) {
            Ok(schedule) => schedule,
            Err(err) => {
                log::error!("{err}");
                event_loop.exit();
                return;
            }
        };
        let content_schedule_idx = content_schedule
            .iter()
            .position(|scene| scene == &requested_scene)
            .unwrap_or(0);
        self.last_switch = Instant::now();
        self.transition = TransitionState::Idle;

        let mut display_scene = requested_scene.clone();
        if requested_scene.path() != LOADING_SCENE_PATH {
            display_scene = Scene::new(LOADING_SCENE_PATH);
        }
        self.scene = display_scene;

        let window = std::sync::Arc::new(
            event_loop
                .create_window(build_window_attributes(event_loop, &self.scene))
                .expect("failed to create window"),
        );

        let render_context = pollster::block_on(RenderContext::new(window));
        let transition_renderer = TransitionRenderer::new(&render_context);

        if let Some((bootstrap_schedule, target_schedule_idx)) =
            build_bootstrap_schedule(&requested_scene, &content_schedule)
        {
            match load_renderer_from_path(LOADING_SCENE_PATH, None, &render_context) {
                Ok(loading_renderer) => {
                    self.schedule = bootstrap_schedule;
                    self.schedule_idx = 0;
                    self.renderers = vec![loading_renderer];
                    self.bootstrap = Some(BootstrapState {
                        target_schedule_idx,
                        first_frame_presented: false,
                        next_load_idx: 1,
                        load_receiver: None,
                    });
                }
                Err(err) => {
                    log::error!(
                        "failed to load startup loading slide '{LOADING_SCENE_PATH}': {err}; falling back to eager preload"
                    );
                    self.scene = requested_scene;
                    self.schedule = content_schedule;
                    self.schedule_idx = content_schedule_idx;
                    self.preload_schedule(&render_context);
                    self.bootstrap = None;
                }
            }
        } else {
            self.schedule = content_schedule;
            self.schedule_idx = content_schedule_idx;
            self.preload_schedule(&render_context);
            self.bootstrap = None;
        }

        self.render_context = Some(render_context);
        self.transition_renderer = Some(transition_renderer);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                if let Some(ctx) = &mut self.render_context {
                    ctx.resize_surface(size);
                }
            }

            WindowEvent::RedrawRequested => {
                if self.bootstrap.is_none()
                    && self.schedule.len() > 1
                    && self.last_switch.elapsed()
                        >= current_slide_duration(
                            self.schedule.get(self.schedule_idx),
                            self.renderer_manifest(self.schedule_idx),
                        )
                    && self.transition.is_idle()
                {
                    self.start_transition();
                }

                let render_result = self.render_frame();
                let mut rendered_ok = false;
                if let Some(result) = render_result {
                    match result {
                        Ok(()) => {
                            rendered_ok = true;
                        }
                        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                            if let Some(ctx) = &mut self.render_context {
                                ctx.reconfigure();
                            }
                        }
                        Err(e) => log::error!("render error: {e}"),
                    }
                }
                self.advance_bootstrap(rendered_ok);
                if let Some(ctx) = &self.render_context {
                    ctx.window.request_redraw();
                }
            }

            _ => {}
        }
    }
}

fn main() {
    let command = parse_command().unwrap_or_else(|message| {
        eprintln!("{message}");
        std::process::exit(2);
    });
    init_logging(command_verbose(&command));

    let run = match command {
        Command::Pack {
            source_dir,
            output_path,
            verbose: _,
        } => {
            let report =
                slide_loader::pack_slide_directory(Path::new(&source_dir), Path::new(&output_path))
                    .unwrap_or_else(|err| {
                        eprintln!("pack failed: {err}");
                        std::process::exit(1);
                    });
            println!(
                "Wrote {} (content={} bytes archive={} bytes overhead={:.2}%)",
                report.output_path.display(),
                report.content_bytes,
                report.archive_bytes,
                report.overhead_ratio() * 100.0
            );
            return;
        }
        Command::Run(run) => run,
    };

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App {
        scene: run.scene,
        slides_dir: run.slides_dir,
        single_scene: run.single_scene,
        render_context: None,
        renderers: Vec::new(),
        transition_renderer: None,
        schedule: Vec::new(),
        schedule_idx: 0,
        last_switch: Instant::now(),
        transition: TransitionState::Idle,
        bootstrap: None,
    };
    event_loop.run_app(&mut app).expect("event loop error");
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SLIDE_DURATION, DEFAULT_SLIDES_DIR, DEFAULT_TRANSITION, LOADING_SCENE_PATH, Scene,
        TRANSITION_DURATION, build_bootstrap_schedule, build_content_schedule,
        current_slide_duration, discover_slide_schedule, parse_pack_command, parse_run_config,
        resolve_transition,
    };
    use crate::slide_manifest::{DisplayConfig, SlideManifest};
    use crate::transition::TransitionKind;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn test_dir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join("vzglyd-main-tests");
        fs::create_dir_all(&base).expect("create base temp dir");
        let unique = format!(
            "{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        );
        let path = base.join(unique);
        fs::create_dir_all(&path).expect("create temp test dir");
        path
    }

    fn write_file(path: &Path) {
        fs::write(path, b"test").expect("write temp file");
    }

    #[test]
    fn transition_defaults_when_manifests_do_not_specify_preferences() {
        let (kind, duration) = resolve_transition(None, None, None, None);

        assert_eq!(kind, DEFAULT_TRANSITION);
        assert_eq!(duration, TRANSITION_DURATION);
    }

    #[test]
    fn outgoing_transition_out_takes_priority() {
        let outgoing = SlideManifest {
            display: Some(DisplayConfig {
                duration_seconds: Some(20),
                transition_in: None,
                transition_out: Some("dissolve".into()),
            }),
            ..Default::default()
        };
        let incoming = SlideManifest {
            display: Some(DisplayConfig {
                duration_seconds: Some(20),
                transition_in: Some("wipe_left".into()),
                transition_out: None,
            }),
            ..Default::default()
        };

        let (kind, duration) = resolve_transition(None, Some(&outgoing), None, Some(&incoming));

        assert_eq!(kind, TransitionKind::Dissolve);
        assert_eq!(duration, TRANSITION_DURATION);
    }

    #[test]
    fn incoming_transition_in_is_used_when_outgoing_has_no_preference() {
        let outgoing = SlideManifest::default();
        let incoming = SlideManifest {
            display: Some(DisplayConfig {
                duration_seconds: Some(20),
                transition_in: Some("wipe_down".into()),
                transition_out: None,
            }),
            ..Default::default()
        };

        let (kind, duration) = resolve_transition(None, Some(&outgoing), None, Some(&incoming));

        assert_eq!(kind, TransitionKind::WipeDown);
        assert_eq!(duration, TRANSITION_DURATION);
    }

    #[test]
    fn explicit_scene_argument_enters_single_scene_mode() {
        let args = vec!["--scene".to_string(), "/tmp/news.vzglyd".to_string()];
        let run = parse_run_config(&args);

        assert_eq!(run.scene.path(), "/tmp/news.vzglyd");
        assert!(run.slides_dir.is_none());
        assert!(run.single_scene);
        assert!(!run.verbose);
    }

    #[test]
    fn default_run_uses_slides_directory() {
        let run = parse_run_config(&[]);

        assert_eq!(run.scene.path(), DEFAULT_SLIDES_DIR);
        assert!(run.slides_dir.is_none());
        assert!(!run.single_scene);
        assert!(!run.verbose);
    }

    #[test]
    fn slides_dir_argument_enables_directory_schedule_mode() {
        let args = vec![
            "--slides-dir".to_string(),
            "/var/lib/vzglyd/slides".to_string(),
        ];
        let run = parse_run_config(&args);

        assert_eq!(run.scene.path(), "/var/lib/vzglyd/slides");
        assert_eq!(run.slides_dir.as_deref(), Some("/var/lib/vzglyd/slides"));
        assert!(!run.single_scene);
    }

    fn two_scene_schedule() -> Vec<Scene> {
        vec![Scene::new("a.vzglyd"), Scene::new("b.vzglyd")]
    }

    #[test]
    fn bootstrap_schedule_prepends_loading_slide() {
        let scene = Scene::new("a.vzglyd");
        let content = two_scene_schedule();
        let (schedule, target_idx) = build_bootstrap_schedule(&scene, &content)
            .expect("loading bootstrap should be enabled");
        let paths: Vec<&str> = schedule.iter().map(|s| s.path()).collect();

        assert_eq!(paths[0], LOADING_SCENE_PATH);
        assert_eq!(paths[1], "a.vzglyd");
        assert_eq!(target_idx, 1);
    }

    #[test]
    fn bootstrap_schedule_keeps_custom_scene_as_first_content_scene() {
        let root = test_dir("bootstrap-custom");
        let vzglyd_path = root.join("custom.vzglyd");
        fs::write(&vzglyd_path, b"placeholder").expect("write");
        let path_str = vzglyd_path.to_string_lossy().into_owned();

        let scene = Scene::new(&path_str);
        let content = build_content_schedule(&scene, Some(root.to_str().unwrap()), false)
            .expect("build custom schedule");
        let (schedule, target_idx) = build_bootstrap_schedule(&scene, &content)
            .expect("loading bootstrap should be enabled");
        let paths: Vec<&str> = schedule.iter().map(|s| s.path()).collect();

        assert_eq!(paths[0], LOADING_SCENE_PATH);
        assert!(paths[1].ends_with("custom.vzglyd"));
        assert_eq!(target_idx, 1);
    }

    #[test]
    fn bootstrap_schedule_is_disabled_for_loading_slide_itself() {
        let scene = Scene::new(LOADING_SCENE_PATH);
        let content = two_scene_schedule();

        assert!(build_bootstrap_schedule(&scene, &content).is_none());
    }

    #[test]
    fn discover_slide_schedule_picks_up_archives_and_packaged_directories() {
        let root = test_dir("slides-dir");
        let packaged_dir = root.join("weather");
        fs::create_dir_all(&packaged_dir).expect("create packaged dir");
        write_file(&packaged_dir.join("manifest.json"));
        write_file(&packaged_dir.join("slide.wasm"));
        write_file(&root.join("clock-0.1.0.vzglyd"));
        fs::create_dir_all(root.join("ignored")).expect("create ignored dir");

        let schedule = discover_slide_schedule(root.to_str().expect("temp path should be utf-8"))
            .expect("discover slide schedule");
        let paths: Vec<&str> = schedule.iter().map(|scene| scene.path()).collect();

        assert_eq!(paths.len(), 2);
        assert!(paths.iter().any(|path| path.ends_with("clock-0.1.0.vzglyd")));
        assert!(paths.iter().any(|path| path.ends_with("weather")));
    }

    #[test]
    fn discover_slide_schedule_errors_when_directory_is_empty() {
        let root = test_dir("slides-dir-empty");
        let error = discover_slide_schedule(root.to_str().expect("temp path should be utf-8"))
            .expect_err("empty slides directory should error");

        assert!(error.contains("contains no .vzglyd archives or slide packages"));
    }

    #[test]
    fn default_slide_duration_is_seven_seconds() {
        assert_eq!(DEFAULT_SLIDE_DURATION, Duration::from_secs(7));
    }

    #[test]
    fn verbose_run_argument_is_recognized() {
        let args = vec![
            "--verbose".to_string(),
            "--scene".to_string(),
            "/tmp/news.vzglyd".to_string(),
        ];
        let run = parse_run_config(&args);

        assert_eq!(run.scene.path(), "/tmp/news.vzglyd");
        assert!(run.slides_dir.is_none());
        assert!(run.single_scene);
        assert!(run.verbose);
    }

    #[test]
    fn verbose_run_argument_is_recognized_after_scene() {
        let args = vec![
            "--scene".to_string(),
            "/tmp/news.vzglyd".to_string(),
            "--verbose".to_string(),
        ];
        let run = parse_run_config(&args);

        assert_eq!(run.scene.path(), "/tmp/news.vzglyd");
        assert!(run.slides_dir.is_none());
        assert!(run.single_scene);
        assert!(run.verbose);
    }

    #[test]
    fn verbose_pack_argument_is_recognized() {
        let args = vec![
            "slides/news".to_string(),
            "-o".to_string(),
            "/tmp/news.vzglyd".to_string(),
            "--verbose".to_string(),
        ];
        let command = parse_pack_command(&args).expect("pack args should parse");
        let super::Command::Pack {
            source_dir,
            output_path,
            verbose,
        } = command
        else {
            panic!("expected pack command");
        };

        assert_eq!(source_dir, "slides/news");
        assert_eq!(output_path, "/tmp/news.vzglyd");
        assert!(verbose);
    }

    // ── Playlist-aware discovery ──────────────────────────────────────────────

    fn write_playlist(dir: &Path, json: &str) {
        fs::write(dir.join(crate::playlist::PLAYLIST_FILENAME), json).expect("write playlist");
    }

    #[test]
    fn playlist_json_orders_slides_by_declaration_not_alphabet() {
        let root = test_dir("playlist-order");
        write_file(&root.join("z.vzglyd"));
        write_file(&root.join("a.vzglyd"));
        write_file(&root.join("m.vzglyd"));
        write_playlist(
            &root,
            r#"{"slides":[{"path":"m.vzglyd"},{"path":"a.vzglyd"},{"path":"z.vzglyd"}]}"#,
        );

        let schedule =
            discover_slide_schedule(root.to_str().unwrap()).expect("discover with playlist");
        let names: Vec<_> = schedule
            .iter()
            .map(|s| Path::new(s.path()).file_name().unwrap().to_str().unwrap())
            .collect();

        assert_eq!(names, ["m.vzglyd", "a.vzglyd", "z.vzglyd"]);
    }

    #[test]
    fn playlist_json_disabled_slides_are_excluded() {
        let root = test_dir("playlist-disabled");
        write_file(&root.join("a.vzglyd"));
        write_file(&root.join("b.vzglyd"));
        write_playlist(
            &root,
            r#"{"slides":[{"path":"a.vzglyd"},{"path":"b.vzglyd","enabled":false}]}"#,
        );

        let schedule = discover_slide_schedule(root.to_str().unwrap()).expect("discover");
        assert_eq!(schedule.len(), 1);
        assert!(schedule[0].path().ends_with("a.vzglyd"));
    }

    #[test]
    fn playlist_json_duration_override_is_stored_on_scene() {
        let root = test_dir("playlist-duration");
        write_file(&root.join("a.vzglyd"));
        write_playlist(&root, r#"{"slides":[{"path":"a.vzglyd","duration_seconds":20}]}"#);

        let schedule = discover_slide_schedule(root.to_str().unwrap()).expect("discover");
        assert_eq!(schedule[0].duration_override, Some(Duration::from_secs(20)));
    }

    #[test]
    fn playlist_json_defaults_apply_when_entry_has_no_duration() {
        let root = test_dir("playlist-defaults");
        write_file(&root.join("a.vzglyd"));
        write_playlist(
            &root,
            r#"{"defaults":{"duration_seconds":10},"slides":[{"path":"a.vzglyd"}]}"#,
        );

        let schedule = discover_slide_schedule(root.to_str().unwrap()).expect("discover");
        assert_eq!(schedule[0].duration_override, Some(Duration::from_secs(10)));
    }

    #[test]
    fn playlist_json_entry_overrides_default_duration() {
        let root = test_dir("playlist-entry-override");
        write_file(&root.join("a.vzglyd"));
        write_playlist(
            &root,
            r#"{"defaults":{"duration_seconds":10},"slides":[{"path":"a.vzglyd","duration_seconds":25}]}"#,
        );

        let schedule = discover_slide_schedule(root.to_str().unwrap()).expect("discover");
        assert_eq!(schedule[0].duration_override, Some(Duration::from_secs(25)));
    }

    #[test]
    fn discover_falls_back_to_alphabetical_without_playlist() {
        let root = test_dir("playlist-fallback");
        write_file(&root.join("b.vzglyd"));
        write_file(&root.join("a.vzglyd"));
        // No playlist.json written

        let schedule = discover_slide_schedule(root.to_str().unwrap()).expect("discover");
        let names: Vec<_> = schedule
            .iter()
            .map(|s| Path::new(s.path()).file_name().unwrap().to_str().unwrap())
            .collect();

        assert_eq!(names, ["a.vzglyd", "b.vzglyd"]);
    }

    // ── current_slide_duration ────────────────────────────────────────────────

    fn manifest_with_duration(secs: u32) -> SlideManifest {
        SlideManifest {
            display: Some(DisplayConfig {
                duration_seconds: Some(secs),
                transition_in: None,
                transition_out: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn current_slide_duration_uses_scene_override_over_manifest() {
        let scene = Scene::new_with_overrides("a.vzglyd", Some(Duration::from_secs(20)), None, None);
        let manifest = manifest_with_duration(7);

        assert_eq!(
            current_slide_duration(Some(&scene), Some(&manifest)),
            Duration::from_secs(20)
        );
    }

    #[test]
    fn current_slide_duration_falls_back_to_manifest_when_no_override() {
        let scene = Scene::new("a.vzglyd");
        let manifest = manifest_with_duration(12);

        assert_eq!(
            current_slide_duration(Some(&scene), Some(&manifest)),
            Duration::from_secs(12)
        );
    }

    #[test]
    fn current_slide_duration_uses_default_when_neither_set() {
        let scene = Scene::new("a.vzglyd");

        assert_eq!(
            current_slide_duration(Some(&scene), None),
            DEFAULT_SLIDE_DURATION
        );
    }

    // ── resolve_transition with scene overrides ───────────────────────────────

    #[test]
    fn resolve_transition_scene_override_wins_over_manifest() {
        let outgoing_scene =
            Scene::new_with_overrides("a.vzglyd", None, None, Some(TransitionKind::Cut));
        let outgoing_manifest = SlideManifest {
            display: Some(DisplayConfig {
                duration_seconds: None,
                transition_in: None,
                transition_out: Some("crossfade".into()),
            }),
            ..Default::default()
        };

        let (kind, _) =
            resolve_transition(Some(&outgoing_scene), Some(&outgoing_manifest), None, None);

        assert_eq!(kind, TransitionKind::Cut);
    }
}
