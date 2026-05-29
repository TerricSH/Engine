use engine_renderer::{PassGraphConfig, ToneMapping};
use engine_serialize::{
    AssetId, ComponentTypeId, Diagnostic, DiagnosticSeverity, EngineVersion, PersistentId,
    SchemaVersion, Value,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

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
    pub tone_mapping: ToneMapping,
    pub pass_graph_config: PassGraphConfig,
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
            tone_mapping: ToneMapping::Aces,
            pass_graph_config: PassGraphConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticsPolicy {
    Strict,
    EditorRepair,
}

// ── Scene serialization / validation helpers ────────────────────────────────

impl Scene {
    /// Save this scene to a RON file at the given path.
    ///
    /// Creates parent directories if they don't exist.  The file is written in
    /// a human-readable RON format.
    pub fn save_to_file(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
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
    pub fn load_from_file(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let ron_string = std::fs::read_to_string(path)?;
        let scene: Scene = ron::de::from_str(&ron_string)?;
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
    /// Scans every entity's component fields for `Value::Asset` entries and
    /// returns a deduplicated list of [`AssetId`] values.
    pub fn collect_asset_dependencies(&self) -> Vec<AssetId> {
        let mut deps: BTreeSet<AssetId> = BTreeSet::new();

        for entity in &self.entities {
            for component in entity.components.values() {
                for value in component.fields.values() {
                    if let Value::Asset(asset) = value {
                        deps.insert(asset.clone());
                    }
                }
            }
        }

        deps.into_iter().collect()
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

fn component(fields: BTreeMap<String, Value>) -> ComponentRecord {
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}
