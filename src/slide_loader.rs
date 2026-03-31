use glam::{Mat3, Mat4, Vec3};
use image::ImageReader;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map as JsonMap, Value as JsonValue};
use vzglyd_slide::{
    CameraKeyframe, CameraPath, DirectionalLight, DrawSource, DrawSpec, FilterMode, Limits,
    MeshAsset, MeshAssetVertex, PipelineKind, RuntimeMeshSet, RuntimeOverlay, SceneAnchor,
    SceneAnchorSet, SceneSpace, ScreenVertex, ShaderSources, SlideSpec, StaticMesh, TextureDesc,
    TextureFormat, WorldLighting, WorldVertex, WrapMode, make_font_atlas,
};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use wasmtime::{Config, Engine, Instance, Memory, Module, Store, Trap, TypedFunc};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::slide_manifest::{AssetRef, SceneAssetRef, SlideManifest};

pub const ABI_VERSION: u32 = vzglyd_slide::ABI_VERSION;
pub(crate) const SLIDE_UPDATE_NO_CHANGE: i32 = 0;
pub(crate) const SLIDE_UPDATE_MESHES_UPDATED: i32 = 1;
pub(crate) const PACKAGE_MANIFEST_NAME: &str = "manifest.json";
pub(crate) const PACKAGE_WASM_NAME: &str = "slide.wasm";
pub(crate) const PACKAGE_SIDECAR_NAME: &str = "sidecar.wasm";
pub(crate) const PACKAGE_ARCHIVE_EXTENSION: &str = "vzglyd";
const ARCHIVE_CACHE_DIR_NAME: &str = "vzglyd_archive_cache";
const TEARDOWN_TIMEOUT: Duration = Duration::from_millis(100);
const HOST_ERROR: i32 = -1;
const HOST_BUFFER_TOO_SMALL: i32 = -2;
const HOST_CHANNEL_EMPTY: i32 = -3;
const HOST_ASSET_NOT_FOUND: i32 = -4;
const WASI_ERRNO_SUCCESS: i32 = 0;
const WASI_ERRNO_BADF: i32 = 8;
const WASI_ERRNO_CONNREFUSED: i32 = 14;
const WASI_ERRNO_FAULT: i32 = 21;
const WASI_ERRNO_INVAL: i32 = 28;
const WASI_ERRNO_IO: i32 = 29;

#[derive(Clone, Default)]
struct HostMeshAssetCatalog {
    encoded_by_key: Arc<HashMap<String, Vec<u8>>>,
}

#[derive(Clone, Default)]
struct HostSceneMetadataCatalog {
    encoded_by_key: Arc<HashMap<String, Vec<u8>>>,
}

struct SlideMailboxState {
    latest: Option<Vec<u8>>,
    dirty: bool,
}

struct SlideMailbox {
    state: Mutex<SlideMailboxState>,
    active: AtomicBool,
}

impl SlideMailbox {
    fn new() -> Self {
        Self {
            state: Mutex::new(SlideMailboxState {
                latest: None,
                dirty: false,
            }),
            active: AtomicBool::new(false),
        }
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Release);
    }

    fn push_latest(&self, bytes: Vec<u8>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.latest = Some(bytes);
        state.dirty = true;
    }
}

/// A byte-message mailbox shared between a sidecar WASM thread and the main slide.
/// The sidecar overwrites the latest payload; the main slide consumes it once.
type SlideChannel = Arc<SlideMailbox>;

struct SlideStore {
    wasi: wasmtime_wasi::WasiCtx,
    rx: SlideChannel,
    label: String,
    mesh_assets: HostMeshAssetCatalog,
    scene_metadata: HostSceneMetadataCatalog,
}

struct SocketTable {
    next_fd: i32,
    sockets: HashMap<i32, std::net::TcpStream>,
}

impl SocketTable {
    fn new() -> Self {
        Self {
            next_fd: 100,
            sockets: HashMap::new(),
        }
    }

    fn reserve(&mut self) -> i32 {
        let fd = self.next_fd;
        self.next_fd += 1;
        fd
    }

    fn is_reserved(&self, fd: i32) -> bool {
        fd >= 100 && fd < self.next_fd
    }

    fn insert_with_fd(&mut self, fd: i32, stream: std::net::TcpStream) {
        self.sockets.insert(fd, stream);
    }

    fn get_mut(&mut self, fd: i32) -> Option<&mut std::net::TcpStream> {
        self.sockets.get_mut(&fd)
    }

    fn remove(&mut self, fd: i32) -> Option<std::net::TcpStream> {
        self.sockets.remove(&fd)
    }
}

struct SidecarStore {
    wasi: wasmtime_wasi::WasiCtx,
    sockets: SocketTable,
    tx: SlideChannel,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlideEntryPaths {
    pub package_root: PathBuf,
    pub manifest_path: PathBuf,
    pub wasm_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("missing manifest at {0}")]
    MissingManifest(String),
    #[error("missing wasm at {0}")]
    MissingWasm(String),
    #[error("manifest parse failed: {0}")]
    ManifestParse(String),
    #[error("manifest validation failed: {0}")]
    ManifestValidation(String),
    #[error("asset load failed: {0}")]
    AssetLoad(String),
    #[error("archive error: {0}")]
    Archive(String),
    #[error("wasm load failed: {0}")]
    WasmLoad(String),
    #[error("wasm missing export: {0}")]
    MissingExport(&'static str),
    #[error("wasm abi version mismatch: found {found}, expected {expected}")]
    AbiVersion { found: u32, expected: u32 },
    #[error("spec decode failed: {0}")]
    SpecDecode(String),
}

pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ImportedVertex {
    pub(crate) position: [f32; 3],
    pub(crate) normal: Option<[f32; 3]>,
    pub(crate) tex_coords: Option<[f32; 2]>,
    pub(crate) color: Option<[f32; 4]>,
}

struct ImportedMesh {
    vertices: Vec<ImportedVertex>,
    indices: Vec<u16>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ImportedExtras {
    pub raw: JsonMap<String, JsonValue>,
    pub vzglyd_id: Option<String>,
    pub vzglyd_pipeline: Option<String>,
    pub vzglyd_material: Option<String>,
    pub vzglyd_anchor: Option<String>,
    pub vzglyd_anchor_tagged: bool,
    pub vzglyd_hidden: bool,
    pub vzglyd_billboard: bool,
    pub vzglyd_entry_camera: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImportedSceneMetadata {
    pub scene_name: Option<String>,
    pub extras: ImportedExtras,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImportedSceneMaterial {
    pub name: Option<String>,
    pub base_color_factor: [f32; 4],
    pub class_hint: Option<String>,
    pub metadata: ImportedExtras,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImportedSceneMeshNode {
    pub id: String,
    pub label: String,
    pub node_name: Option<String>,
    pub mesh_name: Option<String>,
    pub node_index: usize,
    pub primitive_index: usize,
    pub world_transform: [[f32; 4]; 4],
    pub vertices: Vec<ImportedVertex>,
    pub indices: Vec<u16>,
    pub material: ImportedSceneMaterial,
    pub metadata: ImportedExtras,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImportedCameraProjection {
    Perspective {
        aspect_ratio: Option<f32>,
        yfov_rad: f32,
        znear: f32,
        zfar: Option<f32>,
    },
    Orthographic {
        xmag: f32,
        ymag: f32,
        znear: f32,
        zfar: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImportedSceneCamera {
    pub id: String,
    pub label: String,
    pub node_name: Option<String>,
    pub camera_name: Option<String>,
    pub node_index: usize,
    pub world_transform: [[f32; 4]; 4],
    pub projection: ImportedCameraProjection,
    pub metadata: ImportedExtras,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImportedSceneAnchor {
    pub id: String,
    pub label: String,
    pub node_name: Option<String>,
    pub node_index: usize,
    pub world_transform: [[f32; 4]; 4],
    pub metadata: ImportedExtras,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImportedSceneDirectionalLight {
    pub id: String,
    pub label: String,
    pub node_name: Option<String>,
    pub light_name: Option<String>,
    pub node_index: usize,
    pub world_transform: [[f32; 4]; 4],
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub metadata: ImportedExtras,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImportedScene {
    pub id: String,
    pub source_path: PathBuf,
    pub label: Option<String>,
    pub entry_camera: Option<String>,
    pub compile_profile: Option<String>,
    pub metadata: ImportedSceneMetadata,
    pub mesh_nodes: Vec<ImportedSceneMeshNode>,
    pub cameras: Vec<ImportedSceneCamera>,
    pub anchors: Vec<ImportedSceneAnchor>,
    pub directional_lights: Vec<ImportedSceneDirectionalLight>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SceneMaterialClass {
    Opaque,
    AlphaTest,
    Transparent,
    Emissive,
    Water,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShaderSourceHint {
    DefaultWorldScene,
}

pub(crate) struct ScreenBackgroundScene {
    pub spec: SlideSpec<WorldVertex>,
    pub shader_source_hint: Option<ShaderSourceHint>,
}

pub(crate) trait PackageMeshVertex: Serialize + DeserializeOwned + bytemuck::Pod {
    fn from_imported(imported: ImportedVertex, fallback: Option<&Self>) -> Self;

    fn from_scene_import(
        imported: ImportedVertex,
        material_class: SceneMaterialClass,
        fallback: Option<&Self>,
    ) -> Self {
        let _ = material_class;
        Self::from_imported(imported, fallback)
    }
}

impl PackageMeshVertex for WorldVertex {
    fn from_imported(imported: ImportedVertex, fallback: Option<&Self>) -> Self {
        let fallback_color = fallback
            .map(|vertex| vertex.color)
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let fallback_mode = fallback.map(|vertex| vertex.mode).unwrap_or(0.0);
        Self {
            position: imported.position,
            normal: imported.normal.unwrap_or([0.0, 1.0, 0.0]),
            color: imported.color.unwrap_or(fallback_color),
            mode: fallback_mode,
        }
    }

    fn from_scene_import(
        imported: ImportedVertex,
        material_class: SceneMaterialClass,
        fallback: Option<&Self>,
    ) -> Self {
        let fallback_color = fallback
            .map(|vertex| vertex.color)
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);
        Self {
            position: imported.position,
            normal: imported.normal.unwrap_or([0.0, 1.0, 0.0]),
            color: imported.color.unwrap_or(fallback_color),
            mode: scene_material_mode(material_class),
        }
    }
}

impl PackageMeshVertex for ScreenVertex {
    fn from_imported(imported: ImportedVertex, fallback: Option<&Self>) -> Self {
        let fallback_uv = fallback
            .map(|vertex| vertex.tex_coords)
            .unwrap_or([0.0, 0.0]);
        let fallback_color = fallback
            .map(|vertex| vertex.color)
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let fallback_mode = fallback.map(|vertex| vertex.mode).unwrap_or(0.0);
        Self {
            position: imported.position,
            tex_coords: imported.tex_coords.unwrap_or(fallback_uv),
            color: imported.color.unwrap_or(fallback_color),
            mode: fallback_mode,
        }
    }
}

pub struct PackReport {
    pub output_path: PathBuf,
    pub content_bytes: u64,
    pub archive_bytes: u64,
}

pub(crate) struct LoadedSpec<V: bytemuck::Pod> {
    pub spec: SlideSpec<V>,
    pub runtime: Option<SlideRuntime>,
    pub shader_source_hint: Option<ShaderSourceHint>,
    pub screen_background_scene: Option<ScreenBackgroundScene>,
}

pub(crate) struct SlideRuntime {
    store: RuntimeStore,
    #[allow(dead_code)]
    instance: Instance,
    memory: Memory,
    update_fn: Option<RuntimeUpdateFunc>,
    teardown_fn: Option<RuntimeI32Func0>,
    overlay_ptr: Option<RuntimeI32Func0>,
    overlay_len: Option<RuntimeI32Func0>,
    dynamic_meshes_ptr: Option<RuntimeI32Func0>,
    dynamic_meshes_len: Option<RuntimeI32Func0>,
    #[allow(dead_code)]
    sidecar: Option<SidecarHandle>,
}

type RuntimeStore = Store<SlideStore>;
type RuntimeUpdateFunc = TypedFunc<f32, i32>;
type RuntimeI32Func0 = TypedFunc<(), i32>;

struct SidecarHandle {
    _thread: std::thread::JoinHandle<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TeardownOutcome {
    MissingExport,
    Completed(i32),
    TimedOut,
}

impl PackReport {
    pub fn overhead_ratio(&self) -> f64 {
        if self.content_bytes == 0 {
            0.0
        } else {
            self.archive_bytes.saturating_sub(self.content_bytes) as f64 / self.content_bytes as f64
        }
    }
}

impl SlideRuntime {
    fn new(
        store: Store<SlideStore>,
        instance: Instance,
        memory: Memory,
        update_fn: Option<TypedFunc<f32, i32>>,
        teardown_fn: Option<TypedFunc<(), i32>>,
        overlay_ptr: Option<TypedFunc<(), i32>>,
        overlay_len: Option<TypedFunc<(), i32>>,
        dynamic_meshes_ptr: Option<TypedFunc<(), i32>>,
        dynamic_meshes_len: Option<TypedFunc<(), i32>>,
    ) -> Self {
        Self {
            store,
            instance,
            memory,
            update_fn,
            teardown_fn,
            overlay_ptr,
            overlay_len,
            dynamic_meshes_ptr,
            dynamic_meshes_len,
            sidecar: None,
        }
    }

    fn attach_sidecar(&mut self, sidecar: SidecarHandle) {
        log::info!("slide:{} attached sidecar thread", self.store.data().label);
        self.sidecar = Some(sidecar);
    }

    pub(crate) fn set_active(&mut self, active: bool) {
        let was_active = self.store.data().rx.is_active();
        if was_active != active {
            log::info!("slide:{} active={active}", self.store.data().label);
            self.store.data().rx.set_active(active);
        }
    }

    pub(crate) fn update(&mut self, dt: f32) -> Result<i32, LoadError> {
        let Some(update_fn) = self.update_fn.as_ref() else {
            return Ok(SLIDE_UPDATE_NO_CHANGE);
        };

        update_fn
            .call(&mut self.store, dt)
            .map_err(|error| LoadError::WasmLoad(error.to_string()))
    }

    pub(crate) fn has_overlay(&self) -> bool {
        self.overlay_ptr.is_some() && self.overlay_len.is_some()
    }

    pub(crate) fn has_dynamic_meshes(&self) -> bool {
        self.dynamic_meshes_ptr.is_some() && self.dynamic_meshes_len.is_some()
    }

    fn teardown(&mut self, timeout: Duration) -> Result<TeardownOutcome, LoadError> {
        let Some(teardown_fn) = self.teardown_fn.as_ref() else {
            return Ok(TeardownOutcome::MissingExport);
        };

        call_teardown_with_timeout(&mut self.store, teardown_fn, timeout)
    }

    pub(crate) fn read_overlay<V>(&mut self) -> Result<Option<RuntimeOverlay<V>>, LoadError>
    where
        V: bytemuck::Pod + DeserializeOwned,
    {
        let (Some(overlay_ptr), Some(overlay_len)) =
            (self.overlay_ptr.as_ref(), self.overlay_len.as_ref())
        else {
            return Ok(None);
        };

        read_overlay_from_store(&mut self.store, &self.memory, overlay_ptr, overlay_len)
    }

    pub(crate) fn read_dynamic_meshes<V>(&mut self) -> Result<Option<RuntimeMeshSet<V>>, LoadError>
    where
        V: bytemuck::Pod + DeserializeOwned,
    {
        let (Some(meshes_ptr), Some(meshes_len)) = (
            self.dynamic_meshes_ptr.as_ref(),
            self.dynamic_meshes_len.as_ref(),
        ) else {
            return Ok(None);
        };

        read_dynamic_meshes_from_store(&mut self.store, &self.memory, meshes_ptr, meshes_len)
    }

    #[cfg(test)]
    fn memory_byte(&mut self, offset: usize) -> Option<u8> {
        self.memory.data(&self.store).get(offset).copied()
    }
}

impl Drop for SlideRuntime {
    fn drop(&mut self) {
        match self.teardown(TEARDOWN_TIMEOUT) {
            Ok(TeardownOutcome::MissingExport) | Ok(TeardownOutcome::Completed(_)) => {}
            Ok(TeardownOutcome::TimedOut) => log::warn!(
                "slide teardown exceeded {}ms; dropping runtime anyway",
                TEARDOWN_TIMEOUT.as_millis()
            ),
            Err(error) => log::warn!("slide teardown failed: {error}"),
        }
    }
}

fn call_teardown_with_timeout<T>(
    store: &mut Store<T>,
    func: &TypedFunc<(), i32>,
    timeout: Duration,
) -> Result<TeardownOutcome, LoadError> {
    store.epoch_deadline_trap();
    store.set_epoch_deadline(1);
    arm_teardown_timeout(store.engine().clone(), timeout)?;

    match func.call(store, ()) {
        Ok(status) => Ok(TeardownOutcome::Completed(status)),
        Err(error) if error.downcast_ref::<Trap>() == Some(&Trap::Interrupt) => {
            Ok(TeardownOutcome::TimedOut)
        }
        Err(error) => Err(LoadError::WasmLoad(error.to_string())),
    }
}

fn arm_teardown_timeout(engine: Engine, timeout: Duration) -> Result<(), LoadError> {
    std::thread::Builder::new()
        .name("vzglyd-slide-teardown-timeout".into())
        .spawn(move || {
            std::thread::sleep(timeout);
            engine.increment_epoch();
        })
        .map(|_| ())
        .map_err(|error| LoadError::WasmLoad(format!("failed to arm teardown timeout: {error}")))
}

fn make_wasm_engine() -> Result<Engine, LoadError> {
    let mut config = Config::new();
    config.epoch_interruption(true);
    Engine::new(&config).map_err(|error| LoadError::WasmLoad(error.to_string()))
}

fn configure_runtime_store<T>(store: &mut Store<T>) {
    // The engine only advances its epoch when we intentionally arm a teardown
    // timeout, so a single tick keeps normal execution unrestricted while still
    // allowing teardown to replace the deadline with a near-term timeout.
    store.set_epoch_deadline(1);
}

trait HostAssetAccess {
    fn mesh_assets(&self) -> &HostMeshAssetCatalog;
    fn scene_metadata(&self) -> &HostSceneMetadataCatalog;
}

impl HostAssetAccess for SlideStore {
    fn mesh_assets(&self) -> &HostMeshAssetCatalog {
        &self.mesh_assets
    }

    fn scene_metadata(&self) -> &HostSceneMetadataCatalog {
        &self.scene_metadata
    }
}

fn read_host_string<T>(
    caller: &mut wasmtime::Caller<'_, T>,
    ptr: i32,
    len: i32,
) -> Result<String, i32> {
    if ptr < 0 || len < 0 {
        return Err(HOST_ERROR);
    }

    let Some(memory) = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
    else {
        return Err(HOST_ERROR);
    };
    let start = ptr as usize;
    let end = start.saturating_add(len as usize);
    let Some(bytes) = memory.data(&mut *caller).get(start..end) else {
        return Err(HOST_ERROR);
    };
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| HOST_ERROR)
}

fn preview_bytes(bytes: &[u8], max_chars: usize) -> String {
    let mut preview = String::from_utf8_lossy(bytes).into_owned();
    preview = preview.replace(['\n', '\r', '\t'], " ");
    preview = preview.split_whitespace().collect::<Vec<_>>().join(" ");
    let total_chars = preview.chars().count();
    if total_chars <= max_chars {
        return preview;
    }

    let truncated = preview.chars().take(max_chars).collect::<String>();
    format!("{truncated}...")
}

fn host_channel_poll(
    caller: &mut wasmtime::Caller<'_, SlideStore>,
    buf_ptr: i32,
    buf_len: i32,
) -> i32 {
    if buf_ptr < 0 || buf_len < 0 {
        return HOST_ERROR;
    }

    let buf_ptr = buf_ptr as usize;
    let buf_len = buf_len as usize;
    let rx = Arc::clone(&caller.data().rx);
    let mut state = rx
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let msg_len = {
        let Some(msg) = state.latest.as_ref() else {
            return HOST_CHANNEL_EMPTY;
        };
        if !state.dirty {
            return HOST_CHANNEL_EMPTY;
        }
        if msg.len() > buf_len {
            return HOST_BUFFER_TOO_SMALL;
        }

        let Some(memory) = caller
            .get_export("memory")
            .and_then(|export| export.into_memory())
        else {
            return HOST_ERROR;
        };

        if memory.write(&mut *caller, buf_ptr, msg).is_err() {
            return HOST_ERROR;
        }

        log::info!(
            "slide:{} consumed {} sidecar bytes: {}",
            caller.data().label,
            msg.len(),
            preview_bytes(msg, 160)
        );
        msg.len() as i32
    };
    state.dirty = false;
    msg_len
}

fn host_slide_log_info(caller: &mut wasmtime::Caller<'_, SlideStore>, ptr: i32, len: i32) -> i32 {
    match read_host_string(caller, ptr, len) {
        Ok(message) => {
            log::info!("slide:{} {message}", caller.data().label);
            0
        }
        Err(status) => status,
    }
}

fn host_sidecar_log_info(
    caller: &mut wasmtime::Caller<'_, SidecarStore>,
    ptr: i32,
    len: i32,
) -> i32 {
    match read_host_string(caller, ptr, len) {
        Ok(message) => {
            log::info!("sidecar:{} {message}", caller.data().label);
            0
        }
        Err(status) => status,
    }
}

fn caller_memory<T>(caller: &mut wasmtime::Caller<'_, T>) -> Result<Memory, i32> {
    caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
        .ok_or(WASI_ERRNO_FAULT)
}

fn guest_range(ptr: i32, len: i32) -> Result<(usize, usize), i32> {
    if ptr < 0 || len < 0 {
        return Err(WASI_ERRNO_INVAL);
    }
    let start = ptr as usize;
    let len = len as usize;
    start
        .checked_add(len)
        .map(|end| (start, end))
        .ok_or(WASI_ERRNO_INVAL)
}

fn write_guest_bytes<T>(
    caller: &mut wasmtime::Caller<'_, T>,
    ptr: i32,
    bytes: &[u8],
) -> Result<(), i32> {
    let (start, _) = guest_range(ptr, bytes.len() as i32)?;
    caller_memory(caller)?
        .write(&mut *caller, start, bytes)
        .map_err(|_| WASI_ERRNO_FAULT)
}

fn write_guest_u32<T>(
    caller: &mut wasmtime::Caller<'_, T>,
    ptr: i32,
    value: u32,
) -> Result<(), i32> {
    write_guest_bytes(caller, ptr, &value.to_le_bytes())
}

fn write_guest_u16<T>(
    caller: &mut wasmtime::Caller<'_, T>,
    ptr: i32,
    value: u16,
) -> Result<(), i32> {
    write_guest_bytes(caller, ptr, &value.to_le_bytes())
}

fn read_guest_bytes<T>(
    caller: &mut wasmtime::Caller<'_, T>,
    ptr: i32,
    len: i32,
) -> Result<Vec<u8>, i32> {
    let (start, end) = guest_range(ptr, len)?;
    let memory = caller_memory(caller)?;
    let data = memory.data(&*caller);
    data.get(start..end)
        .map(|slice| slice.to_vec())
        .ok_or(WASI_ERRNO_FAULT)
}

fn read_iovecs<T>(
    caller: &mut wasmtime::Caller<'_, T>,
    ptr: i32,
    len: i32,
) -> Result<Vec<(usize, usize)>, i32> {
    let (start, end) = guest_range(ptr, len.checked_mul(8).ok_or(WASI_ERRNO_INVAL)?)?;
    let memory = caller_memory(caller)?;
    let data = memory.data(&*caller);
    let raw = data.get(start..end).ok_or(WASI_ERRNO_FAULT)?;
    let mut regions = Vec::with_capacity(len as usize);
    for chunk in raw.chunks_exact(8) {
        let base_ptr =
            u32::from_le_bytes(chunk[0..4].try_into().map_err(|_| WASI_ERRNO_FAULT)?) as usize;
        let base_len =
            u32::from_le_bytes(chunk[4..8].try_into().map_err(|_| WASI_ERRNO_FAULT)?) as usize;
        let end = base_ptr.checked_add(base_len).ok_or(WASI_ERRNO_INVAL)?;
        if data.get(base_ptr..end).is_none() {
            return Err(WASI_ERRNO_FAULT);
        }
        regions.push((base_ptr, base_len));
    }
    Ok(regions)
}

fn register_tcp_extension(linker: &mut wasmtime::Linker<SidecarStore>) -> Result<(), LoadError> {
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "sock_open",
            |mut caller: wasmtime::Caller<'_, SidecarStore>,
             _af: i32,
             _socktype: i32,
             _proto: i32,
             fd_out_ptr: i32|
             -> i32 {
                let fd = caller.data_mut().sockets.reserve() as u32;
                match write_guest_u32(&mut caller, fd_out_ptr, fd) {
                    Ok(()) => WASI_ERRNO_SUCCESS,
                    Err(errno) => errno,
                }
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "sock_connect",
            |mut caller: wasmtime::Caller<'_, SidecarStore>,
             fd: i32,
             addr_ptr: i32,
             addr_len: i32|
             -> i32 {
                if !caller.data().sockets.is_reserved(fd)
                    || caller.data().sockets.sockets.contains_key(&fd)
                {
                    return WASI_ERRNO_BADF;
                }

                let bytes = match read_guest_bytes(&mut caller, addr_ptr, addr_len) {
                    Ok(bytes) => bytes,
                    Err(errno) => return errno,
                };
                if bytes.len() < 8 {
                    return WASI_ERRNO_INVAL;
                }

                let port = u16::from_be_bytes([bytes[2], bytes[3]]);
                let ip = std::net::Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7]);
                let addr = std::net::SocketAddr::new(std::net::IpAddr::V4(ip), port);

                match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
                    Ok(stream) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        caller.data_mut().sockets.insert_with_fd(fd, stream);
                        WASI_ERRNO_SUCCESS
                    }
                    Err(error) => match error.kind() {
                        std::io::ErrorKind::ConnectionRefused => WASI_ERRNO_CONNREFUSED,
                        _ => WASI_ERRNO_IO,
                    },
                }
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "sock_send",
            |mut caller: wasmtime::Caller<'_, SidecarStore>,
             fd: i32,
             si_data_ptr: i32,
             si_data_len: i32,
             _si_flags: i32,
             datalen_out_ptr: i32|
             -> i32 {
                use std::io::Write;

                let regions = match read_iovecs(&mut caller, si_data_ptr, si_data_len) {
                    Ok(regions) => regions,
                    Err(errno) => return errno,
                };
                let bytes = {
                    let memory = match caller_memory(&mut caller) {
                        Ok(memory) => memory,
                        Err(errno) => return errno,
                    };
                    let data = memory.data(&caller);
                    let mut bytes = Vec::new();
                    for (ptr, len) in regions {
                        bytes.extend_from_slice(&data[ptr..ptr + len]);
                    }
                    bytes
                };

                let written = match caller.data_mut().sockets.get_mut(fd) {
                    Some(stream) => match stream.write(&bytes) {
                        Ok(written) => written as u32,
                        Err(_) => return WASI_ERRNO_IO,
                    },
                    None => return WASI_ERRNO_BADF,
                };

                match write_guest_u32(&mut caller, datalen_out_ptr, written) {
                    Ok(()) => WASI_ERRNO_SUCCESS,
                    Err(errno) => errno,
                }
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "sock_recv",
            |mut caller: wasmtime::Caller<'_, SidecarStore>,
             fd: i32,
             ri_data_ptr: i32,
             ri_data_len: i32,
             _ri_flags: i32,
             datalen_out_ptr: i32,
             roflags_out_ptr: i32|
             -> i32 {
                use std::io::Read;

                let regions = match read_iovecs(&mut caller, ri_data_ptr, ri_data_len) {
                    Ok(regions) => regions,
                    Err(errno) => return errno,
                };
                let total_capacity: usize = regions.iter().map(|(_, len)| *len).sum();
                let mut buffer = vec![0u8; total_capacity];
                let received = match caller.data_mut().sockets.get_mut(fd) {
                    Some(stream) => match stream.read(&mut buffer) {
                        Ok(received) => received,
                        Err(_) => return WASI_ERRNO_IO,
                    },
                    None => return WASI_ERRNO_BADF,
                };

                let memory = match caller_memory(&mut caller) {
                    Ok(memory) => memory,
                    Err(errno) => return errno,
                };
                let mut written = 0usize;
                for (ptr, len) in regions {
                    if written >= received {
                        break;
                    }
                    let chunk_len = len.min(received - written);
                    if memory
                        .write(&mut caller, ptr, &buffer[written..written + chunk_len])
                        .is_err()
                    {
                        return WASI_ERRNO_FAULT;
                    }
                    written += chunk_len;
                }
                if write_guest_u32(&mut caller, datalen_out_ptr, received as u32).is_err() {
                    return WASI_ERRNO_FAULT;
                }
                if write_guest_u16(&mut caller, roflags_out_ptr, 0).is_err() {
                    return WASI_ERRNO_FAULT;
                }
                WASI_ERRNO_SUCCESS
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "sock_shutdown",
            |mut caller: wasmtime::Caller<'_, SidecarStore>, fd: i32, _how: i32| -> i32 {
                if let Some(stream) = caller.data_mut().sockets.remove(fd) {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    WASI_ERRNO_SUCCESS
                } else {
                    WASI_ERRNO_BADF
                }
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
fn load_sidecar(
    engine: &Engine,
    wasm_bytes: &[u8],
    tx: SlideChannel,
) -> Result<SidecarHandle, LoadError> {
    load_sidecar_with_config(engine, wasm_bytes, tx, &[], "test-sidecar")
}

fn load_sidecar_with_config(
    engine: &Engine,
    wasm_bytes: &[u8],
    tx: SlideChannel,
    wasi_preopens: &[String],
    runtime_label: &str,
) -> Result<SidecarHandle, LoadError> {
    let module =
        Module::new(engine, wasm_bytes).map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    let engine = engine.clone();
    let wasi_preopens = wasi_preopens.to_vec();
    let runtime_label = runtime_label.to_string();
    log::info!(
        "sidecar:{} spawning (preopens={})",
        runtime_label,
        wasi_preopens.len()
    );
    let thread = std::thread::Builder::new()
        .name("vzglyd-sidecar".into())
        .spawn(move || {
            log::info!("sidecar:{} thread started", runtime_label);
            let result = run_sidecar_module(&engine, &module, tx, &wasi_preopens, &runtime_label);
            #[cfg(test)]
            if let Err(error) = result {
                panic!("sidecar thread failed: {error}");
            }
            #[cfg(not(test))]
            if let Err(error) = result {
                log::warn!("sidecar thread failed: {error}");
            } else {
                log::info!("sidecar:{} exited cleanly", runtime_label);
            }
        })
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;

    Ok(SidecarHandle { _thread: thread })
}

fn run_sidecar_module(
    engine: &Engine,
    module: &Module,
    tx: SlideChannel,
    wasi_preopens: &[String],
    runtime_label: &str,
) -> Result<(), LoadError> {
    let mut wasi_builder = wasmtime_wasi::sync::WasiCtxBuilder::new();
    wasi_builder.inherit_stdout().inherit_stderr();
    wasi_builder
        .inherit_env()
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    configure_sidecar_preopens(&mut wasi_builder, wasi_preopens)?;
    let wasi = wasi_builder.build();
    let mut store = Store::new(
        engine,
        SidecarStore {
            wasi,
            sockets: SocketTable::new(),
            tx,
            label: runtime_label.to_string(),
        },
    );
    configure_runtime_store(&mut store);
    let mut linker: wasmtime::Linker<SidecarStore> = wasmtime::Linker::new(engine);
    linker.allow_shadowing(true);
    wasmtime_wasi::sync::add_to_linker(&mut linker, |s: &mut SidecarStore| &mut s.wasi)
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    register_tcp_extension(&mut linker)?;
    linker
        .func_wrap(
            "vzglyd_host",
            "channel_push",
            |mut caller: wasmtime::Caller<'_, SidecarStore>, ptr: i32, len: i32| -> i32 {
                let tx = Arc::clone(&caller.data().tx);
                let bytes = match read_guest_bytes(&mut caller, ptr, len) {
                    Ok(bytes) => bytes,
                    Err(_) => return HOST_ERROR,
                };
                log::info!(
                    "sidecar:{} pushed {} bytes: {}",
                    caller.data().label,
                    bytes.len(),
                    preview_bytes(&bytes, 160)
                );
                tx.push_latest(bytes);
                WASI_ERRNO_SUCCESS
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    linker
        .func_wrap(
            "vzglyd_host",
            "channel_poll",
            |_caller: wasmtime::Caller<'_, SidecarStore>, _ptr: i32, _len: i32| -> i32 {
                HOST_CHANNEL_EMPTY
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    linker
        .func_wrap(
            "vzglyd_host",
            "log_info",
            |mut caller: wasmtime::Caller<'_, SidecarStore>, ptr: i32, len: i32| -> i32 {
                host_sidecar_log_info(&mut caller, ptr, len)
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    linker
        .func_wrap(
            "vzglyd_host",
            "channel_active",
            |caller: wasmtime::Caller<'_, SidecarStore>| -> i32 {
                i32::from(caller.data().tx.is_active())
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;

    let instance = linker
        .instantiate(&mut store, module)
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    log::info!("sidecar:{} instantiated", store.data().label);

    if let Ok(run_fn) = instance.get_typed_func::<(), i32>(&mut store, "vzglyd_sidecar_run") {
        log::info!("sidecar:{} invoking vzglyd_sidecar_run", store.data().label);
        let status = run_fn
            .call(&mut store, ())
            .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
        log::info!(
            "sidecar:{} vzglyd_sidecar_run -> {status}",
            store.data().label
        );
        return Ok(());
    }

    if let Ok(start_fn) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
        log::info!("sidecar:{} invoking _start", store.data().label);
        match start_fn.call(&mut store, ()) {
            Ok(()) => return Ok(()),
            Err(error)
                if error
                    .downcast_ref::<wasmtime_wasi::I32Exit>()
                    .is_some_and(|exit| exit.0 == 0) =>
            {
                return Ok(());
            }
            Err(error) => {
                return Err(LoadError::WasmLoad(format!(
                    "sidecar _start failed: {error}"
                )));
            }
        }
    }

    Err(LoadError::MissingExport("vzglyd_sidecar_run"))
}

fn configure_sidecar_preopens(
    wasi_builder: &mut wasmtime_wasi::sync::WasiCtxBuilder,
    wasi_preopens: &[String],
) -> Result<(), LoadError> {
    for spec in wasi_preopens {
        let (host, guest) = parse_sidecar_preopen(spec)?;
        let dir = wasmtime_wasi::sync::Dir::open_ambient_dir(
            &host,
            wasmtime_wasi::sync::ambient_authority(),
        )
        .map_err(|error| {
            LoadError::WasmLoad(format!(
                "failed to open sidecar preopen '{}' for '{}': {error}",
                host.display(),
                guest
            ))
        })?;
        wasi_builder
            .preopened_dir(dir, guest)
            .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    }
    Ok(())
}

fn parse_sidecar_preopen(spec: &str) -> Result<(PathBuf, String), LoadError> {
    let Some((host, guest)) = spec.rsplit_once(':') else {
        return Err(LoadError::ManifestValidation(format!(
            "sidecar preopen '{spec}' must have the form /host/path:/guest/path"
        )));
    };
    if host.is_empty() || guest.is_empty() {
        return Err(LoadError::ManifestValidation(format!(
            "sidecar preopen '{spec}' must include both host and guest paths"
        )));
    }
    Ok((PathBuf::from(host), guest.to_string()))
}

fn host_mesh_asset_len<T: HostAssetAccess>(
    caller: &mut wasmtime::Caller<'_, T>,
    key_ptr: i32,
    key_len: i32,
) -> i32 {
    let Ok(key) = read_host_string(caller, key_ptr, key_len) else {
        return HOST_ERROR;
    };
    caller
        .data()
        .mesh_assets()
        .encoded_by_key
        .get(&key)
        .map(|bytes| bytes.len() as i32)
        .unwrap_or(HOST_ASSET_NOT_FOUND)
}

fn host_mesh_asset_read<T: HostAssetAccess>(
    caller: &mut wasmtime::Caller<'_, T>,
    key_ptr: i32,
    key_len: i32,
    buf_ptr: i32,
    buf_len: i32,
) -> i32 {
    if buf_ptr < 0 || buf_len < 0 {
        return HOST_ERROR;
    }
    let Ok(key) = read_host_string(caller, key_ptr, key_len) else {
        return HOST_ERROR;
    };
    let Some(bytes) = caller
        .data()
        .mesh_assets()
        .encoded_by_key
        .get(&key)
        .cloned()
    else {
        return HOST_ASSET_NOT_FOUND;
    };
    if bytes.len() > buf_len as usize {
        return HOST_BUFFER_TOO_SMALL;
    }
    let Some(memory) = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
    else {
        return HOST_ERROR;
    };
    if memory
        .write(&mut *caller, buf_ptr as usize, &bytes)
        .is_err()
    {
        return HOST_ERROR;
    }
    bytes.len() as i32
}

fn host_scene_metadata_len<T: HostAssetAccess>(
    caller: &mut wasmtime::Caller<'_, T>,
    key_ptr: i32,
    key_len: i32,
) -> i32 {
    let Ok(key) = read_host_string(caller, key_ptr, key_len) else {
        return HOST_ERROR;
    };
    caller
        .data()
        .scene_metadata()
        .encoded_by_key
        .get(&key)
        .map(|bytes| bytes.len() as i32)
        .unwrap_or(HOST_ASSET_NOT_FOUND)
}

fn host_scene_metadata_read<T: HostAssetAccess>(
    caller: &mut wasmtime::Caller<'_, T>,
    key_ptr: i32,
    key_len: i32,
    buf_ptr: i32,
    buf_len: i32,
) -> i32 {
    if buf_ptr < 0 || buf_len < 0 {
        return HOST_ERROR;
    }
    let Ok(key) = read_host_string(caller, key_ptr, key_len) else {
        return HOST_ERROR;
    };
    let Some(bytes) = caller
        .data()
        .scene_metadata()
        .encoded_by_key
        .get(&key)
        .cloned()
    else {
        return HOST_ASSET_NOT_FOUND;
    };
    if bytes.len() > buf_len as usize {
        return HOST_BUFFER_TOO_SMALL;
    }
    let Some(memory) = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
    else {
        return HOST_ERROR;
    };
    if memory
        .write(&mut *caller, buf_ptr as usize, &bytes)
        .is_err()
    {
        return HOST_ERROR;
    }
    bytes.len() as i32
}

fn add_asset_host_functions<T: HostAssetAccess + Send + 'static>(
    linker: &mut wasmtime::Linker<T>,
) -> Result<(), LoadError> {
    linker
        .func_wrap(
            "vzglyd_host",
            "mesh_asset_len",
            |mut caller: wasmtime::Caller<'_, T>, key_ptr: i32, key_len: i32| -> i32 {
                host_mesh_asset_len(&mut caller, key_ptr, key_len)
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    linker
        .func_wrap(
            "vzglyd_host",
            "mesh_asset_read",
            |mut caller: wasmtime::Caller<'_, T>,
             key_ptr: i32,
             key_len: i32,
             buf_ptr: i32,
             buf_len: i32|
             -> i32 {
                host_mesh_asset_read(&mut caller, key_ptr, key_len, buf_ptr, buf_len)
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    linker
        .func_wrap(
            "vzglyd_host",
            "scene_metadata_len",
            |mut caller: wasmtime::Caller<'_, T>, key_ptr: i32, key_len: i32| -> i32 {
                host_scene_metadata_len(&mut caller, key_ptr, key_len)
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    linker
        .func_wrap(
            "vzglyd_host",
            "scene_metadata_read",
            |mut caller: wasmtime::Caller<'_, T>,
             key_ptr: i32,
             key_len: i32,
             buf_ptr: i32,
             buf_len: i32|
             -> i32 {
                host_scene_metadata_read(&mut caller, key_ptr, key_len, buf_ptr, buf_len)
            },
        )
        .map_err(|error| LoadError::WasmLoad(error.to_string()))?;
    Ok(())
}

fn read_overlay_from_store<T, V>(
    store: &mut Store<T>,
    memory: &Memory,
    ptr_fn: &TypedFunc<(), i32>,
    len_fn: &TypedFunc<(), i32>,
) -> Result<Option<RuntimeOverlay<V>>, LoadError>
where
    V: bytemuck::Pod + DeserializeOwned,
{
    let ptr = ptr_fn
        .call(&mut *store, ())
        .map_err(|error| LoadError::WasmLoad(error.to_string()))? as usize;
    let len = len_fn
        .call(&mut *store, ())
        .map_err(|error| LoadError::WasmLoad(error.to_string()))? as usize;
    if len == 0 {
        return Ok(None);
    }

    let data = memory
        .data(&*store)
        .get(ptr..ptr + len)
        .ok_or_else(|| LoadError::WasmLoad("overlay slice out of bounds".into()))?;
    let overlay = postcard::from_bytes::<RuntimeOverlay<V>>(data)
        .map_err(|error| LoadError::SpecDecode(error.to_string()))?;
    Ok(Some(overlay))
}

fn read_dynamic_meshes_from_store<T, V>(
    store: &mut Store<T>,
    memory: &Memory,
    ptr_fn: &TypedFunc<(), i32>,
    len_fn: &TypedFunc<(), i32>,
) -> Result<Option<RuntimeMeshSet<V>>, LoadError>
where
    V: bytemuck::Pod + DeserializeOwned,
{
    let ptr = ptr_fn
        .call(&mut *store, ())
        .map_err(|error| LoadError::WasmLoad(error.to_string()))? as usize;
    let len = len_fn
        .call(&mut *store, ())
        .map_err(|error| LoadError::WasmLoad(error.to_string()))? as usize;
    if len == 0 {
        return Ok(None);
    }

    let data = memory
        .data(&*store)
        .get(ptr..ptr + len)
        .ok_or_else(|| LoadError::WasmLoad("dynamic mesh slice out of bounds".into()))?;
    let meshes = postcard::from_bytes::<RuntimeMeshSet<V>>(data)
        .map_err(|error| LoadError::SpecDecode(error.to_string()))?;
    Ok(Some(meshes))
}

pub struct AssetLoader {
    package_root: PathBuf,
    canonical_package_root: PathBuf,
}

impl AssetLoader {
    pub fn new(package_root: &Path) -> Result<Self, LoadError> {
        let canonical_package_root = std::fs::canonicalize(package_root).map_err(|error| {
            LoadError::AssetLoad(format!(
                "failed to resolve package root '{}': {error}",
                package_root.display()
            ))
        })?;
        Ok(Self {
            package_root: package_root.to_path_buf(),
            canonical_package_root,
        })
    }

    pub fn load_texture(
        &self,
        path: &str,
        fallback_desc: &TextureDesc,
    ) -> Result<TextureData, LoadError> {
        let resolved = self.resolve(path)?;
        match resolved
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref()
        {
            Some("png") => {
                let image = ImageReader::open(&resolved)
                    .map_err(|error| {
                        LoadError::AssetLoad(format!(
                            "failed to open texture '{}': {error}",
                            resolved.display()
                        ))
                    })?
                    .decode()
                    .map_err(|error| {
                        LoadError::AssetLoad(format!(
                            "failed to decode texture '{}': {error}",
                            resolved.display()
                        ))
                    })?
                    .into_rgba8();
                Ok(TextureData {
                    width: image.width(),
                    height: image.height(),
                    data: image.into_raw(),
                })
            }
            Some("rgba") | Some("rgba8") => {
                if fallback_desc.format != TextureFormat::Rgba8Unorm {
                    return Err(LoadError::AssetLoad(format!(
                        "raw texture '{}' requires rgba8 fallback metadata",
                        resolved.display()
                    )));
                }
                let data = std::fs::read(&resolved).map_err(|error| {
                    LoadError::AssetLoad(format!(
                        "failed to read texture '{}': {error}",
                        resolved.display()
                    ))
                })?;
                let expected_len = fallback_desc.width as usize * fallback_desc.height as usize * 4;
                if data.len() != expected_len {
                    return Err(LoadError::AssetLoad(format!(
                        "raw texture '{}' has {} bytes but expected {expected_len}",
                        resolved.display(),
                        data.len()
                    )));
                }
                Ok(TextureData {
                    width: fallback_desc.width,
                    height: fallback_desc.height,
                    data,
                })
            }
            Some(other) => Err(LoadError::AssetLoad(format!(
                "unsupported texture format '.{other}' for '{}'",
                resolved.display()
            ))),
            None => Err(LoadError::AssetLoad(format!(
                "texture '{}' has no file extension",
                resolved.display()
            ))),
        }
    }

    pub fn load_shader(&self, path: &str) -> Result<String, LoadError> {
        let resolved = self.resolve(path)?;
        std::fs::read_to_string(&resolved).map_err(|error| {
            LoadError::AssetLoad(format!(
                "failed to read shader '{}': {error}",
                resolved.display()
            ))
        })
    }

    pub fn load_static_mesh<V: PackageMeshVertex>(
        &self,
        path: &str,
        template: &StaticMesh<V>,
    ) -> Result<StaticMesh<V>, LoadError> {
        let resolved = self.resolve(path)?;
        let imported = load_glb_mesh(&resolved)?;
        let fallback = template.vertices.first();
        Ok(StaticMesh {
            label: template.label.clone(),
            vertices: imported
                .vertices
                .into_iter()
                .map(|vertex| V::from_imported(vertex, fallback))
                .collect(),
            indices: imported.indices,
        })
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, LoadError> {
        let joined = self.package_root.join(path);
        let canonical = std::fs::canonicalize(&joined).map_err(|error| {
            LoadError::AssetLoad(format!("failed to resolve package asset '{path}': {error}"))
        })?;
        if !canonical.starts_with(&self.canonical_package_root) {
            return Err(LoadError::AssetLoad(format!(
                "package asset '{path}' escapes the package root"
            )));
        }
        Ok(canonical)
    }
}

fn load_glb_mesh(path: &Path) -> Result<ImportedMesh, LoadError> {
    let scene = load_glb_scene(path, None)?;
    flatten_scene_mesh_nodes(&scene, path)
}

pub(crate) fn load_authored_scene_from_manifest(
    manifest: &SlideManifest,
    package_root: &Path,
    requested_id: Option<&str>,
) -> Result<Option<ImportedScene>, LoadError> {
    let Some(scene_asset) = manifest.scene_asset(requested_id) else {
        return match requested_id {
            Some(id) => Err(LoadError::AssetLoad(format!(
                "manifest does not declare a scene asset with id '{id}'"
            ))),
            None => Ok(None),
        };
    };

    let loader = AssetLoader::new(package_root)?;
    let resolved = loader.resolve(&scene_asset.path)?;
    load_glb_scene(&resolved, Some(scene_asset)).map(Some)
}

fn load_glb_scene(
    path: &Path,
    selector: Option<&SceneAssetRef>,
) -> Result<ImportedScene, LoadError> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("glb") {
        return Err(LoadError::AssetLoad(format!(
            "unsupported scene format for '{}': only .glb scene assets are supported",
            path.display()
        )));
    }

    let bytes = std::fs::read(path).map_err(|error| {
        LoadError::AssetLoad(format!(
            "failed to read scene '{}': {error}",
            path.display()
        ))
    })?;
    let gltf = gltf::Gltf::from_slice(&bytes).map_err(|error| {
        LoadError::AssetLoad(format!("failed to parse GLB '{}': {error}", path.display()))
    })?;
    let blob = gltf.blob.as_deref().ok_or_else(|| {
        LoadError::AssetLoad(format!(
            "GLB '{}' is missing its binary buffer chunk",
            path.display()
        ))
    })?;

    for buffer in gltf.document.buffers() {
        if !matches!(buffer.source(), gltf::buffer::Source::Bin) {
            return Err(LoadError::AssetLoad(format!(
                "GLB '{}' references an external buffer; package scene assets must be self-contained",
                path.display()
            )));
        }
    }

    let gltf_scene = gltf
        .document
        .default_scene()
        .or_else(|| gltf.document.scenes().next())
        .ok_or_else(|| {
            LoadError::AssetLoad(format!(
                "GLB '{}' does not declare a scene to import",
                path.display()
            ))
        })?;

    let scene_name = gltf_scene.name().map(str::to_owned);
    let file_stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_owned);
    let scene_id = selector
        .and_then(|scene| scene.id.clone())
        .or_else(|| scene_name.clone())
        .or_else(|| file_stem.clone())
        .unwrap_or_else(|| "scene".into());

    let mut imported = ImportedScene {
        id: scene_id.clone(),
        source_path: path.to_path_buf(),
        label: selector
            .and_then(|scene| scene.label.clone())
            .or_else(|| scene_name.clone())
            .or_else(|| file_stem.clone()),
        entry_camera: selector.and_then(|scene| scene.entry_camera.clone()),
        compile_profile: selector
            .and_then(|scene| scene.compile_profile.clone())
            .or_else(|| Some("default_world".into())),
        metadata: ImportedSceneMetadata {
            scene_name,
            extras: ImportedExtras::default(),
        },
        mesh_nodes: Vec::new(),
        cameras: Vec::new(),
        anchors: Vec::new(),
        directional_lights: Vec::new(),
        warnings: Vec::new(),
    };
    imported.metadata.extras = parse_imported_extras(
        gltf_scene.extras(),
        &format!("scene '{}'", imported.id),
        &mut imported.warnings,
    );

    for node in gltf_scene.nodes() {
        append_glb_scene_node(&mut imported, node, Mat4::IDENTITY, blob, path)?;
    }

    Ok(imported)
}

fn flatten_scene_mesh_nodes(scene: &ImportedScene, path: &Path) -> Result<ImportedMesh, LoadError> {
    if scene.mesh_nodes.is_empty() {
        return Err(LoadError::AssetLoad(format!(
            "GLB '{}' did not produce any triangle geometry",
            path.display()
        )));
    }

    let mut imported = ImportedMesh {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for mesh_node in &scene.mesh_nodes {
        let vertex_offset = imported.vertices.len();
        if vertex_offset + mesh_node.vertices.len() > u16::MAX as usize + 1 {
            return Err(LoadError::AssetLoad(format!(
                "GLB '{}' exceeds the engine's u16 static mesh index limit",
                path.display()
            )));
        }

        imported.vertices.extend(mesh_node.vertices.iter().copied());
        for index in &mesh_node.indices {
            let final_index = vertex_offset.checked_add(*index as usize).ok_or_else(|| {
                LoadError::AssetLoad(format!(
                    "GLB '{}' produced an index that overflows the static mesh limit",
                    path.display()
                ))
            })?;
            let final_index = u16::try_from(final_index).map_err(|_| {
                LoadError::AssetLoad(format!(
                    "GLB '{}' produced an index outside the engine's u16 range",
                    path.display()
                ))
            })?;
            imported.indices.push(final_index);
        }
    }

    Ok(imported)
}

fn append_glb_scene_node(
    imported_scene: &mut ImportedScene,
    node: gltf::Node<'_>,
    parent_transform: Mat4,
    blob: &[u8],
    path: &Path,
) -> Result<(), LoadError> {
    let local_transform = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world_transform = parent_transform * local_transform;
    let node_name = node.name().map(str::to_owned);
    let display_name = node_name
        .clone()
        .unwrap_or_else(|| format!("node_{}", node.index()));
    let metadata = parse_imported_extras(
        node.extras(),
        &format!("node '{display_name}'"),
        &mut imported_scene.warnings,
    );
    let children: Vec<_> = node.children().collect();
    let mesh = node.mesh();
    let camera = node.camera();
    let light = node.light();

    if node.skin().is_some() {
        imported_scene.warnings.push(format!(
            "ignored skin data on node '{display_name}' while importing scene '{}'",
            imported_scene.id
        ));
    }
    if node.weights().is_some_and(|weights| !weights.is_empty()) {
        imported_scene.warnings.push(format!(
            "ignored morph target weights on node '{display_name}' while importing scene '{}'",
            imported_scene.id
        ));
    }

    if let Some(mesh) = mesh.as_ref() {
        let mesh_name = mesh.name().map(str::to_owned);
        let primitives: Vec<_> = mesh.primitives().collect();
        let primitive_count = primitives.len();
        for (primitive_index, primitive) in primitives.into_iter().enumerate() {
            let (primitive_mesh, material) = import_scene_primitive(
                primitive,
                world_transform,
                blob,
                path,
                &mut imported_scene.warnings,
                &display_name,
            )?;
            imported_scene.mesh_nodes.push(ImportedSceneMeshNode {
                id: stable_scene_mesh_id(
                    &metadata,
                    node_name.as_deref(),
                    mesh_name.as_deref(),
                    node.index(),
                    primitive_index,
                    primitive_count,
                ),
                label: scene_mesh_label(
                    node_name.as_deref(),
                    mesh_name.as_deref(),
                    node.index(),
                    primitive_index,
                    primitive_count,
                ),
                node_name: node_name.clone(),
                mesh_name: mesh_name.clone(),
                node_index: node.index(),
                primitive_index,
                world_transform: world_transform.to_cols_array_2d(),
                vertices: primitive_mesh.vertices,
                indices: primitive_mesh.indices,
                material,
                metadata: metadata.clone(),
            });
        }
    }

    if let Some(camera) = camera.as_ref() {
        imported_scene.cameras.push(ImportedSceneCamera {
            id: stable_scene_camera_id(
                &metadata,
                node_name.as_deref(),
                camera.name(),
                node.index(),
            ),
            label: camera_label(node_name.as_deref(), camera.name(), node.index()),
            node_name: node_name.clone(),
            camera_name: camera.name().map(str::to_owned),
            node_index: node.index(),
            world_transform: world_transform.to_cols_array_2d(),
            projection: import_camera_projection(camera),
            metadata: metadata.clone(),
        });
    }

    if let Some(light) = light.as_ref() {
        match light.kind() {
            gltf::khr_lights_punctual::Kind::Directional => {
                let direction = world_transform.transform_vector3(Vec3::Z).normalize_or_zero();
                if direction.length_squared() == 0.0 {
                    imported_scene.warnings.push(format!(
                        "ignored directional light on node '{display_name}' with a degenerate transform while importing scene '{}'",
                        imported_scene.id
                    ));
                } else {
                    imported_scene
                        .directional_lights
                        .push(ImportedSceneDirectionalLight {
                            id: stable_scene_light_id(
                                &metadata,
                                node_name.as_deref(),
                                light.name(),
                                node.index(),
                            ),
                            label: scene_light_label(node_name.as_deref(), light.name(), node.index()),
                            node_name: node_name.clone(),
                            light_name: light.name().map(str::to_owned),
                            node_index: node.index(),
                            world_transform: world_transform.to_cols_array_2d(),
                            direction: direction.to_array(),
                            color: light.color(),
                            intensity: light.intensity(),
                            metadata: metadata.clone(),
                        });
                }
            }
            gltf::khr_lights_punctual::Kind::Point => imported_scene.warnings.push(format!(
                "ignored unsupported point light on node '{display_name}' while importing scene '{}'",
                imported_scene.id
            )),
            gltf::khr_lights_punctual::Kind::Spot { .. } => imported_scene.warnings.push(format!(
                "ignored unsupported spot light on node '{display_name}' while importing scene '{}'",
                imported_scene.id
            )),
        }
    }

    let is_anchor = metadata.vzglyd_anchor_tagged || metadata.vzglyd_id.is_some();
    if mesh.is_none() && camera.is_none() && light.is_none() && is_anchor {
        imported_scene.anchors.push(ImportedSceneAnchor {
            id: stable_anchor_id(&metadata, node_name.as_deref(), node.index()),
            label: anchor_label(node_name.as_deref(), node.index()),
            node_name: node_name.clone(),
            node_index: node.index(),
            world_transform: world_transform.to_cols_array_2d(),
            metadata: metadata.clone(),
        });
    } else if mesh.is_none() && camera.is_none() && light.is_none() && children.is_empty() {
        imported_scene.warnings.push(format!(
            "ignored unsupported empty node '{display_name}' while importing scene '{}'",
            imported_scene.id
        ));
    }

    for child in children {
        append_glb_scene_node(imported_scene, child, world_transform, blob, path)?;
    }

    Ok(())
}

fn import_scene_primitive(
    primitive: gltf::Primitive<'_>,
    world_transform: Mat4,
    blob: &[u8],
    path: &Path,
    warnings: &mut Vec<String>,
    node_label: &str,
) -> Result<(ImportedMesh, ImportedSceneMaterial), LoadError> {
    if primitive.mode() != gltf::mesh::Mode::Triangles {
        return Err(LoadError::AssetLoad(format!(
            "GLB '{}' uses primitive mode {:?}; only triangle meshes are supported",
            path.display(),
            primitive.mode()
        )));
    }

    let reader = primitive.reader(|buffer| match buffer.source() {
        gltf::buffer::Source::Bin => Some(blob),
        gltf::buffer::Source::Uri(_) => None,
    });
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or_else(|| {
            LoadError::AssetLoad(format!(
                "GLB '{}' contains a primitive without POSITION data",
                path.display()
            ))
        })?
        .collect();
    let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(Iterator::collect);
    let tex_coords: Option<Vec<[f32; 2]>> = reader
        .read_tex_coords(0)
        .map(|coords| coords.into_f32().collect());
    let vertex_colors: Option<Vec<[f32; 4]>> = reader
        .read_colors(0)
        .map(|colors| colors.into_rgba_f32().collect());
    let primitive_indices: Vec<u32> = reader
        .read_indices()
        .map(|indices| indices.into_u32().collect())
        .unwrap_or_else(|| (0..positions.len() as u32).collect());
    if positions.len() > u16::MAX as usize + 1 {
        return Err(LoadError::AssetLoad(format!(
            "GLB '{}' exceeds the engine's u16 static mesh index limit",
            path.display()
        )));
    }

    let material = primitive.material();
    let material_factor = material.pbr_metallic_roughness().base_color_factor();
    let material_color = (material_factor != [1.0, 1.0, 1.0, 1.0]).then_some(material_factor);
    let material_metadata = parse_imported_extras(
        material.extras(),
        &format!("material on node '{node_label}'"),
        warnings,
    );
    let normal_transform = Mat3::from_mat4(world_transform).inverse().transpose();
    let mut imported = ImportedMesh {
        vertices: Vec::with_capacity(positions.len()),
        indices: Vec::with_capacity(primitive_indices.len()),
    };

    for (vertex_index, position) in positions.iter().enumerate() {
        let world_position = world_transform.transform_point3(Vec3::from_array(*position));
        let transformed_normal = normals
            .as_ref()
            .and_then(|normals| normals.get(vertex_index).copied())
            .map(|normal| {
                normal_transform
                    .mul_vec3(Vec3::from_array(normal))
                    .normalize_or_zero()
                    .to_array()
            });
        let color = vertex_colors
            .as_ref()
            .and_then(|colors| colors.get(vertex_index).copied())
            .map(|vertex_color| multiply_rgba(vertex_color, material_factor))
            .or(material_color);
        imported.vertices.push(ImportedVertex {
            position: world_position.to_array(),
            normal: transformed_normal,
            tex_coords: tex_coords
                .as_ref()
                .and_then(|coords| coords.get(vertex_index).copied()),
            color,
        });
    }

    for index in primitive_indices {
        let final_index = u16::try_from(index).map_err(|_| {
            LoadError::AssetLoad(format!(
                "GLB '{}' produced an index outside the engine's u16 range",
                path.display()
            ))
        })?;
        imported.indices.push(final_index);
    }

    fill_missing_normals(&mut imported);

    Ok((
        imported,
        ImportedSceneMaterial {
            name: material.name().map(str::to_owned),
            base_color_factor: material_factor,
            class_hint: material_metadata
                .vzglyd_material
                .clone()
                .or_else(|| material.name().map(str::to_owned)),
            metadata: material_metadata,
        },
    ))
}

fn import_camera_projection(camera: &gltf::Camera<'_>) -> ImportedCameraProjection {
    match camera.projection() {
        gltf::camera::Projection::Perspective(perspective) => {
            ImportedCameraProjection::Perspective {
                aspect_ratio: perspective.aspect_ratio(),
                yfov_rad: perspective.yfov(),
                znear: perspective.znear(),
                zfar: perspective.zfar(),
            }
        }
        gltf::camera::Projection::Orthographic(orthographic) => {
            ImportedCameraProjection::Orthographic {
                xmag: orthographic.xmag(),
                ymag: orthographic.ymag(),
                znear: orthographic.znear(),
                zfar: orthographic.zfar(),
            }
        }
    }
}

fn parse_imported_extras(
    extras: &gltf::json::Extras,
    context: &str,
    warnings: &mut Vec<String>,
) -> ImportedExtras {
    let Some(raw) = extras.as_ref() else {
        return ImportedExtras::default();
    };

    let value = match serde_json::from_str::<JsonValue>(raw.get()) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("ignored invalid extras on {context}: {error}"));
            return ImportedExtras::default();
        }
    };
    let JsonValue::Object(raw) = value else {
        warnings.push(format!("ignored non-object extras on {context}"));
        return ImportedExtras::default();
    };

    ImportedExtras {
        vzglyd_id: read_extra_string(&raw, "vzglyd_id", context, warnings),
        vzglyd_pipeline: read_extra_string(&raw, "vzglyd_pipeline", context, warnings),
        vzglyd_material: read_extra_string(&raw, "vzglyd_material", context, warnings),
        vzglyd_anchor: read_extra_anchor(&raw, context, warnings),
        vzglyd_anchor_tagged: read_extra_anchor_tagged(&raw, context, warnings),
        vzglyd_hidden: read_extra_bool(&raw, "vzglyd_hidden", context, warnings),
        vzglyd_billboard: read_extra_bool(&raw, "vzglyd_billboard", context, warnings),
        vzglyd_entry_camera: read_extra_bool(&raw, "vzglyd_entry_camera", context, warnings),
        raw,
    }
}

fn read_extra_string(
    extras: &JsonMap<String, JsonValue>,
    key: &str,
    context: &str,
    warnings: &mut Vec<String>,
) -> Option<String> {
    match extras.get(key) {
        Some(JsonValue::String(value)) => Some(value.clone()),
        Some(other) => {
            warnings.push(format!(
                "ignored non-string extras key '{key}' on {context}: {other}"
            ));
            None
        }
        None => None,
    }
}

fn read_extra_bool(
    extras: &JsonMap<String, JsonValue>,
    key: &str,
    context: &str,
    warnings: &mut Vec<String>,
) -> bool {
    match extras.get(key) {
        Some(JsonValue::Bool(value)) => *value,
        Some(other) => {
            warnings.push(format!(
                "ignored non-bool extras key '{key}' on {context}: {other}"
            ));
            false
        }
        None => false,
    }
}

fn read_extra_anchor(
    extras: &JsonMap<String, JsonValue>,
    context: &str,
    warnings: &mut Vec<String>,
) -> Option<String> {
    match extras.get("vzglyd_anchor") {
        Some(JsonValue::String(value)) => Some(value.clone()),
        Some(JsonValue::Bool(true)) | None => None,
        Some(JsonValue::Bool(false)) => None,
        Some(other) => {
            warnings.push(format!(
                "ignored unsupported extras key 'vzglyd_anchor' on {context}: {other}"
            ));
            None
        }
    }
}

fn read_extra_anchor_tagged(
    extras: &JsonMap<String, JsonValue>,
    context: &str,
    warnings: &mut Vec<String>,
) -> bool {
    match extras.get("vzglyd_anchor") {
        Some(JsonValue::String(_)) => true,
        Some(JsonValue::Bool(value)) => *value,
        None => false,
        Some(other) => {
            warnings.push(format!(
                "ignored unsupported extras key 'vzglyd_anchor' on {context}: {other}"
            ));
            false
        }
    }
}

fn scene_mesh_label(
    node_name: Option<&str>,
    mesh_name: Option<&str>,
    node_index: usize,
    primitive_index: usize,
    primitive_count: usize,
) -> String {
    let mut label = node_name
        .or(mesh_name)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("mesh_node_{node_index}"));
    if primitive_count > 1 {
        label.push_str(&format!("#primitive_{primitive_index}"));
    }
    label
}

fn stable_scene_mesh_id(
    metadata: &ImportedExtras,
    node_name: Option<&str>,
    mesh_name: Option<&str>,
    node_index: usize,
    primitive_index: usize,
    primitive_count: usize,
) -> String {
    let mut id = metadata
        .vzglyd_id
        .clone()
        .or_else(|| node_name.map(str::to_owned))
        .or_else(|| mesh_name.map(str::to_owned))
        .unwrap_or_else(|| format!("mesh_node_{node_index}"));
    if primitive_count > 1 {
        id.push_str(&format!("#primitive_{primitive_index}"));
    }
    id
}

fn camera_label(node_name: Option<&str>, camera_name: Option<&str>, node_index: usize) -> String {
    node_name
        .or(camera_name)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("camera_node_{node_index}"))
}

fn stable_scene_camera_id(
    metadata: &ImportedExtras,
    node_name: Option<&str>,
    camera_name: Option<&str>,
    node_index: usize,
) -> String {
    metadata
        .vzglyd_id
        .clone()
        .or_else(|| node_name.map(str::to_owned))
        .or_else(|| camera_name.map(str::to_owned))
        .unwrap_or_else(|| format!("camera_node_{node_index}"))
}

fn scene_light_label(
    node_name: Option<&str>,
    light_name: Option<&str>,
    node_index: usize,
) -> String {
    node_name
        .or(light_name)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("light_node_{node_index}"))
}

fn stable_scene_light_id(
    metadata: &ImportedExtras,
    node_name: Option<&str>,
    light_name: Option<&str>,
    node_index: usize,
) -> String {
    metadata
        .vzglyd_id
        .clone()
        .or_else(|| node_name.map(str::to_owned))
        .or_else(|| light_name.map(str::to_owned))
        .unwrap_or_else(|| format!("light_node_{node_index}"))
}

fn anchor_label(node_name: Option<&str>, node_index: usize) -> String {
    node_name
        .map(str::to_owned)
        .unwrap_or_else(|| format!("anchor_node_{node_index}"))
}

fn stable_anchor_id(
    metadata: &ImportedExtras,
    node_name: Option<&str>,
    node_index: usize,
) -> String {
    metadata
        .vzglyd_id
        .clone()
        .or_else(|| node_name.map(str::to_owned))
        .unwrap_or_else(|| format!("anchor_node_{node_index}"))
}

fn fill_missing_normals(imported: &mut ImportedMesh) {
    if imported
        .vertices
        .iter()
        .all(|vertex| vertex.normal.is_some())
    {
        return;
    }

    let mut accum = vec![Vec3::ZERO; imported.vertices.len()];
    for triangle in imported.indices.chunks_exact(3) {
        let i0 = triangle[0] as usize;
        let i1 = triangle[1] as usize;
        let i2 = triangle[2] as usize;
        let p0 = Vec3::from_array(imported.vertices[i0].position);
        let p1 = Vec3::from_array(imported.vertices[i1].position);
        let p2 = Vec3::from_array(imported.vertices[i2].position);
        let face_normal = (p1 - p0).cross(p2 - p0);
        accum[i0] += face_normal;
        accum[i1] += face_normal;
        accum[i2] += face_normal;
    }

    for (vertex, sum) in imported.vertices.iter_mut().zip(accum) {
        if vertex.normal.is_none() {
            vertex.normal = Some(sum.normalize_or_zero().to_array());
        }
    }
}

fn multiply_rgba(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

const DEFAULT_WORLD_SCENE_COMPILE_PROFILE: &str = "default_world";
const FIXED_CAMERA_DURATION_SECONDS: f32 = 1.0;
const AUTHORED_CAMERA_STEP_SECONDS: f32 = 8.0;

fn scene_material_mode(material_class: SceneMaterialClass) -> f32 {
    match material_class {
        SceneMaterialClass::Opaque => 0.0,
        SceneMaterialClass::AlphaTest => 1.0,
        SceneMaterialClass::Transparent => 2.0,
        SceneMaterialClass::Emissive => 3.0,
        SceneMaterialClass::Water => 5.0,
    }
}

fn normalize_scene_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

fn resolve_scene_material_class(mesh_node: &ImportedSceneMeshNode) -> SceneMaterialClass {
    let hint = mesh_node
        .metadata
        .vzglyd_material
        .as_deref()
        .or(mesh_node.material.metadata.vzglyd_material.as_deref())
        .or(mesh_node.material.class_hint.as_deref());
    let normalized = hint.map(normalize_scene_token);
    match normalized.as_deref() {
        Some("alpha_test") | Some("alphatest") | Some("cutout") => SceneMaterialClass::AlphaTest,
        Some("transparent") | Some("alpha_blend") | Some("blend") => {
            SceneMaterialClass::Transparent
        }
        Some("emissive") => SceneMaterialClass::Emissive,
        Some("water") => SceneMaterialClass::Water,
        Some("opaque") => SceneMaterialClass::Opaque,
        _ if mesh_node.material.base_color_factor[3] < 0.999 => SceneMaterialClass::Transparent,
        _ => SceneMaterialClass::Opaque,
    }
}

fn resolve_scene_pipeline(
    scene_id: &str,
    mesh_node: &ImportedSceneMeshNode,
    material_class: SceneMaterialClass,
) -> PipelineKind {
    let default = if matches!(
        material_class,
        SceneMaterialClass::Transparent | SceneMaterialClass::Water
    ) {
        PipelineKind::Transparent
    } else {
        PipelineKind::Opaque
    };

    match mesh_node
        .metadata
        .vzglyd_pipeline
        .as_deref()
        .map(normalize_scene_token)
        .as_deref()
    {
        Some("opaque") => PipelineKind::Opaque,
        Some("transparent") => PipelineKind::Transparent,
        Some(other) => {
            log::warn!(
                "scene '{}' node '{}' requested unsupported vzglyd_pipeline '{}'; using {:?}",
                scene_id,
                mesh_node.id,
                other,
                default
            );
            default
        }
        None => default,
    }
}

fn scene_camera_keyframe(camera: &ImportedSceneCamera, time: f32) -> CameraKeyframe {
    let transform = Mat4::from_cols_array_2d(&camera.world_transform);
    let eye = transform.transform_point3(Vec3::ZERO);
    let forward = transform.transform_vector3(-Vec3::Z).normalize_or_zero();
    let up = transform.transform_vector3(Vec3::Y).normalize_or_zero();
    let target = eye
        + if forward.length_squared() > 0.0 {
            forward
        } else {
            -Vec3::Z
        };
    let fov_y_deg = match camera.projection {
        ImportedCameraProjection::Perspective { yfov_rad, .. } => yfov_rad.to_degrees(),
        ImportedCameraProjection::Orthographic { ymag, .. } => {
            (2.0 * ymag.max(0.1).atan()).to_degrees().clamp(20.0, 100.0)
        }
    };

    CameraKeyframe {
        time,
        position: eye.to_array(),
        target: target.to_array(),
        up: if up.length_squared() > 0.0 {
            up.to_array()
        } else {
            Vec3::Y.to_array()
        },
        fov_y_deg: fov_y_deg.clamp(20.0, 100.0),
    }
}

fn fixed_camera_path(keyframe: CameraKeyframe) -> CameraPath {
    CameraPath {
        looped: false,
        keyframes: vec![
            CameraKeyframe {
                time: 0.0,
                ..keyframe.clone()
            },
            CameraKeyframe {
                time: FIXED_CAMERA_DURATION_SECONDS,
                ..keyframe
            },
        ],
    }
}

fn matches_scene_camera_selector(camera: &ImportedSceneCamera, selector: &str) -> bool {
    let selector = normalize_scene_token(selector);
    [
        Some(camera.id.as_str()),
        camera.node_name.as_deref(),
        camera.camera_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|candidate| normalize_scene_token(candidate) == selector)
}

fn compiled_scene_bounds(mesh_nodes: &[&ImportedSceneMeshNode]) -> Option<(Vec3, Vec3)> {
    let mut bounds: Option<(Vec3, Vec3)> = None;
    for mesh_node in mesh_nodes {
        for vertex in &mesh_node.vertices {
            let position = Vec3::from_array(vertex.position);
            bounds = Some(match bounds {
                Some((min, max)) => (min.min(position), max.max(position)),
                None => (position, position),
            });
        }
    }
    bounds
}

fn default_scene_camera_path(mesh_nodes: &[&ImportedSceneMeshNode]) -> Option<CameraPath> {
    let (min, max) = compiled_scene_bounds(mesh_nodes)?;
    let center = (min + max) * 0.5;
    let extent = (max - min).max(Vec3::splat(1.0));
    let radius = extent.max_element().max(1.0);
    let keyframe = CameraKeyframe {
        time: 0.0,
        position: (center + Vec3::new(radius * 1.4, radius, radius * 1.4)).to_array(),
        target: center.to_array(),
        up: Vec3::Y.to_array(),
        fov_y_deg: 50.0,
    };
    Some(fixed_camera_path(keyframe))
}

fn compile_scene_camera_path(
    scene: &ImportedScene,
    visible_mesh_nodes: &[&ImportedSceneMeshNode],
) -> Option<CameraPath> {
    let visible_cameras: Vec<&ImportedSceneCamera> = scene
        .cameras
        .iter()
        .filter(|camera| !camera.metadata.vzglyd_hidden)
        .collect();

    if let Some(selector) = scene.entry_camera.as_deref() {
        if let Some(camera) = visible_cameras
            .iter()
            .copied()
            .find(|camera| matches_scene_camera_selector(camera, selector))
        {
            return Some(fixed_camera_path(scene_camera_keyframe(camera, 0.0)));
        }
    }
    if let Some(camera) = visible_cameras
        .iter()
        .copied()
        .find(|camera| camera.metadata.vzglyd_entry_camera)
    {
        return Some(fixed_camera_path(scene_camera_keyframe(camera, 0.0)));
    }

    match visible_cameras.as_slice() {
        [] => default_scene_camera_path(visible_mesh_nodes),
        [camera] => Some(fixed_camera_path(scene_camera_keyframe(camera, 0.0))),
        cameras => Some(CameraPath {
            looped: true,
            keyframes: cameras
                .iter()
                .enumerate()
                .map(|(index, camera)| {
                    scene_camera_keyframe(camera, index as f32 * AUTHORED_CAMERA_STEP_SECONDS)
                })
                .collect(),
        }),
    }
}

fn default_font_texture() -> TextureDesc {
    TextureDesc {
        label: "font_atlas".into(),
        width: 256,
        height: 8,
        format: TextureFormat::Rgba8Unorm,
        wrap_u: WrapMode::ClampToEdge,
        wrap_v: WrapMode::ClampToEdge,
        wrap_w: WrapMode::ClampToEdge,
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        mip_filter: FilterMode::Nearest,
        data: make_font_atlas(),
    }
}

fn default_secondary_material_texture() -> TextureDesc {
    // Neutral 1×1 grey fallback for the second world-material slot.
    // Keep the historical `noise_tex` label so existing overrides still resolve.
    solid_texture("noise_tex", [128, 128, 128, 255])
}

fn solid_texture(label: &str, rgba: [u8; 4]) -> TextureDesc {
    TextureDesc {
        label: label.into(),
        width: 1,
        height: 1,
        format: TextureFormat::Rgba8Unorm,
        wrap_u: WrapMode::Repeat,
        wrap_v: WrapMode::Repeat,
        wrap_w: WrapMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mip_filter: FilterMode::Nearest,
        data: rgba.to_vec(),
    }
}

fn ensure_compiled_scene_textures<V: bytemuck::Pod>(spec: &mut SlideSpec<V>) {
    if spec.textures.is_empty() {
        spec.textures.push(default_font_texture());
    }
    if spec.textures.len() == 1 {
        spec.textures.push(default_secondary_material_texture());
    }
    if spec.textures.len() == 2 {
        spec.textures
            .push(solid_texture("scene_material_a", [214, 205, 184, 255]));
    }
    if spec.textures.len() == 3 {
        spec.textures
            .push(solid_texture("scene_material_b", [102, 140, 168, 255]));
    }
    spec.textures_used = spec.textures.len() as u32;
}

fn normalize_imported_directional_intensity(intensity: f32) -> f32 {
    intensity.max(0.0).min(4.0)
}

fn compile_scene_lighting(
    scene: &ImportedScene,
    fallback: Option<&WorldLighting>,
) -> Option<WorldLighting> {
    let visible_directional_lights: Vec<&ImportedSceneDirectionalLight> = scene
        .directional_lights
        .iter()
        .filter(|light| !light.metadata.vzglyd_hidden)
        .collect();

    if visible_directional_lights.len() > 1 {
        log::warn!(
            "scene '{}' defines {} visible directional lights; using only '{}'",
            scene.id,
            visible_directional_lights.len(),
            visible_directional_lights[0].id
        );
    }

    let Some(light) = visible_directional_lights.first().copied() else {
        return fallback.cloned();
    };

    let mut lighting = fallback.cloned().unwrap_or_default();
    lighting.directional_light = Some(DirectionalLight::new(
        light.direction,
        light.color,
        normalize_imported_directional_intensity(light.intensity),
    ));
    Some(lighting)
}

fn compile_authored_scene_into_spec<V>(
    spec: &mut SlideSpec<V>,
    scene: &ImportedScene,
) -> Result<(), LoadError>
where
    V: PackageMeshVertex,
{
    let preserved_static_meshes = std::mem::take(&mut spec.static_meshes);
    let preserved_static_draws = spec
        .draws
        .iter()
        .filter_map(|draw| match draw.source {
            DrawSource::Static(slot) => Some((slot, draw.pipeline, draw.clone())),
            DrawSource::Dynamic(_) => None,
        })
        .collect::<Vec<_>>();
    let preserved_dynamic_draws: Vec<DrawSpec> = spec
        .draws
        .iter()
        .filter(|draw| matches!(draw.source, DrawSource::Dynamic(_)))
        .cloned()
        .collect();
    let fallback_vertex = preserved_static_meshes
        .first()
        .and_then(|mesh| mesh.vertices.first());
    let visible_mesh_nodes: Vec<&ImportedSceneMeshNode> = scene
        .mesh_nodes
        .iter()
        .filter(|mesh_node| !mesh_node.metadata.vzglyd_hidden)
        .collect();
    if visible_mesh_nodes.is_empty() {
        return Err(LoadError::AssetLoad(format!(
            "scene '{}' did not contain any visible mesh nodes to compile",
            scene.id
        )));
    }

    let mut static_meshes = Vec::with_capacity(visible_mesh_nodes.len());
    let mut opaque_draws = Vec::new();
    let mut transparent_draws = Vec::new();
    for mesh_node in &visible_mesh_nodes {
        let material_class = resolve_scene_material_class(mesh_node);
        let pipeline = resolve_scene_pipeline(&scene.id, mesh_node, material_class);
        let mesh_index = static_meshes.len();
        static_meshes.push(StaticMesh {
            label: mesh_node.label.clone(),
            vertices: mesh_node
                .vertices
                .iter()
                .copied()
                .map(|imported| V::from_scene_import(imported, material_class, fallback_vertex))
                .collect(),
            indices: mesh_node.indices.clone(),
        });
        let draw = DrawSpec {
            label: mesh_node.label.clone(),
            source: DrawSource::Static(mesh_index),
            pipeline,
            index_range: 0..mesh_node.indices.len() as u32,
        };
        match pipeline {
            PipelineKind::Opaque => opaque_draws.push(draw),
            PipelineKind::Transparent => transparent_draws.push(draw),
        }
    }

    let preserved_static_mesh_offset = static_meshes.len();
    let mut preserved_opaque_draws = Vec::new();
    let mut preserved_transparent_draws = Vec::new();
    for (slot, pipeline, mut draw) in preserved_static_draws {
        draw.source = DrawSource::Static(slot + preserved_static_mesh_offset);
        match pipeline {
            PipelineKind::Opaque => preserved_opaque_draws.push(draw),
            PipelineKind::Transparent => preserved_transparent_draws.push(draw),
        }
    }

    static_meshes.extend(preserved_static_meshes);

    spec.name = scene.label.clone().unwrap_or_else(|| scene.id.clone());
    spec.scene_space = SceneSpace::World3D;
    spec.camera_path =
        compile_scene_camera_path(scene, &visible_mesh_nodes).or_else(|| spec.camera_path.clone());
    spec.static_meshes = static_meshes;
    spec.draws = preserved_opaque_draws;
    spec.draws.extend(opaque_draws);
    spec.draws.extend(transparent_draws);
    spec.draws.extend(preserved_transparent_draws);
    spec.draws.extend(preserved_dynamic_draws);
    spec.lighting = compile_scene_lighting(scene, spec.lighting.as_ref());
    ensure_compiled_scene_textures(spec);
    Ok(())
}

fn maybe_compile_authored_scene<V>(
    spec: &mut SlideSpec<V>,
    manifest: &SlideManifest,
    package_root: &Path,
) -> Result<bool, LoadError>
where
    V: PackageMeshVertex,
{
    let has_scene_assets = manifest
        .assets
        .as_ref()
        .is_some_and(|assets| !assets.scenes.is_empty());
    if !has_scene_assets {
        return Ok(false);
    }
    if manifest.scene_space.as_deref() == Some("screen_2d") {
        return Err(LoadError::AssetLoad(
            "authored scene compilation only supports world_3d slides".into(),
        ));
    }

    let scene =
        load_authored_scene_from_manifest(manifest, package_root, None)?.ok_or_else(|| {
            LoadError::AssetLoad(
                "manifest declares scene assets but no scene could be selected".into(),
            )
        })?;
    let compile_profile = scene
        .compile_profile
        .as_deref()
        .unwrap_or(DEFAULT_WORLD_SCENE_COMPILE_PROFILE);
    if normalize_scene_token(compile_profile) != DEFAULT_WORLD_SCENE_COMPILE_PROFILE {
        return Err(LoadError::AssetLoad(format!(
            "scene '{}' requested unsupported compile_profile '{}'; expected '{}'",
            scene.id, compile_profile, DEFAULT_WORLD_SCENE_COMPILE_PROFILE
        )));
    }
    for warning in &scene.warnings {
        log::warn!("scene '{}': {}", scene.id, warning);
    }

    compile_authored_scene_into_spec(spec, &scene)?;
    Ok(true)
}

fn empty_world_scene_spec() -> SlideSpec<WorldVertex> {
    SlideSpec {
        name: "screen_backdrop_scene".into(),
        limits: Limits::pi4(),
        scene_space: SceneSpace::World3D,
        camera_path: None,
        shaders: None,
        overlay: None,
        font: None,
        textures_used: 0,
        textures: vec![],
        static_meshes: vec![],
        dynamic_meshes: vec![],
        draws: vec![],
        lighting: None,
    }
}

fn maybe_compile_screen_background_scene(
    manifest: &SlideManifest,
    package_root: &Path,
) -> Result<Option<ScreenBackgroundScene>, LoadError> {
    let has_scene_assets = manifest
        .assets
        .as_ref()
        .is_some_and(|assets| !assets.scenes.is_empty());
    if !has_scene_assets {
        return Ok(None);
    }

    let scene =
        load_authored_scene_from_manifest(manifest, package_root, None)?.ok_or_else(|| {
            LoadError::AssetLoad(
                "manifest declares scene assets but no scene could be selected".into(),
            )
        })?;
    let mut spec = empty_world_scene_spec();
    compile_authored_scene_into_spec(&mut spec, &scene)?;

    Ok(Some(ScreenBackgroundScene {
        spec,
        shader_source_hint: Some(ShaderSourceHint::DefaultWorldScene),
    }))
}

fn encode_mesh_asset(imported: &ImportedMesh) -> Result<Vec<u8>, LoadError> {
    postcard::to_stdvec(&MeshAsset {
        vertices: imported
            .vertices
            .iter()
            .map(|vertex| MeshAssetVertex {
                position: vertex.position,
                normal: vertex.normal.unwrap_or([0.0, 1.0, 0.0]),
                tex_coords: vertex.tex_coords.unwrap_or([0.0, 0.0]),
                color: vertex.color.unwrap_or([1.0, 1.0, 1.0, 1.0]),
            })
            .collect(),
        indices: imported.indices.clone(),
    })
    .map_err(|error| LoadError::AssetLoad(format!("failed to encode mesh asset: {error}")))
}

fn encode_scene_metadata(scene: &ImportedScene) -> Result<Vec<u8>, LoadError> {
    postcard::to_stdvec(&SceneAnchorSet {
        scene_id: scene.id.clone(),
        scene_label: scene.label.clone(),
        scene_name: scene.metadata.scene_name.clone(),
        anchors: scene
            .anchors
            .iter()
            .map(|anchor| SceneAnchor {
                id: anchor.id.clone(),
                label: anchor.label.clone(),
                node_name: anchor.node_name.clone(),
                tag: anchor.metadata.vzglyd_anchor.clone(),
                world_transform: anchor.world_transform,
            })
            .collect(),
    })
    .map_err(|error| LoadError::AssetLoad(format!("failed to encode scene metadata: {error}")))
}

fn build_host_mesh_asset_catalog(
    manifest: &SlideManifest,
    package_root: &Path,
) -> Result<HostMeshAssetCatalog, LoadError> {
    let Some(assets) = manifest.assets.as_ref() else {
        return Ok(HostMeshAssetCatalog::default());
    };
    if assets.meshes.is_empty() {
        return Ok(HostMeshAssetCatalog::default());
    }

    let loader = AssetLoader::new(package_root)?;
    let mut encoded_by_key = HashMap::with_capacity(assets.meshes.len());
    for mesh in &assets.meshes {
        let resolved = loader.resolve(&mesh.path)?;
        let runtime_key = mesh_runtime_key(mesh, &resolved);
        let imported = load_glb_mesh(&resolved)?;
        let encoded = encode_mesh_asset(&imported)?;
        if encoded_by_key
            .insert(runtime_key.clone(), encoded)
            .is_some()
        {
            return Err(LoadError::AssetLoad(format!(
                "mesh asset key '{runtime_key}' is declared more than once"
            )));
        }
    }
    Ok(HostMeshAssetCatalog {
        encoded_by_key: Arc::new(encoded_by_key),
    })
}

fn build_host_scene_metadata_catalog(
    manifest: &SlideManifest,
    package_root: &Path,
) -> Result<HostSceneMetadataCatalog, LoadError> {
    let Some(assets) = manifest.assets.as_ref() else {
        return Ok(HostSceneMetadataCatalog::default());
    };
    if assets.scenes.is_empty() {
        return Ok(HostSceneMetadataCatalog::default());
    }

    let loader = AssetLoader::new(package_root)?;
    let mut encoded_by_key = HashMap::with_capacity(assets.scenes.len());
    for scene_ref in &assets.scenes {
        let resolved = loader.resolve(&scene_ref.path)?;
        let imported = load_glb_scene(&resolved, Some(scene_ref))?;
        let runtime_key = imported.id.clone();
        let encoded = encode_scene_metadata(&imported)?;
        if encoded_by_key
            .insert(runtime_key.clone(), encoded)
            .is_some()
        {
            return Err(LoadError::AssetLoad(format!(
                "scene metadata key '{runtime_key}' is declared more than once"
            )));
        }
    }
    Ok(HostSceneMetadataCatalog {
        encoded_by_key: Arc::new(encoded_by_key),
    })
}

fn mesh_runtime_key(asset_ref: &AssetRef, resolved: &Path) -> String {
    asset_ref
        .id
        .clone()
        .or_else(|| asset_ref.label.clone())
        .or_else(|| {
            resolved
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| asset_ref.path.clone())
}

struct PackFile {
    archive_path: PathBuf,
    source_path: PathBuf,
    bytes: u64,
}

/// Wrap raw wire-format spec bytes in a minimal WASM module.
///
/// The returned bytes can be passed directly to `load_slide_from_wasm_bytes`.
/// `spec_bytes` must already carry the 1-byte wire version prefix (as produced
/// by `terrain_slide::serialized_spec()`).
#[cfg(test)]
pub fn make_spec_wasm_bytes(spec_bytes: &[u8]) -> Vec<u8> {
    let escaped: String = spec_bytes.iter().map(|b| format!("\\{b:02x}")).collect();
    let len = spec_bytes.len();
    let pages = (len / 65536) + 2;
    let wat_src = format!(
        "(module\n\
           (memory (export \"memory\") {pages})\n\
           (data (i32.const 0) \"{escaped}\")\n\
           (func (export \"vzglyd_abi_version\") (result i32) i32.const 1)\n\
           (func (export \"vzglyd_spec_ptr\") (result i32) i32.const 0)\n\
           (func (export \"vzglyd_spec_len\") (result i32) i32.const {len})\n\
           (func (export \"vzglyd_init\") (result i32) i32.const 0)\n\
           (func (export \"vzglyd_update\") (param f32) (result i32) i32.const 0)\n\
         )"
    );
    wat::parse_str(&wat_src).expect("compile WAT to WASM")
}

/// Load a slide spec from raw WASM bytes (e.g. embedded via `include_bytes!`).
/// No manifest is returned — suitable for built-in bundled slides.
#[cfg(test)]
pub fn load_slide_from_wasm_bytes<V>(wasm_bytes: &[u8]) -> Result<LoadedSpec<V>, LoadError>
where
    V: PackageMeshVertex,
{
    let engine = make_wasm_engine()?;
    let module =
        Module::new(&engine, wasm_bytes).map_err(|e| LoadError::WasmLoad(e.to_string()))?;
    let (loaded, _channel) = load_slide(
        &engine,
        &module,
        "test-slide",
        HostMeshAssetCatalog::default(),
        HostSceneMetadataCatalog::default(),
        None,
    )?;
    Ok(loaded)
}

pub fn load_slide_from_wasm<V>(
    wasm_path: &str,
    params_bytes: Option<&[u8]>,
) -> Result<(LoadedSpec<V>, SlideManifest), LoadError>
where
    V: PackageMeshVertex,
{
    const MAX_WASM_BYTES: u64 = 10 * 1024 * 1024;
    let entry = resolve_runtime_entry_paths(wasm_path)?;
    let meta = std::fs::metadata(&entry.wasm_path)
        .map_err(|_| LoadError::MissingWasm(entry.wasm_path.display().to_string()))?;
    if meta.len() > MAX_WASM_BYTES {
        return Err(LoadError::WasmLoad("wasm file exceeds size cap".into()));
    }

    let manifest = load_manifest(&entry.manifest_path)?;
    validate_declared_scene_assets(&manifest, &entry.package_root)?;
    log::info!(
        "Loading slide '{:?}' from {} (package_root={} version={:?} author={:?})",
        manifest.name,
        entry.wasm_path.display(),
        entry.package_root.display(),
        manifest.version,
        manifest.author
    );

    let engine = make_wasm_engine()?;
    let module = Module::from_file(&engine, &entry.wasm_path)
        .map_err(|e| LoadError::WasmLoad(e.to_string()))?;
    let host_mesh_assets = build_host_mesh_asset_catalog(&manifest, &entry.package_root)?;
    let host_scene_metadata = build_host_scene_metadata_catalog(&manifest, &entry.package_root)?;

    let slide_runtime_label = entry.wasm_path.display().to_string();
    let (mut loaded, channel) = load_slide(
        &engine,
        &module,
        &slide_runtime_label,
        host_mesh_assets,
        host_scene_metadata,
        params_bytes,
    )?;
    let screen_background_scene = if loaded.spec.scene_space == SceneSpace::Screen2D {
        maybe_compile_screen_background_scene(&manifest, &entry.package_root)?
    } else {
        None
    };
    let scene_compiled = if loaded.spec.scene_space == SceneSpace::Screen2D {
        false
    } else {
        maybe_compile_authored_scene(&mut loaded.spec, &manifest, &entry.package_root)?
    };
    apply_package_resource_overrides(
        &mut loaded.spec,
        &manifest,
        &entry.package_root,
        !scene_compiled,
    )?;
    loaded.shader_source_hint = (scene_compiled
        && loaded
            .spec
            .shaders
            .as_ref()
            .and_then(|shaders| {
                shaders
                    .fragment_wgsl
                    .as_deref()
                    .or(shaders.vertex_wgsl.as_deref())
            })
            .is_none())
    .then_some(ShaderSourceHint::DefaultWorldScene);
    loaded.screen_background_scene = screen_background_scene;

    let sidecar_path = entry.package_root.join(PACKAGE_SIDECAR_NAME);
    if sidecar_path.is_file() {
        let sidecar_bytes = std::fs::read(&sidecar_path).map_err(|error| {
            LoadError::WasmLoad(format!(
                "failed to read sidecar '{}': {error}",
                sidecar_path.display()
            ))
        })?;
        let sidecar_preopens = manifest
            .sidecar
            .as_ref()
            .map(|sidecar| sidecar.wasi_preopens.clone())
            .unwrap_or_default();
        let sidecar_runtime_label = sidecar_path.display().to_string();
        let sidecar = load_sidecar_with_config(
            &engine,
            &sidecar_bytes,
            channel,
            &sidecar_preopens,
            &sidecar_runtime_label,
        )?;
        if let Some(runtime) = loaded.runtime.as_mut() {
            runtime.attach_sidecar(sidecar);
        } else {
            log::warn!(
                "slide '{}' bundled '{}' but does not expose a runtime; sidecar ignored",
                entry.wasm_path.display(),
                sidecar_path.display()
            );
        }
    }

    Ok((loaded, manifest))
}

pub fn load_slide_from_archive<V>(
    archive_path: &str,
    params_bytes: Option<&[u8]>,
) -> Result<(LoadedSpec<V>, SlideManifest), LoadError>
where
    V: PackageMeshVertex,
{
    if !is_archive_path(Path::new(archive_path)) {
        return Err(LoadError::Archive(format!(
            "'{archive_path}' is not a .{PACKAGE_ARCHIVE_EXTENSION} archive"
        )));
    }
    load_slide_from_wasm(archive_path, params_bytes)
}

pub fn pack_slide_directory(
    package_root: &Path,
    output_path: &Path,
) -> Result<PackReport, LoadError> {
    if !package_root.is_dir() {
        return Err(LoadError::Archive(format!(
            "package root '{}' is not a directory",
            package_root.display()
        )));
    }

    prepare_package_directory_for_pack(package_root)?;

    let manifest_path = package_root.join(PACKAGE_MANIFEST_NAME);
    let manifest = load_manifest(&manifest_path)?;
    let pack_files = collect_pack_files(package_root, &manifest)?;

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| {
                LoadError::Archive(format!(
                    "failed to create output directory '{}': {error}",
                    parent.display()
                ))
            })?;
        }
    }

    let archive_file = File::create(output_path).map_err(|error| {
        LoadError::Archive(format!(
            "failed to create archive '{}': {error}",
            output_path.display()
        ))
    })?;
    let mut zip = ZipWriter::new(archive_file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    for pack_file in &pack_files {
        zip.start_file(pack_file.archive_path.to_string_lossy().as_ref(), options)
            .map_err(|error| {
                LoadError::Archive(format!(
                    "failed to add '{}' to archive '{}': {error}",
                    pack_file.archive_path.display(),
                    output_path.display()
                ))
            })?;
        let mut source = File::open(&pack_file.source_path).map_err(|error| {
            LoadError::Archive(format!(
                "failed to open package file '{}': {error}",
                pack_file.source_path.display()
            ))
        })?;
        std::io::copy(&mut source, &mut zip).map_err(|error| {
            LoadError::Archive(format!(
                "failed to write '{}' into archive '{}': {error}",
                pack_file.archive_path.display(),
                output_path.display()
            ))
        })?;
    }

    zip.finish().map_err(|error| {
        LoadError::Archive(format!(
            "failed to finalize archive '{}': {error}",
            output_path.display()
        ))
    })?;

    let content_bytes = pack_files.iter().map(|file| file.bytes).sum();
    let archive_bytes = std::fs::metadata(output_path)
        .map_err(|error| {
            LoadError::Archive(format!(
                "failed to stat archive '{}': {error}",
                output_path.display()
            ))
        })?
        .len();

    Ok(PackReport {
        output_path: output_path.to_path_buf(),
        content_bytes,
        archive_bytes,
    })
}

fn prepare_package_directory_for_pack(package_root: &Path) -> Result<(), LoadError> {
    let build_script = package_root.join("build.sh");
    if !build_script.is_file() {
        return Ok(());
    }

    let status = Command::new("bash")
        .arg("build.sh")
        .current_dir(package_root)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| {
            LoadError::Archive(format!(
                "failed to run build script '{}': {error}",
                build_script.display()
            ))
        })?;

    if status.success() {
        return Ok(());
    }

    Err(LoadError::Archive(format!(
        "build script '{}' exited with status {status}",
        build_script.display()
    )))
}

pub(crate) fn resolve_slide_entry_paths(path: &str) -> SlideEntryPaths {
    let input = Path::new(path);
    if path_points_to_package_dir(input) {
        SlideEntryPaths {
            package_root: input.to_path_buf(),
            manifest_path: input.join(PACKAGE_MANIFEST_NAME),
            wasm_path: input.join(PACKAGE_WASM_NAME),
        }
    } else {
        let wasm_path = input.to_path_buf();
        let package_root = wasm_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        SlideEntryPaths {
            package_root,
            manifest_path: wasm_path.with_extension("json"),
            wasm_path,
        }
    }
}

fn resolve_runtime_entry_paths(path: &str) -> Result<SlideEntryPaths, LoadError> {
    let input = Path::new(path);
    if is_archive_path(input) {
        let package_root = extract_archive_to_cache(input)?;
        Ok(SlideEntryPaths {
            package_root: package_root.clone(),
            manifest_path: package_root.join(PACKAGE_MANIFEST_NAME),
            wasm_path: package_root.join(PACKAGE_WASM_NAME),
        })
    } else {
        Ok(resolve_slide_entry_paths(path))
    }
}

fn path_points_to_package_dir(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_dir(),
        Err(_) => path.extension().and_then(|ext| ext.to_str()) != Some("wasm"),
    }
}

fn is_archive_path(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some(PACKAGE_ARCHIVE_EXTENSION)
}

fn collect_pack_files(
    package_root: &Path,
    manifest: &SlideManifest,
) -> Result<Vec<PackFile>, LoadError> {
    let asset_loader = AssetLoader::new(package_root)?;
    let mut files = BTreeMap::<PathBuf, PathBuf>::new();
    files.insert(
        PathBuf::from(PACKAGE_MANIFEST_NAME),
        asset_loader.resolve(PACKAGE_MANIFEST_NAME)?,
    );
    files.insert(
        PathBuf::from(PACKAGE_WASM_NAME),
        asset_loader.resolve(PACKAGE_WASM_NAME)?,
    );
    if package_root.join(PACKAGE_SIDECAR_NAME).is_file() {
        files.insert(
            PathBuf::from(PACKAGE_SIDECAR_NAME),
            asset_loader.resolve(PACKAGE_SIDECAR_NAME)?,
        );
    }

    if let Some(assets) = manifest.assets.as_ref() {
        for texture in &assets.textures {
            files.insert(
                PathBuf::from(&texture.path),
                asset_loader.resolve(&texture.path)?,
            );
        }
        for mesh in &assets.meshes {
            files.insert(PathBuf::from(&mesh.path), asset_loader.resolve(&mesh.path)?);
        }
        for scene in &assets.scenes {
            files.insert(
                PathBuf::from(&scene.path),
                asset_loader.resolve(&scene.path)?,
            );
        }
    }
    if let Some(shaders) = manifest.shaders.as_ref() {
        if let Some(vertex) = shaders.vertex.as_ref() {
            files.insert(PathBuf::from(vertex), asset_loader.resolve(vertex)?);
        }
        if let Some(fragment) = shaders.fragment.as_ref() {
            files.insert(PathBuf::from(fragment), asset_loader.resolve(fragment)?);
        }
    }

    files
        .into_iter()
        .map(|(archive_path, source_path)| {
            let bytes = std::fs::metadata(&source_path)
                .map_err(|error| {
                    LoadError::Archive(format!(
                        "failed to stat package file '{}': {error}",
                        source_path.display()
                    ))
                })?
                .len();
            Ok(PackFile {
                archive_path,
                source_path,
                bytes,
            })
        })
        .collect()
}

fn extract_archive_to_cache(archive_path: &Path) -> Result<PathBuf, LoadError> {
    let canonical_archive = std::fs::canonicalize(archive_path).map_err(|error| {
        LoadError::Archive(format!(
            "failed to resolve archive '{}': {error}",
            archive_path.display()
        ))
    })?;
    let cache_dir = std::env::temp_dir().join(ARCHIVE_CACHE_DIR_NAME);
    std::fs::create_dir_all(&cache_dir).map_err(|error| {
        LoadError::Archive(format!(
            "failed to create archive cache '{}': {error}",
            cache_dir.display()
        ))
    })?;
    let cache_key = archive_cache_key(&canonical_archive)?;
    let extracted_root = cache_dir.join(cache_key);

    if archive_extract_complete(&canonical_archive, &extracted_root)? {
        return Ok(extracted_root);
    }
    if extracted_root.exists() {
        let _ = std::fs::remove_dir_all(&extracted_root);
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging_root = cache_dir.join(format!(".extract-{}-{}", std::process::id(), nonce));
    std::fs::create_dir_all(&staging_root).map_err(|error| {
        LoadError::Archive(format!(
            "failed to create archive staging dir '{}': {error}",
            staging_root.display()
        ))
    })?;

    let extract_result = extract_archive_entries(&canonical_archive, &staging_root);
    if let Err(error) = extract_result {
        let _ = std::fs::remove_dir_all(&staging_root);
        return Err(error);
    }

    match std::fs::rename(&staging_root, &extracted_root) {
        Ok(()) => Ok(extracted_root),
        Err(_) if archive_extract_complete(&canonical_archive, &extracted_root)? => {
            let _ = std::fs::remove_dir_all(&staging_root);
            Ok(extracted_root)
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging_root);
            Err(LoadError::Archive(format!(
                "failed to finalize extracted archive '{}': {error}",
                canonical_archive.display()
            )))
        }
    }
}

fn extract_zip_entries<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    label: &str,
    output_root: &Path,
) -> Result<(), LoadError> {
    for idx in 0..archive.len() {
        let mut entry = archive.by_index(idx).map_err(|error| {
            LoadError::Archive(format!(
                "failed to read archive entry #{idx} from '{label}': {error}"
            ))
        })?;
        let relative_path = entry.enclosed_name().ok_or_else(|| {
            LoadError::Archive(format!("archive '{label}' contains an unsafe entry path"))
        })?;
        let output_path = output_root.join(relative_path);

        if entry.is_dir() {
            std::fs::create_dir_all(&output_path).map_err(|error| {
                LoadError::Archive(format!(
                    "failed to create extracted directory '{}': {error}",
                    output_path.display()
                ))
            })?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                LoadError::Archive(format!(
                    "failed to create extracted parent '{}': {error}",
                    parent.display()
                ))
            })?;
        }

        let mut output_file = File::create(&output_path).map_err(|error| {
            LoadError::Archive(format!(
                "failed to create extracted file '{}': {error}",
                output_path.display()
            ))
        })?;
        std::io::copy(&mut entry, &mut output_file).map_err(|error| {
            LoadError::Archive(format!(
                "failed to extract '{}' from '{label}': {error}",
                output_path.display()
            ))
        })?;
    }
    Ok(())
}

fn extract_archive_entries(archive_path: &Path, output_root: &Path) -> Result<(), LoadError> {
    let archive_file = File::open(archive_path).map_err(|error| {
        LoadError::Archive(format!(
            "failed to open archive '{}': {error}",
            archive_path.display()
        ))
    })?;
    let mut archive = ZipArchive::new(archive_file).map_err(|error| {
        LoadError::Archive(format!(
            "failed to read archive '{}': {error}",
            archive_path.display()
        ))
    })?;
    let label = archive_path.display().to_string();
    extract_zip_entries(&mut archive, &label, output_root)
}

pub fn extract_embedded_bytes_to_cache(bytes: &[u8]) -> Result<PathBuf, LoadError> {
    let cache_dir = std::env::temp_dir().join(ARCHIVE_CACHE_DIR_NAME);
    std::fs::create_dir_all(&cache_dir).map_err(|error| {
        LoadError::Archive(format!(
            "failed to create archive cache '{}': {error}",
            cache_dir.display()
        ))
    })?;

    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let cache_key = format!("builtin-{:016x}", hasher.finish());
    let extracted_root = cache_dir.join(&cache_key);

    if extracted_root.join(PACKAGE_MANIFEST_NAME).is_file()
        && extracted_root.join(PACKAGE_WASM_NAME).is_file()
    {
        return Ok(extracted_root);
    }
    if extracted_root.exists() {
        let _ = std::fs::remove_dir_all(&extracted_root);
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging_root = cache_dir.join(format!(".extract-builtin-{}-{}", std::process::id(), nonce));
    std::fs::create_dir_all(&staging_root).map_err(|error| {
        LoadError::Archive(format!(
            "failed to create staging dir '{}': {error}",
            staging_root.display()
        ))
    })?;

    let cursor = std::io::Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor).map_err(|error| {
        LoadError::Archive(format!("embedded loading slide corrupt: {error}"))
    })?;
    let extract_result = extract_zip_entries(&mut archive, "embedded", &staging_root);
    if let Err(error) = extract_result {
        let _ = std::fs::remove_dir_all(&staging_root);
        return Err(error);
    }

    match std::fs::rename(&staging_root, &extracted_root) {
        Ok(()) => Ok(extracted_root),
        Err(_)
            if extracted_root.join(PACKAGE_MANIFEST_NAME).is_file()
                && extracted_root.join(PACKAGE_WASM_NAME).is_file() =>
        {
            let _ = std::fs::remove_dir_all(&staging_root);
            Ok(extracted_root)
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging_root);
            Err(LoadError::Archive(format!(
                "failed to finalize embedded loading slide cache: {error}"
            )))
        }
    }
}

fn archive_extract_complete(archive_path: &Path, root: &Path) -> Result<bool, LoadError> {
    if !root.join(PACKAGE_MANIFEST_NAME).is_file() || !root.join(PACKAGE_WASM_NAME).is_file() {
        return Ok(false);
    }
    if archive_contains_entry(archive_path, PACKAGE_SIDECAR_NAME)? {
        return Ok(root.join(PACKAGE_SIDECAR_NAME).is_file());
    }
    Ok(true)
}

fn archive_contains_entry(archive_path: &Path, expected_name: &str) -> Result<bool, LoadError> {
    let archive_file = File::open(archive_path).map_err(|error| {
        LoadError::Archive(format!(
            "failed to open archive '{}': {error}",
            archive_path.display()
        ))
    })?;
    let mut archive = ZipArchive::new(archive_file).map_err(|error| {
        LoadError::Archive(format!(
            "failed to read archive '{}': {error}",
            archive_path.display()
        ))
    })?;
    for idx in 0..archive.len() {
        let entry = archive.by_index(idx).map_err(|error| {
            LoadError::Archive(format!(
                "failed to inspect archive entry #{idx} from '{}': {error}",
                archive_path.display()
            ))
        })?;
        if entry.name() == expected_name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn archive_cache_key(archive_path: &Path) -> Result<String, LoadError> {
    let metadata = std::fs::metadata(archive_path).map_err(|error| {
        LoadError::Archive(format!(
            "failed to stat archive '{}': {error}",
            archive_path.display()
        ))
    })?;
    let modified = metadata
        .modified()
        .unwrap_or(UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut hasher = DefaultHasher::new();
    archive_path.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    modified.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

pub(crate) fn load_manifest(manifest_path: &Path) -> Result<SlideManifest, LoadError> {
    let manifest_path_buf = manifest_path.to_path_buf();
    let manifest_path = manifest_path_buf.to_string_lossy().to_string();
    let manifest_str = std::fs::read_to_string(&manifest_path)
        .map_err(|_| LoadError::MissingManifest(manifest_path.clone()))?;
    let manifest: SlideManifest =
        serde_json::from_str(&manifest_str).map_err(|e| LoadError::ManifestParse(e.to_string()))?;
    let package_root = manifest_path_buf.parent().unwrap_or_else(|| Path::new("."));
    manifest
        .validate(package_root)
        .map_err(|e| LoadError::ManifestValidation(e.to_string()))?;
    Ok(manifest)
}

fn apply_package_resource_overrides<V>(
    spec: &mut SlideSpec<V>,
    manifest: &SlideManifest,
    package_root: &Path,
    allow_static_mesh_overrides: bool,
) -> Result<(), LoadError>
where
    V: PackageMeshVertex,
{
    if manifest.assets.is_none() && manifest.shaders.is_none() {
        return Ok(());
    }

    let loader = AssetLoader::new(package_root)?;
    apply_external_textures(spec, manifest, &loader)?;
    if allow_static_mesh_overrides {
        apply_external_static_meshes(spec, manifest, &loader)?;
    }
    apply_external_shaders(spec, manifest, &loader)?;
    Ok(())
}

fn validate_declared_scene_assets(
    manifest: &SlideManifest,
    package_root: &Path,
) -> Result<(), LoadError> {
    let Some(assets) = manifest.assets.as_ref() else {
        return Ok(());
    };
    if assets.scenes.is_empty() {
        return Ok(());
    }

    let loader = AssetLoader::new(package_root)?;
    for scene in &assets.scenes {
        loader.resolve(&scene.path)?;
    }
    Ok(())
}

fn apply_external_textures<V>(
    spec: &mut SlideSpec<V>,
    manifest: &SlideManifest,
    loader: &AssetLoader,
) -> Result<(), LoadError>
where
    V: PackageMeshVertex,
{
    let Some(assets) = manifest.assets.as_ref() else {
        return Ok(());
    };

    for (texture_index, asset_ref) in resolve_texture_override_targets(spec, &assets.textures)? {
        let texture_desc = &mut spec.textures[texture_index];
        let texture = loader.load_texture(&asset_ref.path, texture_desc)?;
        texture_desc.width = texture.width;
        texture_desc.height = texture.height;
        texture_desc.data = texture.data;
    }

    Ok(())
}

fn apply_external_static_meshes<V>(
    spec: &mut SlideSpec<V>,
    manifest: &SlideManifest,
    loader: &AssetLoader,
) -> Result<(), LoadError>
where
    V: PackageMeshVertex,
{
    let Some(assets) = manifest.assets.as_ref() else {
        return Ok(());
    };

    for (mesh_index, asset_ref) in resolve_static_mesh_override_targets(spec, &assets.meshes)? {
        let template = &spec.static_meshes[mesh_index];
        spec.static_meshes[mesh_index] = loader.load_static_mesh(&asset_ref.path, template)?;
    }

    Ok(())
}

fn asset_ref_targets_static_mesh(asset_ref: &AssetRef) -> bool {
    asset_ref.slot.is_some() || asset_ref.label.is_some() || asset_ref.id.is_none()
}

fn resolve_texture_override_targets<'a, V>(
    spec: &SlideSpec<V>,
    asset_refs: &'a [AssetRef],
) -> Result<Vec<(usize, &'a AssetRef)>, LoadError>
where
    V: PackageMeshVertex,
{
    let mut assignments = Vec::with_capacity(asset_refs.len());
    let mut claimed = vec![false; spec.textures.len()];
    let mut next_implicit = 0usize;

    for asset_ref in asset_refs {
        let target = if let Some(slot) = asset_ref.slot {
            if slot >= spec.textures.len() {
                return Err(LoadError::AssetLoad(format!(
                    "texture asset '{}' targets slot {} but slide spec only defines {} textures",
                    asset_ref.path,
                    slot,
                    spec.textures.len()
                )));
            }
            slot
        } else if let Some(label) = asset_ref.label.as_deref() {
            spec.textures
                .iter()
                .position(|texture| texture.label == label)
                .ok_or_else(|| {
                    LoadError::AssetLoad(format!(
                        "texture asset '{}' targets unknown texture label '{}'",
                        asset_ref.path, label
                    ))
                })?
        } else {
            while next_implicit < claimed.len() && claimed[next_implicit] {
                next_implicit += 1;
            }
            if next_implicit >= spec.textures.len() {
                return Err(LoadError::AssetLoad(format!(
                    "manifest declares {} implicit texture overrides but slide spec only defines {} textures",
                    asset_refs.len(),
                    spec.textures.len()
                )));
            }
            let slot = next_implicit;
            next_implicit += 1;
            slot
        };

        if claimed[target] {
            return Err(LoadError::AssetLoad(format!(
                "texture asset '{}' targets texture slot {} more than once",
                asset_ref.path, target
            )));
        }
        claimed[target] = true;
        assignments.push((target, asset_ref));
    }

    Ok(assignments)
}

fn resolve_static_mesh_override_targets<'a, V>(
    spec: &SlideSpec<V>,
    asset_refs: &'a [AssetRef],
) -> Result<Vec<(usize, &'a AssetRef)>, LoadError>
where
    V: PackageMeshVertex,
{
    let override_refs: Vec<&AssetRef> = asset_refs
        .iter()
        .filter(|asset_ref| asset_ref_targets_static_mesh(asset_ref))
        .collect();
    let mut assignments = Vec::with_capacity(override_refs.len());
    let mut claimed = vec![false; spec.static_meshes.len()];
    let mut next_implicit = 0usize;

    for asset_ref in override_refs {
        let target = if let Some(slot) = asset_ref.slot {
            if slot >= spec.static_meshes.len() {
                return Err(LoadError::AssetLoad(format!(
                    "mesh asset '{}' targets slot {} but slide spec only defines {} static meshes",
                    asset_ref.path,
                    slot,
                    spec.static_meshes.len()
                )));
            }
            slot
        } else if let Some(label) = asset_ref.label.as_deref() {
            spec.static_meshes
                .iter()
                .position(|mesh| mesh.label == label)
                .ok_or_else(|| {
                    LoadError::AssetLoad(format!(
                        "mesh asset '{}' targets unknown static mesh label '{}'",
                        asset_ref.path, label
                    ))
                })?
        } else {
            while next_implicit < claimed.len() && claimed[next_implicit] {
                next_implicit += 1;
            }
            if next_implicit >= spec.static_meshes.len() {
                return Err(LoadError::AssetLoad(format!(
                    "manifest declares {} implicit mesh overrides but slide spec only defines {} static meshes",
                    assignments.len() + 1,
                    spec.static_meshes.len()
                )));
            }
            let slot = next_implicit;
            next_implicit += 1;
            slot
        };

        if claimed[target] {
            return Err(LoadError::AssetLoad(format!(
                "mesh asset '{}' targets static mesh slot {} more than once",
                asset_ref.path, target
            )));
        }
        claimed[target] = true;
        assignments.push((target, asset_ref));
    }

    Ok(assignments)
}

fn apply_external_shaders<V>(
    spec: &mut SlideSpec<V>,
    manifest: &SlideManifest,
    loader: &AssetLoader,
) -> Result<(), LoadError>
where
    V: PackageMeshVertex,
{
    let Some(shader_paths) = manifest.shaders.as_ref() else {
        return Ok(());
    };

    let mut merged = spec.shaders.clone().unwrap_or(ShaderSources {
        vertex_wgsl: None,
        fragment_wgsl: None,
    });

    if let Some(vertex_path) = shader_paths.vertex.as_deref() {
        merged.vertex_wgsl = Some(loader.load_shader(vertex_path)?);
    }
    if let Some(fragment_path) = shader_paths.fragment.as_deref() {
        merged.fragment_wgsl = Some(loader.load_shader(fragment_path)?);
    }

    if merged.vertex_wgsl.is_some() || merged.fragment_wgsl.is_some() {
        spec.shaders = Some(merged);
    }

    Ok(())
}

// ── Slide instantiation ───────────────────────────────────────────────────────

/// Instantiate a slide module through the shared WASI linker, run `_start`
/// when present, then extract its spec and runtime exports.
fn load_slide<V>(
    engine: &Engine,
    module: &Module,
    runtime_label: &str,
    mesh_assets: HostMeshAssetCatalog,
    scene_metadata: HostSceneMetadataCatalog,
    params_bytes: Option<&[u8]>,
) -> Result<(LoadedSpec<V>, SlideChannel), LoadError>
where
    V: PackageMeshVertex,
{
    let mut wasi_builder = wasmtime_wasi::sync::WasiCtxBuilder::new();
    if log::log_enabled!(log::Level::Info) {
        wasi_builder.inherit_stdout().inherit_stderr();
    }
    let wasi = wasi_builder.build();
    let channel: SlideChannel = Arc::new(SlideMailbox::new());
    let mut store: Store<SlideStore> = Store::new(
        engine,
        SlideStore {
            wasi,
            rx: Arc::clone(&channel),
            label: runtime_label.to_string(),
            mesh_assets,
            scene_metadata,
        },
    );
    configure_runtime_store(&mut store);
    let mut linker: wasmtime::Linker<SlideStore> = wasmtime::Linker::new(engine);
    wasmtime_wasi::sync::add_to_linker(&mut linker, |s| &mut s.wasi)
        .map_err(|e| LoadError::WasmLoad(e.to_string()))?;
    add_asset_host_functions(&mut linker)?;
    linker
        .func_wrap(
            "vzglyd_host",
            "channel_poll",
            |mut caller: wasmtime::Caller<'_, SlideStore>, buf_ptr: i32, buf_len: i32| -> i32 {
                host_channel_poll(&mut caller, buf_ptr, buf_len)
            },
        )
        .map_err(|e| LoadError::WasmLoad(e.to_string()))?;
    linker
        .func_wrap(
            "vzglyd_host",
            "log_info",
            |mut caller: wasmtime::Caller<'_, SlideStore>, ptr: i32, len: i32| -> i32 {
                host_slide_log_info(&mut caller, ptr, len)
            },
        )
        .map_err(|e| LoadError::WasmLoad(e.to_string()))?;

    let instance = linker
        .instantiate(&mut store, module)
        .map_err(|e| LoadError::WasmLoad(e.to_string()))?;
    log::info!("slide:{} instantiated", store.data().label);

    // Run `_start` for WASI command-style modules so `main()` can populate the
    // spec buffer. `proc_exit(0)` appears as a `wasmtime_wasi::I32Exit` with
    // code `0`; treat that as the normal clean-exit path and continue.
    if let Ok(start) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
        log::info!("slide:{} invoking _start", store.data().label);
        match start.call(&mut store, ()) {
            Ok(()) => {}
            Err(e) => {
                let clean = e
                    .downcast_ref::<wasmtime_wasi::I32Exit>()
                    .map_or(false, |exit: &wasmtime_wasi::I32Exit| exit.0 == 0);
                if !clean {
                    return Err(LoadError::WasmLoad(format!("_start failed: {e}")));
                }
            }
        }
        log::info!("slide:{} completed _start", store.data().label);
    }

    let loaded = extract_spec(store, instance, params_bytes)?;
    Ok((loaded, channel))
}

// ── Generic spec extraction ───────────────────────────────────────────────────

fn extract_spec<V>(
    mut store: Store<SlideStore>,
    instance: Instance,
    params_bytes: Option<&[u8]>,
) -> Result<LoadedSpec<V>, LoadError>
where
    V: PackageMeshVertex,
{
    let spec = decode_spec(&mut store, instance.clone(), params_bytes)?;
    log::info!("slide:{} decoded immutable spec", store.data().label);
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or(LoadError::MissingExport("memory"))?;
    let update_fn = instance
        .get_typed_func::<f32, i32>(&mut store, "vzglyd_update")
        .ok();
    let teardown_fn = instance
        .get_typed_func::<(), i32>(&mut store, "vzglyd_teardown")
        .ok();
    let overlay_ptr = instance
        .get_typed_func::<(), i32>(&mut store, "vzglyd_overlay_ptr")
        .ok();
    let overlay_len = instance
        .get_typed_func::<(), i32>(&mut store, "vzglyd_overlay_len")
        .ok();
    let dynamic_meshes_ptr = instance
        .get_typed_func::<(), i32>(&mut store, "vzglyd_dynamic_meshes_ptr")
        .ok();
    let dynamic_meshes_len = instance
        .get_typed_func::<(), i32>(&mut store, "vzglyd_dynamic_meshes_len")
        .ok();
    log::info!(
        "slide:{} runtime exports update={} overlay={} dynamic_meshes={} teardown={}",
        store.data().label,
        update_fn.is_some(),
        overlay_ptr.is_some() && overlay_len.is_some(),
        dynamic_meshes_ptr.is_some() && dynamic_meshes_len.is_some(),
        teardown_fn.is_some()
    );

    Ok(LoadedSpec {
        spec,
        runtime: Some(SlideRuntime::new(
            store,
            instance,
            memory,
            update_fn,
            teardown_fn,
            overlay_ptr,
            overlay_len,
            dynamic_meshes_ptr,
            dynamic_meshes_len,
        )),
        shader_source_hint: None,
        screen_background_scene: None,
    })
}

/// Given an initialised instance, call the stable ABI exports, run `vzglyd_init`,
/// and decode the immutable slide spec from linear memory.
fn decode_spec<V>(
    store: &mut Store<SlideStore>,
    instance: Instance,
    params_bytes: Option<&[u8]>,
) -> Result<SlideSpec<V>, LoadError>
where
    V: PackageMeshVertex,
{
    let abi_fn = instance
        .get_typed_func::<(), u32>(&mut *store, "vzglyd_abi_version")
        .map_err(|_| LoadError::MissingExport("vzglyd_abi_version"))?;
    let abi_ver = abi_fn
        .call(&mut *store, ())
        .map_err(|e| LoadError::WasmLoad(e.to_string()))?;
    log::info!("slide:{} ABI version {abi_ver}", store.data().label);
    if abi_ver != ABI_VERSION {
        return Err(LoadError::AbiVersion {
            found: abi_ver,
            expected: ABI_VERSION,
        });
    }

    // Optional configure protocol: write JSON params before vzglyd_init.
    if let Some(params) = params_bytes {
        let ptr_fn = instance.get_typed_func::<(), i32>(&mut *store, "vzglyd_params_ptr").ok();
        let cap_fn = instance.get_typed_func::<(), u32>(&mut *store, "vzglyd_params_capacity").ok();
        let cfg_fn = instance.get_typed_func::<i32, i32>(&mut *store, "vzglyd_configure").ok();
        if let (Some(ptr_fn), Some(cap_fn), Some(cfg_fn)) = (ptr_fn, cap_fn, cfg_fn) {
            let capacity = cap_fn
                .call(&mut *store, ())
                .map_err(|e| LoadError::WasmLoad(e.to_string()))? as usize;
            let ptr = ptr_fn
                .call(&mut *store, ())
                .map_err(|e| LoadError::WasmLoad(e.to_string()))? as usize;
            let write_len = params.len().min(capacity);
            let memory = instance
                .get_memory(&mut *store, "memory")
                .ok_or(LoadError::MissingExport("memory"))?;
            memory
                .write(&mut *store, ptr, &params[..write_len])
                .map_err(|e| LoadError::WasmLoad(format!("write params failed: {e}")))?;
            let status = cfg_fn
                .call(&mut *store, write_len as i32)
                .map_err(|error| LoadError::WasmLoad(format!("vzglyd_configure failed: {error}")))?;
            log::info!("slide:{} vzglyd_configure({write_len}) -> {status}", store.data().label);
        }
    }

    if let Ok(init) = instance.get_typed_func::<(), i32>(&mut *store, "vzglyd_init") {
        log::info!("slide:{} invoking vzglyd_init", store.data().label);
        let status = init
            .call(&mut *store, ())
            .map_err(|error| LoadError::WasmLoad(format!("vzglyd_init failed: {error}")))?;
        log::info!("slide:{} vzglyd_init -> {status}", store.data().label);
    }

    let ptr_fn = instance
        .get_typed_func::<(), i32>(&mut *store, "vzglyd_spec_ptr")
        .map_err(|_| LoadError::MissingExport("vzglyd_spec_ptr"))?;
    let len_fn = instance
        .get_typed_func::<(), i32>(&mut *store, "vzglyd_spec_len")
        .map_err(|_| LoadError::MissingExport("vzglyd_spec_len"))?;
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or(LoadError::MissingExport("memory"))?;

    let ptr = ptr_fn
        .call(&mut *store, ())
        .map_err(|e| LoadError::WasmLoad(e.to_string()))? as usize;
    let len = len_fn
        .call(&mut *store, ())
        .map_err(|e| LoadError::WasmLoad(e.to_string()))? as usize;

    // Copy out before the store borrow changes shape.
    let data = memory
        .data(&*store)
        .get(ptr..ptr + len)
        .ok_or_else(|| LoadError::WasmLoad("spec slice out of bounds".into()))?
        .to_vec();

    if data.is_empty() {
        return Err(LoadError::SpecDecode("empty spec blob".into()));
    }
    let wire_ver = data[0];
    if wire_ver != 1 {
        return Err(LoadError::SpecDecode(format!(
            "unexpected wire version {wire_ver}"
        )));
    }
    postcard::from_bytes(&data[1..]).map_err(|e| LoadError::SpecDecode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ColorType, ImageFormat};
    use vzglyd_slide::{
        DrawSource, DrawSpec, Limits, PipelineKind, SceneAnchorSet, SceneSpace, SlideSpec,
        SpecError, StaticMesh,
    };
    use std::fs::File;
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipArchive, ZipWriter};

    fn temp_package_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("vzglyd_{label}_{}_{}", std::process::id(), unique));
        std::fs::create_dir_all(&path).expect("create temp package dir");
        path
    }

    fn relative_package_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let path =
            PathBuf::from("target").join(format!("vzglyd_{label}_{}_{}", std::process::id(), unique));
        std::fs::create_dir_all(&path).expect("create relative package dir");
        path
    }

    fn write_package_root(dir: &Path, manifest: &str, wasm: &[u8]) {
        std::fs::write(dir.join(PACKAGE_MANIFEST_NAME), manifest).expect("write manifest");
        std::fs::write(dir.join(PACKAGE_WASM_NAME), wasm).expect("write wasm");
    }

    fn reference_scene_anchor_wasm() -> Vec<u8> {
        std::fs::read("../vzglyd-courtyard/courtyard.wasm").expect("read courtyard wasm")
    }

    fn write_png(path: &Path, width: u32, height: u32, data: &[u8]) {
        image::save_buffer_with_format(
            path,
            data,
            width,
            height,
            ColorType::Rgba8,
            ImageFormat::Png,
        )
        .expect("write png");
    }

    fn fallback_texture_desc(width: u32, height: u32) -> TextureDesc {
        TextureDesc {
            label: "test".into(),
            width,
            height,
            format: TextureFormat::Rgba8Unorm,
            wrap_u: vzglyd_slide::WrapMode::ClampToEdge,
            wrap_v: vzglyd_slide::WrapMode::ClampToEdge,
            wrap_w: vzglyd_slide::WrapMode::ClampToEdge,
            mag_filter: vzglyd_slide::FilterMode::Nearest,
            min_filter: vzglyd_slide::FilterMode::Nearest,
            mip_filter: vzglyd_slide::FilterMode::Nearest,
            data: vec![0; width as usize * height as usize * 4],
        }
    }

    fn test_world_vertex(position: [f32; 3], color: [f32; 4]) -> WorldVertex {
        WorldVertex {
            position,
            normal: [0.0, 1.0, 0.0],
            color,
            mode: 0.0,
        }
    }

    fn make_scene_compile_base_spec(limits: Limits) -> SlideSpec<WorldVertex> {
        SlideSpec {
            name: "scene_compile_template".into(),
            limits,
            scene_space: SceneSpace::World3D,
            camera_path: None,
            shaders: None,
            overlay: None,
            font: None,
            textures_used: 0,
            textures: vec![],
            static_meshes: vec![],
            dynamic_meshes: vec![],
            draws: vec![],
            lighting: None,
        }
    }

    fn make_static_mesh_override_spec() -> SlideSpec<WorldVertex> {
        let mesh0 = StaticMesh {
            label: "course_mesh".into(),
            vertices: vec![
                test_world_vertex([0.0, 0.0, 0.0], [0.20, 0.40, 0.80, 1.0]),
                test_world_vertex([1.0, 0.0, 0.0], [0.20, 0.40, 0.80, 1.0]),
                test_world_vertex([0.0, 0.0, 1.0], [0.20, 0.40, 0.80, 1.0]),
            ],
            indices: vec![0, 1, 2],
        };
        let mesh1 = StaticMesh {
            label: "kart_body".into(),
            vertices: vec![
                test_world_vertex([0.0, 0.2, 0.0], [0.90, 0.22, 0.14, 1.0]),
                test_world_vertex([0.6, 0.2, 0.0], [0.90, 0.22, 0.14, 1.0]),
                test_world_vertex([0.0, 0.2, 0.6], [0.90, 0.22, 0.14, 1.0]),
            ],
            indices: vec![0, 1, 2],
        };

        SlideSpec {
            name: "mesh_override_test".into(),
            limits: Limits {
                max_vertices: 256,
                max_indices: 512,
                max_static_meshes: 4,
                max_dynamic_meshes: 0,
                max_textures: 0,
                max_texture_bytes: 0,
                max_texture_dim: 1,
            },
            scene_space: SceneSpace::World3D,
            camera_path: None,
            shaders: None,
            overlay: None,
            font: None,
            textures_used: 0,
            textures: vec![],
            static_meshes: vec![mesh0.clone(), mesh1.clone()],
            dynamic_meshes: vec![],
            draws: vec![
                DrawSpec {
                    label: "course_draw".into(),
                    source: DrawSource::Static(0),
                    pipeline: PipelineKind::Opaque,
                    index_range: 0..mesh0.indices.len() as u32,
                },
                DrawSpec {
                    label: "kart_draw".into(),
                    source: DrawSource::Static(1),
                    pipeline: PipelineKind::Opaque,
                    index_range: 0..mesh1.indices.len() as u32,
                },
            ],
            lighting: None,
        }
    }

    fn make_spec_wire_bytes<V>(spec: &SlideSpec<V>) -> Vec<u8>
    where
        V: Serialize + DeserializeOwned + bytemuck::Pod,
    {
        let mut wire = vec![1];
        wire.extend(postcard::to_stdvec(spec).expect("serialize test spec"));
        wire
    }

    fn make_triangle_glb(points: [[f32; 3]; 3], normal: [f32; 3]) -> Vec<u8> {
        fn push_f32(buf: &mut Vec<u8>, value: f32) {
            buf.extend_from_slice(&value.to_le_bytes());
        }

        fn vec3_json(values: [f32; 3]) -> String {
            format!("[{:.6},{:.6},{:.6}]", values[0], values[1], values[2])
        }

        let mut min = points[0];
        let mut max = points[0];
        for point in points.iter().skip(1) {
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }

        let mut bin = Vec::new();
        for point in points {
            for component in point {
                push_f32(&mut bin, component);
            }
        }
        for _ in 0..3 {
            for component in normal {
                push_f32(&mut bin, component);
            }
        }
        for index in [0_u16, 1, 2] {
            bin.extend_from_slice(&index.to_le_bytes());
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }

        let mut json = format!(
            concat!(
                "{{",
                "\"asset\":{{\"version\":\"2.0\"}},",
                "\"scene\":0,",
                "\"scenes\":[{{\"nodes\":[0]}}],",
                "\"nodes\":[{{\"mesh\":0}}],",
                "\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0,\"NORMAL\":1}},\"indices\":2,\"mode\":4}}]}}],",
                "\"buffers\":[{{\"byteLength\":{buffer_len}}}],",
                "\"bufferViews\":[",
                "{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":36}},",
                "{{\"buffer\":0,\"byteOffset\":36,\"byteLength\":36}},",
                "{{\"buffer\":0,\"byteOffset\":72,\"byteLength\":6}}",
                "],",
                "\"accessors\":[",
                "{{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"min\":{min},\"max\":{max}}},",
                "{{\"bufferView\":1,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\"}},",
                "{{\"bufferView\":2,\"componentType\":5123,\"count\":3,\"type\":\"SCALAR\"}}",
                "]",
                "}}"
            ),
            buffer_len = bin.len(),
            min = vec3_json(min),
            max = vec3_json(max),
        )
        .into_bytes();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }

        let total_len = 12 + 8 + json.len() + 8 + bin.len();
        let mut glb = Vec::with_capacity(total_len);
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(&(total_len as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        glb
    }

    fn append_vec3_f32(bin: &mut Vec<u8>, values: &[[f32; 3]]) -> (usize, usize) {
        let offset = bin.len();
        for value in values {
            for component in value {
                bin.extend_from_slice(&component.to_le_bytes());
            }
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        (offset, bin.len() - offset)
    }

    fn append_u16(bin: &mut Vec<u8>, values: &[u16]) -> (usize, usize) {
        let offset = bin.len();
        for value in values {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        (offset, bin.len() - offset)
    }

    fn vec3_bounds(points: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
        let mut min = points[0];
        let mut max = points[0];
        for point in points.iter().skip(1) {
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        (min, max)
    }

    fn vec3_json(values: [f32; 3]) -> String {
        format!("[{:.6},{:.6},{:.6}]", values[0], values[1], values[2])
    }

    fn encode_glb_chunks(mut json: Vec<u8>, mut bin: Vec<u8>) -> Vec<u8> {
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }

        let total_len = 12 + 8 + json.len() + 8 + bin.len();
        let mut glb = Vec::with_capacity(total_len);
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(&(total_len as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        glb
    }

    fn make_multi_node_scene_glb() -> Vec<u8> {
        make_multi_node_scene_glb_with_anchor(Some("spawn_marker"), Some("spawn"))
    }

    fn make_multi_node_scene_glb_with_anchor(
        anchor_id: Option<&str>,
        anchor_tag: Option<&str>,
    ) -> Vec<u8> {
        let ground_positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let ground_normals = [[0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let ground_indices = [0_u16, 1, 2];
        let tree_positions = [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let tree_indices = [0_u16, 1, 2];

        let mut bin = Vec::new();
        let (ground_pos_off, ground_pos_len) = append_vec3_f32(&mut bin, &ground_positions);
        let (ground_nrm_off, ground_nrm_len) = append_vec3_f32(&mut bin, &ground_normals);
        let (ground_idx_off, ground_idx_len) = append_u16(&mut bin, &ground_indices);
        let (tree_pos_off, tree_pos_len) = append_vec3_f32(&mut bin, &tree_positions);
        let (tree_idx_off, tree_idx_len) = append_u16(&mut bin, &tree_indices);
        let (ground_min, ground_max) = vec3_bounds(&ground_positions);
        let (tree_min, tree_max) = vec3_bounds(&tree_positions);
        let mut anchor_extras = Vec::new();
        if let Some(anchor_tag) = anchor_tag {
            anchor_extras.push(format!("\"vzglyd_anchor\":\"{anchor_tag}\""));
        }
        if let Some(anchor_id) = anchor_id {
            anchor_extras.push(format!("\"vzglyd_id\":\"{anchor_id}\""));
        }
        let anchor_extras = anchor_extras.join(",");

        let json = format!(
            concat!(
                "{{",
                "\"asset\":{{\"version\":\"2.0\"}},",
                "\"scene\":0,",
                "\"scenes\":[{{\"name\":\"WorldScene\",\"extras\":{{\"set\":\"test_scene\"}},\"nodes\":[0]}}],",
                "\"nodes\":[",
                "{{\"name\":\"Root\",\"children\":[1,2,3,4,5]}},",
                "{{\"name\":\"Ground\",\"mesh\":0,\"extras\":{{\"vzglyd_id\":\"ground_mesh\",\"vzglyd_material\":\"opaque\"}}}},",
                "{{\"name\":\"Tree\",\"mesh\":1,\"translation\":[2.0,0.0,1.0],\"extras\":{{\"vzglyd_id\":\"tree_mesh\",\"vzglyd_billboard\":true,\"vzglyd_pipeline\":\"transparent\"}}}},",
                "{{\"name\":\"OverviewCamera\",\"camera\":0,\"translation\":[0.0,2.0,4.0],\"extras\":{{\"vzglyd_id\":\"overview\",\"vzglyd_entry_camera\":true}}}},",
                "{{\"name\":\"SpawnAnchor\",\"translation\":[3.0,0.0,2.0],\"extras\":{{{anchor_extras}}}}},",
                "{{\"name\":\"IgnoredEmpty\",\"translation\":[5.0,0.0,0.0]}}",
                "],",
                "\"cameras\":[{{\"name\":\"SceneCamera\",\"type\":\"perspective\",\"perspective\":{{\"aspectRatio\":1.777778,\"yfov\":0.7,\"znear\":0.1,\"zfar\":100.0}}}}],",
                "\"materials\":[",
                "{{\"name\":\"opaque\",\"extras\":{{\"vzglyd_material\":\"opaque\"}},\"pbrMetallicRoughness\":{{\"baseColorFactor\":[1.0,0.8,0.6,1.0]}}}},",
                "{{\"name\":\"water\",\"extras\":{{\"vzglyd_material\":\"water\"}},\"pbrMetallicRoughness\":{{\"baseColorFactor\":[0.2,0.4,1.0,0.8]}}}}",
                "],",
                "\"meshes\":[",
                "{{\"name\":\"GroundMesh\",\"primitives\":[{{\"attributes\":{{\"POSITION\":0,\"NORMAL\":1}},\"indices\":2,\"material\":0,\"mode\":4}}]}},",
                "{{\"name\":\"TreeMesh\",\"primitives\":[{{\"attributes\":{{\"POSITION\":3}},\"indices\":4,\"material\":1,\"mode\":4}}]}}",
                "],",
                "\"buffers\":[{{\"byteLength\":{buffer_len}}}],",
                "\"bufferViews\":[",
                "{{\"buffer\":0,\"byteOffset\":{ground_pos_off},\"byteLength\":{ground_pos_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{ground_nrm_off},\"byteLength\":{ground_nrm_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{ground_idx_off},\"byteLength\":{ground_idx_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{tree_pos_off},\"byteLength\":{tree_pos_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{tree_idx_off},\"byteLength\":{tree_idx_len}}}",
                "],",
                "\"accessors\":[",
                "{{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"min\":{ground_min},\"max\":{ground_max}}},",
                "{{\"bufferView\":1,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\"}},",
                "{{\"bufferView\":2,\"componentType\":5123,\"count\":3,\"type\":\"SCALAR\"}},",
                "{{\"bufferView\":3,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"min\":{tree_min},\"max\":{tree_max}}},",
                "{{\"bufferView\":4,\"componentType\":5123,\"count\":3,\"type\":\"SCALAR\"}}",
                "]",
                "}}"
            ),
            buffer_len = bin.len(),
            ground_pos_off = ground_pos_off,
            ground_pos_len = ground_pos_len,
            ground_nrm_off = ground_nrm_off,
            ground_nrm_len = ground_nrm_len,
            ground_idx_off = ground_idx_off,
            ground_idx_len = ground_idx_len,
            tree_pos_off = tree_pos_off,
            tree_pos_len = tree_pos_len,
            tree_idx_off = tree_idx_off,
            tree_idx_len = tree_idx_len,
            ground_min = vec3_json(ground_min),
            ground_max = vec3_json(ground_max),
            tree_min = vec3_json(tree_min),
            tree_max = vec3_json(tree_max),
            anchor_extras = anchor_extras,
        )
        .into_bytes();

        encode_glb_chunks(json, bin)
    }

    fn make_multi_node_scene_glb_with_directional_light() -> Vec<u8> {
        let ground_positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let ground_normals = [[0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let ground_indices = [0_u16, 1, 2];
        let tree_positions = [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let tree_indices = [0_u16, 1, 2];

        let mut bin = Vec::new();
        let (ground_pos_off, ground_pos_len) = append_vec3_f32(&mut bin, &ground_positions);
        let (ground_nrm_off, ground_nrm_len) = append_vec3_f32(&mut bin, &ground_normals);
        let (ground_idx_off, ground_idx_len) = append_u16(&mut bin, &ground_indices);
        let (tree_pos_off, tree_pos_len) = append_vec3_f32(&mut bin, &tree_positions);
        let (tree_idx_off, tree_idx_len) = append_u16(&mut bin, &tree_indices);
        let (ground_min, ground_max) = vec3_bounds(&ground_positions);
        let (tree_min, tree_max) = vec3_bounds(&tree_positions);

        let json = format!(
            concat!(
                "{{",
                "\"asset\":{{\"version\":\"2.0\"}},",
                "\"extensionsUsed\":[\"KHR_lights_punctual\"],",
                "\"extensions\":{{",
                "\"KHR_lights_punctual\":{{",
                "\"lights\":[{{\"name\":\"Sun\",\"type\":\"directional\",\"color\":[0.9,0.8,0.7],\"intensity\":1.5}}]",
                "}}",
                "}},",
                "\"scene\":0,",
                "\"scenes\":[{{\"name\":\"WorldScene\",\"extras\":{{\"set\":\"test_scene\"}},\"nodes\":[0]}}],",
                "\"nodes\":[",
                "{{\"name\":\"Root\",\"children\":[1,2,3,4,5,6]}},",
                "{{\"name\":\"Ground\",\"mesh\":0,\"extras\":{{\"vzglyd_id\":\"ground_mesh\",\"vzglyd_material\":\"opaque\"}}}},",
                "{{\"name\":\"Tree\",\"mesh\":1,\"translation\":[2.0,0.0,1.0],\"extras\":{{\"vzglyd_id\":\"tree_mesh\",\"vzglyd_billboard\":true,\"vzglyd_pipeline\":\"transparent\"}}}},",
                "{{\"name\":\"OverviewCamera\",\"camera\":0,\"translation\":[0.0,2.0,4.0],\"extras\":{{\"vzglyd_id\":\"overview\",\"vzglyd_entry_camera\":true}}}},",
                "{{\"name\":\"SpawnAnchor\",\"translation\":[3.0,0.0,2.0],\"extras\":{{\"vzglyd_anchor\":\"spawn\",\"vzglyd_id\":\"spawn_marker\"}}}},",
                "{{\"name\":\"IgnoredEmpty\",\"translation\":[5.0,0.0,0.0]}},",
                "{{\"name\":\"SunNode\",\"extensions\":{{\"KHR_lights_punctual\":{{\"light\":0}}}}}}",
                "],",
                "\"cameras\":[{{\"name\":\"SceneCamera\",\"type\":\"perspective\",\"perspective\":{{\"aspectRatio\":1.777778,\"yfov\":0.7,\"znear\":0.1,\"zfar\":100.0}}}}],",
                "\"materials\":[",
                "{{\"name\":\"opaque\",\"extras\":{{\"vzglyd_material\":\"opaque\"}},\"pbrMetallicRoughness\":{{\"baseColorFactor\":[1.0,0.8,0.6,1.0]}}}},",
                "{{\"name\":\"water\",\"extras\":{{\"vzglyd_material\":\"water\"}},\"pbrMetallicRoughness\":{{\"baseColorFactor\":[0.2,0.4,1.0,0.8]}}}}",
                "],",
                "\"meshes\":[",
                "{{\"name\":\"GroundMesh\",\"primitives\":[{{\"attributes\":{{\"POSITION\":0,\"NORMAL\":1}},\"indices\":2,\"material\":0,\"mode\":4}}]}},",
                "{{\"name\":\"TreeMesh\",\"primitives\":[{{\"attributes\":{{\"POSITION\":3}},\"indices\":4,\"material\":1,\"mode\":4}}]}}",
                "],",
                "\"buffers\":[{{\"byteLength\":{buffer_len}}}],",
                "\"bufferViews\":[",
                "{{\"buffer\":0,\"byteOffset\":{ground_pos_off},\"byteLength\":{ground_pos_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{ground_nrm_off},\"byteLength\":{ground_nrm_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{ground_idx_off},\"byteLength\":{ground_idx_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{tree_pos_off},\"byteLength\":{tree_pos_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{tree_idx_off},\"byteLength\":{tree_idx_len}}}",
                "],",
                "\"accessors\":[",
                "{{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"min\":{ground_min},\"max\":{ground_max}}},",
                "{{\"bufferView\":1,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\"}},",
                "{{\"bufferView\":2,\"componentType\":5123,\"count\":3,\"type\":\"SCALAR\"}},",
                "{{\"bufferView\":3,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"min\":{tree_min},\"max\":{tree_max}}},",
                "{{\"bufferView\":4,\"componentType\":5123,\"count\":3,\"type\":\"SCALAR\"}}",
                "]",
                "}}"
            ),
            buffer_len = bin.len(),
            ground_pos_off = ground_pos_off,
            ground_pos_len = ground_pos_len,
            ground_nrm_off = ground_nrm_off,
            ground_nrm_len = ground_nrm_len,
            ground_idx_off = ground_idx_off,
            ground_idx_len = ground_idx_len,
            tree_pos_off = tree_pos_off,
            tree_pos_len = tree_pos_len,
            tree_idx_off = tree_idx_off,
            tree_idx_len = tree_idx_len,
            ground_min = vec3_json(ground_min),
            ground_max = vec3_json(ground_max),
            tree_min = vec3_json(tree_min),
            tree_max = vec3_json(tree_max),
        )
        .into_bytes();

        encode_glb_chunks(json, bin)
    }

    fn make_runtime_test_wasm(spec_bytes: &[u8]) -> Vec<u8> {
        let escaped: String = spec_bytes
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect();
        let len = spec_bytes.len();
        let pages = (len / 65536) + 2;
        let wat_src = format!(
            "(module\n\
               (memory (export \"memory\") {pages})\n\
               (data (i32.const 0) \"{escaped}\")\n\
               (func (export \"vzglyd_abi_version\") (result i32) i32.const 1)\n\
               (func (export \"vzglyd_spec_ptr\") (result i32) i32.const 0)\n\
               (func (export \"vzglyd_spec_len\") (result i32) i32.const {len})\n\
               (func (export \"vzglyd_init\") (result i32) i32.const 0)\n\
               (func (export \"vzglyd_update\") (param f32) (result i32)\n\
                 i32.const 0\n\
                 i32.const 0\n\
                 i32.store8\n\
                 i32.const 1)\n\
             )"
        );
        wat::parse_str(&wat_src).expect("compile runtime test WAT to WASM")
    }

    fn make_teardown_test_wasm(spec_bytes: &[u8], teardown_body: &str) -> Vec<u8> {
        let escaped: String = spec_bytes
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect();
        let len = spec_bytes.len();
        let pages = (len / 65536) + 2;
        let wat_src = format!(
            "(module\n\
               (memory (export \"memory\") {pages})\n\
               (data (i32.const 0) \"{escaped}\")\n\
               (func (export \"vzglyd_abi_version\") (result i32) i32.const 1)\n\
               (func (export \"vzglyd_spec_ptr\") (result i32) i32.const 0)\n\
               (func (export \"vzglyd_spec_len\") (result i32) i32.const {len})\n\
               (func (export \"vzglyd_init\") (result i32) i32.const 0)\n\
               (func (export \"vzglyd_update\") (param f32) (result i32) i32.const 0)\n\
               (func (export \"vzglyd_teardown\") (result i32)\n\
                 {teardown_body}\n\
               )\n\
             )"
        );
        wat::parse_str(&wat_src).expect("compile teardown test WAT to WASM")
    }

    fn make_mesh_asset_host_test_wasm(spec_bytes: &[u8], key: &str) -> Vec<u8> {
        let escaped_spec: String = spec_bytes
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect();
        let escaped_key: String = key
            .as_bytes()
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect();
        let len = spec_bytes.len();
        let spec_offset = 4096;
        let key_offset = 256;
        let out_offset = 8192;
        let pages = ((out_offset + len) / 65536) + 2;
        let wat_src = format!(
            "(module\n\
               (import \"vzglyd_host\" \"mesh_asset_len\" (func $mesh_asset_len (param i32 i32) (result i32)))\n\
               (import \"vzglyd_host\" \"mesh_asset_read\" (func $mesh_asset_read (param i32 i32 i32 i32) (result i32)))\n\
               (memory (export \"memory\") {pages})\n\
               (data (i32.const {key_offset}) \"{escaped_key}\")\n\
               (data (i32.const {spec_offset}) \"{escaped_spec}\")\n\
               (func (export \"vzglyd_abi_version\") (result i32) i32.const 1)\n\
               (func (export \"vzglyd_spec_ptr\") (result i32) i32.const {spec_offset})\n\
               (func (export \"vzglyd_spec_len\") (result i32) i32.const {len})\n\
               (func (export \"vzglyd_init\") (result i32)\n\
                 (local $len i32)\n\
                 (local $status i32)\n\
                 i32.const {key_offset}\n\
                 i32.const {key_len}\n\
                 call $mesh_asset_len\n\
                 local.set $len\n\
                 i32.const 0\n\
                 local.get $len\n\
                 i32.store\n\
                 i32.const {key_offset}\n\
                 i32.const {key_len}\n\
                 i32.const {out_offset}\n\
                 local.get $len\n\
                 call $mesh_asset_read\n\
                 local.set $status\n\
                 i32.const 4\n\
                 local.get $status\n\
                 i32.store\n\
                 i32.const 0)\n\
               (func (export \"vzglyd_update\") (param f32) (result i32) i32.const 0)\n\
             )",
            key_len = key.len(),
        );
        wat::parse_str(&wat_src).expect("compile mesh asset host test WAT to WASM")
    }

    fn make_channel_poll_empty_test_wasm(spec_bytes: &[u8]) -> Vec<u8> {
        let escaped_spec: String = spec_bytes
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect();
        let len = spec_bytes.len();
        let marker_offset = 64;
        let buffer_offset = 4096;
        let spec_offset = 8192;
        let pages = ((spec_offset + len) / 65536) + 2;
        let wat_src = format!(
            "(module\n\
               (import \"vzglyd_host\" \"channel_poll\" (func $channel_poll (param i32 i32) (result i32)))\n\
               (memory (export \"memory\") {pages})\n\
               (data (i32.const {spec_offset}) \"{escaped_spec}\")\n\
               (func (export \"vzglyd_abi_version\") (result i32) i32.const 1)\n\
               (func (export \"vzglyd_spec_ptr\") (result i32) i32.const {spec_offset})\n\
               (func (export \"vzglyd_spec_len\") (result i32) i32.const {len})\n\
               (func (export \"vzglyd_init\") (result i32)\n\
                 i32.const {buffer_offset}\n\
                 i32.const 32\n\
                 call $channel_poll\n\
                 i32.const -3\n\
                 i32.eq\n\
                 if\n\
                   i32.const {marker_offset}\n\
                   i32.const 1\n\
                   i32.store8\n\
                 end\n\
                 i32.const 0)\n\
               (func (export \"vzglyd_update\") (param f32) (result i32) i32.const 0)\n\
             )"
        );
        wat::parse_str(&wat_src).expect("compile channel poll test WAT to WASM")
    }

    fn make_channel_poll_runtime_wasm(spec_bytes: &[u8], buf_len: usize) -> Vec<u8> {
        let escaped_spec: String = spec_bytes
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect();
        let len = spec_bytes.len();
        let spec_offset = 4096;
        let buffer_offset = 8192;
        let pages = ((buffer_offset + buf_len + len) / 65536) + 2;
        let wat_src = format!(
            "(module\n\
               (import \"vzglyd_host\" \"channel_poll\" (func $channel_poll (param i32 i32) (result i32)))\n\
               (memory (export \"memory\") {pages})\n\
               (data (i32.const {spec_offset}) \"{escaped_spec}\")\n\
               (func (export \"vzglyd_abi_version\") (result i32) i32.const 1)\n\
               (func (export \"vzglyd_spec_ptr\") (result i32) i32.const {spec_offset})\n\
               (func (export \"vzglyd_spec_len\") (result i32) i32.const {len})\n\
               (func (export \"vzglyd_init\") (result i32) i32.const 0)\n\
               (func (export \"vzglyd_update\") (param f32) (result i32)\n\
                 i32.const {buffer_offset}\n\
                 i32.const {buf_len}\n\
                 call $channel_poll)\n\
             )"
        );
        wat::parse_str(&wat_src).expect("compile channel poll runtime WAT to WASM")
    }

    fn make_sidecar_push_test_wasm(message: &[u8]) -> Vec<u8> {
        let escaped_message: String = message.iter().map(|byte| format!("\\{byte:02x}")).collect();
        let len = message.len();
        let wat_src = format!(
            "(module\n\
               (import \"vzglyd_host\" \"channel_push\" (func $channel_push (param i32 i32) (result i32)))\n\
               (memory (export \"memory\") 1)\n\
               (data (i32.const 0) \"{escaped_message}\")\n\
               (func (export \"vzglyd_sidecar_run\") (result i32)\n\
                 i32.const 0\n\
                 i32.const {len}\n\
                 call $channel_push\n\
                 drop\n\
                 i32.const 0)\n\
             )"
        );
        wat::parse_str(&wat_src).expect("compile sidecar push WAT to WASM")
    }

    fn make_sidecar_poll_and_log_test_wasm(message: &[u8], log_message: &str) -> Vec<u8> {
        let escaped_message: String = message.iter().map(|byte| format!("\\{byte:02x}")).collect();
        let escaped_log: String = log_message
            .as_bytes()
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect();
        let message_len = message.len();
        let log_len = log_message.len();
        let log_offset = message_len;
        let wat_src = format!(
            "(module\n\
               (import \"vzglyd_host\" \"channel_push\" (func $channel_push (param i32 i32) (result i32)))\n\
               (import \"vzglyd_host\" \"channel_poll\" (func $channel_poll (param i32 i32) (result i32)))\n\
               (import \"vzglyd_host\" \"log_info\" (func $log_info (param i32 i32) (result i32)))\n\
               (memory (export \"memory\") 1)\n\
               (data (i32.const 0) \"{escaped_message}\")\n\
               (data (i32.const {log_offset}) \"{escaped_log}\")\n\
               (func (export \"vzglyd_sidecar_run\") (result i32)\n\
                 i32.const 32\n\
                 i32.const 16\n\
                 call $channel_poll\n\
                 drop\n\
                 i32.const {log_offset}\n\
                 i32.const {log_len}\n\
                 call $log_info\n\
                 drop\n\
                 i32.const 0\n\
                 i32.const {message_len}\n\
                 call $channel_push\n\
                 drop\n\
                 i32.const 0)\n\
             )"
        );
        wat::parse_str(&wat_src).expect("compile sidecar poll/log WAT to WASM")
    }

    fn make_sidecar_multi_push_test_wasm(messages: &[&[u8]]) -> Vec<u8> {
        let mut data_offset = 0usize;
        let mut data_segments = String::new();
        let mut push_calls = String::new();
        for message in messages {
            let escaped_message: String =
                message.iter().map(|byte| format!("\\{byte:02x}")).collect();
            data_segments.push_str(&format!(
                "  (data (i32.const {data_offset}) \"{escaped_message}\")\n"
            ));
            push_calls.push_str(&format!(
                "    i32.const {data_offset}\n    i32.const {}\n    call $channel_push\n    drop\n",
                message.len()
            ));
            data_offset += message.len();
        }

        let wat_src = format!(
            "(module\n\
               (import \"vzglyd_host\" \"channel_push\" (func $channel_push (param i32 i32) (result i32)))\n\
               (memory (export \"memory\") 1)\n\
               {data_segments}\
               (func (export \"vzglyd_sidecar_run\") (result i32)\n\
{push_calls}\
                 i32.const 0)\n\
             )"
        );
        wat::parse_str(&wat_src).expect("compile sidecar multi-push WAT to WASM")
    }

    fn make_socket_sidecar_test_wasm(ip: [u8; 4], port: u16) -> Vec<u8> {
        let address = [
            0u8,
            0,
            (port >> 8) as u8,
            (port & 0xff) as u8,
            ip[0],
            ip[1],
            ip[2],
            ip[3],
        ];
        let escaped_address: String = address.iter().map(|byte| format!("\\{byte:02x}")).collect();
        let wat_src = format!(
            "(module\n\
               (import \"wasi_snapshot_preview1\" \"sock_open\" (func $sock_open (param i32 i32 i32 i32) (result i32)))\n\
               (import \"wasi_snapshot_preview1\" \"sock_connect\" (func $sock_connect (param i32 i32 i32) (result i32)))\n\
               (import \"wasi_snapshot_preview1\" \"sock_send\" (func $sock_send (param i32 i32 i32 i32 i32) (result i32)))\n\
               (import \"wasi_snapshot_preview1\" \"sock_recv\" (func $sock_recv (param i32 i32 i32 i32 i32 i32) (result i32)))\n\
               (import \"wasi_snapshot_preview1\" \"sock_shutdown\" (func $sock_shutdown (param i32 i32) (result i32)))\n\
               (import \"vzglyd_host\" \"channel_push\" (func $channel_push (param i32 i32) (result i32)))\n\
               (memory (export \"memory\") 1)\n\
               (data (i32.const 0) \"{escaped_address}\")\n\
               (data (i32.const 32) \"ping\")\n\
               (data (i32.const 64) \"\\20\\00\\00\\00\\04\\00\\00\\00\")\n\
               (data (i32.const 72) \"\\80\\00\\00\\00\\04\\00\\00\\00\")\n\
               (func (export \"vzglyd_sidecar_run\") (result i32)\n\
                 (local $fd i32)\n\
                 i32.const 2\n\
                 i32.const 1\n\
                 i32.const 0\n\
                 i32.const 96\n\
                 call $sock_open\n\
                 drop\n\
                 i32.const 96\n\
                 i32.load\n\
                 local.set $fd\n\
                 local.get $fd\n\
                 i32.const 0\n\
                 i32.const 8\n\
                 call $sock_connect\n\
                 drop\n\
                 local.get $fd\n\
                 i32.const 64\n\
                 i32.const 1\n\
                 i32.const 0\n\
                 i32.const 100\n\
                 call $sock_send\n\
                 drop\n\
                 local.get $fd\n\
                 i32.const 72\n\
                 i32.const 1\n\
                 i32.const 0\n\
                 i32.const 104\n\
                 i32.const 108\n\
                 call $sock_recv\n\
                 drop\n\
                 i32.const 128\n\
                 i32.const 4\n\
                 call $channel_push\n\
                 drop\n\
                 local.get $fd\n\
                 i32.const 2\n\
                 call $sock_shutdown\n\
                 drop\n\
                 i32.const 0)\n\
             )"
        );
        wat::parse_str(&wat_src).expect("compile socket sidecar WAT to WASM")
    }

    /// A WASM that exports vzglyd_abi_version = 99 must produce AbiVersion error.
    #[test]
    fn loader_rejects_bad_abi() {
        let wasm = wat::parse_str(
            r#"(module
                 (memory (export "memory") 1)
                 (func (export "vzglyd_abi_version") (result i32) i32.const 99)
                 (func (export "vzglyd_spec_ptr")    (result i32) i32.const 0)
                 (func (export "vzglyd_spec_len")    (result i32) i32.const 0)
               )"#,
        )
        .unwrap();
        let err = load_slide_from_wasm_bytes::<WorldVertex>(&wasm)
            .err()
            .expect("should fail");
        assert!(
            matches!(
                err,
                LoadError::AbiVersion {
                    found: 99,
                    expected: 1
                }
            ),
            "expected AbiVersion error, got {err}"
        );
    }

    #[test]
    fn directory_entries_resolve_to_canonical_package_names() {
        let paths = resolve_slide_entry_paths("slides/terrain");

        assert_eq!(paths.package_root, PathBuf::from("slides/terrain"));
        assert_eq!(
            paths.manifest_path,
            PathBuf::from("slides/terrain/manifest.json")
        );
        assert_eq!(paths.wasm_path, PathBuf::from("slides/terrain/slide.wasm"));
    }

    #[test]
    fn bare_wasm_entries_keep_legacy_manifest_resolution() {
        let paths = resolve_slide_entry_paths("slides/terrain/terrain_slide.wasm");

        assert_eq!(paths.package_root, PathBuf::from("slides/terrain"));
        assert_eq!(
            paths.manifest_path,
            PathBuf::from("slides/terrain/terrain_slide.json")
        );
        assert_eq!(
            paths.wasm_path,
            PathBuf::from("slides/terrain/terrain_slide.wasm")
        );
    }

    #[test]
    fn directory_path_loads_slide_package() {
        const PATH: &str = "slides/terrain";
        if !std::path::Path::new(PATH).exists() {
            eprintln!("skip: {PATH} not found");
            return;
        }

        let (loaded, manifest) =
            load_slide_from_wasm::<WorldVertex>(PATH, None).expect("load terrain package");
        let spec = loaded.spec;
        spec.validate().expect("spec should pass Pi4 budget check");
        assert_eq!(spec.name, "terrain_scene");
        assert_eq!(manifest.name.as_deref(), Some("Terrain (Rust)"));
    }

    #[test]
    fn courtyard_package_loads_and_validates() {
        const PATH: &str = "slides/courtyard";
        if !std::path::Path::new(PATH).exists() {
            eprintln!("skip: {PATH} not found");
            return;
        }

        let (mut loaded, manifest) =
            load_slide_from_wasm::<WorldVertex>(PATH, None).expect("load courtyard package");
        loaded
            .spec
            .validate()
            .expect("courtyard package should satisfy Pi 4 limits");
        assert_eq!(loaded.spec.name, "Courtyard");
        assert_eq!(loaded.spec.scene_space, SceneSpace::World3D);
        assert_eq!(loaded.spec.static_meshes.len(), 4);
        assert_eq!(loaded.shader_source_hint, None);
        assert!(
            loaded
                .spec
                .shaders
                .as_ref()
                .and_then(|shaders| shaders.fragment_wgsl.as_deref())
                .is_some_and(|shader| shader.contains("mood_fog_color")),
            "courtyard package should load its custom moody sky shader"
        );
        assert_eq!(
            manifest
                .assets
                .as_ref()
                .expect("assets")
                .scenes
                .first()
                .expect("scene asset")
                .id
                .as_deref(),
            Some("courtyard")
        );
        let authored_path = loaded
            .spec
            .camera_path
            .as_ref()
            .expect("authored camera path");
        assert!(authored_path.looped);
        assert_eq!(authored_path.keyframes.len(), 4);
        assert!(
            authored_path.keyframes[0]
                .position
                .iter()
                .zip([0.0_f32, 3.6, 7.2])
                .all(|(actual, expected)| (actual - expected).abs() < 1e-4),
            "courtyard camera should open from the authored overview shot"
        );
        assert_eq!(
            authored_path.keyframes[3].position,
            authored_path.keyframes[0].position
        );
        let authored_camera = &authored_path.keyframes[0];
        assert!(
            authored_camera.target[1] < authored_camera.position[1]
                && authored_camera.target[2] < authored_camera.position[2],
            "courtyard camera should pitch downward into the courtyard"
        );
        let ground = loaded
            .spec
            .static_meshes
            .get(1)
            .expect("compiled ground mesh should exist");
        let i0 = ground.indices[0] as usize;
        let i1 = ground.indices[1] as usize;
        let i2 = ground.indices[2] as usize;
        let p0 = glam::Vec3::from(ground.vertices[i0].position);
        let p1 = glam::Vec3::from(ground.vertices[i1].position);
        let p2 = glam::Vec3::from(ground.vertices[i2].position);
        assert!(
            (p1 - p0).cross(p2 - p0).y > 0.0,
            "reference ground should face upward for the default world camera"
        );

        let runtime_meshes = loaded
            .runtime
            .as_mut()
            .expect("courtyard package should keep runtime")
            .read_dynamic_meshes::<WorldVertex>()
            .expect("read authored anchor mesh")
            .expect("courtyard package should export anchored runtime geometry");
        let mesh = runtime_meshes.meshes.first().expect("runtime mesh");
        assert_eq!(mesh.mesh_index, 0);
        assert!(
            mesh.vertices[3].position[0].abs() < 1e-5
                && (mesh.vertices[3].position[1] - 1.42).abs() < 1e-5
                && (mesh.vertices[3].position[2] + 7.4).abs() < 1e-5
        );
    }

    #[test]
    fn courtyard_package_round_trips_through_archive() {
        const PATH: &str = "slides/courtyard";
        if !std::path::Path::new(PATH).exists() {
            eprintln!("skip: {PATH} not found");
            return;
        }

        let archive_path = temp_package_dir("courtyard_archive").join("courtyard.vzglyd");
        pack_slide_directory(Path::new(PATH), &archive_path).expect("pack courtyard archive");

        let (mut dir_loaded, dir_manifest) =
            load_slide_from_wasm::<WorldVertex>(PATH, None).expect("load directory package");
        let (mut archive_loaded, archive_manifest) =
            load_slide_from_archive::<WorldVertex>(
                archive_path.to_string_lossy().as_ref(),
                None,
            )
            .expect("load archive package");

        assert_eq!(archive_manifest.name, dir_manifest.name);
        assert_eq!(
            archive_manifest
                .assets
                .as_ref()
                .expect("archive assets")
                .scenes
                .first()
                .expect("archive scene")
                .entry_camera
                .as_deref(),
            None
        );
        assert_eq!(archive_loaded.spec.name, dir_loaded.spec.name);
        assert_eq!(archive_loaded.spec.scene_space, dir_loaded.spec.scene_space);

        let dir_meshes = dir_loaded
            .runtime
            .as_mut()
            .expect("directory runtime")
            .read_dynamic_meshes::<WorldVertex>()
            .expect("read directory runtime meshes")
            .expect("directory runtime mesh payload");
        let archive_meshes = archive_loaded
            .runtime
            .as_mut()
            .expect("archive runtime")
            .read_dynamic_meshes::<WorldVertex>()
            .expect("read archive runtime meshes")
            .expect("archive runtime mesh payload");
        assert_eq!(archive_meshes.meshes.len(), dir_meshes.meshes.len());
        assert_eq!(
            archive_meshes.meshes[0].vertices.len(),
            dir_meshes.meshes[0].vertices.len()
        );
        assert!(
            (archive_meshes.meshes[0].vertices[3].position[0]
                - dir_meshes.meshes[0].vertices[3].position[0])
                .abs()
                < 1e-5
        );
    }

    #[cfg(unix)]
    #[test]
    fn asset_loader_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let package_dir = temp_package_dir("asset_loader_symlink");
        let assets_dir = package_dir.join("assets");
        std::fs::create_dir_all(&assets_dir).expect("create assets dir");

        let outside_path = package_dir.with_extension("outside");
        std::fs::write(&outside_path, "not inside package").expect("write outside file");
        symlink(&outside_path, assets_dir.join("escape.wgsl")).expect("create symlink");

        let loader = AssetLoader::new(&package_dir).expect("create asset loader");
        let error = loader
            .load_shader("assets/escape.wgsl")
            .expect_err("symlink escape should fail");

        assert!(
            error.to_string().contains("escapes the package root"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn scene_metadata_catalog_exposes_anchor_sets_to_runtime() {
        let package_dir = temp_package_dir("scene_metadata_catalog");
        let assets_dir = package_dir.join("assets");
        std::fs::create_dir_all(&assets_dir).expect("create assets dir");
        std::fs::write(assets_dir.join("world.glb"), make_multi_node_scene_glb())
            .expect("write world scene");
        std::fs::write(
            package_dir.join(PACKAGE_MANIFEST_NAME),
            r#"{
                "name":"Scene Metadata Test",
                "abi_version":1,
                "scene_space":"world_3d",
                "assets":{
                    "scenes":[
                        {
                            "path":"assets/world.glb",
                            "id":"hero_world",
                            "label":"Hero World",
                            "entry_camera":"overview",
                            "compile_profile":"default_world"
                        }
                    ]
                }
            }"#,
        )
        .expect("write manifest");
        let manifest =
            load_manifest(&package_dir.join(PACKAGE_MANIFEST_NAME)).expect("load manifest");

        let catalog = build_host_scene_metadata_catalog(&manifest, &package_dir)
            .expect("build metadata catalog");
        let encoded = catalog
            .encoded_by_key
            .get("hero_world")
            .expect("scene metadata runtime key");
        let anchors: SceneAnchorSet =
            postcard::from_bytes(encoded).expect("decode scene anchor set");

        assert_eq!(anchors.scene_id, "hero_world");
        assert_eq!(anchors.scene_label.as_deref(), Some("Hero World"));
        assert_eq!(anchors.scene_name.as_deref(), Some("WorldScene"));
        assert_eq!(anchors.anchors.len(), 1);
        assert_eq!(anchors.anchors[0].id, "spawn_marker");
        assert_eq!(anchors.anchors[0].tag.as_deref(), Some("spawn"));
        assert_eq!(anchors.anchors[0].translation(), [3.0, 0.0, 2.0]);
    }

    #[test]
    fn scene_assets_compile_into_world_spec_with_default_shader_hint() {
        let package_dir = temp_package_dir("scene_compile_spec");
        let assets_dir = package_dir.join("assets");
        std::fs::create_dir_all(&assets_dir).expect("create assets dir");
        std::fs::write(assets_dir.join("world.glb"), make_multi_node_scene_glb())
            .expect("write world scene");

        let spec = make_scene_compile_base_spec(Limits {
            max_vertices: 256,
            max_indices: 512,
            max_static_meshes: 4,
            max_dynamic_meshes: 0,
            max_textures: 4,
            max_texture_bytes: 256 * 1024,
            max_texture_dim: 512,
        });
        let wasm = make_spec_wasm_bytes(&make_spec_wire_bytes(&spec));
        write_package_root(
            &package_dir,
            r#"{
                "name":"Scene Compile Test",
                "abi_version":1,
                "scene_space":"world_3d",
                "assets":{
                    "scenes":[
                        {
                            "path":"assets/world.glb",
                            "id":"hero_world",
                            "label":"Hero World",
                            "entry_camera":"overview",
                            "compile_profile":"default_world"
                        }
                    ]
                }
            }"#,
            &wasm,
        );

        let (loaded, _) =
            load_slide_from_wasm::<WorldVertex>(package_dir.to_string_lossy().as_ref(), None)
                .expect("load compiled scene package");
        let spec = loaded.spec;

        assert_eq!(
            loaded.shader_source_hint,
            Some(ShaderSourceHint::DefaultWorldScene)
        );
        spec.validate()
            .expect("compiled scene spec should satisfy Pi-style limits");
        assert_eq!(spec.name, "Hero World");
        assert_eq!(spec.scene_space, SceneSpace::World3D);
        assert_eq!(spec.static_meshes.len(), 2);
        assert_eq!(spec.draws.len(), 2);
        assert_eq!(spec.draws[0].pipeline, PipelineKind::Opaque);
        assert_eq!(spec.draws[1].pipeline, PipelineKind::Transparent);
        assert_eq!(spec.textures.len(), 4);
        assert_eq!(spec.textures[1].width, 1);
        assert_eq!(spec.textures[1].height, 1);
        assert_eq!(spec.textures[1].data, vec![128, 128, 128, 255]);
        assert!(
            spec.static_meshes[0]
                .vertices
                .iter()
                .all(|vertex| vertex.mode == 0.0),
            "opaque scene meshes should compile to opaque shading mode"
        );
        assert!(
            spec.static_meshes[1]
                .vertices
                .iter()
                .all(|vertex| vertex.mode == 5.0),
            "water scene meshes should compile to water shading mode"
        );

        let camera_path = spec
            .camera_path
            .as_ref()
            .expect("compiled scene should select an entry camera");
        assert_eq!(camera_path.keyframes.len(), 2);
        assert_eq!(camera_path.keyframes[0].position, [0.0, 2.0, 4.0]);
        assert_eq!(camera_path.keyframes[0].target, [0.0, 2.0, 3.0]);
    }

    #[test]
    fn scene_compilation_threads_directional_light_into_spec() {
        let package_dir = temp_package_dir("scene_compile_light");
        let assets_dir = package_dir.join("assets");
        std::fs::create_dir_all(&assets_dir).expect("create assets dir");
        std::fs::write(
            assets_dir.join("world.glb"),
            make_multi_node_scene_glb_with_directional_light(),
        )
        .expect("write world scene");

        let spec = make_scene_compile_base_spec(Limits::pi4());
        let wasm = make_spec_wasm_bytes(&make_spec_wire_bytes(&spec));
        write_package_root(
            &package_dir,
            r#"{
                "name":"Scene Compile Light Test",
                "abi_version":1,
                "scene_space":"world_3d",
                "assets":{
                    "scenes":[
                        {
                            "path":"assets/world.glb",
                            "id":"hero_world",
                            "label":"Hero World",
                            "entry_camera":"overview",
                            "compile_profile":"default_world"
                        }
                    ]
                }
            }"#,
            &wasm,
        );

        let (loaded, _) =
            load_slide_from_wasm::<WorldVertex>(package_dir.to_string_lossy().as_ref(), None)
                .expect("load compiled scene package");
        let lighting = loaded.spec.lighting.expect("compiled scene lighting");
        let directional = lighting
            .directional_light
            .expect("compiled scene directional light");

        assert_eq!(lighting.ambient_color, [1.0, 1.0, 1.0]);
        assert_eq!(lighting.ambient_intensity, 0.22);
        assert_eq!(directional.direction, [0.0, 0.0, 1.0]);
        assert_eq!(directional.color, [0.9, 0.8, 0.7]);
        assert_eq!(directional.intensity, 1.5);
    }

    #[test]
    fn reference_slide_reads_authored_anchor_into_runtime_mesh() {
        let package_dir = temp_package_dir("courtyard");
        let assets_dir = package_dir.join("assets");
        std::fs::create_dir_all(&assets_dir).expect("create assets dir");
        std::fs::write(assets_dir.join("world.glb"), make_multi_node_scene_glb())
            .expect("write world scene");

        write_package_root(
            &package_dir,
            r#"{
                "name":"Courtyard",
                "abi_version":1,
                "scene_space":"world_3d",
                "assets":{
                    "scenes":[
                        {
                            "path":"assets/world.glb",
                            "id":"courtyard",
                            "label":"Courtyard",
                            "entry_camera":"overview",
                            "compile_profile":"default_world"
                        }
                    ]
                }
            }"#,
            &reference_scene_anchor_wasm(),
        );

        let (mut loaded, _) =
            load_slide_from_wasm::<WorldVertex>(package_dir.to_string_lossy().as_ref(), None)
                .expect("load authored-anchor courtyard slide");
        loaded
            .spec
            .validate()
            .expect("compiled courtyard slide should validate");
        assert_eq!(
            loaded.shader_source_hint,
            Some(ShaderSourceHint::DefaultWorldScene)
        );

        let runtime_meshes = loaded
            .runtime
            .as_mut()
            .expect("courtyard slide should keep runtime")
            .read_dynamic_meshes::<WorldVertex>()
            .expect("read authored anchor marker")
            .expect("courtyard slide should export an anchored runtime mesh");
        let mesh = runtime_meshes.meshes.first().expect("marker mesh");

        assert_eq!(mesh.mesh_index, 0);
        assert_eq!(mesh.index_count, 12);
        assert!(
            (mesh.vertices[0].position[0] - 2.82).abs() < 1e-5
                && mesh.vertices[0].position[1].abs() < 1e-5
                && (mesh.vertices[0].position[2] - 1.82).abs() < 1e-5
        );
        assert!(
            (mesh.vertices[3].position[0] - 3.0).abs() < 1e-5
                && (mesh.vertices[3].position[1] - 0.7).abs() < 1e-5
                && (mesh.vertices[3].position[2] - 2.0).abs() < 1e-5
        );
    }

    #[test]
    fn missing_anchor_keys_fail_predictably_during_reference_slide_init() {
        let package_dir = temp_package_dir("scene_anchor_missing");
        let assets_dir = package_dir.join("assets");
        std::fs::create_dir_all(&assets_dir).expect("create assets dir");
        std::fs::write(
            assets_dir.join("world.glb"),
            make_multi_node_scene_glb_with_anchor(Some("other_anchor"), Some("spawn")),
        )
        .expect("write world scene");

        write_package_root(
            &package_dir,
            r#"{
                "name":"Scene Anchor Missing",
                "abi_version":1,
                "scene_space":"world_3d",
                "assets":{
                    "scenes":[
                        {
                            "path":"assets/world.glb",
                            "id":"hero_world",
                            "label":"Hero World",
                            "entry_camera":"overview",
                            "compile_profile":"default_world"
                        }
                    ]
                }
            }"#,
            &reference_scene_anchor_wasm(),
        );

        let error = match load_slide_from_wasm::<WorldVertex>(
            package_dir.to_string_lossy().as_ref(),
            None,
        ) {
            Ok(_) => panic!("missing authored anchor should fail"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("vzglyd_init failed"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn scene_compilation_still_uses_spec_budget_validation() {
        let package_dir = temp_package_dir("scene_compile_budget");
        let assets_dir = package_dir.join("assets");
        std::fs::create_dir_all(&assets_dir).expect("create assets dir");
        std::fs::write(assets_dir.join("world.glb"), make_multi_node_scene_glb())
            .expect("write world scene");

        let spec = make_scene_compile_base_spec(Limits {
            max_vertices: 256,
            max_indices: 512,
            max_static_meshes: 1,
            max_dynamic_meshes: 0,
            max_textures: 4,
            max_texture_bytes: 256 * 1024,
            max_texture_dim: 512,
        });
        let wasm = make_spec_wasm_bytes(&make_spec_wire_bytes(&spec));
        write_package_root(
            &package_dir,
            r#"{
                "name":"Scene Compile Budget Test",
                "abi_version":1,
                "scene_space":"world_3d",
                "assets":{
                    "scenes":[
                        {"path":"assets/world.glb","id":"hero_world"}
                    ]
                }
            }"#,
            &wasm,
        );

        let (loaded, _) =
            load_slide_from_wasm::<WorldVertex>(package_dir.to_string_lossy().as_ref(), None)
                .expect("load compiled scene package");
        let error = loaded
            .spec
            .validate()
            .expect_err("compiled scene should still fail static mesh budget validation");

        assert!(matches!(
            error,
            SpecError::StaticMeshesExceeded { count: 2, max: 1 }
        ));
    }

    #[test]
    fn package_mesh_overrides_can_target_slots_and_labels() {
        let package_dir = temp_package_dir("external_mesh_selectors");
        let assets_dir = package_dir.join("assets");
        std::fs::create_dir_all(&assets_dir).expect("create assets dir");

        let spec = make_static_mesh_override_spec();
        let wasm = make_spec_wasm_bytes(&make_spec_wire_bytes(&spec));
        let course_glb = make_triangle_glb(
            [[2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 0.0, 1.0]],
            [0.0, 1.0, 0.0],
        );
        let kart_glb = make_triangle_glb(
            [[-1.0, 0.4, 0.0], [-0.2, 0.4, 0.0], [-1.0, 0.4, 0.8]],
            [0.0, 1.0, 0.0],
        );

        std::fs::write(assets_dir.join("course.glb"), &course_glb).expect("write course glb");
        std::fs::write(assets_dir.join("kart.glb"), &kart_glb).expect("write kart glb");

        let manifest = r#"{
            "name":"Mesh Override Test",
            "abi_version":1,
            "scene_space":"world_3d",
            "assets":{
                "meshes":[
                    {"path":"assets/course.glb","slot":0},
                    {"path":"assets/kart.glb","label":"kart_body"}
                ]
            }
        }"#;
        write_package_root(&package_dir, manifest, &wasm);

        let (loaded, _) =
            load_slide_from_wasm::<WorldVertex>(package_dir.to_string_lossy().as_ref(), None)
                .expect("load package with targeted mesh overrides");
        let spec = loaded.spec;

        assert_eq!(spec.static_meshes[0].vertices[0].position, [2.0, 0.0, 0.0]);
        assert_eq!(spec.static_meshes[1].vertices[0].position, [-1.0, 0.4, 0.0]);
        assert_eq!(
            spec.static_meshes[0].vertices[0].color,
            [0.20, 0.40, 0.80, 1.0]
        );
        assert_eq!(
            spec.static_meshes[1].vertices[0].color,
            [0.90, 0.22, 0.14, 1.0]
        );
        assert_eq!(spec.static_meshes[0].indices, vec![0, 1, 2]);
        assert_eq!(spec.static_meshes[1].indices, vec![0, 1, 2]);
    }

    #[test]
    fn id_only_mesh_assets_are_runtime_visible_without_overriding_static_meshes() {
        const RUNTIME_KEY: &str = "runtime_kart";
        const OUT_OFFSET: usize = 8192;

        let package_dir = temp_package_dir("runtime_mesh_asset");
        let assets_dir = package_dir.join("assets");
        std::fs::create_dir_all(&assets_dir).expect("create assets dir");

        let spec = make_static_mesh_override_spec();
        let wasm = make_mesh_asset_host_test_wasm(&make_spec_wire_bytes(&spec), RUNTIME_KEY);
        let runtime_glb = make_triangle_glb(
            [[4.0, 0.5, 0.0], [4.8, 0.5, 0.0], [4.0, 0.5, 0.7]],
            [0.0, 1.0, 0.0],
        );
        std::fs::write(assets_dir.join("runtime.glb"), &runtime_glb).expect("write runtime glb");
        let expected_encoded = encode_mesh_asset(
            &load_glb_mesh(&assets_dir.join("runtime.glb")).expect("load runtime glb"),
        )
        .expect("encode runtime mesh asset");

        write_package_root(
            &package_dir,
            &format!(
                r#"{{
                    "name":"Runtime Mesh Asset Test",
                    "abi_version":1,
                    "scene_space":"world_3d",
                    "assets":{{
                        "meshes":[
                            {{"path":"assets/runtime.glb","id":"{RUNTIME_KEY}"}}
                        ]
                    }}
                }}"#
            ),
            &wasm,
        );

        let (mut loaded, _) =
            load_slide_from_wasm::<WorldVertex>(package_dir.to_string_lossy().as_ref(), None)
                .expect("load package with runtime mesh asset");

        assert_eq!(
            loaded.spec.static_meshes[0].vertices[0].position,
            [0.0, 0.0, 0.0]
        );
        assert_eq!(
            loaded.spec.static_meshes[1].vertices[0].position,
            [0.0, 0.2, 0.0]
        );

        let runtime = loaded
            .runtime
            .as_mut()
            .expect("runtime should be preserved");
        let len_bytes = [
            runtime.memory_byte(0).expect("len byte 0"),
            runtime.memory_byte(1).expect("len byte 1"),
            runtime.memory_byte(2).expect("len byte 2"),
            runtime.memory_byte(3).expect("len byte 3"),
        ];
        let status_bytes = [
            runtime.memory_byte(4).expect("status byte 0"),
            runtime.memory_byte(5).expect("status byte 1"),
            runtime.memory_byte(6).expect("status byte 2"),
            runtime.memory_byte(7).expect("status byte 3"),
        ];
        let len = i32::from_le_bytes(len_bytes);
        let status = i32::from_le_bytes(status_bytes);
        assert_eq!(len as usize, expected_encoded.len());
        assert_eq!(status as usize, expected_encoded.len());
        for (offset, byte) in expected_encoded.iter().enumerate() {
            assert_eq!(
                runtime.memory_byte(OUT_OFFSET + offset),
                Some(*byte),
                "runtime asset byte mismatch at offset {offset}"
            );
        }
    }

    #[test]
    fn pack_slide_directory_runs_build_script_before_archiving() {
        let package_dir = temp_package_dir("pack_build_script");
        let built_dir = package_dir.join("built");
        std::fs::create_dir_all(&built_dir).expect("create built dir");

        let manifest = r#"{
            "name":"Built Package",
            "abi_version":1,
            "scene_space":"screen_2d"
        }"#;
        let wasm = b"fresh-slide-wasm";
        let sidecar = b"fresh-sidecar-wasm";
        std::fs::write(built_dir.join(PACKAGE_MANIFEST_NAME), manifest).expect("write manifest");
        std::fs::write(built_dir.join(PACKAGE_WASM_NAME), wasm).expect("write wasm");
        std::fs::write(built_dir.join(PACKAGE_SIDECAR_NAME), sidecar).expect("write sidecar");

        let build_script = r#"#!/usr/bin/env bash
set -euo pipefail
cp built/manifest.json manifest.json
cp built/slide.wasm slide.wasm
cp built/sidecar.wasm sidecar.wasm
"#;
        std::fs::write(package_dir.join("build.sh"), build_script).expect("write build script");

        let archive_path = package_dir.join("built.vzglyd");
        pack_slide_directory(&package_dir, &archive_path).expect("pack archive");

        let archive_file = File::open(&archive_path).expect("open archive");
        let mut archive = ZipArchive::new(archive_file).expect("read zip archive");

        let mut archived_manifest = String::new();
        archive
            .by_name(PACKAGE_MANIFEST_NAME)
            .expect("manifest entry")
            .read_to_string(&mut archived_manifest)
            .expect("read archived manifest");
        assert!(archived_manifest.contains("\"Built Package\""));

        let mut archived_wasm = Vec::new();
        archive
            .by_name(PACKAGE_WASM_NAME)
            .expect("slide entry")
            .read_to_end(&mut archived_wasm)
            .expect("read archived wasm");
        assert_eq!(archived_wasm, wasm);

        let mut archived_sidecar = Vec::new();
        archive
            .by_name(PACKAGE_SIDECAR_NAME)
            .expect("sidecar entry")
            .read_to_end(&mut archived_sidecar)
            .expect("read archived sidecar");
        assert_eq!(archived_sidecar, sidecar);
    }

    #[test]
    fn pack_slide_directory_runs_build_script_for_relative_package_paths() {
        let package_dir = relative_package_dir("pack_build_script_relative");
        let built_dir = package_dir.join("built");
        std::fs::create_dir_all(&built_dir).expect("create built dir");

        let manifest = r#"{
            "name":"Relative Built Package",
            "abi_version":1,
            "scene_space":"screen_2d"
        }"#;
        let wasm = b"relative-slide-wasm";
        std::fs::write(built_dir.join(PACKAGE_MANIFEST_NAME), manifest).expect("write manifest");
        std::fs::write(built_dir.join(PACKAGE_WASM_NAME), wasm).expect("write wasm");

        let build_script = r#"#!/usr/bin/env bash
set -euo pipefail
cp built/manifest.json manifest.json
cp built/slide.wasm slide.wasm
"#;
        std::fs::write(package_dir.join("build.sh"), build_script).expect("write build script");

        let archive_path = package_dir.join("relative-built.vzglyd");
        pack_slide_directory(&package_dir, &archive_path).expect("pack archive");

        let archive_file = File::open(&archive_path).expect("open archive");
        let mut archive = ZipArchive::new(archive_file).expect("read zip archive");

        let mut archived_manifest = String::new();
        archive
            .by_name(PACKAGE_MANIFEST_NAME)
            .expect("manifest entry")
            .read_to_string(&mut archived_manifest)
            .expect("read archived manifest");
        assert!(archived_manifest.contains("\"Relative Built Package\""));
    }

    #[test]
    fn packed_archive_round_trip_preserves_mesh_overrides() {
        let package_dir = temp_package_dir("archive_mesh_roundtrip");
        let assets_dir = package_dir.join("assets");
        std::fs::create_dir_all(&assets_dir).expect("create assets dir");

        let spec = make_static_mesh_override_spec();
        let wasm = make_spec_wasm_bytes(&make_spec_wire_bytes(&spec));
        let kart_glb = make_triangle_glb(
            [[1.5, 0.3, -0.2], [2.1, 0.3, -0.2], [1.5, 0.3, 0.4]],
            [0.0, 1.0, 0.0],
        );
        std::fs::write(assets_dir.join("kart.glb"), &kart_glb).expect("write kart glb");

        write_package_root(
            &package_dir,
            r#"{
                "name":"Mesh Override Test",
                "abi_version":1,
                "scene_space":"world_3d",
                "assets":{
                    "meshes":[
                        {"path":"assets/kart.glb","label":"kart_body"}
                    ]
                }
            }"#,
            &wasm,
        );

        let archive_path = package_dir.join("mesh_override.vzglyd");
        pack_slide_directory(&package_dir, &archive_path).expect("pack archive");

        let archive_file = File::open(&archive_path).expect("open archive");
        let mut archive = ZipArchive::new(archive_file).expect("read zip archive");
        let names: Vec<String> = (0..archive.len())
            .map(|idx| {
                archive
                    .by_index(idx)
                    .expect("archive entry")
                    .name()
                    .to_string()
            })
            .collect();
        assert!(names.iter().any(|name| name == "assets/kart.glb"));

        let (dir_loaded, _) =
            load_slide_from_wasm::<WorldVertex>(package_dir.to_string_lossy().as_ref(), None)
                .expect("load directory package");
        let (archive_loaded, _) = load_slide_from_archive::<WorldVertex>(
            archive_path.to_string_lossy().as_ref(),
            None,
        )
        .expect("load archive package");

        assert_eq!(
            archive_loaded.spec.static_meshes[1].vertices[0].position,
            dir_loaded.spec.static_meshes[1].vertices[0].position
        );
        assert_eq!(
            archive_loaded.spec.static_meshes[1].indices,
            dir_loaded.spec.static_meshes[1].indices
        );
    }

    #[test]
    fn archive_loader_rejects_unsafe_zip_entries() {
        let archive_path = temp_package_dir("archive_escape").join("escape.vzglyd");
        let file = File::create(&archive_path).expect("create archive");
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("../escape.txt", options)
            .expect("create malicious entry");
        zip.write_all(b"oops").expect("write malicious entry");
        zip.finish().expect("finish archive");

        let error = match load_slide_from_archive::<WorldVertex>(
            archive_path.to_string_lossy().as_ref(),
            None,
        ) {
            Ok(_) => panic!("unsafe archive path should fail"),
            Err(error) => error,
        };

        assert!(
            error.to_string().contains("unsafe entry path"),
            "unexpected error: {error}"
        );
    }
}
