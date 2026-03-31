use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use vzglyd_slide::ABI_VERSION;
use thiserror::Error;

use crate::transition::TransitionKind;

pub(crate) const MIN_DISPLAY_DURATION_SECONDS: u32 = 1;
pub(crate) const MAX_DISPLAY_DURATION_SECONDS: u32 = 300;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SlideManifest {
    pub name: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub abi_version: Option<u32>,
    pub scene_space: Option<String>,
    pub assets: Option<ManifestAssets>,
    pub shaders: Option<ManifestShaders>,
    pub display: Option<DisplayConfig>,
    pub requirements: Option<ManifestRequirements>,
    pub sidecar: Option<ManifestSidecar>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ManifestAssets {
    #[serde(default)]
    pub textures: Vec<AssetRef>,
    #[serde(default)]
    pub meshes: Vec<AssetRef>,
    #[serde(default)]
    pub scenes: Vec<SceneAssetRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetRef {
    pub path: String,
    #[serde(default)]
    pub usage: Option<String>,
    #[serde(default)]
    pub slot: Option<usize>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SceneAssetRef {
    pub path: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub entry_camera: Option<String>,
    #[serde(default)]
    pub compile_profile: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ManifestShaders {
    pub vertex: Option<String>,
    pub fragment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DisplayConfig {
    pub duration_seconds: Option<u32>,
    pub transition_in: Option<String>,
    pub transition_out: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ManifestRequirements {
    pub min_texture_dim: Option<u32>,
    pub uses_depth_buffer: Option<bool>,
    pub uses_transparency: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ManifestSidecar {
    #[serde(default)]
    pub wasi_preopens: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestValidationError {
    #[error("abi_version {found} does not match engine ABI {expected}")]
    AbiVersion { found: u32, expected: u32 },
    #[error("unknown scene_space '{0}'")]
    UnknownSceneSpace(String),
    #[error("path '{0}' must remain within the package directory")]
    PathEscapesPackage(String),
    #[error(
        "display duration {0}s out of bounds [{MIN_DISPLAY_DURATION_SECONDS}, {MAX_DISPLAY_DURATION_SECONDS}]s"
    )]
    DurationSecondsOutOfBounds(u32),
    #[error("invalid sidecar preopen '{0}'")]
    InvalidSidecarPreopen(String),
}

impl SlideManifest {
    pub(crate) fn validate(&self, _package_root: &Path) -> Result<(), ManifestValidationError> {
        if let Some(found) = self.abi_version {
            if found != ABI_VERSION {
                return Err(ManifestValidationError::AbiVersion {
                    found,
                    expected: ABI_VERSION,
                });
            }
        }

        if let Some(scene_space) = self.scene_space.as_deref() {
            if !matches!(scene_space, "screen_2d" | "world_3d") {
                return Err(ManifestValidationError::UnknownSceneSpace(
                    scene_space.to_string(),
                ));
            }
        }

        if let Some(assets) = &self.assets {
            for texture in &assets.textures {
                validate_package_relative_path(&texture.path)?;
            }
            for mesh in &assets.meshes {
                validate_package_relative_path(&mesh.path)?;
            }
            for scene in &assets.scenes {
                validate_package_relative_path(&scene.path)?;
            }
        }

        if let Some(shaders) = &self.shaders {
            if let Some(vertex) = shaders.vertex.as_deref() {
                validate_package_relative_path(vertex)?;
            }
            if let Some(fragment) = shaders.fragment.as_deref() {
                validate_package_relative_path(fragment)?;
            }
        }

        if let Some(duration_seconds) = self.display_duration_seconds() {
            if !(MIN_DISPLAY_DURATION_SECONDS..=MAX_DISPLAY_DURATION_SECONDS)
                .contains(&duration_seconds)
            {
                return Err(ManifestValidationError::DurationSecondsOutOfBounds(
                    duration_seconds,
                ));
            }
        }

        if let Some(sidecar) = &self.sidecar {
            for preopen in &sidecar.wasi_preopens {
                validate_sidecar_preopen(preopen)?;
            }
        }

        Ok(())
    }

    pub(crate) fn transition_in_kind(&self) -> Option<TransitionKind> {
        self.display
            .as_ref()
            .and_then(|display| display.transition_in.as_deref())
            .map(parse_transition_kind)
    }

    pub(crate) fn transition_out_kind(&self) -> Option<TransitionKind> {
        self.display
            .as_ref()
            .and_then(|display| display.transition_out.as_deref())
            .map(parse_transition_kind)
    }

    pub(crate) fn display_duration_seconds(&self) -> Option<u32> {
        self.display
            .as_ref()
            .and_then(|display| display.duration_seconds)
    }

    pub(crate) fn scene_asset(&self, requested_id: Option<&str>) -> Option<&SceneAssetRef> {
        let assets = self.assets.as_ref()?;
        match requested_id {
            Some(id) => assets
                .scenes
                .iter()
                .find(|scene| scene.id.as_deref() == Some(id)),
            None => assets.scenes.first(),
        }
    }
}

fn validate_package_relative_path(path: &str) -> Result<(), ManifestValidationError> {
    let candidate = Path::new(path);
    for component in candidate.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(ManifestValidationError::PathEscapesPackage(
                    path.to_string(),
                ));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(())
}

fn validate_sidecar_preopen(spec: &str) -> Result<(), ManifestValidationError> {
    let Some((host, guest)) = spec.rsplit_once(':') else {
        return Err(ManifestValidationError::InvalidSidecarPreopen(
            spec.to_string(),
        ));
    };
    if host.is_empty() || guest.is_empty() {
        return Err(ManifestValidationError::InvalidSidecarPreopen(
            spec.to_string(),
        ));
    }
    if !Path::new(host).is_absolute() || !Path::new(guest).is_absolute() {
        return Err(ManifestValidationError::InvalidSidecarPreopen(
            spec.to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn parse_transition_kind(kind: &str) -> TransitionKind {
    match kind {
        "crossfade" => TransitionKind::Crossfade,
        "wipe_left" => TransitionKind::WipeLeft,
        "wipe_down" => TransitionKind::WipeDown,
        "dissolve" => TransitionKind::Dissolve,
        "cut" => TransitionKind::Cut,
        other => {
            log::warn!("unknown transition kind '{other}', defaulting to crossfade");
            TransitionKind::Crossfade
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use vzglyd_slide::ABI_VERSION;

    use super::{
        AssetRef, DisplayConfig, ManifestAssets, ManifestRequirements, ManifestShaders,
        ManifestSidecar, ManifestValidationError, SceneAssetRef, SlideManifest,
    };
    use crate::transition::TransitionKind;

    #[test]
    fn minimal_manifest_parses_without_new_sections() {
        let json = r#"{"name":"Test"}"#;
        let manifest: SlideManifest = serde_json::from_str(json).expect("parse manifest");

        assert_eq!(manifest.name.as_deref(), Some("Test"));
        assert!(manifest.display.is_none());
        assert!(manifest.assets.is_none());
        assert!(manifest.shaders.is_none());
        assert!(manifest.sidecar.is_none());
    }

    #[test]
    fn full_manifest_parses_with_all_sections() {
        let json = r#"{
            "name":"Terrain (Rust)",
            "version":"1.0.0",
            "author":"VZGLYD",
            "description":"A procedural terrain with cel shading",
            "abi_version":1,
            "scene_space":"world_3d",
            "assets":{
                "textures":[
                    {"path":"assets/noise.png","usage":"material"},
                    {"path":"assets/font_atlas.png","usage":"font","slot":0,"label":"font_atlas"}
                ],
                "meshes":[
                    {"path":"assets/kart.glb","slot":0,"label":"kart_body"}
                ],
                "scenes":[
                    {
                        "path":"assets/world.glb",
                        "id":"hero_world",
                        "label":"Hero World",
                        "entry_camera":"overview",
                        "compile_profile":"default_world"
                    }
                ]
            },
            "shaders":{
                "vertex":"shaders/vertex.wgsl",
                "fragment":"shaders/fragment.wgsl"
            },
            "display":{
                "duration_seconds":20,
                "transition_in":"crossfade",
                "transition_out":"dissolve"
            },
            "requirements":{
                "min_texture_dim":128,
                "uses_depth_buffer":true,
                "uses_transparency":true
            },
            "sidecar":{
                "wasi_preopens":["/tmp/vzglyd-reminders:/data"]
            }
        }"#;
        let manifest: SlideManifest = serde_json::from_str(json).expect("parse manifest");

        assert_eq!(manifest.abi_version, Some(1));
        assert_eq!(manifest.scene_space.as_deref(), Some("world_3d"));
        assert_eq!(
            manifest
                .assets
                .as_ref()
                .expect("assets")
                .textures
                .first()
                .expect("texture")
                .path,
            "assets/noise.png"
        );
        assert_eq!(
            manifest
                .shaders
                .as_ref()
                .expect("shaders")
                .fragment
                .as_deref(),
            Some("shaders/fragment.wgsl")
        );
        assert_eq!(manifest.display_duration_seconds(), Some(20));
        assert_eq!(
            manifest.transition_out_kind(),
            Some(TransitionKind::Dissolve)
        );
        assert_eq!(
            manifest
                .requirements
                .as_ref()
                .expect("requirements")
                .min_texture_dim,
            Some(128)
        );
        assert_eq!(
            manifest
                .assets
                .as_ref()
                .expect("assets")
                .meshes
                .first()
                .expect("mesh")
                .path,
            "assets/kart.glb"
        );
        let scene = manifest
            .assets
            .as_ref()
            .expect("assets")
            .scenes
            .first()
            .expect("scene");
        assert_eq!(scene.path, "assets/world.glb");
        assert_eq!(scene.id.as_deref(), Some("hero_world"));
        assert_eq!(scene.entry_camera.as_deref(), Some("overview"));
        assert_eq!(scene.compile_profile.as_deref(), Some("default_world"));
        assert_eq!(
            manifest.sidecar,
            Some(ManifestSidecar {
                wasi_preopens: vec!["/tmp/vzglyd-reminders:/data".to_string()],
            })
        );
    }

    #[test]
    fn invalid_abi_version_is_rejected() {
        let manifest = SlideManifest {
            abi_version: Some(99),
            ..Default::default()
        };

        let error = manifest
            .validate(Path::new("slides/terrain"))
            .expect_err("abi mismatch should fail");

        assert_eq!(
            error,
            ManifestValidationError::AbiVersion {
                found: 99,
                expected: ABI_VERSION
            }
        );
    }

    #[test]
    fn unknown_scene_space_is_rejected() {
        let manifest = SlideManifest {
            scene_space: Some("isometric".into()),
            ..Default::default()
        };

        let error = manifest
            .validate(Path::new("slides/terrain"))
            .expect_err("unknown scene space should fail");

        assert_eq!(
            error,
            ManifestValidationError::UnknownSceneSpace("isometric".into())
        );
    }

    #[test]
    fn asset_path_traversal_is_rejected() {
        let manifest = SlideManifest {
            assets: Some(ManifestAssets {
                textures: vec![AssetRef {
                    path: "../secret.png".into(),
                    usage: Some("material".into()),
                    slot: None,
                    label: None,
                    id: None,
                }],
                meshes: vec![],
                scenes: vec![],
            }),
            ..Default::default()
        };

        let error = manifest
            .validate(Path::new("slides/terrain"))
            .expect_err("path traversal should fail");

        assert_eq!(
            error,
            ManifestValidationError::PathEscapesPackage("../secret.png".into())
        );
    }

    #[test]
    fn scene_asset_path_traversal_is_rejected() {
        let manifest = SlideManifest {
            assets: Some(ManifestAssets {
                textures: vec![],
                meshes: vec![],
                scenes: vec![SceneAssetRef {
                    path: "../world.glb".into(),
                    label: Some("World".into()),
                    id: Some("hero_world".into()),
                    entry_camera: Some("overview".into()),
                    compile_profile: Some("default_world".into()),
                }],
            }),
            ..Default::default()
        };

        let error = manifest
            .validate(Path::new("slides/terrain"))
            .expect_err("scene path traversal should fail");

        assert_eq!(
            error,
            ManifestValidationError::PathEscapesPackage("../world.glb".into())
        );
    }

    #[test]
    fn shader_path_traversal_is_rejected() {
        let manifest = SlideManifest {
            shaders: Some(ManifestShaders {
                vertex: Some("/tmp/vertex.wgsl".into()),
                fragment: None,
            }),
            ..Default::default()
        };

        let error = manifest
            .validate(Path::new("slides/terrain"))
            .expect_err("absolute shader path should fail");

        assert_eq!(
            error,
            ManifestValidationError::PathEscapesPackage("/tmp/vertex.wgsl".into())
        );
    }

    #[test]
    fn display_duration_out_of_bounds_is_rejected() {
        let manifest = SlideManifest {
            display: Some(DisplayConfig {
                duration_seconds: Some(301),
                transition_in: None,
                transition_out: None,
            }),
            ..Default::default()
        };

        let error = manifest
            .validate(Path::new("slides/terrain"))
            .expect_err("duration should fail bounds check");

        assert_eq!(
            error,
            ManifestValidationError::DurationSecondsOutOfBounds(301)
        );
    }

    #[test]
    fn sidecar_preopens_require_absolute_host_and_guest_paths() {
        let manifest = SlideManifest {
            sidecar: Some(ManifestSidecar {
                wasi_preopens: vec!["relative:/data".to_string()],
            }),
            ..Default::default()
        };

        let error = manifest
            .validate(Path::new("slides/reminders"))
            .expect_err("relative host path should fail");

        assert_eq!(
            error,
            ManifestValidationError::InvalidSidecarPreopen("relative:/data".to_string())
        );
    }

    #[test]
    fn unknown_display_transition_falls_back_to_crossfade() {
        let manifest = SlideManifest {
            display: Some(DisplayConfig {
                duration_seconds: Some(20),
                transition_in: Some("mystery".into()),
                transition_out: None,
            }),
            ..Default::default()
        };

        assert_eq!(
            manifest.transition_in_kind(),
            Some(TransitionKind::Crossfade)
        );
    }

    #[test]
    fn requirements_section_round_trips() {
        let manifest = SlideManifest {
            requirements: Some(ManifestRequirements {
                min_texture_dim: Some(128),
                uses_depth_buffer: Some(true),
                uses_transparency: Some(false),
            }),
            sidecar: Some(ManifestSidecar {
                wasi_preopens: vec!["/tmp/vzglyd-reminders:/data".to_string()],
            }),
            ..Default::default()
        };
        let json = serde_json::to_string(&manifest).expect("serialize manifest");
        let decoded: SlideManifest = serde_json::from_str(&json).expect("deserialize manifest");

        assert_eq!(
            decoded.requirements,
            Some(ManifestRequirements {
                min_texture_dim: Some(128),
                uses_depth_buffer: Some(true),
                uses_transparency: Some(false),
            })
        );
        assert_eq!(
            decoded.sidecar,
            Some(ManifestSidecar {
                wasi_preopens: vec!["/tmp/vzglyd-reminders:/data".to_string()],
            })
        );
    }

    #[test]
    fn asset_refs_parse_slot_and_label_selectors() {
        let json = r#"{
            "assets":{
                "textures":[
                    {
                        "path":"assets/detail.rgba",
                        "usage":"detail",
                        "slot":2,
                        "label":"detail_map"
                    }
                ]
            }
        }"#;
        let manifest: SlideManifest = serde_json::from_str(json).expect("parse manifest");
        let texture = manifest
            .assets
            .as_ref()
            .expect("assets")
            .textures
            .first()
            .expect("texture");

        assert_eq!(texture.slot, Some(2));
        assert_eq!(texture.label.as_deref(), Some("detail_map"));
    }

    #[test]
    fn mesh_and_scene_asset_refs_parse_selectors_and_scene_metadata() {
        let json = r#"{
            "assets":{
                "meshes":[
                    {
                        "path":"assets/kart.glb",
                        "slot":1,
                        "label":"kart_body"
                    }
                ],
                "scenes":[
                    {
                        "path":"assets/world.glb",
                        "id":"hero_world",
                        "label":"World",
                        "entry_camera":"overview",
                        "compile_profile":"default_world"
                    }
                ]
            }
        }"#;
        let manifest: SlideManifest = serde_json::from_str(json).expect("parse manifest");
        let assets = manifest.assets.as_ref().expect("assets");
        let mesh = assets.meshes.first().expect("mesh");

        assert_eq!(mesh.slot, Some(1));
        assert_eq!(mesh.label.as_deref(), Some("kart_body"));
        let scene = assets.scenes.first().expect("scene");
        assert_eq!(scene.id.as_deref(), Some("hero_world"));
        assert_eq!(scene.label.as_deref(), Some("World"));
        assert_eq!(scene.entry_camera.as_deref(), Some("overview"));
        assert_eq!(scene.compile_profile.as_deref(), Some("default_world"));
    }

    #[test]
    fn scene_asset_selection_prefers_id_and_falls_back_to_first_scene() {
        let manifest = SlideManifest {
            assets: Some(ManifestAssets {
                textures: vec![],
                meshes: vec![],
                scenes: vec![
                    SceneAssetRef {
                        path: "assets/world.glb".into(),
                        label: Some("World".into()),
                        id: Some("hero_world".into()),
                        entry_camera: Some("overview".into()),
                        compile_profile: Some("default_world".into()),
                    },
                    SceneAssetRef {
                        path: "assets/bonus.glb".into(),
                        label: Some("Bonus".into()),
                        id: Some("bonus_world".into()),
                        entry_camera: Some("bonus_cam".into()),
                        compile_profile: Some("default_world".into()),
                    },
                ],
            }),
            ..Default::default()
        };

        assert_eq!(
            manifest
                .scene_asset(Some("bonus_world"))
                .expect("selected scene")
                .path,
            "assets/bonus.glb"
        );
        assert_eq!(
            manifest.scene_asset(None).expect("default scene").path,
            "assets/world.glb"
        );
        assert!(manifest.scene_asset(Some("missing")).is_none());
    }
}
