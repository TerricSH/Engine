use std::collections::{BTreeSet, VecDeque};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
use engine_asset::cook::{
    cook_orchestrate_checked_with_registry, decode_cooked_material, read_cooked_artifact,
    AssetType, DependencyGraph, SourceAssetEntry, SourceManifest,
};
use engine_asset::project::GameProject;
use engine_core::game_loop::GameLoop;
use engine_core::{create_vulkan_backend_renderer, EngineConfig, EngineRuntime};
use engine_editor::animation_preview::AnimationPreviewPanel;
use engine_editor::asset_browser::{
    refresh_project_asset_list, AssetBrowserPanel as ProjectAssetBrowserPanel,
};
use engine_editor::gizmo::{update_gizmo, GizmoMode, GizmoSpace, GizmoSystem};
use engine_editor::gizmo_overlay::build_gizmo_ui_batch;
use engine_editor::material_editor::{
    load_material, MaterialEditorPanel, MaterialSaveAccess, MaterialSaveRequest,
};
use engine_editor::{
    create_prefab_asset_from_scene, prepare_prefab_instantiation_from_registry,
    prepare_unpack_prefab, EditorPlayMode, EditorPlaySession, EditorScene,
    PrefabAssetCreateRequest, PrefabAuthoringError, PrefabUnpackMode, SceneViewPanel,
};
use engine_editor_host::{EditorHostClient, EditorHostConfig, HostDirective, HostEvent, WebAsset};
use engine_renderer::{AssetId, Rect as RendererRect, UiBatch};
use engine_scene::components::{Camera, CameraProjection, Transform};
use engine_scene::{
    extract_renderer_input_from_world_with_viewport, ComponentRecord, Entity, EntityRecord,
    RenderViewportContext, Scene, SceneSettings, World,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity, PersistentId, SchemaVersion, Value};
use glam::{Mat4, Quat, Vec2, Vec3};

mod dispatch;
mod protocol;
mod snapshot;
use protocol::{InputModifiers, ScreenRect};

const BUILTIN_DEFAULT_MATERIAL_ID: &str = "mat-default";
const EDITOR_CAMERA_ID_PREFIX: &str = "__engine_editor_camera";
const EDITOR_LIGHT_ID_PREFIX: &str = "__engine_editor_light";
const DEFAULT_REACT_LAYOUT: &str = r#"{"zones":{"left":{"panels":["hierarchy"],"active":"hierarchy","collapsed":false},"center":{"panels":["scene","game"],"active":"scene","collapsed":false},"right":{"panels":["inspector","settings"],"active":"inspector","collapsed":false},"bottom":{"panels":["project","console","material","animation","profiler","terrain","build"],"active":"project","collapsed":false}},"leftWidth":272,"rightWidth":326,"bottomHeight":260}"#;

static EDITOR_WEB_ASSETS: &[WebAsset] = &[
    WebAsset::new(
        "index.html",
        include_bytes!("../editor-web/dist/index.html"),
    ),
    WebAsset::new(
        "assets/editor.js",
        include_bytes!("../editor-web/dist/assets/editor.js"),
    ),
    WebAsset::new(
        "assets/editor.css",
        include_bytes!("../editor-web/dist/assets/editor.css"),
    ),
];

fn reveal_in_file_manager(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");

    command
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not reveal {}: {error}", path.display()))
}

fn launch_editor_window(project_path: &Path) -> Result<u32, String> {
    let project = GameProject::load(project_path)
        .map_err(|error| format!("Could not open project {}: {error}", project_path.display()))?;
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not resolve the editor executable: {error}"))?;
    std::process::Command::new(executable)
        .arg("editor")
        .arg(&project.manifest_path)
        .spawn()
        .map(|child| child.id())
        .map_err(|error| {
            format!(
                "Could not launch an editor for {}: {error}",
                project.manifest_path.display()
            )
        })
}

fn selected_material_asset(scene: &Scene, selected: Option<&PersistentId>) -> Option<String> {
    let selected = selected?;
    let entity = scene
        .entities
        .iter()
        .find(|entity| &entity.persistent_id == selected)?;
    let renderable = entity.components.get("engine.renderable")?;
    match renderable.fields.get("material")? {
        Value::Asset(asset) => Some(asset.id.clone()),
        Value::Str(asset) => Some(asset.clone()),
        _ => None,
    }
}

fn assign_material_to_selected_command(
    editor_scene: &EditorScene,
    material_asset: &str,
) -> Result<Box<dyn engine_editor::Command>, String> {
    let material_asset = material_asset.trim();
    if material_asset.is_empty() {
        return Err("No material is open in the Material Editor".to_string());
    }
    let entity_id = editor_scene
        .selected_entity
        .as_ref()
        .ok_or_else(|| "Select an entity with a Mesh Renderer component".to_string())?;
    let entity = editor_scene
        .scene
        .entities
        .iter()
        .find(|entity| &entity.persistent_id == entity_id)
        .ok_or_else(|| format!("Selected entity '{entity_id}' is no longer in the scene"))?;
    let renderable = entity
        .components
        .get("engine.renderable")
        .ok_or_else(|| format!("Entity '{entity_id}' does not have a Mesh Renderer component"))?;
    if !renderable.fields.contains_key("material") {
        return Err(format!(
            "Entity '{entity_id}' has no writable Mesh Renderer material field"
        ));
    }

    Ok(Box::new(engine_editor::SetComponentField::new(
        entity_id.clone(),
        "engine.renderable".to_string(),
        "material".to_string(),
        Value::Asset(AssetId::new(material_asset)),
    )))
}

#[derive(Clone, Debug)]
struct ProjectMaterialSource {
    source_path: PathBuf,
}

#[derive(Clone, Debug)]
struct MaterialSaveOutcome {
    source_path: PathBuf,
    cooked_path: PathBuf,
}

fn project_material_save_access(project: &GameProject, material_id: &str) -> MaterialSaveAccess {
    match resolve_project_material_source(project, material_id) {
        Ok(_) => MaterialSaveAccess::Writable,
        Err(reason) => MaterialSaveAccess::ReadOnly(reason),
    }
}

/// Resolve one exact project-owned material entry. Built-ins, unknown IDs,
/// duplicate declarations, non-material declarations, and unsafe source paths
/// are deliberately rejected before any file is written.
fn resolve_project_material_source(
    project: &GameProject,
    material_id: &str,
) -> Result<ProjectMaterialSource, String> {
    if material_id == BUILTIN_DEFAULT_MATERIAL_ID {
        return Err(format!(
            "Built-in material '{BUILTIN_DEFAULT_MATERIAL_ID}' is read-only because it has no project source Material."
        ));
    }
    if material_id.trim().is_empty() {
        return Err("The selected material ID is empty and cannot be saved.".to_string());
    }

    let source_root = std::fs::canonicalize(&project.asset_source).map_err(|error| {
        format!(
            "Could not resolve project asset source {}: {error}",
            project.asset_source.display()
        )
    })?;
    if !source_root.is_dir() {
        return Err(format!(
            "Project asset source is not a directory: {}",
            source_root.display()
        ));
    }

    let mut manifest_paths = Vec::new();
    for entry in std::fs::read_dir(&source_root).map_err(|error| {
        format!(
            "Could not enumerate project source manifests in {}: {error}",
            source_root.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "Could not enumerate project source manifests in {}: {error}",
                source_root.display()
            )
        })?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("manifest"))
        {
            manifest_paths.push(path);
        }
    }
    manifest_paths.sort();

    let mut matches: Vec<(PathBuf, SourceAssetEntry)> = Vec::new();
    for manifest_path in manifest_paths {
        let bytes = std::fs::read(&manifest_path).map_err(|error| {
            format!(
                "Could not read source manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        let manifest: SourceManifest = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "Invalid source manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        if manifest.schema_version != CURRENT_MANIFEST_VERSION {
            return Err(format!(
                "Source manifest {} uses unsupported schema {}.{}.{}.",
                manifest_path.display(),
                manifest.schema_version.major,
                manifest.schema_version.minor,
                manifest.schema_version.patch
            ));
        }
        matches.extend(
            manifest
                .assets
                .into_iter()
                .filter(|entry| entry.id.id == material_id)
                .map(|entry| (manifest_path.clone(), entry)),
        );
    }

    if matches.is_empty() {
        return Err(format!(
            "Material '{material_id}' is not declared by a project source manifest and is read-only."
        ));
    }
    if matches.len() != 1 {
        let manifests = matches
            .iter()
            .map(|(path, _)| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Material ID '{material_id}' is ambiguous because it is declared {} times: {manifests}",
            matches.len()
        ));
    }

    let (manifest_path, entry) = matches.pop().expect("one match was checked above");
    if entry.asset_type != AssetType::Material {
        return Err(format!(
            "Asset '{material_id}' in {} is {:?}, not a project source Material.",
            manifest_path.display(),
            entry.asset_type
        ));
    }

    let relative_source = Path::new(&entry.source_path);
    if relative_source.as_os_str().is_empty()
        || !relative_source
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "Material '{material_id}' has an unsafe source_path '{}'; expected a non-empty relative path without '.' or '..'.",
            entry.source_path
        ));
    }
    let source_path =
        std::fs::canonicalize(source_root.join(relative_source)).map_err(|error| {
            format!(
                "Could not resolve source for material '{material_id}' at '{}': {error}",
                entry.source_path
            )
        })?;
    if !source_path.starts_with(&source_root) || !source_path.is_file() {
        return Err(format!(
            "Material '{material_id}' source resolves outside the project source directory or is not a regular file: {}",
            source_path.display()
        ));
    }

    Ok(ProjectMaterialSource { source_path })
}

fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

/// Flush a same-directory temporary file before atomically replacing `path`.
/// `tempfile::persist` uses the platform's replace-existing primitive, so the
/// original remains reachable if replacement fails.
fn atomic_write_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "Could not create a temporary file beside {}: {error}",
            path.display()
        )
    })?;
    temporary
        .write_all(contents)
        .map_err(|error| format!("Could not write temporary {}: {error}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("Could not flush temporary {}: {error}", path.display()))?;
    let persisted = temporary.persist(path).map_err(|error| {
        format!(
            "Could not atomically replace {}: {}",
            path.display(),
            error.error
        )
    })?;
    persisted
        .sync_all()
        .map_err(|error| format!("Could not flush {}: {error}", path.display()))?;
    Ok(())
}

fn save_scene_atomically(scene: &Scene, path: &Path) -> Result<(), String> {
    let serialized = ron::ser::to_string_pretty(scene, ron::ser::PrettyConfig::default())
        .map_err(|error| format!("Could not serialize scene '{}': {error}", scene.scene_id))?;
    atomic_write_file(path, serialized.as_bytes())
}

fn checked_cook_material_bytes(
    project: &GameProject,
    material_id: &str,
) -> Result<Vec<u8>, String> {
    let staging_parent = project
        .cooked_assets
        .parent()
        .unwrap_or(project.root.as_path());
    if !staging_parent.starts_with(&project.root) {
        return Err(format!(
            "Cook staging directory would escape the project root: {}",
            staging_parent.display()
        ));
    }
    std::fs::create_dir_all(staging_parent).map_err(|error| {
        format!(
            "Could not create material cook staging parent {}: {error}",
            staging_parent.display()
        )
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".material-save-cook-")
        .tempdir_in(staging_parent)
        .map_err(|error| format!("Could not create material cook staging directory: {error}"))?;
    let mut graph = DependencyGraph::new();
    let runtime_builder = EngineRuntime::builder(EngineConfig {
        application_name: format!("{}-material-cook", project.manifest.name),
        gpu_timestamps: true,
    });
    let report = cook_orchestrate_checked_with_registry(
        &project.asset_source,
        staging.path(),
        &mut graph,
        runtime_builder.asset_type_registry(),
    );
    if !report.is_success() {
        let details = report
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.severity,
                    DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
                )
            })
            .take(8)
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(if details.is_empty() {
            "Checked project asset cook failed without an error diagnostic.".to_string()
        } else {
            format!("Checked project asset cook failed:\n{details}")
        });
    }
    let result = report
        .results
        .iter()
        .find(|result| result.success && result.asset_id == material_id)
        .ok_or_else(|| {
            format!("Checked cook did not report material asset '{material_id}' as successful.")
        })?;
    if result.asset_type != AssetType::Material {
        return Err(format!(
            "Checked cook reported asset '{material_id}' as {:?}, not Material.",
            result.asset_type
        ));
    }
    let staging_root = std::fs::canonicalize(staging.path()).map_err(|error| {
        format!(
            "Could not resolve material cook staging directory {}: {error}",
            staging.path().display()
        )
    })?;
    let staged_path =
        std::fs::canonicalize(staging.path().join(&result.output_path)).map_err(|error| {
            format!(
                "Could not resolve staged material artifact {}: {error}",
                result.output_path.display()
            )
        })?;
    if !staged_path.starts_with(&staging_root) || !staged_path.is_file() {
        return Err(format!(
            "Checked cook produced an unsafe material artifact path: {}",
            staged_path.display()
        ));
    }
    let artifact = read_cooked_artifact(&staged_path).map_err(|error| {
        format!(
            "Staged material artifact {} failed header validation: {error}",
            staged_path.display()
        )
    })?;
    decode_cooked_material(&artifact).map_err(|error| {
        format!(
            "Staged material artifact {} failed payload validation: {error}",
            staged_path.display()
        )
    })?;
    std::fs::read(&staged_path).map_err(|error| {
        format!(
            "Could not read staged material artifact {}: {error}",
            staged_path.display()
        )
    })
}

fn rollback_material_source(
    failure: String,
    source_path: &Path,
    original_source: &[u8],
    cooked_restore: Option<(&Path, Option<&[u8]>)>,
) -> String {
    let mut rollback_errors = Vec::new();
    if let Err(error) = atomic_write_file(source_path, original_source) {
        rollback_errors.push(format!("source restore failed: {error}"));
    }
    if let Some((cooked_path, original_cooked)) = cooked_restore {
        let restore = match original_cooked {
            Some(bytes) => atomic_write_file(cooked_path, bytes),
            None if cooked_path.exists() => std::fs::remove_file(cooked_path).map_err(|error| {
                format!(
                    "Could not remove new cooked artifact {}: {error}",
                    cooked_path.display()
                )
            }),
            None => Ok(()),
        };
        if let Err(error) = restore {
            rollback_errors.push(format!("cooked artifact restore failed: {error}"));
        }
    }

    if rollback_errors.is_empty() {
        format!("{failure}\nThe original material source was restored.")
    } else {
        format!(
            "{failure}\nMaterial save rollback also failed:\n{}",
            rollback_errors.join("\n")
        )
    }
}

fn save_project_material(
    runtime: &mut EngineRuntime,
    project: &GameProject,
    request: &MaterialSaveRequest,
) -> Result<MaterialSaveOutcome, String> {
    let resolved = resolve_project_material_source(project, &request.material_asset)?;
    let original_source = std::fs::read(&resolved.source_path).map_err(|error| {
        format!(
            "Could not back up material source {}: {error}",
            resolved.source_path.display()
        )
    })?;
    let mut source_json = serde_json::to_vec_pretty(&request.source)
        .map_err(|error| format!("Could not serialize MaterialSource-v0: {error}"))?;
    source_json.push(b'\n');
    atomic_write_file(&resolved.source_path, &source_json)?;

    let cooked_bytes = match checked_cook_material_bytes(project, &request.material_asset) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(rollback_material_source(
                error,
                &resolved.source_path,
                &original_source,
                None,
            ));
        }
    };

    let cooked_path = project
        .cooked_assets
        .join(format!("{}.cooked", request.material_asset));
    let original_cooked = match read_optional_file(&cooked_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(rollback_material_source(
                error,
                &resolved.source_path,
                &original_source,
                None,
            ));
        }
    };
    if let Err(error) = atomic_write_file(&cooked_path, &cooked_bytes) {
        return Err(rollback_material_source(
            error,
            &resolved.source_path,
            &original_source,
            None,
        ));
    }

    if let Err(error) = super::project_app::load_project_assets(runtime, project) {
        return Err(rollback_material_source(
            format!("Runtime registry refresh rejected the saved material: {error}"),
            &resolved.source_path,
            &original_source,
            Some((&cooked_path, original_cooked.as_deref())),
        ));
    }

    Ok(MaterialSaveOutcome {
        source_path: resolved.source_path,
        cooked_path,
    })
}

fn editor_render_viewport(
    viewport: ScreenRect,
    scale_factor: f64,
    window_size: Vec2,
) -> Option<(Vec2, Vec2, RenderViewportContext)> {
    if !viewport.x.is_finite()
        || !viewport.y.is_finite()
        || !viewport.width.is_finite()
        || !viewport.height.is_finite()
        || !scale_factor.is_finite()
        || scale_factor <= 0.0
        || viewport.width <= 0.0
        || viewport.height <= 0.0
        || !window_size.is_finite()
        || window_size.x <= 0.0
        || window_size.y <= 0.0
    {
        return None;
    }

    // DOMRect coordinates are CSS logical pixels. Mapping both edges independently keeps
    // adjacent dock regions seam-free on fractional DPI scales.
    let scale = scale_factor as f32;
    let min = Vec2::new((viewport.x * scale).round(), (viewport.y * scale).round())
        .clamp(Vec2::ZERO, window_size);
    let max = Vec2::new(
        ((viewport.x + viewport.width) * scale).round(),
        ((viewport.y + viewport.height) * scale).round(),
    )
    .clamp(Vec2::ZERO, window_size);
    if max.x <= min.x || max.y <= min.y {
        return None;
    }

    let normalized = RendererRect {
        min: [min.x / window_size.x, min.y / window_size.y],
        max: [max.x / window_size.x, max.y / window_size.y],
    };
    let render_viewport = RenderViewportContext::new(
        window_size.x.round() as u32,
        window_size.y.round() as u32,
        normalized,
    )?;
    Some((min, max, render_viewport))
}

fn log_scene_diagnostics(context: &str, diagnostics: Vec<engine_serialize::Diagnostic>) {
    for diagnostic in diagnostics {
        tracing::error!(
            code = diagnostic.code,
            entity = ?diagnostic.entity,
            message = diagnostic.message,
            "{context}"
        );
    }
}

fn summarize_scene_diagnostics(diagnostics: &[engine_serialize::Diagnostic]) -> String {
    diagnostics
        .iter()
        .take(4)
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>()
        .join("; ")
}

fn editor_camera_entity(scene: &Scene) -> EntityRecord {
    let mut suffix = 0_u64;
    let persistent_id = loop {
        let candidate = if suffix == 0 {
            EDITOR_CAMERA_ID_PREFIX.to_string()
        } else {
            format!("{EDITOR_CAMERA_ID_PREFIX}_{suffix}")
        };
        if scene
            .entities
            .iter()
            .all(|entity| entity.persistent_id != candidate)
        {
            break candidate;
        }
        suffix += 1;
    };
    let record = |fields| ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    };
    EntityRecord {
        persistent_id,
        parent: None,
        name: Some("Editor Camera".into()),
        enabled: true,
        components: std::collections::BTreeMap::from([
            (
                "engine.camera".into(),
                record(std::collections::BTreeMap::from([
                    ("clear_flags".into(), Value::UInt(3)),
                    (
                        "clear_color".into(),
                        Value::Color([0.055, 0.06, 0.075, 1.0]),
                    ),
                    ("aperture".into(), Value::Float32(1.0)),
                    ("shutter_speed".into(), Value::Float32(1.0)),
                    ("iso".into(), Value::Float32(100.0)),
                ])),
            ),
            (
                "engine.transform".into(),
                record(std::collections::BTreeMap::from([
                    ("translation".into(), Value::Vec3([0.0, 0.0, 5.0])),
                    ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
                    ("scale".into(), Value::Vec3([1.0, 1.0, 1.0])),
                ])),
            ),
        ]),
    }
}

fn editor_light_entity(scene: &Scene) -> EntityRecord {
    let mut suffix = 0_u64;
    let persistent_id = loop {
        let candidate = if suffix == 0 {
            EDITOR_LIGHT_ID_PREFIX.to_string()
        } else {
            format!("{EDITOR_LIGHT_ID_PREFIX}_{suffix}")
        };
        if scene
            .entities
            .iter()
            .all(|entity| entity.persistent_id != candidate)
        {
            break candidate;
        }
        suffix += 1;
    };
    EntityRecord {
        persistent_id,
        parent: None,
        name: Some("Editor Light".into()),
        enabled: true,
        components: std::collections::BTreeMap::from([(
            "engine.light".into(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: std::collections::BTreeMap::from([
                    ("kind".into(), Value::Enum("Directional".into())),
                    ("color".into(), Value::Vec3([1.0, 0.96, 0.9])),
                    ("intensity".into(), Value::Float32(2.5)),
                    ("direction".into(), Value::Vec3([-0.35, -0.8, -0.45])),
                    ("shadow_mode".into(), Value::UInt(0)),
                ]),
            },
        )]),
    }
}

fn editor_preview_scene(
    runtime: &engine_core::EngineRuntime,
    authoring_scene: &Scene,
) -> (Scene, Vec<Diagnostic>) {
    authoring_preview_scene(runtime, authoring_scene, true)
}

fn game_preview_scene(
    runtime: &engine_core::EngineRuntime,
    authoring_scene: &Scene,
) -> (Scene, Vec<Diagnostic>) {
    authoring_preview_scene(runtime, authoring_scene, false)
}

fn authoring_preview_scene(
    runtime: &engine_core::EngineRuntime,
    authoring_scene: &Scene,
    use_editor_camera: bool,
) -> (Scene, Vec<Diagnostic>) {
    let mut preview = authoring_scene.clone();
    let mut diagnostics = Vec::new();
    for entity in &mut preview.entities {
        // Edit mode displays the authoring scene but must not instantiate game
        // behaviours.  Otherwise every inspector edit or gizmo commit reloads
        // the preview and repeatedly invokes OnDestroy/OnCreate outside Play.
        // The component remains intact in `authoring_scene` and is restored by
        // the normal Play transition.
        entity.components.remove("engine.script");
        if use_editor_camera {
            if let Some(camera) = entity.components.get_mut("engine.camera") {
                // Authoring cameras remain as selectable entities (including
                // their Transform), but only the dedicated editor camera renders
                // while outside Play.
                camera.enabled = false;
            }
        }

        let Some(renderable) = entity.components.get_mut("engine.renderable") else {
            continue;
        };
        for (field, fallback) in [("mesh", "mesh-cube"), ("material", "mat-default")] {
            let Some(Value::Asset(asset)) = renderable.fields.get(field) else {
                continue;
            };
            if runtime.asset_registry().contains(asset) {
                continue;
            }
            let missing = asset.clone();
            renderable
                .fields
                .insert(field.into(), Value::Asset(AssetId::new(fallback)));
            let mut diagnostic = Diagnostic::new(
                    "EDASSET_MISSING",
                    DiagnosticSeverity::Warning,
                    "editor.asset-browser",
                    format!(
                        "asset '{}' is missing; editor preview uses '{}' until the authoring reference is repaired",
                        missing.id, fallback
                    ),
                )
                .entity(entity.persistent_id.clone())
                .path(format!(
                    "entities[{}].components[engine.renderable].fields[{field}]",
                    entity.persistent_id
                ));
            diagnostic.asset = Some(missing);
            diagnostics.push(diagnostic);
        }
    }
    if use_editor_camera {
        let editor_camera = editor_camera_entity(&preview);
        preview.scene_settings.active_camera = Some(editor_camera.persistent_id.clone());
        preview.entities.push(editor_camera);
        let has_scene_light = preview.entities.iter().any(|entity| {
            entity.enabled
                && entity
                    .components
                    .get("engine.light")
                    .is_some_and(|light| light.enabled)
        });
        if !has_scene_light {
            let editor_light = editor_light_entity(&preview);
            preview.entities.push(editor_light);
        }
    }
    (preview, diagnostics)
}

fn restore_editor_preview(
    game_loop: &mut GameLoop,
    authoring_scene: &Scene,
) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    let (preview_scene, warnings) = editor_preview_scene(&game_loop.runtime, authoring_scene);
    game_loop.load_scene(preview_scene)?;
    game_loop.init_physics();
    // Play-attachment errors were copied into the editor diagnostics by the
    // caller; do not let them poison later edit-mode health checks.
    game_loop.runtime.diagnostics_collector_mut().clear_frame();
    Ok(warnings)
}

fn synchronize_editor_preview(game_loop: &mut GameLoop, editor_scene: &mut EditorScene) {
    let (preview_scene, missing_diagnostics) =
        editor_preview_scene(&game_loop.runtime, &editor_scene.scene);
    editor_scene.diagnostics.clear();
    editor_scene.diagnostics.push_many(missing_diagnostics);
    if let Err(diagnostics) = game_loop.load_scene(preview_scene) {
        log_scene_diagnostics("editor scene synchronisation failed", diagnostics);
    } else {
        game_loop.init_physics();
    }
}

fn synchronize_game_preview(game_loop: &mut GameLoop, editor_scene: &mut EditorScene) {
    let (preview_scene, missing_diagnostics) =
        game_preview_scene(&game_loop.runtime, &editor_scene.scene);
    editor_scene.diagnostics.clear();
    editor_scene.diagnostics.push_many(missing_diagnostics);
    if let Err(diagnostics) = game_loop.load_scene(preview_scene) {
        log_scene_diagnostics("game-view preview synchronisation failed", diagnostics);
    } else {
        game_loop.init_physics();
    }
}

fn recover_play_after_script_error(
    play_session: &mut EditorPlaySession,
    game_loop: &mut GameLoop,
    error: impl Into<String>,
) -> Vec<Diagnostic> {
    let error = error.into();
    let mut diagnostics = vec![Diagnostic::new(
        "EDPLAY_SCRIPT_UPDATE_FAILED",
        DiagnosticSeverity::Error,
        "editor.play-mode",
        format!("Play stopped after a game script update failed: {error}"),
    )];
    let mut preview_warnings = Vec::new();
    let restore_result = play_session.stop(|authoring_scene| {
        match restore_editor_preview(game_loop, &authoring_scene) {
            Ok(warnings) => {
                preview_warnings = warnings;
                Ok(())
            }
            Err(diagnostics) => Err(diagnostics),
        }
    });
    match restore_result {
        Ok(true) => diagnostics.extend(preview_warnings),
        Ok(false) => {}
        Err(restore_diagnostics) => {
            // A failed rollback must not spin the same failing script on every
            // redraw. Leave the snapshot intact in Paused mode so Stop can be
            // retried after the underlying issue is repaired.
            let _ = play_session.pause();
            diagnostics.extend(restore_diagnostics);
        }
    }
    diagnostics
}

fn recover_play_after_scene_transition_error(
    play_session: &mut EditorPlaySession,
    game_loop: &mut GameLoop,
    error: impl Into<String>,
) -> Vec<Diagnostic> {
    let error = error.into();
    let mut diagnostics = vec![Diagnostic::new(
        "EDPLAY_SCENE_TRANSITION_FAILED",
        DiagnosticSeverity::Error,
        "editor.play-mode",
        format!("Play stopped after a scene transition failed: {error}"),
    )];
    let mut preview_warnings = Vec::new();
    let restore_result = play_session.stop(|authoring_scene| {
        match restore_editor_preview(game_loop, &authoring_scene) {
            Ok(warnings) => {
                preview_warnings = warnings;
                Ok(())
            }
            Err(diagnostics) => Err(diagnostics),
        }
    });
    match restore_result {
        Ok(true) => diagnostics.extend(preview_warnings),
        Ok(false) => {}
        Err(restore_diagnostics) => {
            let _ = play_session.pause();
            diagnostics.extend(restore_diagnostics);
        }
    }
    diagnostics
}

#[cfg(test)]
fn execute_selected_asset_assignment(
    browser: &ProjectAssetBrowserPanel,
    editor_scene: &mut EditorScene,
) -> Result<bool, engine_editor::EditorError> {
    let Some(target_entity) = editor_scene.selected_entity.clone() else {
        return Ok(false);
    };
    let Some(command) = browser.selected_assignment_command(target_entity) else {
        return Ok(false);
    };
    editor_scene.execute(Box::new(command))?;
    Ok(true)
}

#[derive(Clone, Copy, Debug)]
struct RuntimeGizmoView {
    world_position: Vec3,
    world_rotation: Quat,
    view: Mat4,
    projection: Mat4,
    viewport_origin: Vec2,
    viewport_size: Vec2,
    interaction_min: Vec2,
    interaction_max: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum GizmoPointerEvent {
    Press(Vec2),
    Move(Vec2),
    Release(Vec2),
    Cancel,
}

fn resolve_runtime_world_matrix(
    world: &World,
    entity: Entity,
    visiting: &mut Vec<Entity>,
) -> Option<Mat4> {
    if visiting.contains(&entity) || !world.is_alive(entity) {
        return None;
    }
    let Some(transform) = world.get::<Transform>(entity) else {
        return Some(Mat4::IDENTITY);
    };
    if !transform.translation.is_finite()
        || !transform.rotation.is_finite()
        || transform.rotation.length_squared() <= f32::EPSILON
        || !transform.scale.is_finite()
    {
        return None;
    }

    visiting.push(entity);
    let parent_world = match transform.parent {
        Some(parent) => resolve_runtime_world_matrix(world, parent, visiting)?,
        None => Mat4::IDENTITY,
    };
    visiting.pop();
    let local = Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation.normalize(),
        transform.translation,
    );
    let resolved = parent_world * local;
    resolved.is_finite().then_some(resolved)
}

fn runtime_gizmo_view(
    runtime: &EngineRuntime,
    entity_id: &str,
    frame: u64,
    viewport_context: RenderViewportContext,
) -> Option<RuntimeGizmoView> {
    let surface_size = viewport_context.surface_size();
    let window_size = Vec2::new(surface_size[0] as f32, surface_size[1] as f32);
    runtime
        .with_world(|world| {
            let input =
                extract_renderer_input_from_world_with_viewport(world, frame, viewport_context)
                    .ok()?;
            let view = input.views.first()?;
            let entity = world.entity_by_persistent_id(entity_id)?;
            world.get::<Transform>(entity)?;
            let world_matrix = resolve_runtime_world_matrix(world, entity, &mut Vec::new())?;
            let (_scale, world_rotation, world_position) =
                world_matrix.to_scale_rotation_translation();
            if !world_position.is_finite()
                || !world_rotation.is_finite()
                || world_rotation.length_squared() <= f32::EPSILON
            {
                return None;
            }

            let viewport_min = Vec2::from_array(view.viewport_rect_normalized.min) * window_size;
            let viewport_max = Vec2::from_array(view.viewport_rect_normalized.max) * window_size;
            let viewport_size = viewport_max - viewport_min;
            if !viewport_min.is_finite()
                || !viewport_size.is_finite()
                || viewport_size.x <= 0.0
                || viewport_size.y <= 0.0
            {
                return None;
            }

            Some(RuntimeGizmoView {
                world_position,
                world_rotation: world_rotation.normalize(),
                view: Mat4::from_cols_array(&view.view_matrix),
                projection: Mat4::from_cols_array(&view.projection_matrix),
                viewport_origin: viewport_min,
                viewport_size,
                interaction_min: viewport_min,
                interaction_max: viewport_max,
            })
        })
        .flatten()
}

fn project_world_point(
    world: Vec3,
    view: Mat4,
    projection: Mat4,
    viewport_size: Vec2,
) -> Option<(Vec2, f32)> {
    let clip = projection * view * world.extend(1.0);
    if !clip.is_finite() || clip.w <= 1.0e-5 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !ndc.is_finite() || !(0.0..=1.0).contains(&ndc.z) {
        return None;
    }
    Some((
        Vec2::new(
            (ndc.x * 0.5 + 0.5) * viewport_size.x,
            (1.0 - (ndc.y * 0.5 + 0.5)) * viewport_size.y,
        ),
        ndc.z,
    ))
}

fn pick_runtime_entity(
    runtime: &EngineRuntime,
    frame: u64,
    viewport_context: RenderViewportContext,
    interaction_min: Vec2,
    interaction_max: Vec2,
    pointer: Vec2,
) -> Option<PersistentId> {
    if pointer.x < interaction_min.x
        || pointer.y < interaction_min.y
        || pointer.x > interaction_max.x
        || pointer.y > interaction_max.y
    {
        return None;
    }

    runtime
        .with_world(|world| {
            let input =
                extract_renderer_input_from_world_with_viewport(world, frame, viewport_context)
                    .ok()?;
            let render_view = input.views.first()?;
            let view = Mat4::from_cols_array(&render_view.view_matrix);
            let projection = Mat4::from_cols_array(&render_view.projection_matrix);
            let surface_size = viewport_context.surface_size();
            let window_size = Vec2::new(surface_size[0] as f32, surface_size[1] as f32);
            let viewport_min =
                Vec2::from_array(render_view.viewport_rect_normalized.min) * window_size;
            let viewport_max =
                Vec2::from_array(render_view.viewport_rect_normalized.max) * window_size;
            let viewport_size = viewport_max - viewport_min;
            let mut best: Option<(f32, PersistentId)> = None;

            for drawable in &input.drawables {
                let Some(entity) = drawable.entity.clone() else {
                    continue;
                };
                let min = Vec3::from_array(drawable.bounds.min);
                let max = Vec3::from_array(drawable.bounds.max);
                let mut screen_min = Vec2::splat(f32::INFINITY);
                let mut screen_max = Vec2::splat(f32::NEG_INFINITY);
                let mut nearest_depth = f32::INFINITY;
                let mut projected_corner = false;
                for x in [min.x, max.x] {
                    for y in [min.y, max.y] {
                        for z in [min.z, max.z] {
                            let Some((screen, depth)) = project_world_point(
                                Vec3::new(x, y, z),
                                view,
                                projection,
                                viewport_size,
                            ) else {
                                continue;
                            };
                            let screen = viewport_min + screen;
                            projected_corner = true;
                            screen_min = screen_min.min(screen);
                            screen_max = screen_max.max(screen);
                            nearest_depth = nearest_depth.min(depth);
                        }
                    }
                }
                if !projected_corner {
                    continue;
                }
                // Thin or very small meshes still receive a practical click
                // target, while overlapping objects choose the nearest depth.
                let padding = Vec2::splat(6.0);
                if pointer.x >= screen_min.x - padding.x
                    && pointer.y >= screen_min.y - padding.y
                    && pointer.x <= screen_max.x + padding.x
                    && pointer.y <= screen_max.y + padding.y
                    && best
                        .as_ref()
                        .is_none_or(|(best_depth, _)| nearest_depth < *best_depth)
                {
                    best = Some((nearest_depth, entity));
                }
            }
            best.map(|(_, entity)| entity)
        })
        .flatten()
}

fn restrict_gizmo_view_to_rect(
    mut view: RuntimeGizmoView,
    minimum: Vec2,
    maximum: Vec2,
) -> Option<RuntimeGizmoView> {
    view.interaction_min = view.viewport_origin.max(minimum);
    view.interaction_max = (view.viewport_origin + view.viewport_size).min(maximum);
    (view.interaction_max.x > view.interaction_min.x
        && view.interaction_max.y > view.interaction_min.y)
        .then_some(view)
}

fn apply_editor_camera(runtime: &EngineRuntime, panel: &SceneViewPanel) -> bool {
    let (pitch, yaw, distance) = panel.camera_orbit();
    let target = Vec3::from_array(*panel.target());
    let pitch = pitch.to_radians();
    let yaw = yaw.to_radians();
    let offset = Vec3::new(
        distance * yaw.cos() * pitch.cos(),
        distance * pitch.sin(),
        distance * yaw.sin() * pitch.cos(),
    );
    let (translation, rotation) = engine_scene::camera_utils::setup_orbit_transform(target, offset);
    if !translation.is_finite() || !rotation.is_finite() {
        return false;
    }

    runtime
        .with_world_mut(|world| {
            let active_camera = world.scene_settings().active_camera.clone();
            let entity = active_camera
                .as_deref()
                .and_then(|id| world.entity_by_persistent_id(id))
                .or_else(|| world.query::<Camera>().next().map(|(entity, _)| entity));
            let Some(entity) = entity else {
                return false;
            };
            if let Some(transform) = world.get_mut::<Transform>(entity) {
                transform.translation = translation;
                transform.rotation = rotation;
                transform.scale = Vec3::ONE;
                transform.parent = None;
            } else {
                world.add_component(
                    entity,
                    Transform {
                        translation,
                        rotation,
                        scale: Vec3::ONE,
                        parent: None,
                    },
                );
            }
            if let Some(camera) = world.get_mut::<Camera>(entity) {
                camera.projection = if panel.orthographic() {
                    CameraProjection::Orthographic
                } else {
                    CameraProjection::Perspective
                };
                camera.ortho_half_height = distance.max(0.1);
            }
            true
        })
        .unwrap_or(false)
}

fn synchronize_editor_preview_and_camera(
    game_loop: &mut GameLoop,
    editor_scene: &mut EditorScene,
    scene_view: &SceneViewPanel,
) {
    synchronize_editor_preview(game_loop, editor_scene);
    let _ = apply_editor_camera(&game_loop.runtime, scene_view);
}

fn synchronize_authoring_view(
    game_loop: &mut GameLoop,
    editor_scene: &mut EditorScene,
    scene_view: &SceneViewPanel,
    viewport_tab: ViewportTab,
) {
    match viewport_tab {
        ViewportTab::Scene => {
            synchronize_editor_preview_and_camera(game_loop, editor_scene, scene_view)
        }
        ViewportTab::Game => synchronize_game_preview(game_loop, editor_scene),
    }
}

fn sync_runtime_transform(runtime: &EngineRuntime, entity_id: &str, authoring: &Transform) -> bool {
    runtime
        .with_world_mut(|world| {
            let Some(entity) = world.entity_by_persistent_id(entity_id) else {
                return false;
            };
            let Some(runtime_transform) = world.get_mut::<Transform>(entity) else {
                return false;
            };
            runtime_transform.translation = authoring.translation;
            runtime_transform.rotation = authoring.rotation;
            runtime_transform.scale = authoring.scale;
            true
        })
        .unwrap_or(false)
}

fn apply_gizmo_preview_delta(
    editor_scene: &mut EditorScene,
    gizmo: &mut GizmoSystem,
    runtime: &EngineRuntime,
    entity_id: &str,
) -> bool {
    let delta = gizmo.take_delta();
    if delta.length_squared() <= 0.0 || !editor_scene.preview_transform_gizmo_drag(gizmo, delta) {
        return false;
    }
    editor_scene
        .selected_transform_for_gizmo()
        .is_some_and(|transform| sync_runtime_transform(runtime, entity_id, &transform))
}

fn offset_gizmo_batch(mut batch: UiBatch, view: RuntimeGizmoView) -> UiBatch {
    for vertex in &mut batch.vertices {
        vertex.position[0] += view.viewport_origin.x;
        vertex.position[1] += view.viewport_origin.y;
    }
    batch.clip_rect.min = view.interaction_min.to_array();
    batch.clip_rect.max = view.interaction_max.to_array();
    batch
}

fn process_gizmo_pointer_events(
    events: Vec<GizmoPointerEvent>,
    editor_scene: &mut EditorScene,
    gizmo: &mut GizmoSystem,
    runtime: &EngineRuntime,
    entity_id: &str,
    view: RuntimeGizmoView,
) -> bool {
    let mut scene_changed = false;
    if editor_scene
        .active_transform_gizmo_entity()
        .is_some_and(|active| active != entity_id)
    {
        scene_changed |= editor_scene.cancel_transform_gizmo_drag();
        gizmo.cancel_drag();
        return scene_changed;
    }
    for event in events {
        let pointer = match event {
            GizmoPointerEvent::Press(pointer)
            | GizmoPointerEvent::Move(pointer)
            | GizmoPointerEvent::Release(pointer) => pointer,
            GizmoPointerEvent::Cancel => {
                scene_changed |= editor_scene.cancel_transform_gizmo_drag();
                gizmo.cancel_drag();
                continue;
            }
        };
        let local_pointer = pointer - view.viewport_origin;

        match event {
            GizmoPointerEvent::Press(_) => {
                if gizmo.dragging {
                    continue;
                }
                let pointer_inside_view = pointer.x >= view.interaction_min.x
                    && pointer.y >= view.interaction_min.y
                    && pointer.x <= view.interaction_max.x
                    && pointer.y <= view.interaction_max.y;
                if !pointer_inside_view {
                    continue;
                }
                let _ = update_gizmo(
                    gizmo,
                    view.world_position,
                    view.world_rotation,
                    &view.view,
                    &view.projection,
                    view.viewport_size,
                    local_pointer,
                    true,
                );
                if gizmo.dragging && !editor_scene.begin_transform_gizmo_drag() {
                    gizmo.cancel_drag();
                }
            }
            GizmoPointerEvent::Move(_) => {
                if !gizmo.dragging {
                    continue;
                }
                if update_gizmo(
                    gizmo,
                    view.world_position,
                    view.world_rotation,
                    &view.view,
                    &view.projection,
                    view.viewport_size,
                    local_pointer,
                    true,
                ) && editor_scene.is_transform_gizmo_drag_active()
                {
                    let _ = apply_gizmo_preview_delta(editor_scene, gizmo, runtime, entity_id);
                }
            }
            GizmoPointerEvent::Release(_) => {
                // A platform is allowed to deliver the final pointer position
                // only with the button-release event.  Sample that position
                // once while the gesture is still active so the last segment
                // (or an entire press/release drag) is not lost.
                if gizmo.dragging
                    && update_gizmo(
                        gizmo,
                        view.world_position,
                        view.world_rotation,
                        &view.view,
                        &view.projection,
                        view.viewport_size,
                        local_pointer,
                        true,
                    )
                    && editor_scene.is_transform_gizmo_drag_active()
                {
                    let _ = apply_gizmo_preview_delta(editor_scene, gizmo, runtime, entity_id);
                }
                let was_dragging = gizmo.dragging;
                let _ = update_gizmo(
                    gizmo,
                    view.world_position,
                    view.world_rotation,
                    &view.view,
                    &view.projection,
                    view.viewport_size,
                    local_pointer,
                    false,
                );
                if was_dragging {
                    match editor_scene.commit_transform_gizmo_drag() {
                        Ok(changed) => scene_changed |= changed,
                        Err(error) => {
                            tracing::error!(%error, "editor gizmo commit failed");
                            scene_changed |= editor_scene.cancel_transform_gizmo_drag();
                        }
                    }
                }
            }
            GizmoPointerEvent::Cancel => unreachable!(),
        }
    }
    scene_changed
}

pub struct EditorApp {
    session_id: String,
    editor_revision: u64,
    editor_event_sequence: u64,
    pending_full_snapshot: bool,
    project: GameProject,
    current_scene_id: String,
    current_scene_path: PathBuf,
    scene_browser_selection: String,
    new_scene_id: String,
    new_scene_folder: String,
    scene_operation_id: String,
    scene_replacement_id: String,
    pending_scene_switch: Option<String>,
    pending_document_action: Option<SceneDocumentAction>,
    scene_document_status: Option<String>,
    close_confirmation_pending: bool,
    pending_recovery: Option<PathBuf>,
    exit_after_frame: bool,
    play_runtime_scene_id: Option<String>,
    game_loop: Option<GameLoop>,
    editor_scene: Option<EditorScene>,
    /// Ordered authoring selection. `EditorScene::selected_entity` remains the
    /// active object used by gizmos and single-object inspectors.
    selected_entity_ids: Vec<String>,
    play_session: EditorPlaySession,
    #[cfg(feature = "target-desktop")]
    input_state: super::project_input::ProjectInputState,
    scene_view: SceneViewPanel,
    gizmo: GizmoSystem,
    gizmo_pointer_events: Vec<GizmoPointerEvent>,
    asset_browser: ProjectAssetBrowserPanel,
    material_editor: MaterialEditorPanel,
    material_editor_selection: Option<String>,
    animation_preview: AnimationPreviewPanel,
    viewport_tab: ViewportTab,
    pending_ui_open_panels: Vec<protocol::UiOpenPanelParams>,
    web_viewport_rect: ScreenRect,
    window_scale_factor: f64,
    surface_occluded: bool,
    surface_zero_sized: bool,
    render_faulted: bool,
    web_viewport_input: WebViewportInputState,
    build_status: Option<String>,
    background_job: Option<EditorBackgroundJob>,
    last_editor_operation: Option<EditorOperationStatus>,
    recent_editor_operations: VecDeque<EditorOperationStatus>,
    next_editor_operation_id: u64,
    editor_build_service: Result<super::editor_build_ops::EditorBuildService, String>,
    editor_build_task: Option<super::editor_build_ops::EditorBuildTask>,
    run_after_build: bool,
    build_output: String,
    package_version: String,
    package_output_root: String,
    project_settings_draft: ProjectSettingsDraft,
    scene_settings_draft: SceneSettings,
    entity_clipboard: Option<engine_editor::EntityClipboard>,
    component_clipboard: Option<engine_editor::ComponentClipboard>,
    performance: engine_editor::performance::PerformancePanel,
    workspace_preferences: EditorWorkspacePreferences,
    saved_workspace_preferences: EditorWorkspacePreferences,
    frame: u64,
    window_w: f32,
    window_h: f32,
    last_frame_time: Instant,
    last_recovery_snapshot: Instant,
    step_play_once: bool,
}

struct EditorBackgroundJob {
    id: u64,
    label: String,
    receiver: mpsc::Receiver<Result<EditorJobOutput, String>>,
    reload_assets: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum EditorJobOutput {
    #[default]
    None,
    SelectAsset(String),
    SelectFolder(String),
    ClearAssetSelection,
}

#[derive(Clone, Debug)]
enum EditorOperationState {
    Running,
    Succeeded,
    CommittedWithWarning(String),
    Failed(String),
}

#[derive(Clone, Debug)]
struct EditorOperationStatus {
    id: u64,
    label: String,
    state: EditorOperationState,
}

#[derive(Default)]
struct WebViewportInputState {
    pointer_id: Option<i64>,
    pointer: Option<Vec2>,
    buttons: u16,
    modifiers: InputModifiers,
    keys: BTreeSet<String>,
    focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SceneDocumentAction {
    Open(String),
    Create {
        scene_id: String,
        folder: PathBuf,
    },
    SaveAs(String),
    Duplicate {
        source_id: String,
        new_id: String,
    },
    SetStartup(String),
    Rename {
        old_id: String,
        new_id: String,
    },
    Delete {
        scene_id: String,
        replacement_startup: Option<String>,
    },
    CancelSwitch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseDocumentAction {
    SaveAndClose,
    DiscardAndClose,
    Cancel,
}

#[derive(Clone, Debug)]
struct ProjectSettingsDraft {
    title: String,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ViewportTab {
    #[default]
    Scene,
    Game,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorFrameOutcome {
    Completed,
    Failed,
}

fn editor_frame_time_ms(gpu_frame_ms: f32, frame_interval_seconds: f32) -> f32 {
    if gpu_frame_ms.is_finite() && gpu_frame_ms > 0.0 {
        gpu_frame_ms
    } else if frame_interval_seconds.is_finite() && frame_interval_seconds >= 0.0 {
        frame_interval_seconds * 1_000.0
    } else {
        0.0
    }
}

fn gizmo_viewport_enabled(gizmos_visible: bool, editing: bool, viewport_tab: ViewportTab) -> bool {
    gizmos_visible && editing && viewport_tab == ViewportTab::Scene
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProjectAssetView {
    #[default]
    Grid,
    List,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct EditorWorkspacePreferences {
    scene_pitch: f32,
    scene_yaw: f32,
    scene_distance: f32,
    scene_target: [f32; 3],
    scene_orthographic: bool,
    scene_camera_speed: f32,
    gizmos_visible: bool,
    snapping_enabled: bool,
    project_asset_view: ProjectAssetView,
    project_asset_folder: String,
    react_layout: Option<String>,
}

impl Default for EditorWorkspacePreferences {
    fn default() -> Self {
        Self {
            scene_pitch: 20.0,
            scene_yaw: 45.0,
            scene_distance: 10.0,
            scene_target: [0.0, 0.0, 0.0],
            scene_orthographic: false,
            scene_camera_speed: 5.0,
            gizmos_visible: true,
            snapping_enabled: false,
            project_asset_view: ProjectAssetView::Grid,
            project_asset_folder: "/".to_string(),
            react_layout: None,
        }
    }
}

fn workspace_preferences_path(project: &GameProject) -> PathBuf {
    project.root.join(".engine/editor-workspace.json")
}

fn scene_recovery_path(project: &GameProject, scene_id: &str) -> PathBuf {
    project
        .root
        .join(".engine/recovery")
        .join(format!("{scene_id}.scene.ron"))
}

fn newer_recovery_snapshot(
    project: &GameProject,
    scene_id: &str,
    scene_path: &Path,
) -> Option<PathBuf> {
    let recovery = scene_recovery_path(project, scene_id);
    let recovery_modified = std::fs::metadata(&recovery).ok()?.modified().ok()?;
    let scene_modified = std::fs::metadata(scene_path).ok()?.modified().ok()?;
    (recovery_modified > scene_modified).then_some(recovery)
}

fn load_workspace_preferences(project: &GameProject) -> EditorWorkspacePreferences {
    let path = workspace_preferences_path(project);
    let Ok(bytes) = std::fs::read(&path) else {
        return EditorWorkspacePreferences::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        tracing::warn!(path = %path.display(), %error, "ignored invalid editor workspace preferences");
        EditorWorkspacePreferences::default()
    })
}

fn save_workspace_preferences(
    project: &GameProject,
    preferences: &EditorWorkspacePreferences,
) -> Result<(), String> {
    let path = workspace_preferences_path(project);
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid workspace preferences path: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let mut json = serde_json::to_string_pretty(preferences)
        .map_err(|error| format!("could not serialize workspace preferences: {error}"))?;
    json.push('\n');
    std::fs::write(&path, json)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

impl EditorApp {
    pub fn new(project: GameProject) -> Self {
        let current_scene_id = project.startup_scene_id().to_string();
        let current_scene_path = project.startup_scene_path().to_path_buf();
        let workspace_preferences = load_workspace_preferences(&project);
        let mut scene_view = SceneViewPanel::new("Scene View");
        scene_view.set_camera_orbit(
            workspace_preferences.scene_pitch,
            workspace_preferences.scene_yaw,
            workspace_preferences.scene_distance,
        );
        scene_view.set_target(workspace_preferences.scene_target);
        scene_view.set_orthographic(workspace_preferences.scene_orthographic);
        scene_view.set_camera_speed(workspace_preferences.scene_camera_speed);
        let mut gizmo = GizmoSystem::new();
        gizmo.snapping = workspace_preferences.snapping_enabled;
        let project_settings_draft = ProjectSettingsDraft {
            title: project.manifest.window.title.clone(),
            width: project.manifest.window.width,
            height: project.manifest.window.height,
        };
        let pending_recovery =
            newer_recovery_snapshot(&project, &current_scene_id, &current_scene_path);
        let editor_build_service =
            super::editor_build_ops::EditorBuildService::for_current_editor()
                .map_err(|error| error.to_string());
        Self {
            session_id: format!(
                "editor-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            editor_revision: 0,
            editor_event_sequence: 0,
            pending_full_snapshot: false,
            project,
            scene_browser_selection: current_scene_id.clone(),
            current_scene_id,
            current_scene_path,
            new_scene_id: String::new(),
            new_scene_folder: String::new(),
            scene_operation_id: String::new(),
            scene_replacement_id: String::new(),
            pending_scene_switch: None,
            pending_document_action: None,
            scene_document_status: None,
            close_confirmation_pending: false,
            pending_recovery,
            exit_after_frame: false,
            play_runtime_scene_id: None,
            game_loop: None,
            editor_scene: None,
            selected_entity_ids: Vec::new(),
            play_session: EditorPlaySession::default(),
            #[cfg(feature = "target-desktop")]
            input_state: super::project_input::ProjectInputState::default(),
            scene_view,
            gizmo,
            gizmo_pointer_events: Vec::new(),
            asset_browser: ProjectAssetBrowserPanel::new(),
            material_editor: MaterialEditorPanel::new(),
            material_editor_selection: None,
            animation_preview: AnimationPreviewPanel::new(),
            viewport_tab: ViewportTab::Scene,
            pending_ui_open_panels: Vec::new(),
            web_viewport_rect: ScreenRect::default(),
            window_scale_factor: 1.0,
            surface_occluded: false,
            surface_zero_sized: false,
            render_faulted: false,
            web_viewport_input: WebViewportInputState::default(),
            build_status: None,
            background_job: None,
            last_editor_operation: None,
            recent_editor_operations: VecDeque::new(),
            next_editor_operation_id: 1,
            editor_build_service,
            editor_build_task: None,
            run_after_build: false,
            build_output: String::new(),
            package_version: "0.1.0-dev".to_string(),
            package_output_root: "dist/releases".to_string(),
            project_settings_draft,
            scene_settings_draft: SceneSettings::default(),
            entity_clipboard: None,
            component_clipboard: None,
            performance: engine_editor::performance::PerformancePanel::new(),
            saved_workspace_preferences: workspace_preferences.clone(),
            workspace_preferences,
            frame: 0,
            window_w: 1600.0,
            window_h: 900.0,
            last_frame_time: Instant::now(),
            last_recovery_snapshot: Instant::now(),
            step_play_once: false,
        }
    }

    fn record_scene_document_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        tracing::error!(%message, "editor scene document operation failed");
        self.scene_document_status = Some(message.clone());
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.diagnostics.push(Diagnostic::new(
                "EDSCENE_DOCUMENT_FAILED",
                DiagnosticSeverity::Error,
                "editor.scene-document",
                message,
            ));
        }
        self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
    }

    fn reload_project_manifest(&mut self) -> Result<(), String> {
        let reloaded = GameProject::load(&self.project.manifest_path)
            .map_err(|error| format!("Could not reload project scene catalog: {error}"))?;
        let current_scene_path = reloaded.scene_path(&self.current_scene_id).ok_or_else(|| {
            format!(
                "Reloaded scene catalog no longer contains the open scene '{}'",
                self.current_scene_id
            )
        })?;
        self.current_scene_path = current_scene_path;
        self.project = reloaded;
        Ok(())
    }

    fn save_current_scene_document(&mut self) -> Result<(), String> {
        let editor_scene = self
            .editor_scene
            .as_mut()
            .ok_or_else(|| "No editor scene is open".to_string())?;
        save_scene_atomically(&editor_scene.scene, &self.current_scene_path)?;
        editor_scene.history.mark_clean();
        let recovery = scene_recovery_path(&self.project, &self.current_scene_id);
        match std::fs::remove_file(&recovery) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(path = %recovery.display(), %error, "could not remove saved recovery snapshot")
            }
        }
        self.pending_recovery = None;
        tracing::info!(
            scene_id = self.current_scene_id,
            scene = %self.current_scene_path.display(),
            "editor scene saved"
        );
        self.scene_document_status = Some(format!("Saved '{}'.", self.current_scene_id));
        Ok(())
    }

    fn maybe_write_recovery_snapshot(&mut self) {
        const RECOVERY_INTERVAL_SECONDS: u64 = 30;
        if self.last_recovery_snapshot.elapsed().as_secs() < RECOVERY_INTERVAL_SECONDS {
            return;
        }
        self.last_recovery_snapshot = Instant::now();
        if !self.play_session.is_editing()
            || !self
                .editor_scene
                .as_ref()
                .is_some_and(EditorScene::is_dirty)
        {
            return;
        }
        let Some(scene) = self.editor_scene.as_ref().map(|scene| &scene.scene) else {
            return;
        };
        let recovery = scene_recovery_path(&self.project, &self.current_scene_id);
        match save_scene_atomically(scene, &recovery) {
            Ok(()) => {
                self.build_status = Some(format!(
                    "Recovery snapshot updated for '{}'.",
                    self.current_scene_id
                ));
            }
            Err(error) => self.record_scene_document_error(format!(
                "Could not write recovery snapshot {}: {error}",
                recovery.display()
            )),
        }
    }

    fn restore_recovery_snapshot(&mut self) -> Result<(), String> {
        let recovery = self
            .pending_recovery
            .clone()
            .ok_or_else(|| "No recovery snapshot is pending".to_string())?;
        let scene = Scene::load_from_file(&recovery).map_err(|error| {
            format!(
                "Could not load recovery snapshot {}: {error}",
                recovery.display()
            )
        })?;
        super::project_scripts::validate_runtime_script_references(&self.project, &scene)?;
        let game_loop = self
            .game_loop
            .as_mut()
            .ok_or_else(|| "Editor runtime is not initialized".to_string())?;
        let (preview_scene, diagnostics) = editor_preview_scene(&game_loop.runtime, &scene);
        game_loop.load_scene(preview_scene).map_err(|diagnostics| {
            format!(
                "Recovery snapshot could not be restored into the editor runtime: {}",
                summarize_scene_diagnostics(&diagnostics)
            )
        })?;
        game_loop.init_physics();
        let mut editor_scene = EditorScene::new_with_component_registry(
            scene,
            std::sync::Arc::clone(game_loop.runtime.component_registry()),
        )
        .map_err(|error| format!("Recovery snapshot is not authorable: {error}"))?;
        editor_scene.history.mark_dirty();
        editor_scene.diagnostics.push_many(diagnostics);
        self.scene_settings_draft = editor_scene.scene.scene_settings.clone();
        self.editor_scene = Some(editor_scene);
        self.selected_entity_ids.clear();
        self.pending_recovery = None;
        self.scene_document_status = Some(format!(
            "Recovered unsaved changes for '{}'; save to keep them.",
            self.current_scene_id
        ));
        Ok(())
    }

    fn discard_recovery_snapshot(&mut self) -> Result<(), String> {
        let Some(recovery) = self.pending_recovery.take() else {
            return Ok(());
        };
        match std::fs::remove_file(&recovery) {
            Ok(()) => {
                self.scene_document_status = Some("Recovery snapshot discarded.".to_string());
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "Could not discard recovery snapshot {}: {error}",
                recovery.display()
            )),
        }
    }

    /// Load and validate a catalog scene before replacing the active document.
    /// `GameLoop::load_scene` is transactional, so any strict ECS failure leaves
    /// the currently rendered authoring preview untouched.
    fn switch_scene_document(&mut self, scene_id: &str) -> Result<bool, String> {
        if !self.play_session.is_editing() {
            return Err("Stop Play before opening another scene.".to_string());
        }
        if scene_id == self.current_scene_id {
            self.pending_scene_switch = None;
            self.pending_document_action = None;
            self.scene_document_status = Some(format!("Scene '{scene_id}' is already open."));
            return Ok(false);
        }

        let scene_path = self.project.scene_path(scene_id).ok_or_else(|| {
            format!("Unknown project scene '{scene_id}'; refresh the project scene catalog.")
        })?;
        let scene = Scene::load_from_file(&scene_path).map_err(|error| {
            format!(
                "Could not load scene '{}' from {}: {error}",
                scene_id,
                scene_path.display()
            )
        })?;
        super::project_scripts::validate_runtime_script_references(&self.project, &scene).map_err(
            |error| format!("Scene '{scene_id}' has invalid script references: {error}"),
        )?;

        let game_loop = self
            .game_loop
            .as_mut()
            .ok_or_else(|| "Editor runtime is not initialized".to_string())?;
        let (preview_scene, preview_diagnostics) = editor_preview_scene(&game_loop.runtime, &scene);
        game_loop.load_scene(preview_scene).map_err(|diagnostics| {
            format!(
                "Scene '{scene_id}' could not be restored into the editor runtime: {}",
                summarize_scene_diagnostics(&diagnostics)
            )
        })?;
        game_loop.init_physics();

        let mut editor_scene = EditorScene::new_with_component_registry(
            scene,
            std::sync::Arc::clone(game_loop.runtime.component_registry()),
        )
        .map_err(|error| format!("Scene '{scene_id}' is not authorable: {error}"))?;
        editor_scene.diagnostics.push_many(preview_diagnostics);
        self.scene_settings_draft = editor_scene.scene.scene_settings.clone();
        self.editor_scene = Some(editor_scene);
        self.selected_entity_ids.clear();
        self.current_scene_id = scene_id.to_string();
        self.current_scene_path = scene_path;
        self.scene_browser_selection = scene_id.to_string();
        self.pending_scene_switch = None;
        self.pending_document_action = None;
        self.gizmo.cancel_drag();
        self.gizmo_pointer_events.clear();
        self.viewport_tab = ViewportTab::Scene;
        self.material_editor.reset();
        self.material_editor_selection = None;
        self.last_frame_time = Instant::now();
        self.scene_document_status = Some(format!("Opened scene '{scene_id}'."));
        self.pending_recovery = newer_recovery_snapshot(
            &self.project,
            &self.current_scene_id,
            &self.current_scene_path,
        );
        tracing::info!(scene_id, scene = %self.current_scene_path.display(), "editor scene opened");
        Ok(true)
    }

    fn request_scene_switch(&mut self, scene_id: String) -> Result<bool, String> {
        if scene_id == self.current_scene_id {
            self.pending_scene_switch = None;
            self.pending_document_action = None;
            self.scene_document_status = Some(format!("Scene '{scene_id}' is already open."));
            return Ok(false);
        }
        if self
            .editor_scene
            .as_ref()
            .is_some_and(EditorScene::is_dirty)
        {
            self.pending_scene_switch = Some(format!("opening scene '{scene_id}'"));
            self.pending_document_action = Some(SceneDocumentAction::Open(scene_id.clone()));
            self.scene_document_status = Some(format!(
                "Unsaved changes: choose Save & Switch, Discard & Switch, or Cancel for '{scene_id}'."
            ));
            return Ok(false);
        }
        self.switch_scene_document(&scene_id)
    }

    fn defer_document_action_if_dirty(
        &mut self,
        action: SceneDocumentAction,
        target_label: String,
    ) -> bool {
        if !self
            .editor_scene
            .as_ref()
            .is_some_and(EditorScene::is_dirty)
        {
            return false;
        }
        self.pending_scene_switch = Some(target_label.clone());
        self.pending_document_action = Some(action);
        self.scene_document_status = Some(format!(
            "Unsaved changes must be saved or discarded before {target_label}."
        ));
        true
    }

    fn rename_scene_document(&mut self, old_id: &str, new_id: &str) -> Result<bool, String> {
        let old_id = old_id.trim();
        let new_id = new_id.trim();
        if old_id.is_empty() || new_id.is_empty() {
            return Err("Scene rename requires both the current and new scene IDs.".to_string());
        }
        super::project_cli::rename_project_scene(&self.project.manifest_path, old_id, new_id)?;
        let reloaded = GameProject::load(&self.project.manifest_path)
            .map_err(|error| format!("Could not reload renamed scene catalog: {error}"))?;

        if old_id == self.current_scene_id {
            self.project = reloaded;
            self.switch_scene_document(new_id)?;
        } else {
            self.current_scene_path =
                reloaded.scene_path(&self.current_scene_id).ok_or_else(|| {
                    format!(
                        "Renaming '{old_id}' removed the current scene '{}' from the catalog",
                        self.current_scene_id
                    )
                })?;
            self.project = reloaded;
        }
        self.scene_browser_selection = new_id.to_string();
        self.scene_operation_id.clear();
        self.new_scene_id.clear();
        self.scene_document_status = Some(format!("Renamed scene '{old_id}' to '{new_id}'."));
        Ok(true)
    }

    fn delete_scene_document(
        &mut self,
        scene_id: &str,
        replacement_startup: Option<&str>,
    ) -> Result<bool, String> {
        let scene_id = scene_id.trim();
        if scene_id.is_empty() {
            return Err("No scene was selected for deletion.".to_string());
        }
        let deleting_current = scene_id == self.current_scene_id;
        let deleted = super::project_cli::delete_project_scene(
            &self.project.manifest_path,
            scene_id,
            replacement_startup,
        )?;
        let reloaded = GameProject::load(&self.project.manifest_path)
            .map_err(|error| format!("Could not reload scene catalog after deletion: {error}"))?;

        if deleting_current {
            let next_scene = deleted
                .replacement_startup
                .clone()
                .unwrap_or_else(|| reloaded.startup_scene_id().to_string());
            self.project = reloaded;
            self.switch_scene_document(&next_scene)?;
            self.scene_browser_selection = next_scene;
        } else {
            self.current_scene_path =
                reloaded.scene_path(&self.current_scene_id).ok_or_else(|| {
                    format!(
                        "Deleting '{scene_id}' removed the current scene '{}' from the catalog",
                        self.current_scene_id
                    )
                })?;
            self.project = reloaded;
            self.scene_browser_selection = self.current_scene_id.clone();
        }
        self.scene_operation_id.clear();
        self.scene_replacement_id.clear();
        self.scene_document_status = Some(format!(
            "Moved scene '{}' to project trash at {} (metadata: {}).",
            deleted.scene_id,
            deleted.trash_directory.display(),
            deleted.metadata_path.display()
        ));
        Ok(true)
    }

    fn apply_scene_document_action(&mut self, action: SceneDocumentAction) -> Result<bool, String> {
        if self.pending_document_action.is_some()
            && !matches!(&action, SceneDocumentAction::CancelSwitch)
        {
            return Err(
                "Resolve or cancel the pending scene document operation before starting another"
                    .to_string(),
            );
        }
        self.apply_scene_document_action_after_confirmation(action, false)
    }

    /// Applies a scene-document action after the caller has explicitly resolved
    /// the dirty-document prompt. A discarded document must stay dirty until a
    /// successful switch replaces it; marking it clean up front would turn a
    /// failed open/create/rename/delete into a false save checkpoint.
    fn apply_scene_document_action_after_confirmation(
        &mut self,
        action: SceneDocumentAction,
        dirty_prompt_resolved: bool,
    ) -> Result<bool, String> {
        match action {
            SceneDocumentAction::Open(scene_id) => {
                if dirty_prompt_resolved {
                    self.switch_scene_document(&scene_id)
                } else {
                    self.request_scene_switch(scene_id)
                }
            }
            SceneDocumentAction::Create { scene_id, folder } => {
                let scene_id = scene_id.trim();
                if scene_id.is_empty() {
                    return Err("New scene ID must not be empty.".to_string());
                }
                if !dirty_prompt_resolved
                    && self.defer_document_action_if_dirty(
                        SceneDocumentAction::Create {
                            scene_id: scene_id.to_string(),
                            folder: folder.clone(),
                        },
                        format!("creating and opening scene '{scene_id}'"),
                    )
                {
                    return Ok(false);
                }
                super::project_cli::create_project_scene_in_folder(
                    &self.project.manifest_path,
                    scene_id,
                    None,
                    &folder,
                )?;
                self.reload_project_manifest()?;
                self.scene_browser_selection = scene_id.to_string();
                self.new_scene_id.clear();
                self.new_scene_folder.clear();
                self.scene_document_status = Some(format!("Created scene '{scene_id}'."));
                self.switch_scene_document(scene_id)
            }
            SceneDocumentAction::SaveAs(scene_id) => {
                let scene_id = scene_id.trim();
                if scene_id.is_empty() {
                    return Err("Save As scene ID must not be empty.".to_string());
                }
                let source = self
                    .editor_scene
                    .as_ref()
                    .ok_or_else(|| "No editor scene is open".to_string())?
                    .scene
                    .clone();
                super::project_cli::duplicate_project_scene(
                    &self.project.manifest_path,
                    scene_id,
                    &source,
                )?;
                self.reload_project_manifest()?;
                self.scene_browser_selection = scene_id.to_string();
                self.new_scene_id.clear();
                self.scene_document_status = Some(format!("Saved scene as '{scene_id}'."));
                self.switch_scene_document(scene_id)
            }
            SceneDocumentAction::Duplicate { source_id, new_id } => {
                let source_id = source_id.trim();
                let new_id = new_id.trim();
                if source_id.is_empty() || new_id.is_empty() {
                    return Err(
                        "Scene duplication requires both source and destination IDs.".to_string(),
                    );
                }
                if !dirty_prompt_resolved
                    && self.defer_document_action_if_dirty(
                        SceneDocumentAction::Duplicate {
                            source_id: source_id.to_string(),
                            new_id: new_id.to_string(),
                        },
                        format!("duplicating and opening scene '{new_id}'"),
                    )
                {
                    return Ok(false);
                }
                let source_path = self.project.scene_path(source_id).ok_or_else(|| {
                    format!("Unknown project scene '{source_id}' cannot be duplicated.")
                })?;
                let source = Scene::load_from_file(&source_path).map_err(|error| {
                    format!(
                        "Could not load scene '{source_id}' from {}: {error}",
                        source_path.display()
                    )
                })?;
                super::project_cli::duplicate_project_scene(
                    &self.project.manifest_path,
                    new_id,
                    &source,
                )?;
                self.reload_project_manifest()?;
                self.scene_browser_selection = new_id.to_string();
                self.scene_operation_id.clear();
                self.new_scene_id.clear();
                self.scene_document_status =
                    Some(format!("Duplicated scene '{source_id}' as '{new_id}'."));
                self.switch_scene_document(new_id)
            }
            SceneDocumentAction::SetStartup(scene_id) => {
                super::project_cli::set_project_startup_scene(
                    &self.project.manifest_path,
                    &scene_id,
                )?;
                self.reload_project_manifest()?;
                self.scene_document_status =
                    Some(format!("Scene '{scene_id}' is now the startup scene."));
                Ok(false)
            }
            SceneDocumentAction::Rename { old_id, new_id } => {
                if !dirty_prompt_resolved
                    && old_id == self.current_scene_id
                    && self.defer_document_action_if_dirty(
                        SceneDocumentAction::Rename {
                            old_id: old_id.clone(),
                            new_id: new_id.clone(),
                        },
                        format!("renaming scene '{old_id}' to '{new_id}'"),
                    )
                {
                    return Ok(false);
                }
                self.rename_scene_document(&old_id, &new_id)
            }
            SceneDocumentAction::Delete {
                scene_id,
                replacement_startup,
            } => {
                if !dirty_prompt_resolved
                    && scene_id == self.current_scene_id
                    && self.defer_document_action_if_dirty(
                        SceneDocumentAction::Delete {
                            scene_id: scene_id.clone(),
                            replacement_startup: replacement_startup.clone(),
                        },
                        format!("deleting scene '{scene_id}'"),
                    )
                {
                    return Ok(false);
                }
                self.delete_scene_document(&scene_id, replacement_startup.as_deref())
            }
            SceneDocumentAction::CancelSwitch => {
                self.pending_scene_switch = None;
                self.pending_document_action = None;
                self.scene_document_status = Some("Scene switch cancelled.".to_string());
                Ok(false)
            }
        }
    }

    fn apply_close_document_action(&mut self, action: CloseDocumentAction) -> Result<(), String> {
        match action {
            CloseDocumentAction::SaveAndClose => {
                self.save_current_scene_document()?;
                self.pending_scene_switch = None;
                self.pending_document_action = None;
                self.close_confirmation_pending = false;
                self.exit_after_frame = true;
            }
            CloseDocumentAction::DiscardAndClose => {
                self.pending_scene_switch = None;
                self.pending_document_action = None;
                self.close_confirmation_pending = false;
                self.exit_after_frame = true;
            }
            CloseDocumentAction::Cancel => {
                self.close_confirmation_pending = false;
                self.scene_document_status = Some("Editor close cancelled.".to_string());
            }
        }
        Ok(())
    }

    fn init_scene(&mut self) {
        let scene = match Scene::load_from_file(&self.current_scene_path) {
            Ok(scene) => scene,
            Err(error) => {
                tracing::error!(
                    scene_id = self.current_scene_id,
                    path = %self.current_scene_path.display(),
                    %error,
                    "editor: failed to load project startup scene"
                );
                std::process::exit(1);
            }
        };
        let mut editor_diagnostics = Vec::new();
        if let Some(ref mut game_loop) = self.game_loop {
            if let Err(error) =
                super::project_scripts::validate_runtime_script_references(&self.project, &scene)
            {
                tracing::error!(%error, "editor: invalid project script references");
                std::process::exit(1);
            }
            if let Err(error) =
                super::project_app::load_project_assets(&mut game_loop.runtime, &self.project)
            {
                tracing::error!(%error, "editor: failed to load project cooked assets");
                std::process::exit(1);
            }
            let requested_asset_folder = self.workspace_preferences.project_asset_folder.clone();
            if let Err(error) = refresh_project_asset_list(
                &mut self.asset_browser,
                game_loop.runtime.asset_registry(),
                &self.project.asset_source,
            ) {
                let message = format!("Could not load the project asset catalog: {error}");
                tracing::error!(%message);
                self.build_status = Some(message);
            } else {
                self.asset_browser
                    .set_current_folder(requested_asset_folder);
                self.workspace_preferences.project_asset_folder =
                    self.asset_browser.current_folder().to_string();
            }
            if let Err(error) = super::project_scripts::prepare_project_scripts(
                &mut game_loop.runtime,
                &self.project,
            ) {
                tracing::error!(%error, "editor: failed to prepare project scripts");
                std::process::exit(1);
            }
            let (preview_scene, missing_diagnostics) =
                editor_preview_scene(&game_loop.runtime, &scene);
            editor_diagnostics = missing_diagnostics;
            if let Err(diagnostics) = game_loop.load_scene(preview_scene) {
                for diagnostic in diagnostics {
                    tracing::error!(
                        code = diagnostic.code,
                        entity = ?diagnostic.entity,
                        component_type = ?diagnostic.fields.get("component_type_id"),
                        message = diagnostic.message,
                        "editor: failed to load project scene"
                    );
                }
                std::process::exit(1);
            }
            if let Err(error) = super::project_scripts::fail_on_script_errors(
                &game_loop.runtime,
                "attachment/OnCreate",
            ) {
                tracing::error!(%error, "editor: script startup failed");
                std::process::exit(1);
            }
            game_loop.init_physics();
        }
        let component_registry = self
            .game_loop
            .as_ref()
            .map(|game_loop| std::sync::Arc::clone(game_loop.runtime.component_registry()))
            .unwrap_or_else(|| {
                tracing::error!("editor: runtime component registry is unavailable");
                std::process::exit(1);
            });
        let mut editor_scene =
            match EditorScene::new_with_component_registry(scene, component_registry) {
                Ok(editor_scene) => editor_scene,
                Err(error) => {
                    tracing::error!(%error, "editor: startup scene is not authorable");
                    std::process::exit(1);
                }
            };
        editor_scene.diagnostics.push_many(editor_diagnostics);
        self.scene_settings_draft = editor_scene.scene.scene_settings.clone();
        self.editor_scene = Some(editor_scene);
        self.selected_entity_ids.clear();
        self.last_frame_time = Instant::now();
        tracing::info!(
            project = self.project.manifest.name,
            scene_id = self.current_scene_id,
            scene = %self.current_scene_path.display(),
            "editor: project scene loaded"
        );
    }

    fn start_play(&mut self) {
        let Some(authoring_scene) = self
            .editor_scene
            .as_ref()
            .map(|editor_scene| editor_scene.scene.clone())
        else {
            return;
        };
        let Some(game_loop) = self.game_loop.as_mut() else {
            return;
        };

        let refreshed_scripts = match super::project_scripts::rebuild_and_reload_project_scripts(
            &mut game_loop.runtime,
            &self.project,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                let diagnostic = Diagnostic::new(
                    "EDPLAY_SCRIPT_REBUILD_FAILED",
                    DiagnosticSeverity::Error,
                    "editor.play-mode",
                    format!(
                        "Play mode did not start because C# scripts failed to rebuild: {error}"
                    ),
                );
                tracing::error!(%error, "editor Play script rebuild failed");
                if let Some(editor_scene) = self.editor_scene.as_mut() {
                    editor_scene.diagnostics.push(diagnostic);
                }
                self.scene_document_status =
                    Some("Play cancelled: C# script rebuild failed.".to_string());
                return;
            }
        };
        tracing::info!(
            assemblies = refreshed_scripts.assemblies,
            "editor Play scripts refreshed"
        );

        match self.play_session.start(&authoring_scene, |scene| {
            let missing =
                super::project_app::missing_render_asset_dependencies(&game_loop.runtime, &scene);
            if !missing.is_empty() {
                return Err(missing
                    .into_iter()
                    .map(|asset| {
                        let mut diagnostic = Diagnostic::new(
                            "EDPLAY_MISSING_ASSET",
                            DiagnosticSeverity::Error,
                            "editor.play-mode",
                            format!(
                                "Play mode cannot start while render asset '{}' is missing",
                                asset.id
                            ),
                        );
                        diagnostic.asset = Some(asset);
                        diagnostic
                    })
                    .collect());
            }
            game_loop.runtime.diagnostics_collector_mut().clear_frame();
            game_loop.load_scene(scene)?;
            let script_errors = game_loop
                .runtime
                .diagnostics_collector()
                .script_diagnostics
                .iter()
                .filter(|diagnostic| {
                    matches!(
                        diagnostic.severity,
                        DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            if script_errors.is_empty() {
                Ok(())
            } else {
                Err(script_errors)
            }
        }) {
            Ok(true) => {
                self.viewport_tab = ViewportTab::Game;
                let mut runtime_scene_id = self.current_scene_id.clone();
                game_loop.init_physics();
                #[cfg(feature = "target-desktop")]
                self.input_state.reset(&mut game_loop.input_map);
                self.last_frame_time = Instant::now();
                match super::project_app::process_pending_scene_transitions(
                    game_loop,
                    &self.project,
                    &mut runtime_scene_id,
                ) {
                    Ok(transitions) => {
                        self.play_runtime_scene_id = Some(runtime_scene_id.clone());
                        tracing::info!(
                            transitions,
                            scene_id = runtime_scene_id,
                            "editor: Play mode started"
                        );
                    }
                    Err(error) => {
                        tracing::error!(%error, "editor Play startup scene transition failed");
                        let diagnostics = recover_play_after_scene_transition_error(
                            &mut self.play_session,
                            game_loop,
                            error,
                        );
                        log_scene_diagnostics(
                            "editor Play stopped after startup scene transition failure",
                            diagnostics.clone(),
                        );
                        if let Some(editor_scene) = self.editor_scene.as_mut() {
                            editor_scene.diagnostics.push_many(diagnostics);
                        }
                        self.play_runtime_scene_id = None;
                        #[cfg(feature = "target-desktop")]
                        self.input_state.reset(&mut game_loop.input_map);
                    }
                }
            }
            Ok(false) => {}
            Err(mut diagnostics) => {
                self.play_runtime_scene_id = None;
                log_scene_diagnostics("editor Play start failed", diagnostics.clone());
                match restore_editor_preview(game_loop, &authoring_scene) {
                    Ok(warnings) => diagnostics.extend(warnings),
                    Err(rollback_diagnostics) => {
                        log_scene_diagnostics(
                            "editor Play rollback failed",
                            rollback_diagnostics.clone(),
                        );
                        diagnostics.extend(rollback_diagnostics);
                    }
                }
                if let Some(editor_scene) = self.editor_scene.as_mut() {
                    editor_scene.diagnostics.push_many(diagnostics);
                }
            }
        }
        if self.play_session.is_editing() {
            self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
        } else {
            self.request_ui_open_panel(protocol::UiPanel::Game, protocol::UiDockZone::Center);
        }
    }

    fn pause_play(&mut self) {
        if self.play_session.pause() {
            tracing::info!("editor: Play mode paused");
        }
    }

    fn resume_play(&mut self) {
        if self.play_session.resume() {
            self.last_frame_time = Instant::now();
            tracing::info!("editor: Play mode resumed");
        }
    }

    fn step_play(&mut self) {
        if self.play_session.mode() == EditorPlayMode::Paused {
            self.step_play_once = true;
            tracing::info!("editor: Play mode scheduled one fixed simulation step");
        }
    }

    fn request_editor_exit(&mut self) {
        if !self.play_session.is_editing() {
            self.stop_play();
            if !self.play_session.is_editing() {
                self.scene_document_status = Some(
                    "Could not close while Play mode failed to restore the authoring scene. Retry Stop first."
                        .to_string(),
                );
                return;
            }
        }
        let has_unsaved_changes = self.gizmo.dragging
            || self
                .editor_scene
                .as_ref()
                .is_some_and(|scene| scene.is_dirty() || scene.is_transform_gizmo_drag_active());
        if has_unsaved_changes {
            self.pending_scene_switch = None;
            self.pending_document_action = None;
            self.close_confirmation_pending = true;
            self.scene_document_status = Some(
                "Unsaved changes: choose Save & Close, Discard & Close, or Cancel Close."
                    .to_string(),
            );
        } else {
            self.exit_after_frame = true;
        }
    }

    fn stop_play(&mut self) {
        let Some(game_loop) = self.game_loop.as_mut() else {
            return;
        };
        match self.play_session.stop(|scene| {
            let (preview_scene, _) = editor_preview_scene(&game_loop.runtime, &scene);
            game_loop.load_scene(preview_scene)
        }) {
            Ok(true) => {
                self.play_runtime_scene_id = None;
                self.viewport_tab = ViewportTab::Scene;
                game_loop.init_physics();
                #[cfg(feature = "target-desktop")]
                self.input_state.reset(&mut game_loop.input_map);
                self.last_frame_time = Instant::now();
                tracing::info!("editor: Play mode stopped; authoring scene restored");
            }
            Ok(false) => {
                self.play_runtime_scene_id = None;
            }
            Err(diagnostics) => {
                self.play_runtime_scene_id = None;
                log_scene_diagnostics("editor Play stop failed", diagnostics);
            }
        }
        if self.play_session.is_editing() {
            self.request_ui_open_panel(protocol::UiPanel::Scene, protocol::UiDockZone::Center);
        } else {
            self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
        }
    }

    fn persist_workspace_preferences_if_changed(&mut self) {
        let (pitch, yaw, distance) = self.scene_view.camera_orbit();
        self.workspace_preferences.scene_pitch = pitch;
        self.workspace_preferences.scene_yaw = yaw;
        self.workspace_preferences.scene_distance = distance;
        self.workspace_preferences.scene_target = *self.scene_view.target();
        self.workspace_preferences.scene_orthographic = self.scene_view.orthographic();
        self.workspace_preferences.scene_camera_speed = self.scene_view.camera_speed();
        self.workspace_preferences.snapping_enabled = self.gizmo.snapping;
        if self.workspace_preferences == self.saved_workspace_preferences {
            return;
        }
        match save_workspace_preferences(&self.project, &self.workspace_preferences) {
            Ok(()) => self.saved_workspace_preferences = self.workspace_preferences.clone(),
            Err(error) => tracing::warn!(%error, "editor workspace preferences were not saved"),
        }
    }

    fn request_ui_open_panel(
        &mut self,
        panel: protocol::UiPanel,
        preferred_zone: protocol::UiDockZone,
    ) {
        let request = protocol::UiOpenPanelParams {
            panel,
            preferred_zone,
        };
        if self.pending_ui_open_panels.last() != Some(&request) {
            self.pending_ui_open_panels.push(request);
        }
    }

    fn take_ui_open_panel_events_json(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_ui_open_panels)
            .into_iter()
            .map(|params| {
                self.editor_event_sequence = self.editor_event_sequence.wrapping_add(1);
                serde_json::to_string(&protocol::BridgeEvent {
                    protocol: protocol::EDITOR_PROTOCOL,
                    session_id: self.session_id.clone(),
                    sequence: self.editor_event_sequence,
                    revision: self.editor_revision,
                    event: protocol::UI_OPEN_PANEL_EVENT,
                    params,
                })
                .expect("editor UI navigation events must serialize")
            })
            .collect()
    }

    fn execute_editor_command(&mut self, command: Box<dyn engine_editor::Command>) -> bool {
        let label = command.name().to_string();
        if !self.play_session.is_editing() {
            self.record_editor_command_error(&label, "Stop Play mode before editing the scene");
            return false;
        }
        let (Some(game_loop), Some(editor_scene)) =
            (self.game_loop.as_mut(), self.editor_scene.as_mut())
        else {
            self.record_editor_command_error(&label, "Editor scene runtime is not initialized");
            return false;
        };
        let result = match editor_scene.execute(command) {
            Ok(()) => {
                let existing_ids = editor_scene
                    .scene
                    .entities
                    .iter()
                    .map(|entity| entity.persistent_id.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                self.selected_entity_ids
                    .retain(|entity_id| existing_ids.contains(entity_id));
                let selection_exists = editor_scene
                    .selected_entity
                    .as_ref()
                    .is_some_and(|id| existing_ids.contains(id));
                if !selection_exists {
                    editor_scene.selected_entity = self.selected_entity_ids.first().cloned();
                } else if let Some(active) = editor_scene.selected_entity.as_ref() {
                    if !self.selected_entity_ids.contains(active) {
                        self.selected_entity_ids.push(active.clone());
                    }
                }
                synchronize_authoring_view(
                    game_loop,
                    editor_scene,
                    &self.scene_view,
                    self.viewport_tab,
                );
                Ok((label == "Set Scene Settings")
                    .then(|| editor_scene.scene.scene_settings.clone()))
            }
            Err(error) => Err(error.to_string()),
        };
        match result {
            Ok(scene_settings) => {
                if let Some(scene_settings) = scene_settings {
                    self.scene_settings_draft = scene_settings;
                }
                true
            }
            Err(error) => {
                self.record_editor_command_error(&label, error);
                false
            }
        }
    }

    fn start_editor_job(
        &mut self,
        label: impl Into<String>,
        reload_assets: bool,
        operation: impl FnOnce() -> Result<EditorJobOutput, String> + Send + 'static,
    ) -> Result<u64, String> {
        let label = label.into();
        if let Some(active) = self.editor_build_task.as_ref() {
            return Err(format!(
                "{} is already running; wait before starting {label}.",
                active.operation().display_name()
            ));
        }
        if let Some(active) = self.background_job.as_ref() {
            return Err(format!(
                "{} is already running; wait for it to finish.",
                active.label
            ));
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(operation());
        });
        let id = self.next_editor_operation_id;
        self.next_editor_operation_id = self.next_editor_operation_id.wrapping_add(1);
        self.build_status = Some(format!("{label} in progress..."));
        self.set_editor_operation_status(EditorOperationStatus {
            id,
            label: label.clone(),
            state: EditorOperationState::Running,
        });
        self.background_job = Some(EditorBackgroundJob {
            id,
            label,
            receiver,
            reload_assets,
        });
        Ok(id)
    }

    fn poll_editor_job(&mut self) -> bool {
        let result = self
            .background_job
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(format!(
                    "{} worker terminated without a result",
                    job.label
                ))),
            });
        let Some(result) = result else {
            return false;
        };
        let job = self
            .background_job
            .take()
            .expect("completed editor job must still be present");
        match result {
            Ok(output) => {
                let refresh_result = (|| {
                    if job.reload_assets {
                        let game_loop = self
                            .game_loop
                            .as_mut()
                            .ok_or_else(|| "Editor runtime is not initialized".to_string())?;
                        super::project_app::load_project_assets(
                            &mut game_loop.runtime,
                            &self.project,
                        )?;
                        self.material_editor_selection = None;
                    }
                    self.refresh_asset_catalog()?;
                    self.apply_editor_job_output(output)
                })();
                match refresh_result {
                    Ok(()) => {
                        self.set_editor_operation_status(EditorOperationStatus {
                            id: job.id,
                            label: job.label.clone(),
                            state: EditorOperationState::Succeeded,
                        });
                        self.build_status = Some(format!("{} completed successfully.", job.label));
                    }
                    Err(error) => {
                        let warning = format!(
                            "{} committed its project files, but the editor could not refresh the result: {error}. Do not retry the mutation; use Refresh after resolving the reported error.",
                            job.label
                        );
                        self.set_editor_operation_status(EditorOperationStatus {
                            id: job.id,
                            label: job.label.clone(),
                            state: EditorOperationState::CommittedWithWarning(warning.clone()),
                        });
                        self.record_build_error(&format!("{} refresh", job.label), warning);
                    }
                }
            }
            Err(error) => {
                self.set_editor_operation_status(EditorOperationStatus {
                    id: job.id,
                    label: job.label.clone(),
                    state: EditorOperationState::Failed(error.clone()),
                });
                self.record_build_error(&job.label, error);
            }
        }
        true
    }

    fn set_editor_operation_status(&mut self, status: EditorOperationStatus) {
        if let Some(existing) = self
            .recent_editor_operations
            .iter_mut()
            .find(|existing| existing.id == status.id)
        {
            *existing = status.clone();
        } else {
            self.recent_editor_operations.push_back(status.clone());
            while self.recent_editor_operations.len() > 16 {
                self.recent_editor_operations.pop_front();
            }
        }
        self.last_editor_operation = Some(status);
    }

    fn apply_editor_job_output(&mut self, output: EditorJobOutput) -> Result<(), String> {
        match output {
            EditorJobOutput::None => {}
            EditorJobOutput::SelectAsset(asset_id) => {
                if !self.asset_browser.reveal_asset(&asset_id) {
                    return Err(format!(
                        "asset '{asset_id}' was committed but is missing from the refreshed catalog"
                    ));
                }
            }
            EditorJobOutput::SelectFolder(folder) => {
                let normalized = folder.trim().replace('\\', "/");
                let requested = if normalized.trim_matches('/').is_empty() {
                    "/".to_string()
                } else {
                    format!("/{}", normalized.trim_matches('/'))
                };
                self.asset_browser.set_current_folder(&requested);
                if !self
                    .asset_browser
                    .current_folder()
                    .eq_ignore_ascii_case(&requested)
                {
                    return Err(format!(
                        "asset folder '{requested}' was committed but is missing from the refreshed catalog"
                    ));
                }
                self.asset_browser.select_asset(None);
            }
            EditorJobOutput::ClearAssetSelection => {
                self.asset_browser.select_asset(None);
            }
        }
        self.workspace_preferences.project_asset_folder =
            self.asset_browser.current_folder().to_string();
        Ok(())
    }

    fn start_editor_build(&mut self, operation: super::editor_build_ops::EditorBuildOperation) {
        let label = operation.kind().display_name();
        if let Some(job) = self.background_job.as_ref() {
            self.build_status = Some(format!(
                "{} is already running; wait before starting {label}.",
                job.label
            ));
            return;
        }
        if let Some(task) = self.editor_build_task.as_ref() {
            self.build_status = Some(format!(
                "{} is already running; cancel it or wait for completion.",
                task.operation().display_name()
            ));
            return;
        }
        let task = match self.editor_build_service.as_ref() {
            Ok(service) => service.start(&self.project.manifest_path, operation),
            Err(error) => {
                self.record_build_error(label, error.clone());
                return;
            }
        };
        match task {
            Ok(task) => {
                self.build_output.clear();
                self.build_status = Some(format!("{label} in progress..."));
                self.request_ui_open_panel(protocol::UiPanel::Build, protocol::UiDockZone::Bottom);
                self.editor_build_task = Some(task);
            }
            Err(error) => self.record_build_error(label, error.to_string()),
        }
    }

    fn poll_editor_build(&mut self) -> bool {
        let Some(task) = self.editor_build_task.as_mut() else {
            return false;
        };
        let output = task.output_snapshot();
        self.build_output = match (output.stdout.trim(), output.stderr.trim()) {
            ("", "") => String::new(),
            (stdout, "") => stdout.to_string(),
            ("", stderr) => stderr.to_string(),
            (stdout, stderr) => format!("{stdout}\n\n--- stderr ---\n{stderr}"),
        };
        let Some(result) = task.try_complete() else {
            return false;
        };
        self.editor_build_task = None;
        match result {
            Ok(super::editor_build_ops::EditorBuildResult::Validated(result)) => {
                self.build_status = Some(format!(
                    "Validated '{}': {} scenes, {} entities, {} declared / {} cooked assets in {:.2}s.",
                    result.project,
                    result.scenes,
                    result.entities,
                    result.declared_assets,
                    result.cooked_assets,
                    result.elapsed.as_secs_f32()
                ));
            }
            Ok(super::editor_build_ops::EditorBuildResult::CookedAndCompiled(result)) => {
                self.build_status = Some(format!(
                    "Cooked and compiled '{}' in {:.2}s{}.",
                    result.project,
                    result.elapsed.as_secs_f32(),
                    if result.scripts_configured {
                        " including project scripts"
                    } else {
                        ""
                    }
                ));
                let reload = self
                    .game_loop
                    .as_mut()
                    .ok_or_else(|| "Editor runtime is not initialized".to_string())
                    .and_then(|game_loop| {
                        super::project_app::load_project_assets(
                            &mut game_loop.runtime,
                            &self.project,
                        )
                        .map(|_| ())
                    });
                if let Err(error) = reload {
                    self.run_after_build = false;
                    self.record_build_error("Cook & Compile asset reload", error);
                } else if let Err(error) = self.refresh_asset_catalog() {
                    self.run_after_build = false;
                    self.record_build_error("Refresh project asset catalog", error);
                } else if self.run_after_build {
                    self.run_after_build = false;
                    match self.launch_project_player() {
                        Ok(pid) => {
                            self.build_status = Some(format!(
                                "Cooked, validated, and started project player ({pid})."
                            ));
                        }
                        Err(error) => self.record_build_error("Run project", error),
                    }
                }
            }
            Ok(super::editor_build_ops::EditorBuildResult::PackagedWindows(result)) => {
                self.build_status = Some(format!(
                    "Packaged Windows player {} in {:.2}s: {} (SHA-256 {}).",
                    result.version,
                    result.elapsed.as_secs_f32(),
                    result.archive_path.display(),
                    result.archive_sha256
                ));
                self.build_output.push_str(&format!(
                    "\n\nRelease root: {}\nArchive: {}\nArchive SHA-256: {}\nSymbols: {}\nSymbols SHA-256: {}\nManifest: {}\nDirty worktree: {}",
                    result.release_root.display(),
                    result.archive_path.display(),
                    result.archive_sha256,
                    result.symbols_archive_path.display(),
                    result.symbols_sha256,
                    result.release_manifest_path.display(),
                    result.dirty
                ));
            }
            Err(error) => {
                self.run_after_build = false;
                self.record_build_error(error.operation.display_name(), error.to_string())
            }
        }
        true
    }

    fn request_run_project(&mut self) {
        if self.editor_build_task.is_some() || self.background_job.is_some() {
            self.record_build_error(
                "Run project",
                "Wait for the active project operation to finish".to_string(),
            );
            return;
        }
        if !self.play_session.is_editing() {
            self.record_build_error(
                "Run project",
                "Stop the in-editor Play session before launching the player".to_string(),
            );
            return;
        }
        if let Err(error) = self.save_current_scene_document() {
            self.record_build_error("Run project", error);
            return;
        }
        let input_save = self
            .game_loop
            .as_ref()
            .ok_or_else(|| "Editor runtime is not initialized".to_string())
            .and_then(|game_loop| {
                super::project_input::save_project_input_map(&self.project, &game_loop.input_map)
            });
        if let Err(error) = input_save {
            self.record_build_error("Run project input settings", error);
            return;
        }
        self.run_after_build = true;
        self.start_editor_build(super::editor_build_ops::EditorBuildOperation::CookAndCompile);
        if self.editor_build_task.is_none() {
            self.run_after_build = false;
        } else {
            self.build_status =
                Some("Saving, validating, cooking, and compiling before Run...".to_string());
        }
    }

    fn launch_project_player(&self) -> Result<u32, String> {
        let executable = std::env::current_exe()
            .map_err(|error| format!("could not resolve editor executable: {error}"))?;
        std::process::Command::new(executable)
            .arg("project")
            .arg("run")
            .arg(&self.project.manifest_path)
            .spawn()
            .map(|child| child.id())
            .map_err(|error| format!("could not launch project player: {error}"))
    }

    fn cancel_editor_build(&mut self) {
        let Some(task) = self.editor_build_task.as_ref() else {
            self.build_status = Some("No cancellable build operation is running.".to_string());
            return;
        };
        match task.cancel() {
            Ok(true) => {
                self.build_status =
                    Some(format!("Cancelling {}...", task.operation().display_name()));
            }
            Ok(false) => {
                self.build_status =
                    Some("The build process has already finished; collecting result.".to_string());
            }
            Err(error) => self.record_build_error("Cancel build", error.to_string()),
        }
    }

    fn rebuild_and_reload_scripts(&mut self) {
        if self.background_job.is_some() || self.editor_build_task.is_some() {
            self.build_status = Some(
                "Wait for the active project operation before rebuilding scripts.".to_string(),
            );
            return;
        }
        let Some(game_loop) = self.game_loop.as_mut() else {
            self.record_build_error(
                "Rebuild & Reload Scripts",
                "Editor runtime is not initialized".to_string(),
            );
            return;
        };
        self.build_status = Some("Rebuilding project scripts...".to_string());
        match super::project_scripts::rebuild_and_reload_project_scripts(
            &mut game_loop.runtime,
            &self.project,
        ) {
            Ok(result) => {
                let verified_classes = game_loop.runtime.verified_script_classes().len();
                self.build_status = Some(format!(
                    "Rebuilt and transactionally reloaded {} script assemblies; {} concrete EngineBehaviour classes verified.",
                    result.assemblies, verified_classes
                ));
                self.request_ui_open_panel(protocol::UiPanel::Build, protocol::UiDockZone::Bottom);
            }
            Err(error) => self.record_build_error("Rebuild & Reload Scripts", error),
        }
    }

    fn verified_script_add_command(
        &self,
        assembly_id: &str,
        class_name: &str,
    ) -> Result<Box<dyn engine_editor::Command>, String> {
        if self.project.script_assembly.is_none() {
            return Err(
                "game.project.json does not configure a compiled script_assembly".to_string(),
            );
        }
        let runtime = &self
            .game_loop
            .as_ref()
            .ok_or_else(|| "Editor runtime is not initialized".to_string())?
            .runtime;
        if !runtime
            .verified_script_classes()
            .iter()
            .any(|class| class.assembly_id == assembly_id && class.class_name == class_name)
        {
            return Err(format!(
                "'{class_name}' is not in the reflection-verified class list for loaded assembly '{assembly_id}'; rebuild and reload scripts"
            ));
        }
        let editor_scene = self
            .editor_scene
            .as_ref()
            .ok_or_else(|| "No editor scene is open".to_string())?;
        let selected_id = editor_scene
            .selected_entity
            .as_ref()
            .ok_or_else(|| "Select an entity before adding a script".to_string())?;
        let entity = editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| &entity.persistent_id == selected_id)
            .ok_or_else(|| format!("Selected entity '{selected_id}' no longer exists"))?;
        if entity.components.contains_key("engine.script") {
            return Err(format!(
                "Entity '{}' already has an engine.script component",
                entity.persistent_id
            ));
        }
        let component = ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([
                ("assembly_id".into(), Value::Str(assembly_id.to_string())),
                ("class_name".into(), Value::Str(class_name.to_string())),
            ]),
        };
        Ok(Box::new(engine_editor::AddComponent::new(
            entity.persistent_id.clone(),
            "engine.script".to_string(),
            component,
        )))
    }

    fn record_build_error(&mut self, label: &str, error: String) {
        self.build_status = Some(format!("{label} failed: {error}"));
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.diagnostics.push(Diagnostic::new(
                "EDBUILD_FAILED",
                DiagnosticSeverity::Error,
                "editor.build",
                format!("{label} failed: {error}"),
            ));
        }
        self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
    }

    fn record_editor_command_error(&mut self, label: &str, error: impl Into<String>) {
        let error = error.into();
        tracing::error!(label, %error, "editor authoring command failed");
        self.build_status = Some(format!("{label} failed: {error}"));
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.diagnostics.push(Diagnostic::new(
                "EDCOMMAND_FAILED",
                DiagnosticSeverity::Error,
                "editor.command",
                format!("{label} failed: {error}"),
            ));
        }
        self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
    }

    fn create_prefab_from_selection(
        &mut self,
        asset_id: String,
        relative_source_path: PathBuf,
        manifest_name: PathBuf,
    ) -> Result<(), String> {
        if !self.play_session.is_editing() {
            return Err("Stop Play before authoring a prefab".to_string());
        }
        if self.background_job.is_some() || self.editor_build_task.is_some() {
            return Err("Wait for the active project operation to finish".to_string());
        }
        let created = (|| {
            let editor_scene = self
                .editor_scene
                .as_ref()
                .ok_or_else(|| "No editor scene is open".to_string())?;
            let selected = editor_scene
                .selected_entity
                .as_ref()
                .ok_or_else(|| "Select an entity hierarchy to create a prefab".to_string())?;
            create_prefab_asset_from_scene(
                &editor_scene.scene,
                selected,
                PrefabAssetCreateRequest {
                    source_root: &self.project.asset_source,
                    manifest_path: &manifest_name,
                    relative_source_path: &relative_source_path,
                    asset_id: AssetId::new(asset_id),
                },
            )
            .map_err(|error| error.to_string())
        })()?;

        self.refresh_asset_catalog()?;
        self.asset_browser
            .select_asset(Some(created.asset_id.clone()));
        let source_path = created.source_path.clone();
        let asset_id = created.asset_id.id.clone();
        self.start_editor_build(super::editor_build_ops::EditorBuildOperation::CookAndCompile);
        if self.editor_build_task.is_some() {
            self.build_status = Some(format!(
                "Created prefab '{asset_id}' at {}; cooking and compiling it through the project build pipeline...",
                source_path.display()
            ));
        }
        Ok(())
    }

    fn instantiate_prefab_asset(
        &mut self,
        asset_id: AssetId,
        parent_id: Option<PersistentId>,
    ) -> Result<(), String> {
        let prepared = (|| {
            let editor_scene = self
                .editor_scene
                .as_ref()
                .ok_or_else(|| "No editor scene is open".to_string())?;
            let game_loop = self
                .game_loop
                .as_ref()
                .ok_or_else(|| "Editor runtime is not initialized".to_string())?;
            if game_loop
                .runtime
                .asset_registry()
                .get::<engine_scene::Prefab>(&asset_id)
                .is_none()
            {
                return Err(format!(
                    "Prefab '{}' is not loaded. Run Cook & Compile Project, then instantiate it.",
                    asset_id.id
                ));
            }
            prepare_prefab_instantiation_from_registry(
                &editor_scene.scene,
                game_loop.runtime.asset_registry(),
                &asset_id,
                parent_id
                    .clone()
                    .map(engine_editor::EntityPasteParent::Entity)
                    .unwrap_or(engine_editor::EntityPasteParent::SceneRoot),
            )
            .map_err(|error| match error {
                PrefabAuthoringError::AssetNotLoaded(missing) => format!(
                    "Prefab '{missing}' is not loaded. Run Cook & Compile Project, then instantiate it."
                ),
                error => error.to_string(),
            })
        })();
        let plan = prepared?;
        let root = plan.root_entity_id().clone();
        let count = plan.entity_ids().len();
        if !self.execute_editor_command(plan.into_command()) {
            self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
            return Err("The prefab instantiation command was rejected".to_string());
        }
        self.selected_entity_ids = vec![root.clone()];
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.selected_entity = Some(root.clone());
        }
        self.build_status = Some(format!(
            "Instantiated prefab '{}' as '{}' ({} entities).",
            asset_id.id, root, count
        ));
        Ok(())
    }

    fn unpack_prefab_instance(
        &mut self,
        entity_id: PersistentId,
        mode: PrefabUnpackMode,
    ) -> Result<(), String> {
        let plan = self
            .editor_scene
            .as_ref()
            .ok_or_else(|| "No editor scene is open".to_string())
            .and_then(|editor_scene| {
                prepare_unpack_prefab(&editor_scene.scene, &entity_id, mode)
                    .map_err(|error| error.to_string())
            });
        let plan = plan?;
        let count = plan.entity_ids().len();
        if !self.execute_editor_command(plan.into_command()) {
            self.request_ui_open_panel(protocol::UiPanel::Console, protocol::UiDockZone::Bottom);
            return Err("The prefab unpack command was rejected".to_string());
        }
        self.selected_entity_ids = vec![entity_id.clone()];
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.selected_entity = Some(entity_id.clone());
        }
        let scope = match mode {
            PrefabUnpackMode::Instance => "instance",
            PrefabUnpackMode::Completely => "instance and nested prefab links",
        };
        self.build_status = Some(format!(
            "Unpacked {scope} at '{entity_id}' ({count} prefab link records removed)."
        ));
        Ok(())
    }

    fn refresh_asset_catalog(
        &mut self,
    ) -> Result<engine_editor::asset_browser::AssetRefreshSummary, String> {
        let game_loop = self
            .game_loop
            .as_ref()
            .ok_or_else(|| "Editor runtime is not initialized".to_string())?;
        let requested_folder = self.workspace_preferences.project_asset_folder.clone();
        let summary = refresh_project_asset_list(
            &mut self.asset_browser,
            game_loop.runtime.asset_registry(),
            &self.project.asset_source,
        )
        .map_err(|error| error.to_string())?;
        self.asset_browser.set_current_folder(requested_folder);
        self.workspace_preferences.project_asset_folder =
            self.asset_browser.current_folder().to_string();
        Ok(summary)
    }

    fn copy_component_to_clipboard(
        &mut self,
        entity_id: &PersistentId,
        component_type: &str,
    ) -> Result<(), String> {
        let editor_scene = self
            .editor_scene
            .as_ref()
            .ok_or_else(|| "No editor scene is open".to_string())?;
        let clipboard = engine_editor::ComponentClipboard::capture(
            &editor_scene.scene,
            entity_id,
            &component_type.to_string(),
        )
        .map_err(|error| error.to_string())?;
        self.component_clipboard = Some(clipboard);
        self.build_status = Some(format!("Copied component '{component_type}'."));
        Ok(())
    }

    fn paste_component_to_entities(
        &mut self,
        entity_ids: Vec<PersistentId>,
        component_type: String,
    ) -> Result<(), String> {
        let commands = {
            let editor_scene = self
                .editor_scene
                .as_ref()
                .ok_or_else(|| "No editor scene is open".to_string())?;
            let clipboard = self
                .component_clipboard
                .as_ref()
                .ok_or_else(|| "The component clipboard is empty".to_string())?;
            entity_ids
                .into_iter()
                .map(|entity_id| {
                    engine_editor::ReplaceComponent::prepare(
                        &editor_scene.scene,
                        entity_id,
                        component_type.clone(),
                        clipboard,
                    )
                    .map(|command| Box::new(command) as Box<dyn engine_editor::Command>)
                    .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        if !self.execute_editor_command(Box::new(engine_editor::CommandBatch::new(
            "Paste Component Values",
            commands,
        ))) {
            return Err("The component changed before paste could be applied".to_string());
        }
        Ok(())
    }

    fn paste_entity_clipboard(
        &mut self,
        parent: engine_editor::EntityPasteParent,
    ) -> Result<(), String> {
        let (command, selected) = {
            let editor_scene = self
                .editor_scene
                .as_ref()
                .ok_or_else(|| "No editor scene is open".to_string())?;
            let clipboard = self
                .entity_clipboard
                .as_ref()
                .ok_or_else(|| "The entity clipboard is empty".to_string())?;
            let command =
                engine_editor::PasteEntityRecords::prepare(&editor_scene.scene, clipboard, parent)
                    .map_err(|error| error.to_string())?;
            let selected = command.pasted_root_ids().to_vec();
            if selected.is_empty() {
                return Err("The prepared paste has no root entity".to_string());
            }
            (
                Box::new(command) as Box<dyn engine_editor::Command>,
                selected,
            )
        };
        if !self.execute_editor_command(command) {
            return Err("The scene changed before the paste could be applied".to_string());
        }
        self.selected_entity_ids = selected.clone();
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.selected_entity = selected.first().cloned();
        }
        Ok(())
    }

    fn duplicate_entities(&mut self, source_ids: &[PersistentId]) -> Result<(), String> {
        let (command, selected) = {
            let editor_scene = self
                .editor_scene
                .as_ref()
                .ok_or_else(|| "No editor scene is open".to_string())?;
            let clipboard =
                engine_editor::EntityClipboard::capture(&editor_scene.scene, source_ids)
                    .map_err(|error| error.to_string())?;
            let command = engine_editor::PasteEntityRecords::prepare(
                &editor_scene.scene,
                &clipboard,
                engine_editor::EntityPasteParent::PreserveOriginal,
            )
            .map_err(|error| error.to_string())?;
            let selected = command.pasted_root_ids().to_vec();
            (
                Box::new(command) as Box<dyn engine_editor::Command>,
                selected,
            )
        };
        if !self.execute_editor_command(command) {
            return Err("The scene changed before duplication could be applied".to_string());
        }
        self.selected_entity_ids = selected.clone();
        if let Some(editor_scene) = self.editor_scene.as_mut() {
            editor_scene.selected_entity = selected.first().cloned();
        }
        Ok(())
    }

    fn process_material_save(&mut self) {
        let request = match self.material_editor.take_save_request() {
            Ok(request) => request,
            Err(error) => {
                self.material_editor.report_save_failure(error.clone());
                if let Some(editor_scene) = self.editor_scene.as_mut() {
                    editor_scene.diagnostics.push(Diagnostic::new(
                        "EDMATERIAL_SAVE_FAILED",
                        DiagnosticSeverity::Error,
                        "editor.material",
                        error,
                    ));
                }
                return;
            }
        };
        let Some(request) = request else {
            return;
        };
        let result = if !self.play_session.is_editing() {
            Err("Stop Play before saving project materials.".to_string())
        } else if let Some(game_loop) = self.game_loop.as_mut() {
            save_project_material(&mut game_loop.runtime, &self.project, &request)
        } else {
            Err("Editor runtime is not initialized".to_string())
        };
        match result {
            Ok(outcome) => {
                self.material_editor.report_save_success(format!(
                    "Saved {} and refreshed {}.",
                    outcome.source_path.display(),
                    outcome.cooked_path.display()
                ));
                if let Err(error) = self.refresh_asset_catalog() {
                    self.record_editor_command_error("Refresh project asset catalog", error);
                }
            }
            Err(error) => {
                self.material_editor.report_save_failure(error.clone());
                let mut diagnostic = Diagnostic::new(
                    "EDMATERIAL_SAVE_FAILED",
                    DiagnosticSeverity::Error,
                    "editor.material",
                    error.clone(),
                );
                diagnostic.asset = Some(AssetId::new(request.material_asset));
                if let Some(editor_scene) = self.editor_scene.as_mut() {
                    editor_scene.diagnostics.push(diagnostic);
                }
                tracing::error!(%error, "editor material save failed");
            }
        }
    }

    fn process_gizmo_inputs(&mut self) {
        if !gizmo_viewport_enabled(
            self.workspace_preferences.gizmos_visible,
            self.play_session.is_editing(),
            self.viewport_tab,
        ) {
            self.gizmo_pointer_events.clear();
            self.gizmo.cancel_drag();
            if let Some(editor_scene) = self.editor_scene.as_mut() {
                let _ = editor_scene.cancel_transform_gizmo_drag();
            }
            return;
        }
        let Some((interaction_min, interaction_max, render_viewport)) = editor_render_viewport(
            self.web_viewport_rect,
            self.window_scale_factor,
            Vec2::new(self.window_w, self.window_h),
        ) else {
            self.gizmo_pointer_events.clear();
            self.gizmo.cancel_drag();
            return;
        };
        let events = std::mem::take(&mut self.gizmo_pointer_events);
        let mut scene_changed = false;
        for event in events {
            if event == GizmoPointerEvent::Cancel {
                if let Some(editor_scene) = self.editor_scene.as_mut() {
                    scene_changed |= editor_scene.cancel_transform_gizmo_drag();
                }
                self.gizmo.cancel_drag();
                continue;
            }
            let press = match event {
                GizmoPointerEvent::Press(pointer) => Some(pointer),
                _ => None,
            };
            if press.is_some_and(|pointer| {
                pointer.x < interaction_min.x
                    || pointer.y < interaction_min.y
                    || pointer.x > interaction_max.x
                    || pointer.y > interaction_max.y
            }) {
                continue;
            }
            let selected = self
                .editor_scene
                .as_ref()
                .and_then(|scene| scene.selected_entity.clone());
            let Some(selected) = selected else {
                if let Some(pointer) = press {
                    let picked = self.game_loop.as_ref().and_then(|game_loop| {
                        pick_runtime_entity(
                            &game_loop.runtime,
                            self.frame,
                            render_viewport,
                            interaction_min,
                            interaction_max,
                            pointer,
                        )
                    });
                    if let Some(editor_scene) = self.editor_scene.as_mut() {
                        editor_scene.selected_entity = picked;
                    }
                }
                continue;
            };
            let view = self.game_loop.as_ref().and_then(|game_loop| {
                runtime_gizmo_view(&game_loop.runtime, &selected, self.frame, render_viewport)
                    .and_then(|view| {
                        restrict_gizmo_view_to_rect(view, interaction_min, interaction_max)
                    })
            });
            let Some(view) = view else {
                self.gizmo.cancel_drag();
                continue;
            };
            if let (Some(game_loop), Some(editor_scene)) =
                (self.game_loop.as_ref(), self.editor_scene.as_mut())
            {
                scene_changed |= process_gizmo_pointer_events(
                    vec![event],
                    editor_scene,
                    &mut self.gizmo,
                    &game_loop.runtime,
                    &selected,
                    view,
                );
            }
            if let Some(pointer) = press.filter(|_| !self.gizmo.dragging) {
                let picked = self.game_loop.as_ref().and_then(|game_loop| {
                    pick_runtime_entity(
                        &game_loop.runtime,
                        self.frame,
                        render_viewport,
                        interaction_min,
                        interaction_max,
                        pointer,
                    )
                });
                if let Some(editor_scene) = self.editor_scene.as_mut() {
                    editor_scene.selected_entity = picked;
                }
            }
        }
        if scene_changed {
            if let (Some(game_loop), Some(editor_scene)) =
                (self.game_loop.as_mut(), self.editor_scene.as_mut())
            {
                synchronize_authoring_view(
                    game_loop,
                    editor_scene,
                    &self.scene_view,
                    self.viewport_tab,
                );
            }
        }
    }

    fn tick_editor_play_mode(&mut self, delta_seconds: f32) {
        let stepping = std::mem::take(&mut self.step_play_once)
            && self.play_session.mode() == EditorPlayMode::Paused;
        if !self.play_session.should_tick() && !stepping {
            return;
        }
        let (Some(game_loop), Some(editor_scene)) =
            (self.game_loop.as_mut(), self.editor_scene.as_mut())
        else {
            return;
        };
        game_loop.update(if stepping { 1.0 / 60.0 } else { delta_seconds });
        if let Err(error) =
            super::project_scripts::fail_on_script_errors(&game_loop.runtime, "update")
        {
            let diagnostics =
                recover_play_after_script_error(&mut self.play_session, game_loop, error);
            editor_scene.diagnostics.push_many(diagnostics);
            self.play_runtime_scene_id = None;
            #[cfg(feature = "target-desktop")]
            self.input_state.reset(&mut game_loop.input_map);
            return;
        }
        let Some(runtime_scene_id) = self.play_runtime_scene_id.as_mut() else {
            return;
        };
        if let Err(error) = super::project_app::process_pending_scene_transitions(
            game_loop,
            &self.project,
            runtime_scene_id,
        ) {
            let diagnostics =
                recover_play_after_scene_transition_error(&mut self.play_session, game_loop, error);
            editor_scene.diagnostics.push_many(diagnostics);
            self.play_runtime_scene_id = None;
            #[cfg(feature = "target-desktop")]
            self.input_state.reset(&mut game_loop.input_map);
        }
    }

    fn render_react_frame(&mut self) -> EditorFrameOutcome {
        if self.editor_scene.is_none() || self.game_loop.is_none() {
            return EditorFrameOutcome::Completed;
        }
        let editor_job_completed = self.poll_editor_job();
        let editor_build_completed = self.poll_editor_build();
        if editor_job_completed || editor_build_completed {
            self.editor_revision = self.editor_revision.wrapping_add(1);
            self.pending_full_snapshot = true;
        }
        self.maybe_write_recovery_snapshot();
        if let Some(game_loop) = self.game_loop.as_ref() {
            engine_editor::animation_preview::refresh_animation_assets(
                &mut self.animation_preview,
                game_loop.runtime.asset_registry(),
            );
        }

        self.process_material_save();
        self.process_gizmo_inputs();

        let now = Instant::now();
        let delta_seconds = now
            .duration_since(self.last_frame_time)
            .as_secs_f32()
            .min(0.1);
        self.last_frame_time = now;
        engine_editor::animation_preview::update_preview(
            &mut self.animation_preview,
            delta_seconds,
        );
        self.tick_web_viewport_camera(delta_seconds);
        if self.play_session.is_editing() && self.viewport_tab == ViewportTab::Scene {
            if let Some(game_loop) = self.game_loop.as_ref() {
                let _ = apply_editor_camera(&game_loop.runtime, &self.scene_view);
            }
        }
        self.tick_editor_play_mode(delta_seconds);

        let Some((interaction_min, interaction_max, render_viewport)) = editor_render_viewport(
            self.web_viewport_rect,
            self.window_scale_factor,
            Vec2::new(self.window_w, self.window_h),
        ) else {
            self.frame = self.frame.wrapping_add(1);
            return EditorFrameOutcome::Completed;
        };
        let gizmo_batch = if gizmo_viewport_enabled(
            self.workspace_preferences.gizmos_visible,
            self.play_session.is_editing(),
            self.viewport_tab,
        ) {
            self.editor_scene.as_ref().and_then(|editor_scene| {
                let selected = editor_scene.selected_entity.as_deref()?;
                let game_loop = self.game_loop.as_ref()?;
                let view =
                    runtime_gizmo_view(&game_loop.runtime, selected, self.frame, render_viewport)?;
                let view = restrict_gizmo_view_to_rect(view, interaction_min, interaction_max)?;
                build_gizmo_ui_batch(
                    &self.gizmo,
                    view.world_position,
                    view.world_rotation,
                    view.view,
                    view.projection,
                    view.viewport_size,
                )
                .map(|batch| offset_gizmo_batch(batch, view))
            })
        } else {
            None
        };

        let mut engine_overlay_batches = Vec::new();
        if let Some(batch) = gizmo_batch {
            engine_overlay_batches.push(batch);
        }
        let overlay_batch_count = engine_overlay_batches.len();
        let overlay_vertex_count = engine_overlay_batches
            .iter()
            .map(|batch: &UiBatch| batch.vertices.len())
            .sum::<usize>();
        let Some(game_loop) = self.game_loop.as_mut() else {
            return EditorFrameOutcome::Completed;
        };
        let outcome = match game_loop.render_embedded_viewport(
            self.frame,
            engine_overlay_batches,
            render_viewport,
        ) {
            Ok(stats) => {
                if self.render_faulted {
                    tracing::info!(frame = self.frame, "editor renderer recovered");
                }
                self.render_faulted = false;
                let _ = game_loop.runtime.with_world(|world| {
                    engine_editor::performance::record_frame(
                        &mut self.performance.frame_stats,
                        world,
                        Some(&stats),
                    );
                });
                self.performance.frame_stats.frame_time_ms =
                    editor_frame_time_ms(self.performance.frame_stats.frame_time_ms, delta_seconds);
                self.performance.frame_stats.asset_count = game_loop
                    .runtime
                    .asset_registry()
                    .cached_ids()
                    .len()
                    .try_into()
                    .unwrap_or(u32::MAX);
                self.performance.commit_frame();
                tracing::debug!(
                    frame = self.frame,
                    draw_calls = stats.draw_calls,
                    overlay_batches = overlay_batch_count,
                    overlay_vertices = overlay_vertex_count,
                    "React editor viewport frame"
                );
                EditorFrameOutcome::Completed
            }
            Err(diagnostics) => {
                if !self.render_faulted {
                    super::log_renderer_diagnostics("editor render", &diagnostics);
                }
                self.render_faulted = true;
                EditorFrameOutcome::Failed
            }
        };
        self.frame = self.frame.wrapping_add(1);
        outcome
    }
    fn initialize_native_surface(
        &mut self,
        display_handle: raw_window_handle::RawDisplayHandle,
        window_handle: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
        scale_factor: f64,
    ) -> Result<(), String> {
        self.surface_zero_sized = width == 0 || height == 0;
        self.surface_occluded = false;
        self.render_faulted = false;
        self.window_w = width.max(1) as f32;
        self.window_h = height.max(1) as f32;
        self.window_scale_factor = scale_factor;
        let backend = create_vulkan_backend_renderer(
            display_handle,
            window_handle,
            width.max(1),
            height.max(1),
            std::env::var("ENGINE_VK_VALIDATION").is_ok(),
            None,
        )
        .map_err(|error| format!("Vulkan backend creation failed: {error}"))?;

        let mut game_loop = GameLoop::new(EngineConfig {
            application_name: format!("{} Editor", self.project.manifest.name),
            gpu_timestamps: true,
        });
        #[cfg(feature = "target-desktop")]
        {
            game_loop.input_map = super::project_input::load_project_input_map(&self.project)
                .map_err(|error| format!("editor input actions failed to load: {error}"))?;
        }
        game_loop.runtime.set_renderer_backend(backend);
        self.game_loop = Some(game_loop);
        self.init_scene();
        tracing::info!("React editor host and Vulkan viewport initialized");
        Ok(())
    }

    fn surface_render_suspended(&self) -> bool {
        self.surface_occluded || self.surface_zero_sized
    }

    fn handle_native_surface_resize(
        &mut self,
        width: u32,
        height: u32,
        scale_factor: Option<f64>,
    ) -> HostDirective {
        if let Some(scale_factor) = scale_factor {
            self.window_scale_factor = scale_factor;
            self.web_viewport_rect = ScreenRect::default();
        }
        self.cancel_web_viewport_input();
        self.surface_zero_sized = width == 0 || height == 0;
        self.window_w = width as f32;
        self.window_h = height as f32;
        if self.surface_zero_sized {
            return HostDirective::Continue;
        }

        if let Some(game_loop) = self.game_loop.as_mut() {
            if let Err(diagnostics) = game_loop.runtime.resize_renderer(width, height) {
                if !self.render_faulted {
                    let operation = if scale_factor.is_some() {
                        "editor DPI resize"
                    } else {
                        "editor resize"
                    };
                    super::log_renderer_diagnostics(operation, &diagnostics);
                }
                self.render_faulted = true;
                return HostDirective::Continue;
            }
        }

        self.last_frame_time = Instant::now();
        if self.surface_occluded {
            HostDirective::Continue
        } else {
            HostDirective::RequestRedraw
        }
    }

    fn handle_dropped_asset(&mut self, path: PathBuf) {
        if !self.play_session.is_editing() {
            self.record_build_error(
                "Import asset",
                "Stop Play mode before importing dropped files".to_string(),
            );
            return;
        }
        if self.background_job.is_some() {
            self.record_build_error(
                "Import asset",
                "Wait for the current asset operation to finish before dropping another file"
                    .to_string(),
            );
            return;
        }
        let mut asset_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("asset")
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        if asset_id.is_empty()
            || !asset_id
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
        {
            asset_id.insert(0, 'a');
        }
        let project_path = self.project.manifest_path.clone();
        let source_path = path.clone();
        let imported_id = asset_id.clone();
        if let Err(error) = self.start_editor_job("Asset import", true, move || {
            super::project_cli::import_project_asset_from(
                project_path,
                source_path,
                asset_id,
                None,
                PathBuf::new(),
            )?;
            Ok(EditorJobOutput::SelectAsset(imported_id))
        }) {
            self.record_build_error("Asset import", error);
            return;
        }
        self.request_ui_open_panel(protocol::UiPanel::Project, protocol::UiDockZone::Bottom);
        self.build_status = Some(format!("Importing dropped asset '{}'.", path.display()));
    }

    fn project_changed_json(&mut self) -> String {
        self.pending_full_snapshot = false;
        self.editor_event_sequence = self.editor_event_sequence.wrapping_add(1);
        serde_json::to_string(&protocol::BridgeEvent {
            protocol: protocol::EDITOR_PROTOCOL,
            session_id: self.session_id.clone(),
            sequence: self.editor_event_sequence,
            revision: self.editor_revision,
            event: protocol::PROJECT_CHANGED_EVENT,
            params: self.editor_snapshot(),
        })
        .expect("editor snapshots must serialize")
    }

    fn telemetry_json(&mut self) -> String {
        self.editor_event_sequence = self.editor_event_sequence.wrapping_add(1);
        serde_json::to_string(&protocol::BridgeEvent {
            protocol: protocol::EDITOR_PROTOCOL,
            session_id: self.session_id.clone(),
            sequence: self.editor_event_sequence,
            revision: self.editor_revision,
            event: protocol::TELEMETRY_EVENT,
            params: self.editor_telemetry(),
        })
        .expect("editor telemetry must serialize")
    }

    fn take_frame_bridge_messages(&mut self, periodic_telemetry: bool) -> Vec<String> {
        let mut messages = if self.pending_full_snapshot {
            vec![self.project_changed_json()]
        } else if periodic_telemetry {
            vec![self.telemetry_json()]
        } else {
            Vec::new()
        };
        messages.extend(self.take_ui_open_panel_events_json());
        messages
    }

    fn handle_close_requested(&mut self) -> bool {
        if !self.play_session.is_editing() {
            self.stop_play();
            if !self.play_session.is_editing() {
                self.scene_document_status = Some(
                    "Could not close while Play mode failed to restore the authoring scene. Retry Stop or cancel Play errors first."
                        .to_string(),
                );
                return false;
            }
        }
        let has_unsaved_changes = self.gizmo.dragging
            || self
                .editor_scene
                .as_ref()
                .is_some_and(|scene| scene.is_dirty() || scene.is_transform_gizmo_drag_active());
        if has_unsaved_changes {
            self.pending_scene_switch = None;
            self.pending_document_action = None;
            self.close_confirmation_pending = true;
            self.scene_document_status = Some(
                "Unsaved changes: choose Save & Close, Discard & Close, or Cancel Close."
                    .to_string(),
            );
            self.editor_revision = self.editor_revision.wrapping_add(1);
            false
        } else {
            true
        }
    }
}

fn host_message_script(message: &str) -> String {
    let argument = serde_json::to_string(message).expect("IPC JSON must be serializable as JS");
    format!("window.__ENGINE_EDITOR_RECEIVE__?.({argument});")
}

fn host_messages(messages: impl IntoIterator<Item = String>) -> HostDirective {
    HostDirective::Batch(
        messages
            .into_iter()
            .map(|message| HostDirective::EvaluateScript(host_message_script(&message)))
            .chain(std::iter::once(HostDirective::RequestRedraw))
            .collect(),
    )
}

impl EditorHostClient for EditorApp {
    fn on_host_event(&mut self, event: HostEvent) -> HostDirective {
        match event {
            HostEvent::SurfaceReady {
                window_handle,
                display_handle,
                size,
                scale_factor,
            } => match self.initialize_native_surface(
                display_handle,
                window_handle,
                size.width,
                size.height,
                scale_factor,
            ) {
                Ok(()) => HostDirective::RequestRedraw,
                Err(error) => {
                    tracing::error!(%error, "editor native surface initialization failed");
                    HostDirective::Exit
                }
            },
            HostEvent::Ipc(raw) => {
                let messages = self.dispatch_ipc_json(&raw).json_messages;
                host_messages(messages)
            }
            HostEvent::FileDropped { paths, position: _ } => {
                for path in paths {
                    self.handle_dropped_asset(path);
                }
                self.editor_revision = self.editor_revision.wrapping_add(1);
                let mut messages = vec![self.project_changed_json()];
                messages.extend(self.take_ui_open_panel_events_json());
                host_messages(messages)
            }
            HostEvent::Resized(size) => {
                self.handle_native_surface_resize(size.width, size.height, None)
            }
            HostEvent::ScaleFactorChanged { scale_factor, size } => {
                self.handle_native_surface_resize(size.width, size.height, Some(scale_factor))
            }
            HostEvent::Occluded(occluded) => {
                self.surface_occluded = occluded;
                if occluded {
                    self.cancel_web_viewport_input();
                    HostDirective::Continue
                } else if self.surface_zero_sized {
                    HostDirective::Continue
                } else {
                    self.last_frame_time = Instant::now();
                    HostDirective::RequestRedraw
                }
            }
            HostEvent::Focused(focused) => {
                if !focused {
                    self.persist_workspace_preferences_if_changed();
                    self.cancel_web_viewport_input();
                    #[cfg(feature = "runtime-subsystems")]
                    if let Some(game_loop) = self.game_loop.as_mut() {
                        game_loop.cancel_ui_pointer();
                    }
                }
                if focused && !self.surface_render_suspended() {
                    HostDirective::RequestRedraw
                } else {
                    HostDirective::Continue
                }
            }
            HostEvent::Redraw => {
                if self.surface_render_suspended() {
                    return HostDirective::Continue;
                }
                let outcome = self.render_react_frame();
                if self.exit_after_frame {
                    return HostDirective::Exit;
                }
                if outcome == EditorFrameOutcome::Failed {
                    let messages = self.take_frame_bridge_messages(false);
                    return if messages.is_empty() {
                        HostDirective::Continue
                    } else {
                        host_messages(messages)
                    };
                }
                let messages = self.take_frame_bridge_messages(self.frame % 12 == 0);
                if messages.is_empty() {
                    HostDirective::RequestRedraw
                } else {
                    host_messages(messages)
                }
            }
            HostEvent::CloseRequested => {
                self.persist_workspace_preferences_if_changed();
                if self.handle_close_requested() {
                    HostDirective::Exit
                } else {
                    let mut messages = vec![self.project_changed_json()];
                    messages.extend(self.take_ui_open_panel_events_json());
                    host_messages(messages)
                }
            }
        }
    }
}
pub fn run_editor(project_path: PathBuf) {
    let project = match GameProject::load(&project_path) {
        Ok(project) => project,
        Err(error) => {
            tracing::error!(path = %project_path.display(), %error, "editor project load failed");
            std::process::exit(2);
        }
    };
    if let Err(error) = super::project_cli::cook_project(&project_path) {
        tracing::error!(path = %project_path.display(), %error, "editor project cook failed");
        std::process::exit(2);
    }
    if let Err(error) = super::project_cli::build_project_scripts(&project_path, false) {
        tracing::error!(path = %project_path.display(), %error, "editor project script build failed");
        std::process::exit(2);
    }
    let title = format!("{} - Engine Editor", project.manifest.name);
    let app = EditorApp::new(project);
    let mut config = EditorHostConfig::new(title, EDITOR_WEB_ASSETS)
        .with_initial_size(1600, 900)
        .with_minimum_size(1120, 680);
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("ENGINE_EDITOR_WEB_DEV_URL") {
        tracing::info!(%url, "loading React editor from the loopback development server");
        config = config.with_development_url(url);
    }
    if let Err(e) = engine_editor_host::run_editor_host(config, app) {
        tracing::error!("editor: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_asset::cook::{MaterialSource, MATERIAL_SOURCE_SCHEMA};
    use engine_asset::project::ProjectManifest;
    use engine_renderer::{BackendRenderer, MaterialUpload, MeshUpload};

    #[test]
    fn editor_frame_time_uses_cpu_interval_until_gpu_timing_is_available() {
        assert_eq!(editor_frame_time_ms(4.25, 1.0 / 60.0), 4.25);
        assert!((editor_frame_time_ms(0.0, 1.0 / 60.0) - 16.666_668).abs() < 0.001);
        assert_eq!(editor_frame_time_ms(f32::NAN, f32::NAN), 0.0);
    }

    #[test]
    fn project_browser_view_and_folder_round_trip_in_workspace_preferences() {
        let mut preferences = EditorWorkspacePreferences {
            project_asset_view: ProjectAssetView::List,
            project_asset_folder: "/materials/environment".to_string(),
            gizmos_visible: false,
            snapping_enabled: true,
            ..EditorWorkspacePreferences::default()
        };
        let json = serde_json::to_vec(&preferences).unwrap();
        let restored: EditorWorkspacePreferences = serde_json::from_slice(&json).unwrap();
        assert_eq!(restored.project_asset_view, ProjectAssetView::List);
        assert_eq!(restored.project_asset_folder, "/materials/environment");
        assert!(!restored.gizmos_visible);
        assert!(restored.snapping_enabled);

        preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(preferences.project_asset_view, ProjectAssetView::Grid);
        assert_eq!(preferences.project_asset_folder, "/");
        assert!(preferences.gizmos_visible);
        assert!(!preferences.snapping_enabled);
    }

    #[test]
    fn gizmo_rendering_requires_visible_editing_scene_viewport() {
        assert!(gizmo_viewport_enabled(true, true, ViewportTab::Scene));
        assert!(!gizmo_viewport_enabled(false, true, ViewportTab::Scene));
        assert!(!gizmo_viewport_enabled(true, false, ViewportTab::Scene));
        assert!(!gizmo_viewport_enabled(true, true, ViewportTab::Game));
    }

    #[test]
    fn script_update_error_stops_play_and_restores_authoring_preview() {
        let authoring = engine_scene::sample_scene();
        let mut play_session = EditorPlaySession::default();
        let mut game_loop = GameLoop::new(EngineConfig::default());
        play_session
            .start(&authoring, |scene| game_loop.load_scene(scene))
            .unwrap();
        let mut runtime_mutation = authoring.clone();
        runtime_mutation.name = "Runtime mutation".into();
        game_loop.load_scene(runtime_mutation).unwrap();

        let diagnostics =
            recover_play_after_script_error(&mut play_session, &mut game_loop, "managed exception");

        assert!(play_session.is_editing());
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "EDPLAY_SCRIPT_UPDATE_FAILED"));
        let restored = game_loop.runtime.scene_ref().unwrap();
        assert_eq!(restored.name, authoring.name);
        assert!(restored
            .scene_settings
            .active_camera
            .as_deref()
            .is_some_and(|camera| camera.starts_with(EDITOR_CAMERA_ID_PREFIX)));
        assert!(restored
            .entities
            .iter()
            .all(|entity| !entity.components.contains_key("engine.script")));
    }

    struct SceneProjectFixture {
        _temp: tempfile::TempDir,
        project: GameProject,
    }

    fn scene_project_fixture() -> SceneProjectFixture {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        std::fs::create_dir_all(root.join("assets/source")).unwrap();
        std::fs::create_dir_all(root.join("assets/cooked")).unwrap();
        std::fs::create_dir_all(root.join("assets/scenes")).unwrap();

        let mut main = engine_scene::sample_scene();
        main.scene_id = "main".into();
        main.name = "Main Authoring Scene".into();
        save_scene_atomically(&main, &root.join("assets/scenes/main.scene.ron")).unwrap();

        let mut level_two = engine_scene::sample_scene();
        level_two.scene_id = "level_two".into();
        level_two.name = "Level Two".into();
        save_scene_atomically(&level_two, &root.join("assets/scenes/level_two.scene.ron")).unwrap();

        let mut invalid = engine_scene::sample_scene();
        invalid.scene_id = "invalid".into();
        invalid.name = "Invalid Runtime Scene".into();
        invalid.entities[0].components.insert(
            "game.unknown".into(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: Default::default(),
            },
        );
        save_scene_atomically(&invalid, &root.join("assets/scenes/invalid.scene.ron")).unwrap();

        let mut manifest = ProjectManifest::new("Editor Scene Documents");
        manifest.startup_scene = PathBuf::from("main");
        manifest.scenes.insert(
            "level_two".into(),
            PathBuf::from("assets/scenes/level_two.scene.ron"),
        );
        manifest.scenes.insert(
            "invalid".into(),
            PathBuf::from("assets/scenes/invalid.scene.ron"),
        );
        manifest.input_actions = None;
        let manifest_path = manifest.write_to_root(&root).unwrap();
        let project = GameProject::load(manifest_path).unwrap();
        SceneProjectFixture {
            _temp: temp,
            project,
        }
    }

    fn editor_app_with_loaded_fixture(project: GameProject) -> EditorApp {
        let mut app = EditorApp::new(project);
        let scene = Scene::load_from_file(&app.current_scene_path).unwrap();
        let mut game_loop = GameLoop::new(EngineConfig::default());
        let (preview, diagnostics) = editor_preview_scene(&game_loop.runtime, &scene);
        assert!(diagnostics.is_empty());
        game_loop.load_scene(preview).unwrap();
        app.game_loop = Some(game_loop);
        app.editor_scene = Some(EditorScene::new(scene));
        app
    }

    fn dispatch_test_request(
        app: &mut EditorApp,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        let request = serde_json::json!({
            "id": format!("test-{method}"),
            "protocol": protocol::EDITOR_PROTOCOL,
            "sessionId": app.session_id.as_str(),
            "baseRevision": app.editor_revision,
            "method": method,
            "params": params,
        });
        let messages = app.dispatch_ipc_json(&request.to_string());
        serde_json::from_str(
            messages
                .json_messages
                .first()
                .expect("dispatch must return a response"),
        )
        .unwrap()
    }

    fn make_scene_dirty(app: &mut EditorApp, name: &str) {
        app.editor_scene
            .as_mut()
            .unwrap()
            .execute(Box::new(engine_editor::SetEntityName::new(
                "cube-01".into(),
                Some(name.into()),
            )))
            .unwrap();
    }

    fn persisted_cube_name(app: &EditorApp) -> Option<String> {
        Scene::load_from_file(&app.current_scene_path)
            .unwrap()
            .entities
            .into_iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .and_then(|entity| entity.name)
    }

    #[derive(Default)]
    struct ResizeBackend;

    impl BackendRenderer for ResizeBackend {
        fn resize(
            &mut self,
            _width: u32,
            _height: u32,
        ) -> Result<(), Vec<engine_renderer::Diagnostic>> {
            Ok(())
        }
    }

    fn send_viewport_bounds(app: &mut EditorApp, viewport: &str, visible: bool, rect: ScreenRect) {
        let request = serde_json::json!({
            "id": "viewport-bounds-test",
            "protocol": protocol::EDITOR_PROTOCOL,
            "sessionId": app.session_id.as_str(),
            "method": "viewport.bounds",
            "params": {
                "viewport": viewport,
                "visible": visible,
                "rect": rect,
            },
        });
        let _ = app.dispatch_ipc_json(&request.to_string());
    }

    fn seed_active_web_input(app: &mut EditorApp) {
        app.web_viewport_input.pointer_id = Some(7);
        app.web_viewport_input.pointer = Some(Vec2::new(20.0, 30.0));
        app.web_viewport_input.buttons = 1;
        app.web_viewport_input.keys.insert("KeyW".to_string());
        app.web_viewport_input.focused = true;
    }

    #[test]
    fn hidden_or_switched_web_viewports_cancel_captured_input() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        app.web_viewport_rect = ScreenRect {
            x: 10.0,
            y: 20.0,
            width: 640.0,
            height: 480.0,
        };
        seed_active_web_input(&mut app);

        send_viewport_bounds(&mut app, "scene", false, ScreenRect::default());
        assert!(app.web_viewport_input.pointer_id.is_none());
        assert_eq!(app.web_viewport_input.buttons, 0);
        assert!(app.web_viewport_input.keys.is_empty());
        assert!(!app.web_viewport_input.focused);
        assert_eq!(app.web_viewport_rect.width, 0.0);

        seed_active_web_input(&mut app);
        send_viewport_bounds(
            &mut app,
            "game",
            true,
            ScreenRect {
                x: 30.0,
                y: 40.0,
                width: 800.0,
                height: 450.0,
            },
        );
        assert_eq!(app.viewport_tab, ViewportTab::Game);
        assert!(app.web_viewport_input.pointer_id.is_none());
        assert_eq!(app.web_viewport_input.buttons, 0);
        assert!(app.web_viewport_input.keys.is_empty());
        assert_eq!(app.web_viewport_rect.x, 30.0);
        assert_eq!(app.web_viewport_rect.width, 800.0);
    }

    #[test]
    fn minimized_or_occluded_surfaces_stop_redraw_and_resume_once_visible() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        app.game_loop
            .as_mut()
            .unwrap()
            .runtime
            .set_renderer_backend(Box::<ResizeBackend>::default());
        seed_active_web_input(&mut app);

        assert_eq!(
            app.handle_native_surface_resize(0, 0, None),
            HostDirective::Continue
        );
        assert!(app.surface_zero_sized);
        assert!(app.web_viewport_input.pointer_id.is_none());
        let frame = app.frame;
        assert_eq!(
            app.on_host_event(HostEvent::Redraw),
            HostDirective::Continue
        );
        assert_eq!(app.frame, frame);

        assert_eq!(
            app.on_host_event(HostEvent::Occluded(true)),
            HostDirective::Continue
        );
        assert_eq!(
            app.handle_native_surface_resize(1280, 720, None),
            HostDirective::Continue
        );
        assert!(!app.surface_zero_sized);
        assert!(app.surface_occluded);
        assert_eq!(
            app.on_host_event(HostEvent::Occluded(false)),
            HostDirective::RequestRedraw
        );
        assert!(!app.surface_render_suspended());
    }

    #[test]
    fn renderer_failure_stops_self_redraw_and_external_retry_can_recover() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        app.web_viewport_rect = ScreenRect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        app.window_w = 640.0;
        app.window_h = 480.0;

        assert_eq!(
            app.on_host_event(HostEvent::Redraw),
            HostDirective::Continue
        );
        assert!(app.render_faulted);
        assert_eq!(
            app.on_host_event(HostEvent::Redraw),
            HostDirective::Continue
        );

        app.game_loop
            .as_mut()
            .unwrap()
            .runtime
            .set_renderer_backend(Box::<crate::qa::QaBackend>::default());
        assert_eq!(
            app.on_host_event(HostEvent::Redraw),
            HostDirective::RequestRedraw
        );
        assert!(!app.render_faulted);
    }

    #[test]
    fn periodic_frame_event_serializes_only_complete_telemetry_domains() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        let messages = app.take_frame_bridge_messages(true);

        assert_eq!(messages.len(), 1);
        let event: serde_json::Value = serde_json::from_str(&messages[0]).unwrap();
        assert_eq!(event["event"], protocol::TELEMETRY_EVENT);
        assert_eq!(event["revision"], app.editor_revision);
        let params = event["params"].as_object().unwrap();
        assert!(params.contains_key("performance"));
        assert!(params.contains_key("animation"));
        assert!(params.contains_key("build"));
        assert_eq!(params.len(), 3);
        assert!(!params.contains_key("hierarchy"));
        assert!(!params.contains_key("projectName"));
    }

    #[test]
    fn completed_background_job_bumps_revision_and_sends_one_full_snapshot() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        let revision = app.editor_revision;
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Err("expected worker failure".to_string()))
            .unwrap();
        app.background_job = Some(EditorBackgroundJob {
            id: 77,
            label: "Async asset operation".to_string(),
            receiver,
            reload_assets: false,
        });

        assert_eq!(app.render_react_frame(), EditorFrameOutcome::Completed);
        assert_eq!(app.editor_revision, revision.wrapping_add(1));
        assert!(app.pending_full_snapshot);

        let messages = app.take_frame_bridge_messages(true);
        let project_events = messages
            .iter()
            .filter_map(|message| serde_json::from_str::<serde_json::Value>(message).ok())
            .filter(|event| event["event"] == protocol::PROJECT_CHANGED_EVENT)
            .collect::<Vec<_>>();
        assert_eq!(project_events.len(), 1);
        assert_eq!(project_events[0]["revision"], app.editor_revision);
        assert!(project_events[0]["params"].get("hierarchy").is_some());
        assert_eq!(
            project_events[0]["params"]["backgroundOperations"][0]["id"],
            77
        );
        assert_eq!(
            project_events[0]["params"]["backgroundOperations"][0]["state"],
            "failed"
        );
        assert!(!app.pending_full_snapshot);
    }

    #[test]
    fn recent_background_operation_statuses_survive_a_newer_job() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        app.set_editor_operation_status(EditorOperationStatus {
            id: 10,
            label: "First".into(),
            state: EditorOperationState::Succeeded,
        });
        app.set_editor_operation_status(EditorOperationStatus {
            id: 11,
            label: "Second".into(),
            state: EditorOperationState::Running,
        });

        let snapshot = serde_json::to_value(app.editor_snapshot()).unwrap();
        let operations = snapshot["backgroundOperations"].as_array().unwrap();
        assert_eq!(operations.len(), 2);
        assert_eq!(operations[0]["id"], 10);
        assert_eq!(operations[0]["state"], "succeeded");
        assert_eq!(operations[1]["id"], 11);
        assert_eq!(operations[1]["state"], "running");
    }

    #[test]
    fn asset_delete_rejects_a_reference_present_only_in_the_unsaved_scene() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        let editor_scene = app.editor_scene.as_mut().unwrap();
        let renderable = editor_scene
            .scene
            .entities
            .iter_mut()
            .find_map(|entity| entity.components.get_mut("engine.renderable"))
            .expect("sample scene contains a renderable");
        renderable.fields.insert(
            "material".into(),
            Value::Asset(AssetId::new("dirty-only-material")),
        );
        editor_scene.history.mark_dirty();

        let response = dispatch_test_request(
            &mut app,
            "assets.delete",
            serde_json::json!({ "assetId": "dirty-only-material" }),
        );

        assert_eq!(response["error"]["code"], "conflict");
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("open authoring scene"));
        assert!(app.background_job.is_none());
    }

    #[test]
    fn active_asset_job_freezes_authoring_mutations_until_dependency_checks_finish() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        let entity_count = app.editor_scene.as_ref().unwrap().scene.entities.len();
        let (_sender, receiver) = mpsc::channel::<Result<EditorJobOutput, String>>();
        app.background_job = Some(EditorBackgroundJob {
            id: 42,
            label: "Delete asset".into(),
            receiver,
            reload_assets: true,
        });

        let response = dispatch_test_request(
            &mut app,
            "scene.createEntity",
            serde_json::json!({ "templateId": "empty" }),
        );

        assert_eq!(response["error"]["code"], "conflict");
        assert_eq!(
            app.editor_scene.as_ref().unwrap().scene.entities.len(),
            entity_count
        );
    }

    #[test]
    fn script_component_can_only_be_created_from_the_loaded_verified_class_list() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        app.project.script_assembly = Some(PathBuf::from("build/scripts/GameScripts.dll"));
        let game_loop = app.game_loop.as_mut().unwrap();
        game_loop.runtime.register_script_host(Box::new(
            engine_script::MockHost::new()
                .with_verified_classes("GameScripts", ["GameScripts.PlayerController"]),
        ));
        game_loop
            .runtime
            .load_script_assembly("GameScripts", "mock", b"managed")
            .unwrap();
        app.editor_scene.as_mut().unwrap().selected_entity = Some("cube-01".into());

        assert!(app
            .verified_script_add_command("GameScripts", "GameScripts.Guessed")
            .err()
            .unwrap()
            .contains("not in the reflection-verified class list"));
        let command = app
            .verified_script_add_command("GameScripts", "GameScripts.PlayerController")
            .expect("verified class must produce an undoable command");
        assert!(app.execute_editor_command(command));

        let component = &app
            .editor_scene
            .as_ref()
            .unwrap()
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap()
            .components["engine.script"];
        assert_eq!(
            component.fields.get("assembly_id"),
            Some(&Value::Str("GameScripts".into()))
        );
        assert_eq!(
            component.fields.get("class_name"),
            Some(&Value::Str("GameScripts.PlayerController".into()))
        );
        assert!(app
            .verified_script_add_command("GameScripts", "GameScripts.PlayerController")
            .err()
            .unwrap()
            .contains("already has an engine.script component"));
    }

    #[test]
    fn dirty_scene_recovery_snapshot_is_detected_and_restored_as_unsaved() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project.clone());
        app.editor_scene
            .as_mut()
            .unwrap()
            .execute(Box::new(engine_editor::SetEntityName::new(
                "cube-01".into(),
                Some("Recovered Cube".into()),
            )))
            .unwrap();
        app.last_recovery_snapshot = Instant::now() - std::time::Duration::from_secs(31);
        app.maybe_write_recovery_snapshot();
        let recovery = scene_recovery_path(&fixture.project, "main");
        assert!(recovery.is_file());

        let mut reopened = editor_app_with_loaded_fixture(fixture.project.clone());
        assert_eq!(
            reopened.pending_recovery.as_deref(),
            Some(recovery.as_path())
        );
        reopened.restore_recovery_snapshot().unwrap();
        let scene = reopened.editor_scene.as_ref().unwrap();
        assert!(scene.is_dirty());
        assert!(!scene.history.can_undo());
        assert_eq!(
            scene
                .scene
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "cube-01")
                .and_then(|entity| entity.name.as_deref()),
            Some("Recovered Cube")
        );
    }

    #[test]
    fn dirty_scene_switch_requires_an_explicit_resolution() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project.clone());
        app.editor_scene
            .as_mut()
            .unwrap()
            .execute(Box::new(engine_editor::SetEntityName::new(
                "cube-01".into(),
                Some("Unsaved Cube".into()),
            )))
            .unwrap();

        assert!(!app.request_scene_switch("level_two".into()).unwrap());
        assert_eq!(app.current_scene_id, "main");
        assert_eq!(
            app.pending_scene_switch.as_deref(),
            Some("opening scene 'level_two'")
        );
        assert_eq!(
            app.pending_document_action,
            Some(SceneDocumentAction::Open("level_two".into()))
        );

        app.apply_scene_document_action(SceneDocumentAction::CancelSwitch)
            .unwrap();
        assert_eq!(app.current_scene_id, "main");
        assert!(app.pending_scene_switch.is_none());
        assert!(app.editor_scene.as_ref().unwrap().is_dirty());
    }

    #[test]
    fn dirty_document_mutations_do_not_touch_files_before_confirmation() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        make_scene_dirty(&mut app, "Unsaved Cube");

        for (method, params, expected) in [
            (
                "document.create",
                serde_json::json!({ "sceneId": "created", "folder": "levels" }),
                SceneDocumentAction::Create {
                    scene_id: "created".into(),
                    folder: PathBuf::from("levels"),
                },
            ),
            (
                "document.duplicate",
                serde_json::json!({ "sourceId": "level_two", "newId": "duplicated" }),
                SceneDocumentAction::Duplicate {
                    source_id: "level_two".into(),
                    new_id: "duplicated".into(),
                },
            ),
            (
                "document.rename",
                serde_json::json!({ "oldId": "main", "newId": "renamed" }),
                SceneDocumentAction::Rename {
                    old_id: "main".into(),
                    new_id: "renamed".into(),
                },
            ),
            (
                "document.delete",
                serde_json::json!({
                    "sceneId": "main",
                    "replacementStartup": "level_two"
                }),
                SceneDocumentAction::Delete {
                    scene_id: "main".into(),
                    replacement_startup: Some("level_two".into()),
                },
            ),
        ] {
            let response = dispatch_test_request(&mut app, method, params);
            assert!(response.get("error").is_none(), "{response}");
            assert_eq!(app.current_scene_id, "main");
            assert_eq!(app.pending_document_action, Some(expected));
            assert_eq!(persisted_cube_name(&app).as_deref(), Some("Cube"));
            assert!(app.project.scene_path("main").is_some());
            assert!(app.project.scene_path("created").is_none());
            assert!(app.project.scene_path("duplicated").is_none());
            assert!(app.project.scene_path("renamed").is_none());

            let response = dispatch_test_request(
                &mut app,
                "document.resolvePendingSwitch",
                serde_json::json!({ "decision": "cancel" }),
            );
            assert!(response.get("error").is_none(), "{response}");
            assert!(app.pending_scene_switch.is_none());
            assert!(app.pending_document_action.is_none());
            assert!(app.editor_scene.as_ref().unwrap().is_dirty());
        }
    }

    #[test]
    fn failed_discarded_switch_keeps_dirty_state_and_exact_pending_target() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        make_scene_dirty(&mut app, "Unsaved Cube");

        let response = dispatch_test_request(
            &mut app,
            "document.open",
            serde_json::json!({ "sceneId": "invalid" }),
        );
        assert!(response.get("error").is_none(), "{response}");

        let response = dispatch_test_request(
            &mut app,
            "document.resolvePendingSwitch",
            serde_json::json!({ "decision": "discard" }),
        );
        assert_eq!(
            response["error"]["code"],
            serde_json::Value::String("validationFailed".into())
        );
        assert_eq!(app.current_scene_id, "main");
        assert!(app.editor_scene.as_ref().unwrap().is_dirty());
        assert_eq!(persisted_cube_name(&app).as_deref(), Some("Cube"));
        assert_eq!(
            app.pending_document_action,
            Some(SceneDocumentAction::Open("invalid".into()))
        );
        assert_eq!(
            app.pending_scene_switch.as_deref(),
            Some("opening scene 'invalid'")
        );
    }

    #[test]
    fn a_second_document_request_cannot_overwrite_the_pending_target() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        make_scene_dirty(&mut app, "Unsaved Cube");
        dispatch_test_request(
            &mut app,
            "document.open",
            serde_json::json!({ "sceneId": "level_two" }),
        );

        let response = dispatch_test_request(
            &mut app,
            "document.create",
            serde_json::json!({ "sceneId": "wrong-target", "folder": "" }),
        );

        assert_eq!(response["error"]["code"], "conflict");
        assert_eq!(
            app.pending_document_action,
            Some(SceneDocumentAction::Open("level_two".into()))
        );
        assert!(app.project.scene_path("wrong-target").is_none());
    }

    #[test]
    fn discard_resolves_each_deferred_document_action_without_losing_its_target() {
        for (method, params, expected_scene) in [
            (
                "document.open",
                serde_json::json!({ "sceneId": "level_two" }),
                "level_two",
            ),
            (
                "document.create",
                serde_json::json!({ "sceneId": "created", "folder": "levels" }),
                "created",
            ),
            (
                "document.duplicate",
                serde_json::json!({ "sourceId": "level_two", "newId": "duplicated" }),
                "duplicated",
            ),
            (
                "document.rename",
                serde_json::json!({ "oldId": "main", "newId": "renamed" }),
                "renamed",
            ),
            (
                "document.delete",
                serde_json::json!({
                    "sceneId": "main",
                    "replacementStartup": "level_two"
                }),
                "level_two",
            ),
        ] {
            let fixture = scene_project_fixture();
            let mut app = editor_app_with_loaded_fixture(fixture.project);
            make_scene_dirty(&mut app, "Must Be Discarded");

            let response = dispatch_test_request(&mut app, method, params);
            assert!(response.get("error").is_none(), "{method}: {response}");
            assert_eq!(app.current_scene_id, "main");
            assert!(app.pending_document_action.is_some());

            let response = dispatch_test_request(
                &mut app,
                "document.resolvePendingSwitch",
                serde_json::json!({ "decision": "discard" }),
            );
            assert!(response.get("error").is_none(), "{method}: {response}");
            assert_eq!(app.current_scene_id, expected_scene, "{method}");
            assert!(app.pending_scene_switch.is_none(), "{method}");
            assert!(app.pending_document_action.is_none(), "{method}");
            assert!(!app.editor_scene.as_ref().unwrap().is_dirty(), "{method}");
            assert!(app
                .editor_scene
                .as_ref()
                .unwrap()
                .scene
                .entities
                .iter()
                .all(|entity| entity.name.as_deref() != Some("Must Be Discarded")));
        }
    }

    #[test]
    fn resolving_dirty_switch_with_save_persists_then_opens_exact_target() {
        let fixture = scene_project_fixture();
        let main_path = fixture.project.scene_path("main").unwrap();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        make_scene_dirty(&mut app, "Saved Cube");

        dispatch_test_request(
            &mut app,
            "document.open",
            serde_json::json!({ "sceneId": "level_two" }),
        );
        let response = dispatch_test_request(
            &mut app,
            "document.resolvePendingSwitch",
            serde_json::json!({ "decision": "save" }),
        );

        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(app.current_scene_id, "level_two");
        assert!(app.pending_document_action.is_none());
        assert!(!app.editor_scene.as_ref().unwrap().is_dirty());
        let saved = Scene::load_from_file(&main_path).unwrap();
        assert_eq!(
            saved
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "cube-01")
                .and_then(|entity| entity.name.as_deref()),
            Some("Saved Cube")
        );
    }

    #[test]
    fn play_mode_cannot_bypass_or_resolve_a_pending_document_prompt() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        make_scene_dirty(&mut app, "Unsaved Cube");
        dispatch_test_request(
            &mut app,
            "document.open",
            serde_json::json!({ "sceneId": "level_two" }),
        );

        let response = dispatch_test_request(
            &mut app,
            "runtime.setMode",
            serde_json::json!({ "mode": "play" }),
        );
        assert_eq!(response["error"]["code"], "conflict");
        assert!(app.play_session.is_editing());
        assert!(app.pending_document_action.is_some());

        // Simulate an already queued native Play transition racing the web
        // prompt. Resolution must still refuse authoring writes in Play mode.
        app.start_play();
        assert!(!app.play_session.is_editing());
        let response = dispatch_test_request(
            &mut app,
            "document.resolvePendingSwitch",
            serde_json::json!({ "decision": "save" }),
        );
        assert_eq!(response["error"]["code"], "editingRequired");
        assert_eq!(persisted_cube_name(&app).as_deref(), Some("Cube"));
        assert!(app.pending_document_action.is_some());
    }

    #[test]
    fn save_and_close_persists_the_open_document_before_exit() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        app.editor_scene
            .as_mut()
            .unwrap()
            .execute(Box::new(engine_editor::SetEntityName::new(
                "cube-01".into(),
                Some("Saved Before Close".into()),
            )))
            .unwrap();
        app.close_confirmation_pending = true;

        app.apply_close_document_action(CloseDocumentAction::SaveAndClose)
            .unwrap();

        assert!(app.exit_after_frame);
        assert!(!app.close_confirmation_pending);
        assert!(!app.editor_scene.as_ref().unwrap().is_dirty());
        let saved = Scene::load_from_file(&app.current_scene_path).unwrap();
        assert_eq!(
            saved
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "cube-01")
                .and_then(|entity| entity.name.as_deref()),
            Some("Saved Before Close")
        );
    }

    #[test]
    fn cancel_or_discard_close_never_implicitly_saves() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        app.editor_scene
            .as_mut()
            .unwrap()
            .execute(Box::new(engine_editor::SetEntityName::new(
                "cube-01".into(),
                Some("Unsaved Before Close".into()),
            )))
            .unwrap();
        app.close_confirmation_pending = true;

        app.apply_close_document_action(CloseDocumentAction::Cancel)
            .unwrap();
        assert!(!app.exit_after_frame);
        assert!(!app.close_confirmation_pending);
        assert!(app.editor_scene.as_ref().unwrap().is_dirty());

        app.close_confirmation_pending = true;
        app.apply_close_document_action(CloseDocumentAction::DiscardAndClose)
            .unwrap();
        assert!(app.exit_after_frame);
        let saved = Scene::load_from_file(&app.current_scene_path).unwrap();
        assert_eq!(
            saved
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "cube-01")
                .and_then(|entity| entity.name.as_deref()),
            Some("Cube")
        );
    }

    #[test]
    fn scene_document_switch_replaces_document_and_resets_editor_state() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project.clone());
        app.editor_scene.as_mut().unwrap().selected_entity = Some("cube-01".into());
        app.material_editor_selection = Some("mat-default".into());
        app.gizmo.dragging = true;

        assert!(app.switch_scene_document("level_two").unwrap());

        assert_eq!(app.current_scene_id, "level_two");
        assert_eq!(
            app.current_scene_path,
            fixture.project.scene_path("level_two").unwrap()
        );
        let editor_scene = app.editor_scene.as_ref().unwrap();
        assert_eq!(editor_scene.scene.scene_id, "level_two");
        assert!(!editor_scene.is_dirty());
        assert!(editor_scene.selected_entity.is_none());
        assert!(!app.gizmo.dragging);
        assert!(app.material_editor_selection.is_none());
        let preview = app.game_loop.as_ref().unwrap().runtime.scene_ref().unwrap();
        assert!(preview
            .entities
            .iter()
            .any(|entity| entity.persistent_id.starts_with(EDITOR_CAMERA_ID_PREFIX)));
        assert!(preview
            .entities
            .iter()
            .all(|entity| !entity.components.contains_key("engine.script")));
    }

    #[test]
    fn failed_scene_document_switch_preserves_document_and_runtime_preview() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        let before_document = app.editor_scene.as_ref().unwrap().scene.clone();
        let before_preview = app
            .game_loop
            .as_ref()
            .unwrap()
            .runtime
            .scene_ref()
            .unwrap()
            .clone();

        let error = app.switch_scene_document("invalid").unwrap_err();

        assert!(error.contains("game.unknown"), "{error}");
        assert_eq!(app.current_scene_id, "main");
        assert_eq!(app.editor_scene.as_ref().unwrap().scene, before_document);
        assert_eq!(
            app.game_loop.as_ref().unwrap().runtime.scene_ref().unwrap(),
            &before_preview
        );
    }

    #[test]
    fn editor_play_tracks_the_open_catalog_scene_and_stop_restores_its_preview() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        app.switch_scene_document("level_two").unwrap();

        app.start_play();
        assert_eq!(app.play_session.mode(), EditorPlayMode::Playing);
        assert_eq!(app.play_runtime_scene_id.as_deref(), Some("level_two"));
        assert_eq!(
            app.game_loop
                .as_ref()
                .unwrap()
                .runtime
                .scene_ref()
                .map(|scene| scene.scene_id.as_str()),
            Some("level_two")
        );

        app.stop_play();
        assert!(app.play_session.is_editing());
        assert!(app.play_runtime_scene_id.is_none());
        let preview = app.game_loop.as_ref().unwrap().runtime.scene_ref().unwrap();
        assert_eq!(preview.scene_id, "level_two");
        assert!(preview
            .entities
            .iter()
            .any(|entity| entity.persistent_id.starts_with(EDITOR_CAMERA_ID_PREFIX)));
    }

    #[test]
    fn loaded_prefab_instantiation_and_unpack_use_editor_history() {
        let fixture = scene_project_fixture();
        let mut app = editor_app_with_loaded_fixture(fixture.project);
        let asset_id = AssetId::new("prefab-cube-test");
        let prefab = engine_editor::prefab_from_scene_subtree(
            &app.editor_scene.as_ref().unwrap().scene,
            &"cube-01".into(),
            asset_id.clone(),
        )
        .unwrap();
        app.game_loop
            .as_mut()
            .unwrap()
            .runtime
            .asset_registry_mut()
            .insert_typed(asset_id.clone(), prefab);
        write_json(
            &app.project.asset_source.join("prefabs.manifest"),
            &SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets: vec![SourceAssetEntry {
                    id: asset_id.clone(),
                    asset_type: AssetType::Prefab,
                    source_path: "prefabs/cube.prefab.ron".to_string(),
                    cook_rules: engine_asset::cook::CookRules::default(),
                }],
            },
        );
        app.refresh_asset_catalog().unwrap();
        assert!(app.asset_browser.select_asset(Some(asset_id.clone())));

        app.instantiate_prefab_asset(asset_id, None).unwrap();

        let root = app
            .editor_scene
            .as_ref()
            .unwrap()
            .selected_entity
            .clone()
            .expect("instantiated root is selected");
        assert_ne!(root, "cube-01");
        assert!(app
            .editor_scene
            .as_ref()
            .unwrap()
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == root)
            .unwrap()
            .components
            .contains_key("engine.prefab_instance_ref"));

        app.unpack_prefab_instance(root.clone(), PrefabUnpackMode::Instance)
            .unwrap();
        assert!(!app
            .editor_scene
            .as_ref()
            .unwrap()
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == root)
            .unwrap()
            .components
            .contains_key("engine.prefab_instance_ref"));
        app.editor_scene.as_mut().unwrap().undo().unwrap();
        assert!(app
            .editor_scene
            .as_ref()
            .unwrap()
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == root)
            .unwrap()
            .components
            .contains_key("engine.prefab_instance_ref"));
    }

    struct MaterialProjectFixture {
        _temp: tempfile::TempDir,
        project: GameProject,
        source_path: PathBuf,
        manifest_entry: SourceAssetEntry,
    }

    fn test_material_source(roughness: f32, base_color: [f32; 4]) -> MaterialSource {
        MaterialSource {
            schema: MATERIAL_SOURCE_SCHEMA.to_string(),
            base_color,
            metallic: 0.2,
            roughness,
            ambient_occlusion: 0.9,
            emissive: [0.0; 3],
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            transparency: "Opaque".to_string(),
            alpha_cutoff: 0.5,
            double_sided: false,
        }
    }

    fn write_json(path: &Path, value: &impl serde::Serialize) {
        let mut bytes = serde_json::to_vec_pretty(value).unwrap();
        bytes.push(b'\n');
        std::fs::write(path, bytes).unwrap();
    }

    fn material_project_fixture() -> MaterialProjectFixture {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let source_root = root.join("assets/source");
        let material_dir = source_root.join("materials");
        let cooked_assets = root.join("assets/cooked");
        std::fs::create_dir_all(&material_dir).unwrap();
        std::fs::create_dir_all(&cooked_assets).unwrap();

        let source_path = material_dir.join("project.material.json");
        write_json(
            &source_path,
            &test_material_source(0.7, [0.2, 0.3, 0.4, 1.0]),
        );
        let manifest_entry = SourceAssetEntry {
            id: AssetId::new("mat-project"),
            asset_type: AssetType::Material,
            source_path: "materials/project.material.json".to_string(),
            cook_rules: engine_asset::cook::CookRules::default(),
        };
        write_json(
            &source_root.join("assets.manifest"),
            &SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets: vec![manifest_entry.clone()],
            },
        );

        let manifest = ProjectManifest::new("Material Save Test");
        let project = GameProject {
            manifest,
            manifest_path: root.join("game.project.json"),
            root: root.clone(),
            startup_scene: root.join("assets/scenes/main.scene.ron"),
            asset_source: std::fs::canonicalize(&source_root).unwrap(),
            cooked_assets,
            script_project: None,
            script_assembly: None,
            input_actions: None,
        };
        MaterialProjectFixture {
            _temp: temp,
            project,
            source_path,
            manifest_entry,
        }
    }

    fn cook_fixture(fixture: &MaterialProjectFixture) {
        let mut graph = DependencyGraph::new();
        let runtime_builder = EngineRuntime::builder(EngineConfig::default());
        let report = cook_orchestrate_checked_with_registry(
            &fixture.project.asset_source,
            &fixture.project.cooked_assets,
            &mut graph,
            runtime_builder.asset_type_registry(),
        );
        assert!(report.is_success(), "{:?}", report.diagnostics);
    }

    #[test]
    fn missing_render_reference_uses_preview_fallback_without_mutating_authoring_scene() {
        let runtime = engine_core::EngineRuntime::new(EngineConfig::default());
        let mut authoring = engine_scene::sample_scene();
        let renderable = authoring
            .entities
            .iter_mut()
            .find_map(|entity| entity.components.get_mut("engine.renderable"))
            .unwrap();
        renderable.fields.insert(
            "material".into(),
            Value::Asset(AssetId::new("missing-material")),
        );

        let (preview, diagnostics) = editor_preview_scene(&runtime, &authoring);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "EDASSET_MISSING");
        let authoring_material = authoring.entities.iter().find_map(|entity| {
            entity
                .components
                .get("engine.renderable")
                .and_then(|component| component.fields.get("material"))
        });
        let preview_material = preview.entities.iter().find_map(|entity| {
            entity
                .components
                .get("engine.renderable")
                .and_then(|component| component.fields.get("material"))
        });
        assert_eq!(
            authoring_material,
            Some(&Value::Asset(AssetId::new("missing-material")))
        );
        assert_eq!(
            preview_material,
            Some(&Value::Asset(AssetId::new("mat-default")))
        );
    }

    #[test]
    fn editor_preview_never_instantiates_authoring_game_scripts() {
        let runtime = engine_core::EngineRuntime::new(EngineConfig::default());
        let mut authoring = engine_scene::sample_scene();
        let scripted = authoring
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap();
        scripted.components.insert(
            "engine.script".into(),
            engine_scene::ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: std::collections::BTreeMap::new(),
            },
        );

        let (preview, _) = editor_preview_scene(&runtime, &authoring);
        assert!(authoring
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap()
            .components
            .contains_key("engine.script"));
        assert!(!preview
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap()
            .components
            .contains_key("engine.script"));
    }

    #[test]
    fn editor_preview_uses_dedicated_camera_without_mutating_game_camera() {
        let runtime = engine_core::EngineRuntime::new(EngineConfig::default());
        let authoring = engine_scene::sample_scene();
        let (preview, diagnostics) = editor_preview_scene(&runtime, &authoring);
        assert!(diagnostics.is_empty());
        assert_eq!(
            authoring.scene_settings.active_camera.as_deref(),
            Some("camera-main")
        );
        assert!(authoring.entities[0].components["engine.camera"].enabled);

        let editor_camera_id = preview.scene_settings.active_camera.as_deref().unwrap();
        assert!(editor_camera_id.starts_with(EDITOR_CAMERA_ID_PREFIX));
        assert_ne!(editor_camera_id, "camera-main");
        assert!(
            !preview
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "camera-main")
                .unwrap()
                .components["engine.camera"]
                .enabled
        );
        let editor_camera = preview
            .entities
            .iter()
            .find(|entity| entity.persistent_id == editor_camera_id)
            .unwrap();
        assert!(editor_camera.components["engine.camera"].enabled);
        assert!(editor_camera.components.contains_key("engine.transform"));
    }

    #[test]
    fn game_preview_uses_authoring_camera_without_instantiating_scripts() {
        let runtime = engine_core::EngineRuntime::new(EngineConfig::default());
        let mut authoring = engine_scene::sample_scene();
        authoring.entities[1].components.insert(
            "engine.script".into(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: Default::default(),
            },
        );

        let (preview, diagnostics) = game_preview_scene(&runtime, &authoring);

        assert!(diagnostics.is_empty());
        assert_eq!(
            preview.scene_settings.active_camera.as_deref(),
            Some("camera-main")
        );
        assert!(
            preview
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "camera-main")
                .unwrap()
                .components["engine.camera"]
                .enabled
        );
        assert!(!preview
            .entities
            .iter()
            .any(|entity| entity.persistent_id.starts_with(EDITOR_CAMERA_ID_PREFIX)));
        assert!(preview
            .entities
            .iter()
            .all(|entity| !entity.components.contains_key("engine.script")));
    }

    #[test]
    fn failed_play_load_rolls_back_to_script_free_editor_preview() {
        let mut authoring = engine_scene::sample_scene();
        authoring.entities[1].components.insert(
            "engine.script".into(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: Default::default(),
            },
        );
        let mut game_loop = GameLoop::new(EngineConfig::default());
        let mut play_session = EditorPlaySession::default();
        let failure = Diagnostic::new(
            "TEST_PLAY_FAILURE",
            DiagnosticSeverity::Error,
            "test",
            "OnCreate failed",
        );

        let result = play_session.start(&authoring, |scene| {
            game_loop.load_scene(scene).unwrap();
            Err::<(), _>(vec![failure])
        });
        assert!(result.is_err());
        assert!(play_session.is_editing());
        assert!(game_loop
            .runtime
            .scene_ref()
            .unwrap()
            .entities
            .iter()
            .any(|entity| entity.components.contains_key("engine.script")));

        restore_editor_preview(&mut game_loop, &authoring).unwrap();
        let restored = game_loop.runtime.scene_ref().unwrap();
        assert!(restored
            .scene_settings
            .active_camera
            .as_deref()
            .unwrap()
            .starts_with(EDITOR_CAMERA_ID_PREFIX));
        assert!(restored
            .entities
            .iter()
            .all(|entity| !entity.components.contains_key("engine.script")));
    }

    #[test]
    fn browser_assignment_uses_editor_history_for_undo_and_redo() {
        let mut runtime = engine_core::EngineRuntime::new(EngineConfig::default());
        let builtin_id = AssetId::new("mesh-cube");
        let mut alternate_mesh: MeshUpload = runtime
            .asset_registry()
            .get::<MeshUpload>(&builtin_id)
            .expect("builtin mesh should exist")
            .get()
            .clone();
        let alternate_id = AssetId::with_path("mesh-alternate", "models/alternate.mesh");
        alternate_mesh.mesh_id = alternate_id.clone();
        alternate_mesh.content_hash = [42; 32];
        runtime.register_mesh_asset(alternate_mesh);

        let mut browser = ProjectAssetBrowserPanel::new();
        let source_root = tempfile::tempdir().unwrap();
        refresh_project_asset_list(&mut browser, runtime.asset_registry(), source_root.path())
            .unwrap();
        assert!(browser.select_asset(Some(alternate_id.clone())));

        let mut editor_scene = EditorScene::new(engine_scene::sample_scene());
        editor_scene.selected_entity = Some("cube-01".to_string());
        assert!(execute_selected_asset_assignment(&browser, &mut editor_scene).unwrap());
        assert!(editor_scene.is_dirty());

        let mesh_value = |scene: &Scene| {
            scene
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "cube-01")
                .and_then(|entity| entity.components.get("engine.renderable"))
                .and_then(|component| component.fields.get("mesh"))
                .cloned()
        };
        assert_eq!(
            mesh_value(&editor_scene.scene),
            Some(Value::Asset(alternate_id.clone()))
        );

        editor_scene.undo().unwrap();
        assert_eq!(
            mesh_value(&editor_scene.scene),
            Some(Value::Asset(builtin_id))
        );

        editor_scene.redo().unwrap();
        assert_eq!(
            mesh_value(&editor_scene.scene),
            Some(Value::Asset(alternate_id))
        );
    }

    #[test]
    fn material_save_updates_source_cooked_payload_and_runtime_registry() {
        let fixture = material_project_fixture();
        cook_fixture(&fixture);
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        super::super::project_app::load_project_assets(&mut runtime, &fixture.project).unwrap();
        assert_eq!(
            runtime
                .asset_registry()
                .get::<MaterialUpload>(&AssetId::new("mat-project"))
                .unwrap()
                .get()
                .roughness,
            0.7
        );

        let request = MaterialSaveRequest {
            material_asset: "mat-project".to_string(),
            source: test_material_source(0.31, [0.8, 0.6, 0.4, 1.0]),
        };
        let outcome = save_project_material(&mut runtime, &fixture.project, &request).unwrap();

        let saved_source: MaterialSource =
            serde_json::from_slice(&std::fs::read(&fixture.source_path).unwrap()).unwrap();
        assert_eq!(saved_source.roughness, 0.31);
        assert_eq!(saved_source.base_color, [0.8, 0.6, 0.4, 1.0]);

        let artifact = read_cooked_artifact(&outcome.cooked_path).unwrap();
        let cooked = decode_cooked_material(&artifact).unwrap();
        assert_eq!(cooked.roughness, 0.31);
        assert_eq!(cooked.base_color, [0.8, 0.6, 0.4, 1.0]);

        let registered = runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("mat-project"))
            .unwrap();
        assert_eq!(registered.get().roughness, 0.31);
        assert_eq!(registered.get().base_color, [0.8, 0.6, 0.4, 1.0]);
        assert_eq!(outcome.source_path, fixture.source_path);
    }

    #[test]
    fn material_save_rejects_builtin_unknown_and_ambiguous_ids() {
        let fixture = material_project_fixture();
        let original_source = std::fs::read(&fixture.source_path).unwrap();
        let mut runtime = EngineRuntime::new(EngineConfig::default());

        let builtin_error = save_project_material(
            &mut runtime,
            &fixture.project,
            &MaterialSaveRequest {
                material_asset: BUILTIN_DEFAULT_MATERIAL_ID.to_string(),
                source: test_material_source(0.2, [1.0; 4]),
            },
        )
        .unwrap_err();
        assert!(builtin_error.contains("Built-in"));

        let unknown_error = save_project_material(
            &mut runtime,
            &fixture.project,
            &MaterialSaveRequest {
                material_asset: "mat-unknown".to_string(),
                source: test_material_source(0.2, [1.0; 4]),
            },
        )
        .unwrap_err();
        assert!(unknown_error.contains("not declared"));

        write_json(
            &fixture.project.asset_source.join("duplicate.manifest"),
            &SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets: vec![fixture.manifest_entry.clone()],
            },
        );
        let ambiguous_error = save_project_material(
            &mut runtime,
            &fixture.project,
            &MaterialSaveRequest {
                material_asset: "mat-project".to_string(),
                source: test_material_source(0.2, [1.0; 4]),
            },
        )
        .unwrap_err();
        assert!(ambiguous_error.contains("ambiguous"));
        assert_eq!(
            std::fs::read(&fixture.source_path).unwrap(),
            original_source
        );
    }

    #[test]
    fn failed_material_cook_restores_original_source_and_cooked_asset() {
        let fixture = material_project_fixture();
        cook_fixture(&fixture);
        let source_before = std::fs::read(&fixture.source_path).unwrap();
        let cooked_path = fixture.project.cooked_assets.join("mat-project.cooked");
        let cooked_before = std::fs::read(&cooked_path).unwrap();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        super::super::project_app::load_project_assets(&mut runtime, &fixture.project).unwrap();

        let mut invalid_source = test_material_source(0.1, [0.9, 0.1, 0.2, 1.0]);
        invalid_source.base_color_texture = Some("../unsafe-texture".to_string());
        let error = save_project_material(
            &mut runtime,
            &fixture.project,
            &MaterialSaveRequest {
                material_asset: "mat-project".to_string(),
                source: invalid_source,
            },
        )
        .unwrap_err();

        assert!(error.contains("original material source was restored"));
        assert_eq!(std::fs::read(&fixture.source_path).unwrap(), source_before);
        assert_eq!(std::fs::read(&cooked_path).unwrap(), cooked_before);
        assert_eq!(
            runtime
                .asset_registry()
                .get::<MaterialUpload>(&AssetId::new("mat-project"))
                .unwrap()
                .get()
                .roughness,
            0.7
        );
    }

    fn gizmo_test_transform(translation: [f32; 3]) -> engine_scene::ComponentRecord {
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([
                ("translation".into(), Value::Vec3(translation)),
                ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
                ("scale".into(), Value::Vec3([1.0, 1.0, 1.0])),
            ]),
        }
    }

    fn gizmo_test_scene_and_runtime() -> (EditorScene, EngineRuntime) {
        let mut scene = engine_scene::sample_scene();
        scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "camera-main")
            .unwrap()
            .components
            .insert(
                "engine.transform".into(),
                gizmo_test_transform([0.0, 0.0, 0.0]),
            );
        scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap()
            .components
            .insert(
                "engine.transform".into(),
                gizmo_test_transform([0.0, 0.0, -5.0]),
            );

        let mut editor_scene = EditorScene::new(scene.clone());
        editor_scene.selected_entity = Some("cube-01".into());
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        runtime.load_scene(scene).unwrap();
        (editor_scene, runtime)
    }

    fn project_gizmo_test_point(view: RuntimeGizmoView, world: Vec3) -> Vec2 {
        let clip = view.projection * view.view * world.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        view.viewport_origin
            + Vec2::new(
                (ndc.x * 0.5 + 0.5) * view.viewport_size.x,
                (1.0 - (ndc.y * 0.5 + 0.5)) * view.viewport_size.y,
            )
    }

    fn project_gizmo_test_axis_interior(view: RuntimeGizmoView, axis: Vec3) -> Vec2 {
        let center = project_gizmo_test_point(view, view.world_position);
        let unit_tip = project_gizmo_test_point(view, view.world_position + axis);
        // Production gizmos keep an 88 px screen-space length independent of
        // camera depth. Pick well inside that visible segment instead of
        // assuming one world unit is the rendered handle length.
        center + (unit_tip - center).normalize() * 44.0
    }

    fn full_test_viewport(width: u32, height: u32) -> RenderViewportContext {
        RenderViewportContext::new(width, height, RendererRect::FULL).unwrap()
    }

    #[test]
    fn scene_view_picking_selects_visible_runtime_entity() {
        let (_, runtime) = gizmo_test_scene_and_runtime();
        let window_size = Vec2::splat(800.0);
        let viewport = full_test_viewport(800, 800);
        let pointer = runtime
            .with_world(|world| {
                let input =
                    extract_renderer_input_from_world_with_viewport(world, 0, viewport).unwrap();
                let view = &input.views[0];
                let drawable = input
                    .drawables
                    .iter()
                    .find(|drawable| drawable.entity.as_deref() == Some("cube-01"))
                    .unwrap();
                let center = (Vec3::from_array(drawable.bounds.min)
                    + Vec3::from_array(drawable.bounds.max))
                    * 0.5;
                project_world_point(
                    center,
                    Mat4::from_cols_array(&view.view_matrix),
                    Mat4::from_cols_array(&view.projection_matrix),
                    window_size,
                )
                .unwrap()
                .0
            })
            .unwrap();

        assert_eq!(
            pick_runtime_entity(&runtime, 0, viewport, Vec2::ZERO, window_size, pointer,)
                .as_deref(),
            Some("cube-01")
        );
        assert_eq!(
            pick_runtime_entity(
                &runtime,
                0,
                viewport,
                Vec2::new(200.0, 200.0),
                Vec2::new(600.0, 600.0),
                Vec2::new(100.0, 100.0),
            ),
            None
        );
    }

    #[test]
    fn embedded_editor_viewport_drives_render_projection_picking_and_gizmo_geometry() {
        let (_, runtime) = gizmo_test_scene_and_runtime();
        let surface = Vec2::new(1000.0, 800.0);
        let viewport_rect = ScreenRect {
            x: 200.0,
            y: 100.0,
            width: 500.0,
            height: 500.0,
        };
        let (_, _, viewport) = editor_render_viewport(viewport_rect, 1.0, surface).unwrap();
        assert_eq!(
            viewport.output_rect(),
            RendererRect {
                min: [0.2, 0.125],
                max: [0.7, 0.75],
            }
        );

        let gizmo_view = runtime_gizmo_view(&runtime, "cube-01", 0, viewport).unwrap();
        assert_eq!(gizmo_view.viewport_origin, Vec2::new(200.0, 100.0));
        assert_eq!(gizmo_view.viewport_size, Vec2::splat(500.0));
        let pointer = project_gizmo_test_point(gizmo_view, gizmo_view.world_position);
        assert_eq!(
            pick_runtime_entity(
                &runtime,
                0,
                viewport,
                Vec2::new(200.0, 100.0),
                Vec2::new(700.0, 600.0),
                pointer,
            )
            .as_deref(),
            Some("cube-01")
        );
        assert_eq!(
            pick_runtime_entity(
                &runtime,
                0,
                viewport,
                Vec2::new(200.0, 100.0),
                Vec2::new(700.0, 600.0),
                Vec2::new(100.0, 400.0),
            ),
            None
        );
    }

    #[test]
    fn queued_gizmo_drag_undo_save_reload_roundtrip() {
        let (mut editor_scene, runtime) = gizmo_test_scene_and_runtime();
        let viewport = full_test_viewport(800, 800);
        let view = runtime_gizmo_view(&runtime, "cube-01", 0, viewport).unwrap();
        let center = project_gizmo_test_point(view, view.world_position);
        let x_tip = project_gizmo_test_point(view, view.world_position + Vec3::X);
        let press = project_gizmo_test_axis_interior(view, Vec3::X);
        let moved = press + (x_tip - center).normalize() * 60.0;
        let mut gizmo = GizmoSystem::new();
        let processed = process_gizmo_pointer_events(
            vec![
                GizmoPointerEvent::Press(press),
                GizmoPointerEvent::Move(moved),
                GizmoPointerEvent::Release(moved),
            ],
            &mut editor_scene,
            &mut gizmo,
            &runtime,
            "cube-01",
            view,
        );
        assert!(
            processed,
            "dragging={} axis={:?} active={} transform={:?}",
            gizmo.dragging,
            gizmo.drag_axis,
            editor_scene.is_transform_gizmo_drag_active(),
            editor_scene.selected_transform_for_gizmo()
        );
        let changed = editor_scene.selected_transform_for_gizmo().unwrap();
        assert!(changed.translation.x > 0.1);
        let runtime_x = runtime
            .with_world(|world| {
                let entity = world.entity_by_persistent_id("cube-01").unwrap();
                world.get::<Transform>(entity).unwrap().translation.x
            })
            .unwrap();
        assert!((runtime_x - changed.translation.x).abs() < 1.0e-5);

        let overlay_view = runtime_gizmo_view(&runtime, "cube-01", 1, viewport).unwrap();
        let overlay = offset_gizmo_batch(
            build_gizmo_ui_batch(
                &gizmo,
                overlay_view.world_position,
                overlay_view.world_rotation,
                overlay_view.view,
                overlay_view.projection,
                overlay_view.viewport_size,
            )
            .unwrap(),
            overlay_view,
        );
        assert_eq!(overlay.clip_rect.min, [0.0, 0.0]);
        assert_eq!(overlay.clip_rect.max, [800.0, 800.0]);

        let temp = tempfile::tempdir().unwrap();
        let saved = temp.path().join("gizmo.scene.ron");
        save_scene_atomically(&editor_scene.scene, &saved).unwrap();
        let reloaded = Scene::load_from_file(&saved).unwrap();
        let reloaded_x = match reloaded
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap()
            .components["engine.transform"]
            .fields["translation"]
        {
            Value::Vec3(value) => value[0],
            ref other => panic!("unexpected translation: {other:?}"),
        };
        assert!((reloaded_x - changed.translation.x).abs() < 1.0e-5);

        editor_scene.undo().unwrap();
        assert_eq!(
            editor_scene
                .selected_transform_for_gizmo()
                .unwrap()
                .translation,
            Vec3::new(0.0, 0.0, -5.0)
        );
        assert!(!editor_scene.history.can_undo());
        assert!(editor_scene.history.can_redo());
    }

    #[test]
    fn gizmo_release_position_applies_final_drag_segment() {
        let (mut editor_scene, runtime) = gizmo_test_scene_and_runtime();
        let view =
            runtime_gizmo_view(&runtime, "cube-01", 0, full_test_viewport(800, 800)).unwrap();
        let center = project_gizmo_test_point(view, view.world_position);
        let x_tip = project_gizmo_test_point(view, view.world_position + Vec3::X);
        let press = project_gizmo_test_axis_interior(view, Vec3::X);
        let released = press + (x_tip - center).normalize() * 60.0;
        let mut gizmo = GizmoSystem::new();
        assert!(process_gizmo_pointer_events(
            vec![
                GizmoPointerEvent::Press(press),
                GizmoPointerEvent::Release(released),
            ],
            &mut editor_scene,
            &mut gizmo,
            &runtime,
            "cube-01",
            view,
        ));
        assert!(
            editor_scene
                .selected_transform_for_gizmo()
                .unwrap()
                .translation
                .x
                > 0.1
        );
        assert!(editor_scene.history.can_undo());
        assert!(!gizmo.dragging);
    }

    #[test]
    fn gizmo_overlay_and_hit_testing_share_the_scene_interaction_rect() {
        let (mut editor_scene, runtime) = gizmo_test_scene_and_runtime();
        let full_view =
            runtime_gizmo_view(&runtime, "cube-01", 0, full_test_viewport(800, 800)).unwrap();
        let visible_view = restrict_gizmo_view_to_rect(
            full_view,
            Vec2::new(250.0, 200.0),
            Vec2::new(650.0, 600.0),
        )
        .unwrap();
        let gizmo = GizmoSystem::new();
        let overlay = offset_gizmo_batch(
            build_gizmo_ui_batch(
                &gizmo,
                visible_view.world_position,
                visible_view.world_rotation,
                visible_view.view,
                visible_view.projection,
                visible_view.viewport_size,
            )
            .unwrap(),
            visible_view,
        );
        assert_eq!(overlay.clip_rect.min, [250.0, 200.0]);
        assert_eq!(overlay.clip_rect.max, [650.0, 600.0]);

        let excluded_view =
            restrict_gizmo_view_to_rect(full_view, Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0))
                .unwrap();
        let center = project_gizmo_test_point(full_view, full_view.world_position);
        let x_tip = project_gizmo_test_point(full_view, full_view.world_position + Vec3::X);
        let press = center.lerp(x_tip, 0.55);
        let mut gizmo = GizmoSystem::new();
        assert!(!process_gizmo_pointer_events(
            vec![GizmoPointerEvent::Press(press)],
            &mut editor_scene,
            &mut gizmo,
            &runtime,
            "cube-01",
            excluded_view,
        ));
        assert!(!gizmo.dragging);
        assert!(!editor_scene.is_transform_gizmo_drag_active());
    }

    #[test]
    fn queued_gizmo_cancel_restores_preview_without_history() {
        let (mut editor_scene, mut runtime) = gizmo_test_scene_and_runtime();
        let view =
            runtime_gizmo_view(&runtime, "cube-01", 0, full_test_viewport(800, 800)).unwrap();
        let center = project_gizmo_test_point(view, view.world_position);
        let x_tip = project_gizmo_test_point(view, view.world_position + Vec3::X);
        let press = project_gizmo_test_axis_interior(view, Vec3::X);
        let moved = press + (x_tip - center).normalize() * 50.0;
        let mut gizmo = GizmoSystem::new();
        assert!(process_gizmo_pointer_events(
            vec![
                GizmoPointerEvent::Press(press),
                GizmoPointerEvent::Move(moved),
                GizmoPointerEvent::Cancel,
            ],
            &mut editor_scene,
            &mut gizmo,
            &runtime,
            "cube-01",
            view,
        ));
        assert_eq!(
            editor_scene
                .selected_transform_for_gizmo()
                .unwrap()
                .translation,
            Vec3::new(0.0, 0.0, -5.0)
        );
        assert!(!editor_scene.history.can_undo());
        assert!(!gizmo.dragging);
        runtime.load_scene(editor_scene.scene.clone()).unwrap();
        let runtime_translation = runtime
            .with_world(|world| {
                let entity = world.entity_by_persistent_id("cube-01").unwrap();
                world.get::<Transform>(entity).unwrap().translation
            })
            .unwrap();
        assert_eq!(runtime_translation, Vec3::new(0.0, 0.0, -5.0));
    }

    #[test]
    fn scene_view_controls_drive_runtime_editor_camera_only() {
        let (editor_scene, _) = gizmo_test_scene_and_runtime();
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        let (preview, diagnostics) = editor_preview_scene(&runtime, &editor_scene.scene);
        assert!(diagnostics.is_empty());
        runtime.load_scene(preview).unwrap();
        let authored_camera = editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "camera-main")
            .unwrap()
            .components["engine.transform"]
            .fields["translation"]
            .clone();
        let mut panel = SceneViewPanel::new("Scene View");
        panel.set_target([0.0, 0.0, 0.0]);
        panel.set_camera_orbit(0.0, 0.0, 7.0);

        assert!(apply_editor_camera(&runtime, &panel));
        let (editor_camera, runtime_authored_camera) = runtime
            .with_world(|world| {
                let editor_id = world.scene_settings().active_camera.as_deref().unwrap();
                let editor_entity = world.entity_by_persistent_id(editor_id).unwrap();
                let authored_entity = world.entity_by_persistent_id("camera-main").unwrap();
                (
                    world.get::<Transform>(editor_entity).unwrap().clone(),
                    world.get::<Transform>(authored_entity).unwrap().clone(),
                )
            })
            .unwrap();
        assert!((editor_camera.translation - Vec3::new(7.0, 0.0, 0.0)).length() < 1.0e-5);
        assert!((editor_camera.rotation * -Vec3::Z - Vec3::NEG_X).length() < 1.0e-5);
        assert_eq!(runtime_authored_camera.translation, Vec3::ZERO);
        assert_eq!(
            editor_scene
                .scene
                .entities
                .iter()
                .find(|entity| entity.persistent_id == "camera-main")
                .unwrap()
                .components["engine.transform"]
                .fields["translation"],
            authored_camera
        );

        let view =
            runtime_gizmo_view(&runtime, "camera-main", 0, full_test_viewport(800, 800)).unwrap();
        assert!(build_gizmo_ui_batch(
            &GizmoSystem::new(),
            view.world_position,
            view.world_rotation,
            view.view,
            view.projection,
            view.viewport_size,
        )
        .is_some());
    }

    #[test]
    fn scene_view_camera_state_survives_ordered_preview_synchronization() {
        let (mut editor_scene, _) = gizmo_test_scene_and_runtime();
        let mut game_loop = GameLoop::new(EngineConfig::default());
        let mut panel = SceneViewPanel::new("Scene View");
        panel.apply_action(engine_editor::SceneViewAction::SetDistance(25.0));

        synchronize_editor_preview_and_camera(&mut game_loop, &mut editor_scene, &panel);

        let editor_camera_translation = game_loop
            .runtime
            .with_world(|world| {
                let camera_id = world.scene_settings().active_camera.as_deref().unwrap();
                let camera = world.entity_by_persistent_id(camera_id).unwrap();
                world.get::<Transform>(camera).unwrap().translation
            })
            .unwrap();
        let pitch = 20.0_f32.to_radians();
        let yaw = 45.0_f32.to_radians();
        let expected = Vec3::new(
            25.0 * yaw.cos() * pitch.cos(),
            25.0 * pitch.sin(),
            25.0 * yaw.sin() * pitch.cos(),
        );
        assert!((editor_camera_translation - expected).length() < 1.0e-5);
    }
}
