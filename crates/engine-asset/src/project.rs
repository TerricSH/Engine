//! Game project manifest and path resolution.
//!
//! A game project is rooted by `game.project.json`. Every content path is
//! relative to that file, so authoring, player, cooker, and packaging commands
//! behave identically regardless of the caller's current working directory.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Project manifest contract understood by this engine version.
pub const GAME_PROJECT_SCHEMA: &str = "GameProject-v0";
/// Conventional project manifest file name.
pub const GAME_PROJECT_FILE_NAME: &str = "game.project.json";
/// Stable scene identifier synthesized for manifests that predate scene catalogs.
pub const LEGACY_STARTUP_SCENE_ID: &str = "main";

/// Rendering backend selected by a game project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectBackend {
    #[default]
    Vulkan,
}

/// Initial native window settings for the player.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectWindow {
    #[serde(default = "default_window_title")]
    pub title: String,
    #[serde(default = "default_window_width")]
    pub width: u32,
    #[serde(default = "default_window_height")]
    pub height: u32,
}

impl Default for ProjectWindow {
    fn default() -> Self {
        Self {
            title: default_window_title(),
            width: default_window_width(),
            height: default_window_height(),
        }
    }
}

/// Portable game project description stored at the project root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub schema: String,
    pub name: String,
    /// Startup scene identifier, or a legacy project-relative `.scene.ron` path.
    ///
    /// Keeping this path-shaped field preserves compatibility with v0 manifests
    /// and existing project creation code. When [`Self::scenes`] is populated,
    /// this value may instead contain one of its keys.
    pub startup_scene: PathBuf,
    /// Named project scenes. Empty maps are interpreted as a legacy single-scene
    /// project whose startup scene has the synthesized ID `main`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scenes: BTreeMap<String, PathBuf>,
    #[serde(default = "default_asset_source")]
    pub asset_source: PathBuf,
    #[serde(default = "default_cooked_assets")]
    pub cooked_assets: PathBuf,
    #[serde(default)]
    pub backend: ProjectBackend,
    #[serde(default)]
    pub window: ProjectWindow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_project: Option<PathBuf>,
    /// Compiled managed game assembly used by runtime players.
    ///
    /// This is deliberately separate from `script_project`: packaged games
    /// ship the DLL but do not need to ship the authoring `.csproj`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_assembly: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_actions: Option<PathBuf>,
}

impl ProjectManifest {
    /// Construct the default manifest for a newly-created project.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            schema: GAME_PROJECT_SCHEMA.to_string(),
            window: ProjectWindow {
                title: name.clone(),
                ..ProjectWindow::default()
            },
            name,
            startup_scene: PathBuf::from("assets/scenes/main.scene.ron"),
            scenes: BTreeMap::from([(
                LEGACY_STARTUP_SCENE_ID.to_string(),
                PathBuf::from("assets/scenes/main.scene.ron"),
            )]),
            asset_source: default_asset_source(),
            cooked_assets: default_cooked_assets(),
            backend: ProjectBackend::Vulkan,
            script_project: None,
            script_assembly: None,
            input_actions: Some(PathBuf::from("config/input.actions.json")),
        }
    }

    /// Validate schema, names, window bounds, and every project-relative path.
    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.schema != GAME_PROJECT_SCHEMA {
            return Err(ProjectError::UnsupportedSchema(self.schema.clone()));
        }
        let name = self.name.trim();
        if name.is_empty() || name.len() > 128 {
            return Err(ProjectError::InvalidName(self.name.clone()));
        }
        let title = self.window.title.trim();
        if title.is_empty() || title.len() > 256 {
            return Err(ProjectError::InvalidWindow(
                "window title must contain 1..=256 characters".into(),
            ));
        }
        if !(1..=16_384).contains(&self.window.width) || !(1..=16_384).contains(&self.window.height)
        {
            return Err(ProjectError::InvalidWindow(
                "window dimensions must be within 1..=16384".into(),
            ));
        }

        let mut portable_scene_ids = BTreeSet::new();
        let mut portable_scene_paths = BTreeSet::new();
        for (id, path) in &self.scenes {
            validate_scene_id(id)?;
            if !portable_scene_ids.insert(id.to_ascii_lowercase()) {
                return Err(ProjectError::InvalidSceneId(id.clone()));
            }
            validate_scene_path(&format!("scenes.{id}"), path)?;
            let portable_path = path
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            if !portable_scene_paths.insert(portable_path) {
                return Err(ProjectError::InvalidPath {
                    field: format!("scenes.{id}"),
                    path: path.clone(),
                    reason: "multiple scene IDs cannot reference the same portable path".into(),
                });
            }
        }
        if self.scenes.is_empty() {
            validate_scene_path("startup_scene", &self.startup_scene)?;
        } else if self.catalog_startup_scene_id().is_none() {
            return Err(ProjectError::InvalidPath {
                field: "startup_scene".into(),
                path: self.startup_scene.clone(),
                reason: "must name a scene ID or a cataloged .scene.ron path".into(),
            });
        }
        validate_relative_path("asset_source", &self.asset_source)?;
        validate_relative_path("cooked_assets", &self.cooked_assets)?;
        if let Some(script_project) = &self.script_project {
            validate_relative_path("script_project", script_project)?;
            if !script_project
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".csproj")
            {
                return Err(ProjectError::InvalidPath {
                    field: "script_project".into(),
                    path: script_project.clone(),
                    reason: "script project must end in .csproj".into(),
                });
            }
        }
        if let Some(script_assembly) = &self.script_assembly {
            validate_relative_path("script_assembly", script_assembly)?;
            if !script_assembly
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".dll")
            {
                return Err(ProjectError::InvalidPath {
                    field: "script_assembly".into(),
                    path: script_assembly.clone(),
                    reason: "script assembly must end in .dll".into(),
                });
            }
        }
        if self.script_project.is_some() && self.script_assembly.is_none() {
            return Err(ProjectError::InvalidPath {
                field: "script_assembly".into(),
                path: PathBuf::new(),
                reason: "script_project requires a compiled script_assembly path".into(),
            });
        }
        if let Some(input_actions) = &self.input_actions {
            validate_relative_path("input_actions", input_actions)?;
            if !input_actions
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".json")
            {
                return Err(ProjectError::InvalidPath {
                    field: "input_actions".into(),
                    path: input_actions.clone(),
                    reason: "input action map must be a JSON file".into(),
                });
            }
        }
        Ok(())
    }

    /// Return a normalized scene catalog, including the implicit legacy scene.
    pub fn scene_catalog(&self) -> BTreeMap<String, PathBuf> {
        if self.scenes.is_empty() {
            BTreeMap::from([(
                LEGACY_STARTUP_SCENE_ID.to_string(),
                self.startup_scene.clone(),
            )])
        } else {
            self.scenes.clone()
        }
    }

    fn catalog_startup_scene_id(&self) -> Option<&str> {
        if self.scenes.is_empty() {
            return Some(LEGACY_STARTUP_SCENE_ID);
        }

        if let Some(reference) = self.startup_scene.to_str() {
            if let Some((id, _)) = self.scenes.get_key_value(reference) {
                return Some(id.as_str());
            }
        }

        self.scenes
            .iter()
            .find(|(_, path)| paths_lexically_equal(path, &self.startup_scene))
            .map(|(id, _)| id.as_str())
    }

    /// Write a pretty, stable JSON manifest into `root`.
    pub fn write_to_root(&self, root: &Path) -> Result<PathBuf, ProjectError> {
        self.validate()?;
        std::fs::create_dir_all(root).map_err(|source| ProjectError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let path = root.join(GAME_PROJECT_FILE_NAME);
        let mut json = serde_json::to_string_pretty(self).map_err(ProjectError::Json)?;
        json.push('\n');
        std::fs::write(&path, json).map_err(|source| ProjectError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(path)
    }
}

/// A loaded project with all important paths resolved against its root.
#[derive(Clone, Debug)]
pub struct GameProject {
    pub manifest: ProjectManifest,
    pub manifest_path: PathBuf,
    pub root: PathBuf,
    pub startup_scene: PathBuf,
    pub asset_source: PathBuf,
    pub cooked_assets: PathBuf,
    pub script_project: Option<PathBuf>,
    pub script_assembly: Option<PathBuf>,
    pub input_actions: Option<PathBuf>,
}

impl GameProject {
    /// Load an authoring project from either its root directory or manifest.
    /// Source assets and an optional script project must exist.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        Self::load_with_authoring_requirements(path.as_ref(), true)
    }

    /// Load a deployable runtime project.
    ///
    /// The startup scene is required, while source assets and source script
    /// projects are deliberately optional because they are not shipped in a
    /// cooked game package.
    pub fn load_runtime(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        Self::load_with_authoring_requirements(path.as_ref(), false)
    }

    fn load_with_authoring_requirements(
        path: &Path,
        require_authoring_inputs: bool,
    ) -> Result<Self, ProjectError> {
        let requested = absolute_path(path)?;
        let manifest_path = if requested.is_dir() {
            requested.join(GAME_PROJECT_FILE_NAME)
        } else {
            requested
        };
        if !manifest_path.is_file() {
            return Err(ProjectError::ManifestNotFound(manifest_path));
        }
        let manifest_path =
            canonicalize_project_path(&manifest_path).map_err(|source| ProjectError::Io {
                path: manifest_path.clone(),
                source,
            })?;
        let root = manifest_path
            .parent()
            .ok_or_else(|| ProjectError::ManifestNotFound(manifest_path.clone()))?
            .to_path_buf();
        let json = std::fs::read_to_string(&manifest_path).map_err(|source| ProjectError::Io {
            path: manifest_path.clone(),
            source,
        })?;
        let manifest: ProjectManifest = serde_json::from_str(&json).map_err(ProjectError::Json)?;
        manifest.validate()?;

        let startup_scene_id = manifest
            .catalog_startup_scene_id()
            .expect("validated manifests have a cataloged startup scene")
            .to_string();
        let scene_catalog = manifest.scene_catalog();
        let mut resolved_scenes = BTreeMap::new();
        for (id, relative) in &scene_catalog {
            let field = if manifest.scenes.is_empty() {
                "startup_scene".to_string()
            } else {
                format!("scenes.{id}")
            };
            let resolved = resolve_inside_root(&root, &field, relative)?;
            if !resolved.is_file() {
                return Err(ProjectError::RequiredFileMissing(resolved));
            }
            resolved_scenes.insert(id.clone(), resolved);
        }
        let startup_scene = resolved_scenes
            .remove(&startup_scene_id)
            .expect("validated startup scene is present in the resolved catalog");
        let asset_source = resolve_inside_root(&root, "asset_source", &manifest.asset_source)?;
        if require_authoring_inputs && !asset_source.is_dir() {
            return Err(ProjectError::RequiredDirectoryMissing(asset_source));
        }
        let cooked_assets = resolve_inside_root(&root, "cooked_assets", &manifest.cooked_assets)?;
        let script_project = manifest
            .script_project
            .as_ref()
            .map(|path| resolve_inside_root(&root, "script_project", path))
            .transpose()?;
        let script_assembly = manifest
            .script_assembly
            .as_ref()
            .map(|path| resolve_inside_root(&root, "script_assembly", path))
            .transpose()?;
        let input_actions = manifest
            .input_actions
            .as_ref()
            .map(|path| resolve_inside_root(&root, "input_actions", path))
            .transpose()?;
        if let Some(path) = &input_actions {
            if !path.is_file() {
                return Err(ProjectError::RequiredFileMissing(path.clone()));
            }
        }
        if require_authoring_inputs {
            if let Some(path) = &script_project {
                if !path.is_file() {
                    return Err(ProjectError::RequiredFileMissing(path.clone()));
                }
            }
        } else if let Some(path) = &script_assembly {
            if !path.is_file() {
                return Err(ProjectError::RequiredFileMissing(path.clone()));
            }
        }

        Ok(Self {
            manifest,
            manifest_path,
            root,
            startup_scene,
            asset_source,
            cooked_assets,
            script_project,
            script_assembly,
            input_actions,
        })
    }

    /// Stable identifier of the configured startup scene.
    pub fn startup_scene_id(&self) -> &str {
        self.manifest
            .catalog_startup_scene_id()
            .unwrap_or(LEGACY_STARTUP_SCENE_ID)
    }

    /// Resolved startup scene path. This is the accessor counterpart of the
    /// retained public [`Self::startup_scene`] field.
    pub fn startup_scene_path(&self) -> &Path {
        &self.startup_scene
    }

    /// Resolve a cataloged scene by its stable ID.
    pub fn scene_path(&self, id: &str) -> Option<PathBuf> {
        if id == self.startup_scene_id() {
            return Some(self.startup_scene.clone());
        }
        let relative = self.manifest.scenes.get(id)?;
        resolve_inside_root(&self.root, &format!("scenes.{id}"), relative).ok()
    }

    /// List all scenes in deterministic ID order with project-root-resolved paths.
    pub fn scenes(&self) -> Vec<(String, PathBuf)> {
        self.manifest
            .scene_catalog()
            .into_keys()
            .filter_map(|id| self.scene_path(&id).map(|path| (id, path)))
            .collect()
    }
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("game project manifest was not found: {0}")]
    ManifestNotFound(PathBuf),
    #[error("unsupported game project schema: {0}")]
    UnsupportedSchema(String),
    #[error("invalid game project name: {0:?}")]
    InvalidName(String),
    #[error("invalid project scene ID: {0:?}")]
    InvalidSceneId(String),
    #[error("invalid project window: {0}")]
    InvalidWindow(String),
    #[error("invalid project path in {field}: {path:?} ({reason})")]
    InvalidPath {
        field: String,
        path: PathBuf,
        reason: String,
    },
    #[error("required project file is missing: {0}")]
    RequiredFileMissing(PathBuf),
    #[error("required project directory is missing: {0}")]
    RequiredDirectoryMissing(PathBuf),
    #[error("failed to access {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid project JSON: {0}")]
    Json(serde_json::Error),
}

fn default_window_title() -> String {
    "Game".into()
}

const fn default_window_width() -> u32 {
    1280
}

const fn default_window_height() -> u32 {
    720
}

fn default_asset_source() -> PathBuf {
    PathBuf::from("assets/source")
}

fn default_cooked_assets() -> PathBuf {
    PathBuf::from("build/cooked")
}

fn absolute_path(path: &Path) -> Result<PathBuf, ProjectError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        let current = std::env::current_dir().map_err(|source| ProjectError::Io {
            path: PathBuf::from("."),
            source,
        })?;
        Ok(current.join(path))
    }
}

fn validate_relative_path(field: &str, path: &Path) -> Result<(), ProjectError> {
    if path.as_os_str().is_empty() {
        return Err(invalid_path(field, path, "path is empty"));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(invalid_path(
                    field,
                    path,
                    "parent-directory traversal is forbidden",
                ));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(invalid_path(field, path, "absolute paths are forbidden"));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_scene_id(id: &str) -> Result<(), ProjectError> {
    let valid = !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(ProjectError::InvalidSceneId(id.to_string()))
    }
}

fn validate_scene_path(field: &str, path: &Path) -> Result<(), ProjectError> {
    validate_relative_path(field, path)?;
    if path
        .to_string_lossy()
        .to_ascii_lowercase()
        .ends_with(".scene.ron")
    {
        Ok(())
    } else {
        Err(ProjectError::InvalidPath {
            field: field.into(),
            path: path.to_path_buf(),
            reason: "scene path must end in .scene.ron".into(),
        })
    }
}

fn paths_lexically_equal(left: &Path, right: &Path) -> bool {
    left.components()
        .filter(|component| !matches!(component, Component::CurDir))
        .eq(right
            .components()
            .filter(|component| !matches!(component, Component::CurDir)))
}

fn resolve_inside_root(root: &Path, field: &str, relative: &Path) -> Result<PathBuf, ProjectError> {
    validate_relative_path(field, relative)?;
    let joined = root.join(relative);
    if joined.exists() {
        let canonical = canonicalize_project_path(&joined).map_err(|source| ProjectError::Io {
            path: joined.clone(),
            source,
        })?;
        if !canonical.starts_with(root) {
            return Err(invalid_path(
                field,
                relative,
                "resolved path escapes the project root",
            ));
        }
        Ok(canonical)
    } else {
        Ok(joined)
    }
}

fn canonicalize_project_path(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = path.canonicalize()?;
    #[cfg(windows)]
    {
        let display = canonical.to_string_lossy();
        if let Some(unc) = display.strip_prefix(r"\\?\UNC\") {
            return Ok(PathBuf::from(format!(r"\\{unc}")));
        }
        if let Some(ordinary) = display.strip_prefix(r"\\?\") {
            return Ok(PathBuf::from(ordinary));
        }
    }
    Ok(canonical)
}

fn invalid_path(field: &str, path: &Path, reason: &str) -> ProjectError {
    ProjectError::InvalidPath {
        field: field.into(),
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("engine-project-{name}-{unique}"))
    }

    fn write_minimal_project(root: &Path) -> PathBuf {
        std::fs::create_dir_all(root.join("assets/scenes")).expect("scene directory");
        std::fs::create_dir_all(root.join("assets/source")).expect("source directory");
        std::fs::create_dir_all(root.join("config")).expect("config directory");
        std::fs::write(root.join("assets/scenes/main.scene.ron"), "scene").expect("scene file");
        std::fs::write(root.join("config/input.actions.json"), "{}").expect("input config");
        ProjectManifest::new("Test Game")
            .write_to_root(root)
            .expect("manifest")
    }

    #[test]
    fn manifest_roundtrip_resolves_paths_from_project_root() {
        let root = unique_dir("roundtrip");
        let manifest_path = write_minimal_project(&root);
        let project = GameProject::load(&manifest_path).expect("load project");
        assert_eq!(project.manifest.name, "Test Game");
        assert_eq!(
            project.root,
            canonicalize_project_path(&root).expect("canonical root")
        );
        assert!(project.startup_scene.is_file());
        assert!(project.asset_source.is_dir());
        assert_eq!(project.startup_scene_id(), LEGACY_STARTUP_SCENE_ID);
        assert_eq!(project.startup_scene_path(), project.startup_scene);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_manifest_without_scene_catalog_gets_a_stable_scene_id() {
        let root = unique_dir("legacy-scene-catalog");
        let manifest_path = write_minimal_project(&root);
        let mut json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&manifest_path).expect("read generated manifest"),
        )
        .expect("manifest JSON");
        json.as_object_mut()
            .expect("manifest object")
            .remove("scenes");
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&json).expect("serialize legacy manifest"),
        )
        .expect("write legacy manifest");

        let project = GameProject::load(&root).expect("load legacy project");
        assert!(project.manifest.scenes.is_empty());
        assert_eq!(project.startup_scene_id(), LEGACY_STARTUP_SCENE_ID);
        assert_eq!(
            project.scene_path(LEGACY_STARTUP_SCENE_ID).as_deref(),
            Some(project.startup_scene_path())
        );
        assert_eq!(
            project.scenes(),
            vec![(
                LEGACY_STARTUP_SCENE_ID.to_string(),
                project.startup_scene.clone()
            )]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn catalog_startup_scene_can_be_selected_by_id() {
        let root = unique_dir("catalog-id");
        write_minimal_project(&root);
        std::fs::write(root.join("assets/scenes/level-two.scene.ron"), "scene")
            .expect("second scene");

        let mut manifest = ProjectManifest::new("Catalog");
        manifest.scenes.insert(
            "level_two".into(),
            PathBuf::from("assets/scenes/level-two.scene.ron"),
        );
        manifest.startup_scene = PathBuf::from("level_two");
        manifest.write_to_root(&root).expect("catalog manifest");

        let project = GameProject::load(&root).expect("load catalog project");
        assert_eq!(project.startup_scene_id(), "level_two");
        assert!(project
            .startup_scene_path()
            .ends_with("assets/scenes/level-two.scene.ron"));
        assert!(project
            .scene_path(LEGACY_STARTUP_SCENE_ID)
            .expect("main scene")
            .ends_with("assets/scenes/main.scene.ron"));
        assert_eq!(
            project
                .scenes()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec!["level_two".to_string(), "main".to_string()]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn catalog_startup_scene_accepts_legacy_path_reference() {
        let root = unique_dir("catalog-path");
        write_minimal_project(&root);
        std::fs::write(root.join("assets/scenes/level-two.scene.ron"), "scene")
            .expect("second scene");

        let mut manifest = ProjectManifest::new("Catalog Path");
        manifest.scenes.insert(
            "level_two".into(),
            PathBuf::from("assets/scenes/level-two.scene.ron"),
        );
        manifest.startup_scene = PathBuf::from("./assets/scenes/level-two.scene.ron");
        manifest.write_to_root(&root).expect("catalog manifest");

        let project = GameProject::load(&root).expect("load catalog project");
        assert_eq!(project.startup_scene_id(), "level_two");
        assert_eq!(
            project.scene_path("level_two").as_deref(),
            Some(project.startup_scene_path())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn manifest_rejects_unknown_startup_id_and_unsafe_catalog_paths() {
        let mut manifest = ProjectManifest::new("Invalid Catalog");
        manifest.startup_scene = PathBuf::from("unknown");
        assert!(matches!(
            manifest.validate(),
            Err(ProjectError::InvalidPath { field, .. }) if field == "startup_scene"
        ));

        manifest.startup_scene = PathBuf::from(LEGACY_STARTUP_SCENE_ID);
        manifest
            .scenes
            .insert("outside".into(), PathBuf::from("../outside.scene.ron"));
        assert!(matches!(
            manifest.validate(),
            Err(ProjectError::InvalidPath { field, .. }) if field == "scenes.outside"
        ));

        manifest.scenes.remove("outside");
        manifest.scenes.insert(
            "invalid/id".into(),
            PathBuf::from("assets/scenes/other.scene.ron"),
        );
        assert!(matches!(
            manifest.validate(),
            Err(ProjectError::InvalidSceneId(id)) if id == "invalid/id"
        ));
    }

    #[test]
    fn manifest_rejects_portable_scene_id_and_path_collisions() {
        let mut manifest = ProjectManifest::new("Portable Catalog");
        manifest.scenes.insert(
            "MAIN".into(),
            PathBuf::from("assets/scenes/other.scene.ron"),
        );
        assert!(matches!(
            manifest.validate(),
            Err(ProjectError::InvalidSceneId(id)) if id == "main" || id == "MAIN"
        ));

        manifest.scenes.remove("MAIN");
        manifest.scenes.insert(
            "duplicate".into(),
            PathBuf::from("ASSETS/SCENES/MAIN.SCENE.RON"),
        );
        assert!(matches!(
            manifest.validate(),
            Err(ProjectError::InvalidPath { field, reason, .. })
                if (field == "scenes.main" || field == "scenes.duplicate")
                    && reason.contains("same portable path")
        ));
    }

    #[test]
    fn loading_catalog_requires_every_declared_scene() {
        let root = unique_dir("catalog-missing-entry");
        write_minimal_project(&root);
        let mut manifest = ProjectManifest::new("Missing Catalog Scene");
        manifest.scenes.insert(
            "missing".into(),
            PathBuf::from("assets/scenes/missing.scene.ron"),
        );
        manifest.write_to_root(&root).expect("catalog manifest");

        assert!(matches!(
            GameProject::load(&root),
            Err(ProjectError::RequiredFileMissing(path))
                if path.ends_with("assets/scenes/missing.scene.ron")
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn directory_input_uses_conventional_manifest_name() {
        let root = unique_dir("directory");
        write_minimal_project(&root);
        let project = GameProject::load(&root).expect("load project directory");
        assert!(project.manifest_path.ends_with(GAME_PROJECT_FILE_NAME));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_load_does_not_require_source_assets() {
        let root = unique_dir("runtime-only");
        write_minimal_project(&root);
        std::fs::remove_dir_all(root.join("assets/source")).expect("remove authoring sources");

        assert!(matches!(
            GameProject::load(&root),
            Err(ProjectError::RequiredDirectoryMissing(_))
        ));
        let runtime = GameProject::load_runtime(&root).expect("load runtime project");
        assert!(runtime.startup_scene.is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_manifest_without_input_actions_remains_loadable() {
        let root = unique_dir("legacy-input");
        std::fs::create_dir_all(root.join("assets/scenes")).unwrap();
        std::fs::create_dir_all(root.join("assets/source")).unwrap();
        std::fs::write(root.join("assets/scenes/main.scene.ron"), "scene").unwrap();
        let mut manifest = ProjectManifest::new("Legacy");
        manifest.input_actions = None;
        manifest.write_to_root(&root).unwrap();

        let project = GameProject::load(&root).unwrap();
        assert!(project.input_actions.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn configured_input_actions_are_required_for_authoring_and_runtime() {
        let root = unique_dir("missing-input");
        write_minimal_project(&root);
        std::fs::remove_file(root.join("config/input.actions.json")).unwrap();

        assert!(matches!(
            GameProject::load(&root),
            Err(ProjectError::RequiredFileMissing(_))
        ));
        assert!(matches!(
            GameProject::load_runtime(&root),
            Err(ProjectError::RequiredFileMissing(_))
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn script_source_is_authoring_only_and_assembly_is_runtime_required() {
        let root = unique_dir("script-inputs");
        write_minimal_project(&root);
        std::fs::create_dir_all(root.join("scripts/GameScripts")).unwrap();
        std::fs::write(
            root.join("scripts/GameScripts/GameScripts.csproj"),
            "<Project />",
        )
        .unwrap();
        let mut manifest = ProjectManifest::new("Scripted");
        manifest.script_project = Some(PathBuf::from("scripts/GameScripts/GameScripts.csproj"));
        manifest.script_assembly = Some(PathBuf::from("build/scripts/GameScripts.dll"));
        manifest.write_to_root(&root).unwrap();

        GameProject::load(&root).expect("authoring load does not require a built DLL");
        assert!(matches!(
            GameProject::load_runtime(&root),
            Err(ProjectError::RequiredFileMissing(_))
        ));

        std::fs::create_dir_all(root.join("build/scripts")).unwrap();
        std::fs::write(root.join("build/scripts/GameScripts.dll"), b"assembly").unwrap();
        std::fs::remove_dir_all(root.join("scripts")).unwrap();
        std::fs::remove_dir_all(root.join("assets/source")).unwrap();
        GameProject::load_runtime(&root).expect("runtime load only requires the DLL");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn manifest_rejects_wrong_script_extensions() {
        let mut manifest = ProjectManifest::new("Scripts");
        manifest.script_project = Some(PathBuf::from("scripts/GameScripts.txt"));
        assert!(matches!(
            manifest.validate(),
            Err(ProjectError::InvalidPath { .. })
        ));

        manifest.script_project = Some(PathBuf::from("scripts/GameScripts.csproj"));
        manifest.script_assembly = Some(PathBuf::from("build/scripts/GameScripts.exe"));
        assert!(matches!(
            manifest.validate(),
            Err(ProjectError::InvalidPath { .. })
        ));
    }

    #[test]
    fn manifest_rejects_unsafe_or_absolute_paths() {
        let mut manifest = ProjectManifest::new("Unsafe");
        manifest.startup_scene = PathBuf::from("../outside.scene.ron");
        assert!(matches!(
            manifest.validate(),
            Err(ProjectError::InvalidPath { .. })
        ));

        manifest.startup_scene = PathBuf::from(r"C:\outside.scene.ron");
        assert!(matches!(
            manifest.validate(),
            Err(ProjectError::InvalidPath { .. })
        ));
    }

    #[test]
    fn load_reports_missing_startup_scene() {
        let root = unique_dir("missing-scene");
        std::fs::create_dir_all(root.join("assets/source")).expect("source directory");
        let path = ProjectManifest::new("Missing Scene")
            .write_to_root(&root)
            .expect("manifest");
        assert!(matches!(
            GameProject::load(path),
            Err(ProjectError::RequiredFileMissing(_))
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
