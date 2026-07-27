use engine_renderer::{
    PassGraphConfig, PostProcessSettings, ReflectionProbe, ToneMapping, TransparencyMode,
};
use engine_serialize::{
    AssetId, ComponentTypeId, Diagnostic, DiagnosticSeverity, EngineVersion, PersistentId,
    SchemaVersion, Value,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use thiserror::Error;

use crate::world::World;

pub const ECS_SCENE_CONTRACT: &str = "ECSScene-v0.1.0";

/// Current scene schema version.  Any major bump or minor > this indicates
/// incompatibility.
pub const SCENE_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 1, 0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub schema_version: SchemaVersion,
    pub engine_version: EngineVersion,
    pub scene_id: PersistentId,
    pub name: String,
    pub entities: Vec<EntityRecord>,
    pub scene_settings: SceneSettings,
    pub dependencies: Vec<AssetId>,
    pub diagnostics_policy: DiagnosticsPolicy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityRecord {
    pub persistent_id: PersistentId,
    pub parent: Option<PersistentId>,
    pub name: Option<String>,
    pub enabled: bool,
    pub components: BTreeMap<ComponentTypeId, ComponentRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentRecord {
    pub schema_version: SchemaVersion,
    pub enabled: bool,
    pub fields: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneSettings {
    pub active_camera: Option<PersistentId>,
    pub default_render_layer: String,
    pub fixed_timestep_seconds: f32,
    pub gravity: Option<[f32; 3]>,
    pub ambient: [f32; 4],
    pub environment_map: Option<AssetId>,
    #[serde(default = "default_environment_intensity")]
    pub environment_intensity: f32,
    #[serde(default)]
    pub environment_rotation_radians: f32,
    #[serde(default)]
    pub reflection_probes: Vec<ReflectionProbe>,
    #[serde(default)]
    pub post_process: PostProcessSettings,
    pub tone_mapping: ToneMapping,
    #[serde(default)]
    pub transparency_mode: TransparencyMode,
    pub pass_graph_config: PassGraphConfig,
    /// Opt-in camera-relative rendering (ENG-01): renderer extraction emits
    /// the base view matrix with its translation removed and shifts every
    /// emitted world position by `-base_camera_position`, restoring f32
    /// precision for content far from the world origin. Rendering-only:
    /// scene data, physics, and scripts keep absolute f32 world coordinates.
    /// Defaults to `false` so existing scenes render unchanged.
    #[serde(default)]
    pub camera_relative_rendering: bool,
    /// Opt-in periodic world-origin shifting (ENG-01 Phase 2). Disabled by
    /// default so existing scenes simulate unchanged.
    #[serde(default)]
    pub origin_shift: OriginShiftSettings,
}

/// Periodic world-origin shift trigger configuration (ENG-01 Phase 2).
///
/// When `enabled`, the runtime evaluates the reference position once per
/// frame at the frame boundary (between the update and the render, alongside
/// scene-transition processing); when its distance from the origin exceeds
/// `threshold` metres, every f32 world-space runtime value is translated by
/// `-reference` and the world origin advances by `reference`, keeping logical
/// positions unchanged while re-centring f32 storage on the viewer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OriginShiftSettings {
    /// Whether periodic origin shifting runs. Defaults to `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Distance in metres beyond which a shift triggers. Values that are
    /// non-finite or `<= 0` disable the trigger defensively.
    /// Defaults to 8000 (roughly the onset of visible f32 jitter).
    #[serde(default = "default_origin_shift_threshold")]
    pub threshold: f32,
    /// Persistent ID of the entity watched for threshold crossing. `None`
    /// watches the active camera (same selection as renderer extraction).
    #[serde(default)]
    pub reference_entity: Option<PersistentId>,
}

/// Default origin-shift trigger distance: 8 km.
pub const DEFAULT_ORIGIN_SHIFT_THRESHOLD: f32 = 8000.0;

fn default_origin_shift_threshold() -> f32 {
    DEFAULT_ORIGIN_SHIFT_THRESHOLD
}

fn default_environment_intensity() -> f32 {
    1.0
}

impl Default for OriginShiftSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: DEFAULT_ORIGIN_SHIFT_THRESHOLD,
            reference_entity: None,
        }
    }
}

impl Default for SceneSettings {
    fn default() -> Self {
        Self {
            active_camera: None,
            default_render_layer: "Default".to_string(),
            fixed_timestep_seconds: 1.0 / 60.0,
            gravity: Some([0.0, -9.81, 0.0]),
            ambient: [0.03, 0.03, 0.03, 1.0],
            environment_map: None,
            environment_intensity: 1.0,
            environment_rotation_radians: 0.0,
            reflection_probes: Vec::new(),
            post_process: PostProcessSettings::default(),
            tone_mapping: ToneMapping::Aces,
            transparency_mode: TransparencyMode::default(),
            pass_graph_config: PassGraphConfig::default(),
            camera_relative_rendering: false,
            origin_shift: OriginShiftSettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticsPolicy {
    Strict,
    EditorRepair,
}

// ── Scene errors ────────────────────────────────────────────────────────────

/// Errors that can occur during scene serialization.
#[derive(Debug, Error)]
pub enum SceneError {
    /// I/O error (file read/write).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// RON error (serialization or deserialization).
    #[error("RON error: {0}")]
    Ron(#[from] ron::Error),
}

/// A component-level problem encountered while restoring a [`World`] from a
/// scene with a [`crate::ComponentRegistry`].
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SceneLoadDiagnostic {
    #[error("entity `{entity_id}` references unknown component type `{component_type_id}`")]
    UnknownComponent {
        entity_id: PersistentId,
        component_type_id: ComponentTypeId,
    },
    #[error(
        "component type `{component_type_id}` on entity `{entity_id}` has no deserialize hook"
    )]
    MissingDeserializeHook {
        entity_id: PersistentId,
        component_type_id: ComponentTypeId,
    },
    #[error(
        "component registry entry `{component_type_id}` created storage for `{storage_type_id}`"
    )]
    StorageFactoryTypeMismatch {
        entity_id: PersistentId,
        component_type_id: ComponentTypeId,
        storage_type_id: ComponentTypeId,
    },
    #[error(
        "deserialize hook for `{component_type_id}` returned a value incompatible with its storage on entity `{entity_id}`"
    )]
    StorageInsertTypeMismatch {
        entity_id: PersistentId,
        component_type_id: ComponentTypeId,
    },
    #[error(
        "component `{component_type_id}` on entity `{entity_id}` has invalid fields: {message}"
    )]
    InvalidComponentFields {
        entity_id: PersistentId,
        component_type_id: ComponentTypeId,
        message: String,
    },
    #[error(
        "singleton component `{component_type_id}` appears on both `{first_entity_id}` and `{entity_id}`"
    )]
    DuplicateSingletonComponent {
        entity_id: PersistentId,
        first_entity_id: PersistentId,
        component_type_id: ComponentTypeId,
    },
}

/// Non-failing scene load result. The partially restored world is retained,
/// while every external-component failure is made visible to the caller.
pub struct SceneLoadReport {
    pub world: World,
    pub diagnostics: Vec<SceneLoadDiagnostic>,
}

impl SceneLoadReport {
    pub fn is_success(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Convert this report into the strict form used by
    /// [`World::try_from_scene_with_registry`].
    pub fn into_result(self) -> Result<World, SceneLoadError> {
        if self.diagnostics.is_empty() {
            Ok(self.world)
        } else {
            Err(SceneLoadError {
                diagnostics: self.diagnostics,
            })
        }
    }
}

/// Aggregated failure returned by strict registry-aware scene loading.
#[derive(Debug, Error)]
#[error("one or more scene components could not be restored")]
pub struct SceneLoadError {
    pub diagnostics: Vec<SceneLoadDiagnostic>,
}

// ── Scene serialization / validation helpers ────────────────────────────────

impl Scene {
    /// Save this scene to a RON file at the given path.
    ///
    /// Creates parent directories if they don't exist.  The file is written in
    /// a human-readable RON format.
    pub fn save_to_file(&self, path: &Path) -> Result<(), SceneError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let ron_string = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?;
        std::fs::write(path, ron_string)?;
        Ok(())
    }

    /// Load a scene from a RON file.
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load_from_file(path: &Path) -> Result<Self, SceneError> {
        let ron_string = std::fs::read_to_string(path)?;
        let scene: Scene = ron::de::from_str(&ron_string).map_err(|e| SceneError::Ron(e.code))?;
        Ok(scene)
    }

    /// Validate schema version compatibility.
    ///
    /// Returns a list of [`Diagnostic`] items describing any version
    /// incompatibilities.  An empty `Vec` means the version is fully compatible.
    ///
    /// - **Error** if `schema_version.major` differs from the expected major.
    /// - **Warning** if `schema_version.minor` is greater than the expected minor
    ///   (newer features may not be understood).
    pub fn check_version(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let expected = SCENE_SCHEMA_VERSION;

        if self.schema_version.major != expected.major {
            diagnostics.push(
                Diagnostic::new(
                    "SC0020",
                    DiagnosticSeverity::Error,
                    "engine-scene",
                    format!(
                        "Scene schema version {}.{}.{} is not compatible with \
                         expected {}.{}.{}",
                        self.schema_version.major,
                        self.schema_version.minor,
                        self.schema_version.patch,
                        expected.major,
                        expected.minor,
                        expected.patch,
                    ),
                )
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .path("schema_version"),
            );
        } else if self.schema_version.minor > expected.minor {
            diagnostics.push(
                Diagnostic::new(
                    "SC0021",
                    DiagnosticSeverity::Warning,
                    "engine-scene",
                    format!(
                        "Scene schema version {}.{}.{} is newer than expected \
                         {}.{}.{}; some features may not be supported",
                        self.schema_version.major,
                        self.schema_version.minor,
                        self.schema_version.patch,
                        expected.major,
                        expected.minor,
                        expected.patch,
                    ),
                )
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .path("schema_version"),
            );
        }

        diagnostics
    }

    /// Collect all asset dependencies referenced by components in this scene.
    ///
    /// Recursively scans every entity's component fields for `Value::Asset`
    /// entries, including assets nested in `Value::List` and `Value::Map`, and
    /// returns a deduplicated list of [`AssetId`] values.
    pub fn collect_asset_dependencies(&self) -> Vec<AssetId> {
        let mut deps: BTreeSet<AssetId> = BTreeSet::new();

        for entity in &self.entities {
            for component in entity.components.values() {
                for value in component.fields.values() {
                    collect_value_asset_dependencies(value, &mut deps);
                }
            }
        }
        if let Some(environment_map) = &self.scene_settings.environment_map {
            deps.insert(environment_map.clone());
        }
        deps.extend(
            self.scene_settings
                .reflection_probes
                .iter()
                .map(|probe| probe.environment_map.clone()),
        );

        deps.into_iter().collect()
    }
}

fn collect_value_asset_dependencies(value: &Value, dependencies: &mut BTreeSet<AssetId>) {
    match value {
        Value::Asset(asset) => {
            dependencies.insert(asset.clone());
        }
        Value::List(values) => {
            for value in values {
                collect_value_asset_dependencies(value, dependencies);
            }
        }
        Value::Map(values) => {
            for value in values.values() {
                collect_value_asset_dependencies(value, dependencies);
            }
        }
        _ => {}
    }
}

pub fn sample_scene() -> Scene {
    let mut camera_components = BTreeMap::new();
    camera_components.insert("engine.camera".to_string(), component(BTreeMap::new()));

    let mut renderable_fields = BTreeMap::new();
    renderable_fields.insert("mesh".to_string(), Value::Asset(AssetId::new("mesh-cube")));
    renderable_fields.insert(
        "material".to_string(),
        Value::Asset(AssetId::new("mat-default")),
    );
    renderable_fields.insert("visible".to_string(), Value::Bool(true));
    renderable_fields.insert(
        "render_layer".to_string(),
        Value::Str("Default".to_string()),
    );
    renderable_fields.insert("cast_shadows".to_string(), Value::Bool(true));
    let mut renderable_components = BTreeMap::new();
    renderable_components.insert(
        "engine.renderable".to_string(),
        component(renderable_fields),
    );

    Scene {
        schema_version: SchemaVersion::new(0, 1, 0),
        engine_version: "0.1.0".to_string(),
        scene_id: "scene-gate04-valid".to_string(),
        name: "Gate 4 Validation Scene".to_string(),
        entities: vec![
            EntityRecord {
                persistent_id: "camera-main".to_string(),
                parent: None,
                name: Some("Main Camera".to_string()),
                enabled: true,
                components: camera_components,
            },
            EntityRecord {
                persistent_id: "cube-01".to_string(),
                parent: None,
                name: Some("Cube".to_string()),
                enabled: true,
                components: renderable_components,
            },
        ],
        scene_settings: SceneSettings {
            active_camera: Some("camera-main".to_string()),
            ..SceneSettings::default()
        },
        dependencies: vec![AssetId::new("mesh-cube"), AssetId::new("mat-default")],
        diagnostics_policy: DiagnosticsPolicy::Strict,
    }
}

/// Build the authoring scene installed into a newly-created project or scene.
///
/// Unlike [`sample_scene`], which is a compact validation fixture, this scene
/// has the transforms and lighting needed to be immediately useful in the
/// editor and game preview.
pub fn starter_scene(scene_id: impl Into<PersistentId>, name: impl Into<String>) -> Scene {
    let (camera_translation, camera_rotation) = crate::camera_utils::setup_orbit_transform(
        glam::Vec3::ZERO,
        glam::Vec3::new(0.0, 2.0, 5.0),
    );
    let transform = |translation: [f32; 3], rotation: [f32; 4]| {
        component(BTreeMap::from([
            ("translation".to_string(), Value::Vec3(translation)),
            ("rotation".to_string(), Value::Quat(rotation)),
            ("scale".to_string(), Value::Vec3([1.0, 1.0, 1.0])),
        ]))
    };

    let camera_components = BTreeMap::from([
        (
            "engine.camera".to_string(),
            component(BTreeMap::from([
                (
                    "projection".to_string(),
                    Value::Enum("Perspective".to_string()),
                ),
                ("near".to_string(), Value::Float32(0.1)),
                ("far".to_string(), Value::Float32(1000.0)),
                (
                    "fov_y".to_string(),
                    Value::Float32(std::f32::consts::FRAC_PI_4),
                ),
                ("ortho_half_height".to_string(), Value::Float32(5.0)),
                (
                    "render_layer_mask".to_string(),
                    Value::UInt(u32::MAX as u64),
                ),
                ("clear_flags".to_string(), Value::UInt(3)),
                (
                    "clear_color".to_string(),
                    Value::Color([0.02, 0.02, 0.06, 1.0]),
                ),
                ("priority".to_string(), Value::Int(0)),
                ("msaa_samples".to_string(), Value::UInt(1)),
                ("hdr_output".to_string(), Value::Bool(false)),
                ("aperture".to_string(), Value::Float32(16.0)),
                ("shutter_speed".to_string(), Value::Float32(1.0 / 60.0)),
                ("iso".to_string(), Value::Float32(100.0)),
                ("ev_compensation".to_string(), Value::Float32(0.0)),
            ])),
        ),
        (
            "engine.transform".to_string(),
            transform(camera_translation.to_array(), camera_rotation.to_array()),
        ),
    ]);

    let cube_components = BTreeMap::from([
        (
            "engine.renderable".to_string(),
            component(BTreeMap::from([
                ("mesh".to_string(), Value::Asset(AssetId::new("mesh-cube"))),
                (
                    "material".to_string(),
                    Value::Asset(AssetId::new("mat-default")),
                ),
                ("visible".to_string(), Value::Bool(true)),
                (
                    "render_layer".to_string(),
                    Value::Str("Default".to_string()),
                ),
                ("cast_shadows".to_string(), Value::Bool(true)),
            ])),
        ),
        (
            "engine.transform".to_string(),
            transform([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]),
        ),
    ]);

    let light_components = BTreeMap::from([
        (
            "engine.light".to_string(),
            component(BTreeMap::from([
                ("kind".to_string(), Value::Enum("Directional".to_string())),
                ("color".to_string(), Value::Vec3([1.0, 0.96, 0.9])),
                ("intensity".to_string(), Value::Float32(2.5)),
                ("range".to_string(), Value::Float32(10.0)),
                ("shadow_mode".to_string(), Value::UInt(0)),
                ("direction".to_string(), Value::Vec3([-0.35, -0.8, -0.45])),
            ])),
        ),
        (
            "engine.transform".to_string(),
            transform([0.0, 3.0, 0.0], [0.0, 0.0, 0.0, 1.0]),
        ),
    ]);

    Scene {
        schema_version: SCENE_SCHEMA_VERSION,
        engine_version: "0.1.0".to_string(),
        scene_id: scene_id.into(),
        name: name.into(),
        entities: vec![
            EntityRecord {
                persistent_id: "camera-main".to_string(),
                parent: None,
                name: Some("Main Camera".to_string()),
                enabled: true,
                components: camera_components,
            },
            EntityRecord {
                persistent_id: "cube-01".to_string(),
                parent: None,
                name: Some("Cube".to_string()),
                enabled: true,
                components: cube_components,
            },
            EntityRecord {
                persistent_id: "light-directional".to_string(),
                parent: None,
                name: Some("Directional Light".to_string()),
                enabled: true,
                components: light_components,
            },
        ],
        scene_settings: SceneSettings {
            active_camera: Some("camera-main".to_string()),
            ..SceneSettings::default()
        },
        dependencies: vec![AssetId::new("mesh-cube"), AssetId::new("mat-default")],
        diagnostics_policy: DiagnosticsPolicy::Strict,
    }
}

fn component(fields: BTreeMap<String, Value>) -> ComponentRecord {
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}
