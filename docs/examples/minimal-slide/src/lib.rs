use bytemuck::{Pod, Zeroable};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use vzglyd_slide::{
    ABI_VERSION, DrawSource, DrawSpec, FilterMode, Limits, PipelineKind, SceneSpace, SlideSpec,
    StaticMesh, TextureDesc, TextureFormat, WrapMode,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, Serialize, Deserialize)]
pub struct Vertex {
    pub position: [f32; 3],
    pub tex_coords: [f32; 2],
    pub color: [f32; 4],
    pub mode: f32,
}

static ENCODED_SPEC: Lazy<Vec<u8>> = Lazy::new(|| postcard::to_stdvec(&build_spec()).unwrap());

fn build_spec() -> SlideSpec<Vertex> {
    SlideSpec {
        name: "minimal-screen".into(),
        limits: Limits::pi4(),
        scene_space: SceneSpace::Screen2D,
        camera_path: None,
        shaders: None,
        overlay: None,
        font: None,
        textures_used: 1,
        textures: vec![TextureDesc {
            label: "white".into(),
            width: 1,
            height: 1,
            format: TextureFormat::Rgba8Unorm,
            wrap_u: WrapMode::ClampToEdge,
            wrap_v: WrapMode::ClampToEdge,
            wrap_w: WrapMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mip_filter: FilterMode::Nearest,
            data: vec![255, 255, 255, 255],
        }],
        static_meshes: vec![StaticMesh {
            label: "quad".into(),
            vertices: vec![
                Vertex {
                    position: [-0.6, -0.6, 0.0],
                    tex_coords: [0.0, 1.0],
                    color: [0.1, 0.4, 0.9, 1.0],
                    mode: 0.0,
                },
                Vertex {
                    position: [0.6, -0.6, 0.0],
                    tex_coords: [1.0, 1.0],
                    color: [0.1, 0.4, 0.9, 1.0],
                    mode: 0.0,
                },
                Vertex {
                    position: [0.6, 0.6, 0.0],
                    tex_coords: [1.0, 0.0],
                    color: [0.8, 0.9, 1.0, 1.0],
                    mode: 0.0,
                },
                Vertex {
                    position: [-0.6, 0.6, 0.0],
                    tex_coords: [0.0, 0.0],
                    color: [0.8, 0.9, 1.0, 1.0],
                    mode: 0.0,
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
        }],
        dynamic_meshes: vec![],
        draws: vec![DrawSpec {
            label: "quad".into(),
            source: DrawSource::Static(0),
            pipeline: PipelineKind::Opaque,
            index_range: 0..6,
        }],
        lighting: None,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_spec_ptr() -> *const u8 {
    ENCODED_SPEC.as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_spec_len() -> u32 {
    ENCODED_SPEC.len() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_update(_dt: f32) -> i32 {
    0
}
