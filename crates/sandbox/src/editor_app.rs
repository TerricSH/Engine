use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
use engine_asset::cook::{
    cook_orchestrate_checked_with_registry, decode_cooked_material, read_cooked_artifact,
    AssetType, DependencyGraph, SourceAssetEntry, SourceManifest,
};
use engine_asset::project::GameProject;
use engine_core::game_loop::GameLoop;
use engine_core::{create_vulkan_backend_renderer, EngineConfig, EngineRuntime};
use engine_editor::asset_browser::{
    draw_asset_browser, refresh_asset_list, AssetBrowserPanel as ProjectAssetBrowserPanel,
};
use engine_editor::gizmo::{update_gizmo, GizmoMode, GizmoSpace, GizmoSystem};
use engine_editor::gizmo_overlay::build_gizmo_ui_batch;
use engine_editor::hierarchy::SequencedSelection;
use engine_editor::material_editor::{
    draw_material_editor, load_material, render_material_preview_rgba8, MaterialEditorPanel,
    MaterialPreviewRequest, MaterialSaveAccess, MaterialSaveRequest,
};
use engine_editor::{
    EditorPlayMode, EditorPlaySession, EditorScene, EditorUi, HierarchyPanel, InspectorContext,
    InspectorPanel, SceneViewPanel, SequencedCommand, SequencedSceneViewAction, UiInteractionPhase,
    UiKey,
};
use engine_renderer::{
    AssetId, ColorSpace, SamplerAddressMode, SamplerDescriptor, TextureMipLevel, TextureUpload,
    TextureUploadFormat, UiBatch,
};
use engine_scene::components::{Camera, Transform};
use engine_scene::{
    extract_renderer_input_from_world, ComponentRecord, Entity, EntityRecord, Scene, World,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity, PersistentId, SchemaVersion, Value};
use glam::{Mat4, Quat, Vec2, Vec3};
use platform::winit::window::Window;
use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use sha2::{Digest, Sha256};

const MATERIAL_PREVIEW_TEXTURE_ID: &str = "editor/material-preview";
const MATERIAL_PREVIEW_SIZE: u32 = 256;
const BUILTIN_DEFAULT_MATERIAL_ID: &str = "mat-default";
const EDITOR_CAMERA_ID_PREFIX: &str = "__engine_editor_camera";

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

fn material_preview_upload(request: &MaterialPreviewRequest) -> TextureUpload {
    let pixels =
        render_material_preview_rgba8(request, MATERIAL_PREVIEW_SIZE, MATERIAL_PREVIEW_SIZE);
    let content_hash: [u8; 32] = Sha256::digest(&pixels).into();
    TextureUpload {
        texture_id: AssetId::new(MATERIAL_PREVIEW_TEXTURE_ID),
        width: MATERIAL_PREVIEW_SIZE,
        height: MATERIAL_PREVIEW_SIZE,
        format: TextureUploadFormat::Rgba8,
        color_space: ColorSpace::Srgb,
        mip_levels: vec![TextureMipLevel {
            width: MATERIAL_PREVIEW_SIZE,
            height: MATERIAL_PREVIEW_SIZE,
            bytes: pixels,
        }],
        sampler: SamplerDescriptor {
            address_u: SamplerAddressMode::ClampToEdge,
            address_v: SamplerAddressMode::ClampToEdge,
            address_w: SamplerAddressMode::ClampToEdge,
            ..SamplerDescriptor::default()
        },
        content_hash,
    }
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

fn map_editor_key(key: platform::KeyCode) -> Option<UiKey> {
    match key {
        platform::KeyCode::Enter => Some(UiKey::Enter),
        platform::KeyCode::Escape => Some(UiKey::Escape),
        platform::KeyCode::Backspace => Some(UiKey::Backspace),
        platform::KeyCode::Delete => Some(UiKey::Delete),
        platform::KeyCode::Tab => Some(UiKey::Tab),
        _ => None,
    }
}

fn should_route_event_to_gameplay(ui: &EditorUi, event: &PlatformEvent) -> bool {
    match event {
        PlatformEvent::MousePressed { x, y, .. } => {
            !ui.pointer_over_widget_at(*x as f32, *y as f32)
        }
        // Releases must always reach gameplay so a press that began in the
        // scene cannot remain held after the pointer crosses editor chrome.
        PlatformEvent::MouseReleased { .. } | PlatformEvent::KeyReleased { .. } => true,
        PlatformEvent::KeyPressed { .. } => !ui.captures_keyboard_input(),
        PlatformEvent::CharacterTyped { .. } => false,
        _ => true,
    }
}

/// Route retained scene-Canvas input while the editor is in Play/Pause.
///
/// This is intentionally separate from the project's action map: gameplay
/// button releases must reach the action map to unstick a held action, while
/// a Canvas release over editor chrome must cancel capture instead of clicking
/// the scene UI underneath it.
#[cfg(feature = "runtime-subsystems")]
fn route_editor_play_ui_event(
    game_loop: &mut GameLoop,
    ui: &EditorUi,
    event: &PlatformEvent,
) -> bool {
    match event {
        PlatformEvent::MouseMoved { x, y } if !ui.pointer_over_widget_at(*x as f32, *y as f32) => {
            game_loop.ui_pointer_move(*x as f32, *y as f32);
            true
        }
        PlatformEvent::MousePressed { button, x, y } if *button == platform::MouseButton::Left => {
            if ui.pointer_over_widget_at(*x as f32, *y as f32) {
                game_loop.cancel_ui_pointer();
                false
            } else {
                game_loop.ui_pointer_move(*x as f32, *y as f32);
                game_loop.ui_pointer_left_press();
                true
            }
        }
        PlatformEvent::MouseReleased { button, x, y } if *button == platform::MouseButton::Left => {
            if ui.pointer_over_widget_at(*x as f32, *y as f32) {
                game_loop.cancel_ui_pointer();
                false
            } else {
                game_loop.ui_pointer_move(*x as f32, *y as f32);
                game_loop.ui_pointer_left_release();
                true
            }
        }
        PlatformEvent::Focused(false) | PlatformEvent::Suspended => {
            game_loop.cancel_ui_pointer();
            true
        }
        _ => false,
    }
}

fn should_apply_history_undo(
    requested: bool,
    had_incomplete_gizmo: bool,
    had_uncommitted_text_change: bool,
    history_push_serial_before_panels: u64,
    history_push_serial_after_panels: u64,
    can_undo: bool,
) -> bool {
    requested
        && !had_incomplete_gizmo
        && can_undo
        && (!had_uncommitted_text_change
            || history_push_serial_after_panels != history_push_serial_before_panels)
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
            ("engine.camera".into(), record(Default::default())),
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

fn editor_preview_scene(
    runtime: &engine_core::EngineRuntime,
    authoring_scene: &Scene,
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
        if let Some(camera) = entity.components.get_mut("engine.camera") {
            // Authoring cameras remain as selectable entities (including
            // their Transform), but only the dedicated editor camera renders
            // while outside Play.
            camera.enabled = false;
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
    let editor_camera = editor_camera_entity(&preview);
    preview.scene_settings.active_camera = Some(editor_camera.persistent_id.clone());
    preview.entities.push(editor_camera);
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

fn edit_current_inspector_selection(
    inspector: &mut InspectorPanel,
    ui: &mut EditorUi,
    editor_scene: &mut EditorScene,
    context: &InspectorContext,
) -> Vec<SequencedCommand> {
    inspector.ui_with_context_ordered(
        ui,
        &editor_scene.scene,
        editor_scene.selected_entity.as_ref(),
        context,
    )
}

fn project_inspector_context(project: &GameProject) -> InspectorContext {
    project
        .script_assembly
        .as_deref()
        .and_then(Path::file_stem)
        .and_then(|stem| stem.to_str())
        .filter(|assembly| !assembly.is_empty() && !assembly.chars().any(char::is_whitespace))
        .map(InspectorContext::with_script_assembly)
        .unwrap_or_default()
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct SequencedGizmoPointerEvent {
    sequence: u64,
    event: GizmoPointerEvent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum OrderedToolbarAction {
    Save,
    Undo,
    Redo,
    StartPlay,
    SetGizmoMode(GizmoMode),
    ToggleGizmoSpace,
    ToggleGizmoSnapping,
}

enum OrderedAuthoringInput {
    Gizmo(SequencedGizmoPointerEvent),
    Toolbar {
        sequence: u64,
        action: OrderedToolbarAction,
    },
    PanelCommand(SequencedCommand),
    Selection(SequencedSelection),
    SceneView(SequencedSceneViewAction),
}

impl OrderedAuthoringInput {
    fn sequence(&self) -> u64 {
        match self {
            Self::Gizmo(event) => event.sequence,
            Self::Toolbar { sequence, .. } => *sequence,
            Self::PanelCommand(command) => command.stamp.sequence,
            Self::Selection(selection) => selection.stamp.sequence,
            Self::SceneView(action) => action.stamp.sequence,
        }
    }

    fn tie_breaker(&self) -> u8 {
        match self {
            Self::PanelCommand(command)
                if command.stamp.phase == UiInteractionPhase::BeforeRawPointer =>
            {
                0
            }
            Self::Gizmo(_) => 1,
            Self::Toolbar { .. }
            | Self::PanelCommand(_)
            | Self::Selection(_)
            | Self::SceneView(_) => 2,
        }
    }
}

fn merge_ordered_authoring_inputs(
    mut toolbar: Vec<OrderedAuthoringInput>,
    gizmo: Vec<SequencedGizmoPointerEvent>,
) -> Vec<OrderedAuthoringInput> {
    toolbar.extend(gizmo.into_iter().map(OrderedAuthoringInput::Gizmo));
    toolbar.sort_by_key(|input| (input.sequence(), input.tie_breaker()));
    toolbar
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
    window_size: Vec2,
) -> Option<RuntimeGizmoView> {
    runtime
        .with_world(|world| {
            let input = extract_renderer_input_from_world(world, frame).ok()?;
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
    ui: &EditorUi,
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
                if !pointer_inside_view || ui.pointer_over_widget_at(pointer.x, pointer.y) {
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
    project: GameProject,
    current_scene_id: String,
    current_scene_path: PathBuf,
    scene_browser_selection: String,
    new_scene_id: String,
    pending_scene_switch: Option<String>,
    scene_document_status: Option<String>,
    close_confirmation_pending: bool,
    exit_after_frame: bool,
    play_runtime_scene_id: Option<String>,
    game_loop: Option<GameLoop>,
    editor_scene: Option<EditorScene>,
    play_session: EditorPlaySession,
    #[cfg(feature = "target-desktop")]
    input_state: super::project_input::ProjectInputState,
    hierarchy: HierarchyPanel,
    scene_view: SceneViewPanel,
    gizmo: GizmoSystem,
    gizmo_pointer_events: Vec<SequencedGizmoPointerEvent>,
    next_platform_event_sequence: u64,
    cancel_gizmo_requested: bool,
    inspector: InspectorPanel,
    asset_browser: ProjectAssetBrowserPanel,
    material_editor: MaterialEditorPanel,
    material_editor_selection: Option<String>,
    ui: EditorUi,
    frame: u64,
    mouse_x: f64,
    mouse_y: f64,
    window_w: f32,
    window_h: f32,
    last_frame_time: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SceneDocumentAction {
    Open(String),
    Create(String),
    SetStartup(String),
    SaveAndSwitch(String),
    DiscardAndSwitch(String),
    CancelSwitch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseDocumentAction {
    SaveAndClose,
    DiscardAndClose,
    Cancel,
}

impl EditorApp {
    pub fn new(project: GameProject) -> Self {
        let current_scene_id = project.startup_scene_id().to_string();
        let current_scene_path = project.startup_scene_path().to_path_buf();
        Self {
            project,
            scene_browser_selection: current_scene_id.clone(),
            current_scene_id,
            current_scene_path,
            new_scene_id: String::new(),
            pending_scene_switch: None,
            scene_document_status: None,
            close_confirmation_pending: false,
            exit_after_frame: false,
            play_runtime_scene_id: None,
            game_loop: None,
            editor_scene: None,
            play_session: EditorPlaySession::default(),
            #[cfg(feature = "target-desktop")]
            input_state: super::project_input::ProjectInputState::default(),
            hierarchy: HierarchyPanel::new("Hierarchy"),
            scene_view: SceneViewPanel::new("Scene View"),
            gizmo: GizmoSystem::new(),
            gizmo_pointer_events: Vec::new(),
            next_platform_event_sequence: 0,
            cancel_gizmo_requested: false,
            inspector: InspectorPanel::new("Inspector"),
            asset_browser: ProjectAssetBrowserPanel::new(),
            material_editor: MaterialEditorPanel::new(),
            material_editor_selection: None,
            ui: EditorUi::new(),
            frame: 0,
            mouse_x: 0.0,
            mouse_y: 0.0,
            window_w: 1600.0,
            window_h: 900.0,
            last_frame_time: Instant::now(),
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
        tracing::info!(
            scene_id = self.current_scene_id,
            scene = %self.current_scene_path.display(),
            "editor scene saved"
        );
        self.scene_document_status = Some(format!("Saved '{}'.", self.current_scene_id));
        Ok(())
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

        let mut editor_scene = EditorScene::new(scene);
        editor_scene.diagnostics.push_many(preview_diagnostics);
        self.ui.cancel_text_edit();
        self.editor_scene = Some(editor_scene);
        self.current_scene_id = scene_id.to_string();
        self.current_scene_path = scene_path;
        self.scene_browser_selection = scene_id.to_string();
        self.pending_scene_switch = None;
        self.hierarchy.set_selected(None);
        self.inspector = InspectorPanel::new("Inspector");
        self.gizmo.cancel_drag();
        self.gizmo_pointer_events.clear();
        self.cancel_gizmo_requested = false;
        self.material_editor.reset();
        self.material_editor_selection = None;
        self.last_frame_time = Instant::now();
        self.scene_document_status = Some(format!("Opened scene '{scene_id}'."));
        tracing::info!(scene_id, scene = %self.current_scene_path.display(), "editor scene opened");
        Ok(true)
    }

    fn request_scene_switch(&mut self, scene_id: String) -> Result<bool, String> {
        if scene_id == self.current_scene_id {
            self.pending_scene_switch = None;
            self.scene_document_status = Some(format!("Scene '{scene_id}' is already open."));
            return Ok(false);
        }
        if self
            .editor_scene
            .as_ref()
            .is_some_and(EditorScene::is_dirty)
        {
            self.pending_scene_switch = Some(scene_id.clone());
            self.scene_document_status = Some(format!(
                "Unsaved changes: choose Save & Switch, Discard & Switch, or Cancel for '{scene_id}'."
            ));
            return Ok(false);
        }
        self.switch_scene_document(&scene_id)
    }

    fn apply_scene_document_action(&mut self, action: SceneDocumentAction) -> Result<bool, String> {
        match action {
            SceneDocumentAction::Open(scene_id) => self.request_scene_switch(scene_id),
            SceneDocumentAction::Create(scene_id) => {
                let scene_id = scene_id.trim();
                if scene_id.is_empty() {
                    return Err("New scene ID must not be empty.".to_string());
                }
                super::project_cli::create_project_scene(
                    &self.project.manifest_path,
                    scene_id,
                    None,
                )?;
                self.reload_project_manifest()?;
                self.scene_browser_selection = scene_id.to_string();
                self.new_scene_id.clear();
                self.scene_document_status = Some(format!("Created scene '{scene_id}'."));
                self.request_scene_switch(scene_id.to_string())
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
            SceneDocumentAction::SaveAndSwitch(scene_id) => {
                self.save_current_scene_document()?;
                self.switch_scene_document(&scene_id)
            }
            SceneDocumentAction::DiscardAndSwitch(scene_id) => {
                self.switch_scene_document(&scene_id)
            }
            SceneDocumentAction::CancelSwitch => {
                self.pending_scene_switch = None;
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
                self.close_confirmation_pending = false;
                self.exit_after_frame = true;
            }
            CloseDocumentAction::DiscardAndClose => {
                self.ui.cancel_text_edit();
                self.pending_scene_switch = None;
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

    fn draw_scene_document_panel(
        &mut self,
        editing: bool,
        dirty: bool,
    ) -> (
        Option<SceneDocumentAction>,
        Option<CloseDocumentAction>,
        f32,
    ) {
        let scene_ids = self
            .project
            .scenes()
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        if !scene_ids
            .iter()
            .any(|id| id == &self.scene_browser_selection)
        {
            self.scene_browser_selection = self.current_scene_id.clone();
        }
        let mut selected_index = scene_ids
            .iter()
            .position(|id| id == &self.scene_browser_selection)
            .unwrap_or(0);
        let can_manage =
            editing && self.pending_scene_switch.is_none() && !self.close_confirmation_pending;
        let document_label = if dirty {
            format!("{} *", self.current_scene_id)
        } else {
            self.current_scene_id.clone()
        };

        self.ui.set_panel_rect(4.0, 38.0, 220.0);
        self.ui.label_value("Open Scene", &document_label);
        self.ui
            .label_value("Catalog Selection", &self.scene_browser_selection);
        if self
            .ui
            .button_enabled("Previous Scene", can_manage && scene_ids.len() > 1)
        {
            selected_index = selected_index.checked_sub(1).unwrap_or(scene_ids.len() - 1);
            self.scene_browser_selection = scene_ids[selected_index].clone();
        }
        if self
            .ui
            .button_enabled("Next Scene", can_manage && scene_ids.len() > 1)
        {
            selected_index = (selected_index + 1) % scene_ids.len();
            self.scene_browser_selection = scene_ids[selected_index].clone();
        }

        let mut action = self
            .ui
            .button_enabled("Open Selected", can_manage)
            .then(|| SceneDocumentAction::Open(self.scene_browser_selection.clone()));
        self.ui
            .label_value("Startup Scene", self.project.startup_scene_id());
        if action.is_none() && self.ui.button_enabled("Set As Startup", can_manage) {
            action = Some(SceneDocumentAction::SetStartup(
                self.scene_browser_selection.clone(),
            ));
        }
        if let Some(committed) = self.ui.text_field("New Scene ID", &self.new_scene_id) {
            self.new_scene_id = committed;
        }
        if action.is_none() && self.ui.button_enabled("Create Scene", can_manage) {
            action = Some(SceneDocumentAction::Create(self.new_scene_id.clone()));
        }
        self.ui.label_value(
            "Scene Status",
            self.scene_document_status.as_deref().unwrap_or("Ready"),
        );

        let mut hierarchy_top = if let Some(target) = self.pending_scene_switch.clone() {
            self.ui.label_value("Pending Switch", &target);
            if self
                .ui
                .button_enabled("Save & Switch", editing && !self.close_confirmation_pending)
            {
                action = Some(SceneDocumentAction::SaveAndSwitch(target.clone()));
            }
            if self.ui.button_enabled(
                "Discard & Switch",
                editing && !self.close_confirmation_pending,
            ) {
                action = Some(SceneDocumentAction::DiscardAndSwitch(target));
            }
            if self.ui.button_enabled(
                "Cancel Scene Switch",
                editing && !self.close_confirmation_pending,
            ) {
                action = Some(SceneDocumentAction::CancelSwitch);
            }
            438.0
        } else {
            326.0
        };
        let mut close_action = None;
        if self.close_confirmation_pending {
            // This is deliberately modal with respect to scene-document
            // actions. A queued Open click must never change which document a
            // later Save & Close writes.
            action = None;
            self.ui
                .label_value("Close Editor", "This scene has unsaved changes.");
            if self.ui.button_enabled("Save & Close", editing) {
                close_action = Some(CloseDocumentAction::SaveAndClose);
            }
            if self.ui.button("Discard & Close") {
                close_action = Some(CloseDocumentAction::DiscardAndClose);
            }
            if self.ui.button("Cancel Close") {
                close_action = Some(CloseDocumentAction::Cancel);
            }
            hierarchy_top += 122.0;
        }
        (action, close_action, hierarchy_top)
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
            refresh_asset_list(&mut self.asset_browser, game_loop.runtime.asset_registry());
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
        let mut editor_scene = EditorScene::new(scene);
        editor_scene.diagnostics.push_many(editor_diagnostics);
        self.editor_scene = Some(editor_scene);
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
                self.ui.cancel_text_edit();
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

    fn stop_play(&mut self) {
        // Play/Pause UI state is not authoring data. Clear any focused editor
        // field before restoring the authoring snapshot so a Stop click cannot
        // commit a Play-time buffer into it.
        self.ui.cancel_text_edit();
        let Some(game_loop) = self.game_loop.as_mut() else {
            return;
        };
        match self.play_session.stop(|scene| {
            let (preview_scene, _) = editor_preview_scene(&game_loop.runtime, &scene);
            game_loop.load_scene(preview_scene)
        }) {
            Ok(true) => {
                self.play_runtime_scene_id = None;
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
    }

    fn render_editor_frame(&mut self) {
        if self.editor_scene.is_none() || self.game_loop.is_none() {
            return;
        }

        // ── 1. Begin UI frame ──────────────────────────────────────
        self.ui
            .set_pointer(self.mouse_x as f32, self.mouse_y as f32);
        self.ui.begin_frame();

        // ── 2. Layout and render panels ────────────────────────────
        let gap = 4.0;
        let content_top = 38.0;
        let left_w = 220.0;
        let right_w = 280.0;
        let center_w = (self.window_w - left_w - right_w - gap * 4.0).max(100.0);
        let center_left = left_w + gap * 2.0;
        let scene_controls_w = center_w.min(380.0);
        let scene_controls_h = 240.0;
        let asset_browser_left = center_left;
        let asset_browser_top = (self.window_h - 260.0).max(content_top + scene_controls_h + gap);
        let asset_browser_w = center_w;
        let inspector_left = left_w + center_w + gap * 3.0;
        let scene_interaction_min = Vec2::new(center_left, content_top + scene_controls_h + gap);
        let scene_interaction_max = Vec2::new(
            center_left + center_w,
            (asset_browser_top - gap).max(scene_interaction_min.y),
        );

        // The renderer currently draws the scene to the full surface, but
        // editor chrome must still reserve its complete regions from viewport
        // tools. The remaining center area is the interaction viewport;
        // concrete widgets declared below take precedence over these blockers.
        self.ui
            .block_pointer_rect(0.0, 0.0, self.window_w, content_top);
        self.ui.block_pointer_rect(
            0.0,
            content_top,
            center_left,
            (self.window_h - content_top).max(1.0),
        );
        self.ui.block_pointer_rect(
            inspector_left,
            content_top,
            (self.window_w - inspector_left).max(1.0),
            (self.window_h - content_top).max(1.0),
        );
        self.ui
            .block_pointer_rect(center_left, content_top, center_w, scene_controls_h + gap);
        self.ui.block_pointer_rect(
            asset_browser_left,
            asset_browser_top,
            asset_browser_w,
            (self.window_h - asset_browser_top).max(1.0),
        );

        // ── Hierarchy (left) ───────────────────────────────────────
        let mode_before_toolbar = self.play_session.mode();
        let had_uncommitted_text_change = self.ui.has_uncommitted_text_change();
        let dirty = self
            .editor_scene
            .as_ref()
            .is_some_and(EditorScene::is_dirty);

        self.ui.set_panel_rect(gap, 4.0, 124.0);
        let save_click_sequences = self
            .ui
            .ordered_button_clicks("Save Scene", mode_before_toolbar == EditorPlayMode::Editing);
        self.ui.set_panel_rect(132.0, 4.0, 124.0);
        let play_click_sequences = match mode_before_toolbar {
            EditorPlayMode::Editing => self.ui.ordered_button_clicks("Play", true),
            EditorPlayMode::Paused => self.ui.ordered_button_clicks("Resume", true),
            EditorPlayMode::Playing => {
                self.ui.label_value("State", "Playing");
                Vec::new()
            }
        };
        let play_clicked = !play_click_sequences.is_empty();
        self.ui.set_panel_rect(260.0, 4.0, 124.0);
        let pause_clicked = if mode_before_toolbar == EditorPlayMode::Playing {
            self.ui.button("Pause")
        } else {
            self.ui.label_value("Pause", "Unavailable");
            false
        };
        self.ui.set_panel_rect(388.0, 4.0, 124.0);
        let stop_clicked = if mode_before_toolbar != EditorPlayMode::Editing {
            self.ui.button("Stop")
        } else {
            self.ui.label_value("Stop", "Unavailable");
            false
        };
        let can_undo = mode_before_toolbar == EditorPlayMode::Editing
            && (had_uncommitted_text_change
                || self
                    .editor_scene
                    .as_ref()
                    .is_some_and(|scene| scene.history.can_undo()));
        self.ui.set_panel_rect(516.0, 4.0, 124.0);
        let undo_click_sequences = self.ui.ordered_button_clicks("Undo", can_undo);
        let can_redo = mode_before_toolbar == EditorPlayMode::Editing
            && !had_uncommitted_text_change
            && self
                .editor_scene
                .as_ref()
                .is_some_and(|scene| scene.history.can_redo());
        self.ui.set_panel_rect(644.0, 4.0, 124.0);
        let redo_click_sequences = self.ui.ordered_button_clicks("Redo", can_redo);

        let toolbar_editing = mode_before_toolbar == EditorPlayMode::Editing;
        self.ui.set_panel_rect(772.0, 4.0, 124.0);
        let translate_click_sequences = if toolbar_editing {
            self.ui.ordered_button_clicks(
                if self.gizmo.mode == GizmoMode::Translate {
                    "[Move]"
                } else {
                    "Move"
                },
                true,
            )
        } else {
            self.ui.label_value("Move", "Unavailable");
            Vec::new()
        };
        self.ui.set_panel_rect(900.0, 4.0, 124.0);
        let rotate_click_sequences = if toolbar_editing {
            self.ui.ordered_button_clicks(
                if self.gizmo.mode == GizmoMode::Rotate {
                    "[Rotate]"
                } else {
                    "Rotate"
                },
                true,
            )
        } else {
            self.ui.label_value("Rotate", "Unavailable");
            Vec::new()
        };
        self.ui.set_panel_rect(1028.0, 4.0, 124.0);
        let scale_click_sequences = if toolbar_editing {
            self.ui.ordered_button_clicks(
                if self.gizmo.mode == GizmoMode::Scale {
                    "[Scale]"
                } else {
                    "Scale"
                },
                true,
            )
        } else {
            self.ui.label_value("Scale", "Unavailable");
            Vec::new()
        };
        self.ui.set_panel_rect(1156.0, 4.0, 124.0);
        let space_click_sequences = if toolbar_editing {
            self.ui.ordered_button_clicks(
                match self.gizmo.space {
                    GizmoSpace::Local => "Space: Local",
                    GizmoSpace::Global => "Space: Global",
                },
                true,
            )
        } else {
            self.ui.label_value("Space", "Unavailable");
            Vec::new()
        };
        self.ui.set_panel_rect(1284.0, 4.0, 124.0);
        let snapping_click_sequences = if toolbar_editing {
            self.ui.ordered_button_clicks(
                if self.gizmo.snapping {
                    "Snap: On"
                } else {
                    "Snap: Off"
                },
                true,
            )
        } else {
            self.ui.label_value("Snap", "Unavailable");
            Vec::new()
        };
        let gizmo_status = if !toolbar_editing {
            "Disabled"
        } else if let Some(editor_scene) = self.editor_scene.as_ref() {
            match editor_scene.selected_entity.as_ref().and_then(|selected| {
                editor_scene
                    .scene
                    .entities
                    .iter()
                    .find(|entity| &entity.persistent_id == selected)
            }) {
                None => "Select Entity",
                Some(entity) if !entity.components.contains_key("engine.transform") => {
                    "Add Transform"
                }
                Some(_) => "Drag Axis",
            }
        } else {
            "No Scene"
        };
        self.ui.set_panel_rect(1412.0, 4.0, 184.0);
        self.ui.label_value("Gizmo", gizmo_status);

        let mut ordered_toolbar_actions = Vec::new();
        let mut append_actions = |sequences: Vec<u64>, action: OrderedToolbarAction| {
            ordered_toolbar_actions.extend(
                sequences
                    .into_iter()
                    .map(|sequence| OrderedAuthoringInput::Toolbar { sequence, action }),
            );
        };
        append_actions(save_click_sequences, OrderedToolbarAction::Save);
        append_actions(undo_click_sequences, OrderedToolbarAction::Undo);
        append_actions(redo_click_sequences, OrderedToolbarAction::Redo);
        if mode_before_toolbar == EditorPlayMode::Editing {
            append_actions(play_click_sequences, OrderedToolbarAction::StartPlay);
        }
        append_actions(
            translate_click_sequences,
            OrderedToolbarAction::SetGizmoMode(GizmoMode::Translate),
        );
        append_actions(
            rotate_click_sequences,
            OrderedToolbarAction::SetGizmoMode(GizmoMode::Rotate),
        );
        append_actions(
            scale_click_sequences,
            OrderedToolbarAction::SetGizmoMode(GizmoMode::Scale),
        );
        append_actions(
            space_click_sequences,
            OrderedToolbarAction::ToggleGizmoSpace,
        );
        append_actions(
            snapping_click_sequences,
            OrderedToolbarAction::ToggleGizmoSnapping,
        );

        if play_clicked && mode_before_toolbar == EditorPlayMode::Paused {
            self.resume_play();
        }
        if pause_clicked {
            self.pause_play();
        }
        if stop_clicked {
            self.stop_play();
        }

        // Use the mode in which this frame's UI was built. In particular, the
        // frame containing a Stop click remains read-only; authoring controls
        // become interactive on the following redraw.
        let editing = mode_before_toolbar == EditorPlayMode::Editing;
        let (scene_document_action, close_document_action, hierarchy_top) =
            self.draw_scene_document_panel(editing, dirty);
        let inspector_context = project_inspector_context(&self.project);
        let Some(editor_scene) = self.editor_scene.as_mut() else {
            return;
        };
        let history_push_serial_before_panels = editor_scene.history.push_serial();
        let Some(game_loop) = self.game_loop.as_mut() else {
            return;
        };

        let mut scene_changed = false;
        let mut gizmo_overlay_batch = None;

        self.ui.set_panel_rect(gap, hierarchy_top, left_w);
        self.ui.separator();
        let inspector_selection = editor_scene.selected_entity.clone();
        let hierarchy_actions =
            self.hierarchy
                .ui_with_authoring_ordered(&mut self.ui, &editor_scene.scene, editing);
        // Hierarchy drawing computes the visual final selection for legacy
        // callers. Ordered replay owns the actual editor selection here.
        self.hierarchy.set_selected(inspector_selection.clone());
        let mut ordered_panel_inputs = Vec::new();
        if editing {
            ordered_panel_inputs.extend(
                hierarchy_actions
                    .commands
                    .into_iter()
                    .map(OrderedAuthoringInput::PanelCommand),
            );
        }
        ordered_panel_inputs.extend(
            hierarchy_actions
                .selections
                .into_iter()
                .map(OrderedAuthoringInput::Selection),
        );
        let inspector_target_exists = inspector_selection.as_ref().is_some_and(|selected| {
            editor_scene
                .scene
                .entities
                .iter()
                .any(|entity| &entity.persistent_id == selected)
        });
        if inspector_selection.is_some() && !inspector_target_exists {
            self.ui.cancel_text_edit();
        }

        // ── Scene View (center) ────────────────────────────────────
        self.ui
            .set_panel_rect(center_left, content_top, scene_controls_w);
        let (_, _, scene_view_actions) = self
            .scene_view
            .ui_with_scene_ordered(&mut self.ui, &editor_scene.scene);
        ordered_panel_inputs.extend(
            scene_view_actions
                .into_iter()
                .map(OrderedAuthoringInput::SceneView),
        );

        // ── Inspector (right) ──────────────────────────────────────
        self.ui
            .set_panel_rect(inspector_left + 4.0, content_top, right_w);
        if editing {
            ordered_panel_inputs.extend(
                edit_current_inspector_selection(
                    &mut self.inspector,
                    &mut self.ui,
                    editor_scene,
                    &inspector_context,
                )
                .into_iter()
                .map(OrderedAuthoringInput::PanelCommand),
            );
        } else {
            self.ui
                .label_value("Inspector", "Stop Play to edit entities.");
        }
        // Asset browser (center). The registry is authoritative for both
        // the displayed asset kind and the exact AssetId assigned to a scene.
        refresh_asset_list(&mut self.asset_browser, game_loop.runtime.asset_registry());
        self.ui
            .set_panel_rect(asset_browser_left, asset_browser_top, asset_browser_w);
        draw_asset_browser(&mut self.ui, &mut self.asset_browser);
        if self.asset_browser.take_refresh_request() {
            let refresh_result = if editing {
                super::project_cli::cook_project(&self.project.manifest_path).and_then(|()| {
                    super::project_app::load_project_assets(&mut game_loop.runtime, &self.project)
                })
            } else {
                Err("Stop Play before reimporting project assets.".to_string())
            };
            match refresh_result {
                Ok(report) => {
                    refresh_asset_list(&mut self.asset_browser, game_loop.runtime.asset_registry());
                    // Force the selected material to be reconstructed from the
                    // refreshed registry snapshot below.
                    self.material_editor_selection = None;
                    tracing::info!(
                        discovered = report.discovered_assets,
                        loaded = report.loaded_assets(),
                        loaded_extensions = report.loaded_extension_assets(),
                        "editor project assets reimported"
                    );
                }
                Err(error) => {
                    let mut diagnostic = Diagnostic::new(
                        "EDASSET_REIMPORT_FAILED",
                        DiagnosticSeverity::Error,
                        "editor.asset-browser",
                        error.clone(),
                    );
                    diagnostic.asset = self.asset_browser.selected_asset().cloned();
                    editor_scene.diagnostics.push(diagnostic);
                    tracing::error!(%error, "editor project asset reimport failed");
                }
            }
        }
        if !editing {
            self.ui
                .label_value("Assignment", "Stop Play to edit scene assets.");
        } else if editor_scene.selected_entity.is_none() {
            self.ui
                .label_value("Assignment", "Select a scene entity first.");
        } else if self
            .asset_browser
            .selected_assignment_command(
                editor_scene
                    .selected_entity
                    .clone()
                    .expect("selection was checked above"),
            )
            .is_none()
        {
            if self.asset_browser.selected_asset().is_some() {
                self.ui.label_value(
                    "Assignment",
                    "Textures are assigned through a material asset.",
                );
            }
        } else if self.ui.button("Assign Selected Asset") {
            let stamp = self.ui.take_last_interaction_stamp();
            let command = editor_scene
                .selected_entity
                .clone()
                .and_then(|entity| self.asset_browser.selected_assignment_command(entity));
            if let (Some(stamp), Some(command)) = (stamp, command) {
                ordered_panel_inputs.push(OrderedAuthoringInput::PanelCommand(
                    SequencedCommand::new(stamp, Box::new(command)),
                ));
            }
        }

        // ── Material preview (lower right) ─────────────────────────
        let selected_material =
            selected_material_asset(&editor_scene.scene, editor_scene.selected_entity.as_ref());
        if selected_material != self.material_editor_selection {
            // The material panel is replaced wholesale on selection change.
            // Discard its focused text buffer before loading the next asset so
            // a blurred value from material A cannot be consumed by the same
            // widget label while material B is active.
            self.ui.cancel_text_edit();
            match selected_material.as_deref() {
                Some(material) => {
                    load_material(
                        &mut self.material_editor,
                        material,
                        game_loop.runtime.asset_registry(),
                    );
                    self.material_editor
                        .set_save_access(project_material_save_access(&self.project, material));
                }
                None => self.material_editor.reset(),
            }
            self.material_editor_selection = selected_material;
        }
        self.ui
            .set_panel_rect(inspector_left + 4.0, content_top + 410.0, right_w);
        if editing {
            draw_material_editor(&mut self.ui, &mut self.material_editor);
        } else {
            self.ui
                .label_value("Material Editor", "Stop Play to edit materials.");
        }

        let save_request = match self.material_editor.take_save_request() {
            Ok(request) => request,
            Err(error) => {
                self.material_editor.report_save_failure(error.clone());
                let mut diagnostic = Diagnostic::new(
                    "EDMATERIAL_SAVE_FAILED",
                    DiagnosticSeverity::Error,
                    "editor.material",
                    error.clone(),
                );
                diagnostic.asset = self.material_editor_selection.as_deref().map(AssetId::new);
                editor_scene.diagnostics.push(diagnostic);
                tracing::error!(%error, "editor material save request failed");
                None
            }
        };
        if let Some(request) = save_request {
            let result = if editing {
                save_project_material(&mut game_loop.runtime, &self.project, &request)
            } else {
                Err("Stop Play before saving project materials.".to_string())
            };
            match result {
                Ok(outcome) => {
                    self.material_editor.report_save_success(format!(
                        "Saved {} and refreshed {}.",
                        outcome.source_path.display(),
                        outcome.cooked_path.display()
                    ));
                    refresh_asset_list(&mut self.asset_browser, game_loop.runtime.asset_registry());
                    tracing::info!(
                        material = %request.material_asset,
                        source = %outcome.source_path.display(),
                        cooked = %outcome.cooked_path.display(),
                        "editor material saved"
                    );
                }
                Err(error) => {
                    self.material_editor.report_save_failure(error.clone());
                    let mut diagnostic = Diagnostic::new(
                        "EDMATERIAL_SAVE_FAILED",
                        DiagnosticSeverity::Error,
                        "editor.material",
                        error.clone(),
                    );
                    diagnostic.asset = Some(AssetId::new(request.material_asset.clone()));
                    editor_scene.diagnostics.push(diagnostic);
                    tracing::error!(
                        material = %request.material_asset,
                        %error,
                        "editor material save failed"
                    );
                }
            }
        }
        if scene_changed {
            let selection_exists = editor_scene
                .selected_entity
                .as_ref()
                .is_some_and(|selected| {
                    editor_scene
                        .scene
                        .entities
                        .iter()
                        .any(|entity| &entity.persistent_id == selected)
                });
            if !selection_exists {
                editor_scene.selected_entity = None;
                self.hierarchy.set_selected(None);
            }
        }

        // Toolbar clicks and raw viewport input carry the same sequence
        // allocated in `on_event`. Replaying the merged stream is what makes
        // both Drag -> Undo and Undo -> Drag (and the corresponding Save/mode
        // pairs) obey platform order even when no redraw occurs in between.
        let _cancel_gizmo = std::mem::take(&mut self.cancel_gizmo_requested);
        let selection_changed_before_replay = editor_scene
            .active_transform_gizmo_entity()
            .is_some_and(|active| editor_scene.selected_entity.as_ref() != Some(active));

        if !editing || selection_changed_before_replay {
            if editor_scene.cancel_transform_gizmo_drag() {
                scene_changed = true;
            }
            self.gizmo.cancel_drag();
            if !editing {
                self.gizmo_pointer_events.clear();
            }
        }

        ordered_panel_inputs.extend(ordered_toolbar_actions);
        let ordered_inputs = merge_ordered_authoring_inputs(
            ordered_panel_inputs,
            std::mem::take(&mut self.gizmo_pointer_events),
        );

        let mut start_play_after_edit = false;
        let mut authoring_barrier_reached = false;
        let mut replay_gizmo_view = None;
        if editing && scene_changed {
            // Panel edits still precede phase-one ordered replay. Refreshing
            // here at least guarantees the first viewport sample uses their
            // actual transform rather than a stale runtime view.
            synchronize_editor_preview_and_camera(game_loop, editor_scene, &self.scene_view);
        }

        for input in ordered_inputs {
            if authoring_barrier_reached {
                continue;
            }
            match input {
                OrderedAuthoringInput::Gizmo(event) => {
                    if !editing {
                        continue;
                    }
                    if event.event == GizmoPointerEvent::Cancel {
                        scene_changed |= editor_scene.cancel_transform_gizmo_drag();
                        self.gizmo.cancel_drag();
                        replay_gizmo_view = None;
                        continue;
                    }
                    let Some(selected) = editor_scene.selected_entity.clone() else {
                        continue;
                    };
                    let view = replay_gizmo_view.or_else(|| {
                        runtime_gizmo_view(
                            &game_loop.runtime,
                            &selected,
                            self.frame,
                            Vec2::new(self.window_w, self.window_h),
                        )
                        .and_then(|view| {
                            restrict_gizmo_view_to_rect(
                                view,
                                scene_interaction_min,
                                scene_interaction_max,
                            )
                        })
                    });
                    let Some(view) = view else {
                        scene_changed |= editor_scene.cancel_transform_gizmo_drag();
                        self.gizmo.cancel_drag();
                        replay_gizmo_view = None;
                        continue;
                    };
                    scene_changed |= process_gizmo_pointer_events(
                        vec![event.event],
                        editor_scene,
                        &mut self.gizmo,
                        &self.ui,
                        &game_loop.runtime,
                        &selected,
                        view,
                    );
                    replay_gizmo_view = self.gizmo.dragging.then_some(view);
                }
                OrderedAuthoringInput::Toolbar { action, .. } => {
                    if !editing {
                        continue;
                    }
                    let had_incomplete_gizmo =
                        self.gizmo.dragging || editor_scene.is_transform_gizmo_drag_active();
                    if had_incomplete_gizmo {
                        scene_changed |= editor_scene.cancel_transform_gizmo_drag();
                        self.gizmo.cancel_drag();
                        replay_gizmo_view = None;
                    }
                    match action {
                        OrderedToolbarAction::Save => {
                            match save_scene_atomically(
                                &editor_scene.scene,
                                &self.current_scene_path,
                            ) {
                                Ok(()) => {
                                    editor_scene.history.mark_clean();
                                    self.scene_document_status =
                                        Some(format!("Saved '{}'.", self.current_scene_id));
                                    tracing::info!(
                                        scene_id = self.current_scene_id,
                                        scene = %self.current_scene_path.display(),
                                        "editor scene saved"
                                    );
                                }
                                Err(error) => {
                                    self.scene_document_status = Some(error.clone());
                                    tracing::error!(%error, "editor scene save failed");
                                }
                            }
                        }
                        OrderedToolbarAction::Undo => {
                            if should_apply_history_undo(
                                true,
                                had_incomplete_gizmo,
                                had_uncommitted_text_change,
                                history_push_serial_before_panels,
                                editor_scene.history.push_serial(),
                                editor_scene.history.can_undo(),
                            ) {
                                match editor_scene.undo() {
                                    Ok(()) => {
                                        scene_changed = true;
                                        synchronize_editor_preview_and_camera(
                                            game_loop,
                                            editor_scene,
                                            &self.scene_view,
                                        );
                                        replay_gizmo_view = None;
                                    }
                                    Err(error) => {
                                        tracing::error!(%error, "editor undo failed");
                                    }
                                }
                            }
                        }
                        OrderedToolbarAction::Redo => {
                            if !had_incomplete_gizmo && editor_scene.history.can_redo() {
                                match editor_scene.redo() {
                                    Ok(()) => {
                                        scene_changed = true;
                                        synchronize_editor_preview_and_camera(
                                            game_loop,
                                            editor_scene,
                                            &self.scene_view,
                                        );
                                        replay_gizmo_view = None;
                                    }
                                    Err(error) => {
                                        tracing::error!(%error, "editor redo failed");
                                    }
                                }
                            }
                        }
                        OrderedToolbarAction::StartPlay => {
                            start_play_after_edit = true;
                            authoring_barrier_reached = true;
                        }
                        OrderedToolbarAction::SetGizmoMode(mode) => self.gizmo.mode = mode,
                        OrderedToolbarAction::ToggleGizmoSpace => {
                            self.gizmo.space = match self.gizmo.space {
                                GizmoSpace::Local => GizmoSpace::Global,
                                GizmoSpace::Global => GizmoSpace::Local,
                            };
                        }
                        OrderedToolbarAction::ToggleGizmoSnapping => {
                            self.gizmo.snapping = !self.gizmo.snapping;
                        }
                    }
                }
                OrderedAuthoringInput::PanelCommand(command) => {
                    if !editing {
                        continue;
                    }
                    if self.gizmo.dragging || editor_scene.is_transform_gizmo_drag_active() {
                        scene_changed |= editor_scene.cancel_transform_gizmo_drag();
                        self.gizmo.cancel_drag();
                    }
                    replay_gizmo_view = None;
                    match editor_scene.execute(command.command) {
                        Ok(()) => {
                            scene_changed = true;
                            let selection_exists = editor_scene
                                .selected_entity
                                .as_ref()
                                .is_some_and(|selected| {
                                    editor_scene
                                        .scene
                                        .entities
                                        .iter()
                                        .any(|entity| &entity.persistent_id == selected)
                                });
                            if !selection_exists {
                                editor_scene.selected_entity = None;
                                self.hierarchy.set_selected(None);
                            }
                            synchronize_editor_preview_and_camera(
                                game_loop,
                                editor_scene,
                                &self.scene_view,
                            );
                        }
                        Err(error) => {
                            tracing::error!(%error, "ordered editor panel command failed");
                        }
                    }
                }
                OrderedAuthoringInput::Selection(selection) => {
                    let changes_target = editor_scene
                        .active_transform_gizmo_entity()
                        .is_some_and(|active| selection.selection.as_ref() != Some(active));
                    if changes_target {
                        scene_changed |= editor_scene.cancel_transform_gizmo_drag();
                        self.gizmo.cancel_drag();
                    }
                    replay_gizmo_view = None;
                    editor_scene.selected_entity = selection.selection.clone();
                    self.hierarchy.set_selected(selection.selection);
                }
                OrderedAuthoringInput::SceneView(action) => {
                    self.scene_view.apply_action(action.action);
                    if editing && action.action.affects_camera() {
                        let _ = apply_editor_camera(&game_loop.runtime, &self.scene_view);
                        // A drag keeps the camera/view captured by its press so
                        // changing the editor camera mid-gesture cannot bend
                        // its axis. The next gesture observes this new camera.
                        if !self.gizmo.dragging {
                            replay_gizmo_view = None;
                        }
                    }
                }
            }
        }

        if scene_changed {
            let selection_exists = editor_scene
                .selected_entity
                .as_ref()
                .is_some_and(|selected| {
                    editor_scene
                        .scene
                        .entities
                        .iter()
                        .any(|entity| &entity.persistent_id == selected)
                });
            if !selection_exists {
                editor_scene.selected_entity = None;
                self.hierarchy.set_selected(None);
            }
        }

        if scene_changed {
            synchronize_editor_preview(game_loop, editor_scene);
        }

        if editing {
            let _ = apply_editor_camera(&game_loop.runtime, &self.scene_view);
        }

        // Build after scene synchronisation so inspector edits, undo/redo and
        // a just-committed drag all render the current transform. The batch is
        // inserted before normal UI batches later, keeping panels above it.
        if editing {
            if let Some(selected) = editor_scene.selected_entity.as_deref() {
                if let Some(view) = runtime_gizmo_view(
                    &game_loop.runtime,
                    selected,
                    self.frame,
                    Vec2::new(self.window_w, self.window_h),
                )
                .and_then(|view| {
                    restrict_gizmo_view_to_rect(view, scene_interaction_min, scene_interaction_max)
                }) {
                    gizmo_overlay_batch = build_gizmo_ui_batch(
                        &self.gizmo,
                        view.world_position,
                        view.world_rotation,
                        view.view,
                        view.projection,
                        view.viewport_size,
                    )
                    .map(|batch| offset_gizmo_batch(batch, view));
                }
            }
        }

        let preview_request = self.material_editor.take_preview_request();

        // ── 3. End UI frame ────────────────────────────────────────
        let now = Instant::now();
        let delta_seconds = now
            .duration_since(self.last_frame_time)
            .as_secs_f32()
            .min(0.1);
        self.last_frame_time = now;
        if self.play_session.should_tick() {
            game_loop.update(delta_seconds);
            if let Err(error) =
                super::project_scripts::fail_on_script_errors(&game_loop.runtime, "update")
            {
                tracing::error!(%error, "editor: script update failed");
                let diagnostics =
                    recover_play_after_script_error(&mut self.play_session, game_loop, error);
                log_scene_diagnostics(
                    "editor Play stopped after script update failure",
                    diagnostics.clone(),
                );
                editor_scene.diagnostics.push_many(diagnostics);
                self.play_runtime_scene_id = None;
                #[cfg(feature = "target-desktop")]
                self.input_state.reset(&mut game_loop.input_map);
                self.last_frame_time = Instant::now();
            } else if let Some(runtime_scene_id) = self.play_runtime_scene_id.as_mut() {
                match super::project_app::process_pending_scene_transitions(
                    game_loop,
                    &self.project,
                    runtime_scene_id,
                ) {
                    Ok(transitions) => {
                        if transitions > 0 {
                            tracing::info!(
                                transitions,
                                scene_id = runtime_scene_id,
                                "editor Play scene transition completed"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "editor Play scene transition failed");
                        let diagnostics = recover_play_after_scene_transition_error(
                            &mut self.play_session,
                            game_loop,
                            error,
                        );
                        log_scene_diagnostics(
                            "editor Play stopped after scene transition failure",
                            diagnostics.clone(),
                        );
                        editor_scene.diagnostics.push_many(diagnostics);
                        self.play_runtime_scene_id = None;
                        #[cfg(feature = "target-desktop")]
                        self.input_state.reset(&mut game_loop.input_map);
                        self.last_frame_time = Instant::now();
                    }
                }
            }
        }

        let mut ui_batches = self.ui.end_frame().build_batches();
        if let Some(batch) = gizmo_overlay_batch {
            ui_batches.insert(0, batch);
        }

        if let Some(request) = preview_request {
            let revision = request.revision;
            match game_loop
                .runtime
                .renderer_mut()
                .upload_texture(material_preview_upload(&request))
            {
                Ok(_) => {
                    if !self
                        .material_editor
                        .complete_preview(revision, MATERIAL_PREVIEW_TEXTURE_ID)
                    {
                        tracing::warn!(
                            revision,
                            "editor material preview completed after a newer edit"
                        );
                    }
                }
                Err(diagnostics) => {
                    self.material_editor.fail_preview(revision);
                    for diagnostic in diagnostics {
                        tracing::error!(
                            code = diagnostic.code,
                            message = diagnostic.message,
                            "editor material preview upload failed"
                        );
                    }
                }
            }
        }

        // ── 4. Render 3D scene ─────────────────────────────────────
        match game_loop
            .runtime
            .render_frame_with_ui(self.frame, ui_batches)
        {
            Ok(stats) => tracing::debug!(
                frame = self.frame,
                draw_calls = stats.draw_calls,
                "editor frame"
            ),
            Err(diags) => {
                for d in &diags {
                    tracing::error!(code = d.code, msg = d.message, "editor render failed");
                }
                std::process::exit(1);
            }
        }

        self.frame += 1;
        let had_scene_document_action = scene_document_action.is_some();
        let had_close_document_action = close_document_action.is_some();
        if let Some(action) = scene_document_action {
            if let Err(error) = self.apply_scene_document_action(action) {
                self.record_scene_document_error(error);
            }
        }
        if let Some(action) = close_document_action {
            if let Err(error) = self.apply_close_document_action(action) {
                self.record_scene_document_error(error);
            }
        }
        if start_play_after_edit
            && !had_scene_document_action
            && !had_close_document_action
            && !self.close_confirmation_pending
        {
            self.start_play();
        }
    }
}

impl WindowApp for EditorApp {
    fn on_create(&mut self, window: Arc<Window>) {
        let size = window.inner_size();
        self.window_w = size.width as f32;
        self.window_h = size.height as f32;
        self.ui.resize(self.window_w, self.window_h);

        let display_handle = match window.display_handle() {
            Ok(h) => h.as_raw(),
            Err(e) => {
                tracing::error!("display handle: {e}");
                std::process::exit(1);
            }
        };
        let window_handle = match window.window_handle() {
            Ok(h) => h.as_raw(),
            Err(e) => {
                tracing::error!("window handle: {e}");
                std::process::exit(1);
            }
        };

        match create_vulkan_backend_renderer(
            display_handle,
            window_handle,
            size.width.max(1),
            size.height.max(1),
            std::env::var("ENGINE_VK_VALIDATION").is_ok(),
            None,
        ) {
            Ok(backend) => {
                let mut game_loop = GameLoop::new(EngineConfig {
                    application_name: format!("{} Editor", self.project.manifest.name),
                });
                #[cfg(feature = "target-desktop")]
                match super::project_input::load_project_input_map(&self.project) {
                    Ok(input_map) => game_loop.input_map = input_map,
                    Err(error) => {
                        tracing::error!(%error, "editor: failed to load project input actions");
                        std::process::exit(1);
                    }
                }
                game_loop.runtime.renderer_mut().set_backend(backend);
                self.game_loop = Some(game_loop);
                tracing::info!("editor: Vulkan backend initialized");
            }
            Err(e) => {
                tracing::error!("Vulkan backend creation failed: {e}");
                std::process::exit(1);
            }
        }

        self.init_scene();
        tracing::info!("editor: fully initialized");
    }

    fn on_event(&mut self, window: &Window, event: PlatformEvent) -> EventFlow {
        if !self.play_session.is_editing() {
            #[cfg(feature = "target-desktop")]
            if should_route_event_to_gameplay(&self.ui, &event) {
                if let Some(game_loop) = self.game_loop.as_mut() {
                    self.input_state
                        .apply_platform_event(&mut game_loop.input_map, &event);
                }
            }
            #[cfg(feature = "runtime-subsystems")]
            if let Some(game_loop) = self.game_loop.as_mut() {
                route_editor_play_ui_event(game_loop, &self.ui, &event);
            }
        }
        let event_sequence = self.next_platform_event_sequence;
        self.next_platform_event_sequence = self.next_platform_event_sequence.wrapping_add(1);
        match event {
            PlatformEvent::MouseMoved { x, y } => {
                self.mouse_x = x;
                self.mouse_y = y;
                self.ui
                    .set_pointer_with_sequence(x as f32, y as f32, event_sequence);
                self.gizmo_pointer_events.push(SequencedGizmoPointerEvent {
                    sequence: event_sequence,
                    event: GizmoPointerEvent::Move(Vec2::new(x as f32, y as f32)),
                });
            }
            PlatformEvent::MousePressed { button, x, y } => {
                if button == platform::MouseButton::Left {
                    self.mouse_x = x;
                    self.mouse_y = y;
                    self.ui
                        .set_pointer_with_sequence(x as f32, y as f32, event_sequence);
                    self.ui.set_mouse_pressed_with_sequence(event_sequence);
                    self.gizmo_pointer_events.push(SequencedGizmoPointerEvent {
                        sequence: event_sequence,
                        event: GizmoPointerEvent::Press(Vec2::new(x as f32, y as f32)),
                    });
                }
            }
            PlatformEvent::MouseReleased { button, x, y } => {
                if button == platform::MouseButton::Left {
                    self.mouse_x = x;
                    self.mouse_y = y;
                    self.ui
                        .set_pointer_with_sequence(x as f32, y as f32, event_sequence);
                    self.ui.set_mouse_released_with_sequence(event_sequence);
                    self.gizmo_pointer_events.push(SequencedGizmoPointerEvent {
                        sequence: event_sequence,
                        event: GizmoPointerEvent::Release(Vec2::new(x as f32, y as f32)),
                    });
                }
            }
            PlatformEvent::CharacterTyped { character } => {
                self.ui
                    .type_character_with_sequence(character, event_sequence);
            }
            PlatformEvent::KeyPressed { key, .. } => {
                if key == platform::KeyCode::Escape {
                    self.cancel_gizmo_requested = true;
                    self.gizmo_pointer_events.push(SequencedGizmoPointerEvent {
                        sequence: event_sequence,
                        event: GizmoPointerEvent::Cancel,
                    });
                }
                if let Some(key) = map_editor_key(key) {
                    self.ui.press_key_with_sequence(key, event_sequence);
                }
            }
            PlatformEvent::Focused(false) => {
                // A release outside the window is not guaranteed to arrive.
                // Clear capture without synthesizing a click, then cancel any
                // uncommitted scene gesture.
                self.ui.cancel_pointer_interaction();
                self.ui.cancel_text_edit();
                self.cancel_gizmo_requested = true;
                self.gizmo_pointer_events.push(SequencedGizmoPointerEvent {
                    sequence: event_sequence,
                    event: GizmoPointerEvent::Cancel,
                });
            }
            PlatformEvent::Suspended => {
                self.ui.cancel_pointer_interaction();
                self.ui.cancel_text_edit();
                self.cancel_gizmo_requested = true;
                self.gizmo_pointer_events.push(SequencedGizmoPointerEvent {
                    sequence: event_sequence,
                    event: GizmoPointerEvent::Cancel,
                });
            }
            PlatformEvent::Resized { width, height } => {
                self.ui.cancel_pointer_interaction();
                self.ui.cancel_text_edit();
                self.cancel_gizmo_requested = true;
                self.gizmo_pointer_events.push(SequencedGizmoPointerEvent {
                    sequence: event_sequence,
                    event: GizmoPointerEvent::Cancel,
                });
                self.window_w = width as f32;
                self.window_h = height as f32;
                self.ui.resize(self.window_w, self.window_h);
                if let Some(ref mut game_loop) = self.game_loop {
                    if let Err(diagnostics) = game_loop.runtime.renderer_mut().resize(width, height)
                    {
                        super::log_renderer_diagnostics("editor resize", &diagnostics);
                        std::process::exit(1);
                    }
                }
            }
            PlatformEvent::Redraw => {
                self.render_editor_frame();
                if self.exit_after_frame {
                    return EventFlow::Exit;
                }
                window.request_redraw();
            }
            PlatformEvent::CloseRequested => {
                if !self.play_session.is_editing() {
                    self.stop_play();
                    if !self.play_session.is_editing() {
                        self.scene_document_status = Some(
                            "Could not close while Play mode failed to restore the authoring scene. Retry Stop or cancel Play errors first."
                                .to_string(),
                        );
                        window.request_redraw();
                        return EventFlow::Continue;
                    }
                }
                let has_unsaved_changes = self.ui.has_uncommitted_text_change()
                    || self.gizmo.dragging
                    || self.editor_scene.as_ref().is_some_and(|scene| {
                        scene.is_dirty() || scene.is_transform_gizmo_drag_active()
                    });
                if has_unsaved_changes {
                    self.pending_scene_switch = None;
                    self.close_confirmation_pending = true;
                    self.scene_document_status = Some(
                        "Unsaved changes: choose Save & Close, Discard & Close, or Cancel Close."
                            .to_string(),
                    );
                    window.request_redraw();
                } else {
                    return EventFlow::Exit;
                }
            }
            _ => {}
        }
        EventFlow::Continue
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
    if let Err(e) = platform::run(
        WindowDescriptor {
            title,
            width: 1600,
            height: 900,
        },
        app,
    ) {
        tracing::error!("editor: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_asset::cook::{MaterialSource, MATERIAL_SOURCE_SCHEMA};
    use engine_asset::project::ProjectManifest;
    use engine_renderer::{MaterialUpload, MeshUpload};

    #[test]
    fn pending_text_undo_only_targets_a_command_created_by_that_redraw() {
        assert!(should_apply_history_undo(true, false, true, 4, 5, true));
        assert!(
            !should_apply_history_undo(true, false, true, 4, 4, true),
            "rejected text must not undo an older unrelated command"
        );
        assert!(should_apply_history_undo(true, false, false, 4, 4, true));
        assert!(!should_apply_history_undo(true, true, false, 4, 4, true));
        assert!(!should_apply_history_undo(true, false, false, 4, 4, false));
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
        assert_eq!(app.pending_scene_switch.as_deref(), Some("level_two"));

        app.apply_scene_document_action(SceneDocumentAction::CancelSwitch)
            .unwrap();
        assert_eq!(app.current_scene_id, "main");
        assert!(app.pending_scene_switch.is_none());
        assert!(app.editor_scene.as_ref().unwrap().is_dirty());
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
        app.hierarchy.set_selected(Some("cube-01".into()));
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
        assert!(app.hierarchy.selected().is_none());
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
            base_color_texture: None,
            transparency: "Opaque".to_string(),
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
        refresh_asset_list(&mut browser, runtime.asset_registry());
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
    fn inspector_commit_targets_old_entity_before_selection_change() {
        let mut editor_scene = EditorScene::new(engine_scene::sample_scene());
        editor_scene.selected_entity = Some("camera-main".into());
        let mut inspector = InspectorPanel::new("Inspector");
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.inject_event(engine_editor::UiEvent::TextFieldCommit(
            "Name".into(),
            "Edited Camera".into(),
        ));

        let commands = edit_current_inspector_selection(
            &mut inspector,
            &mut ui,
            &mut editor_scene,
            &InspectorContext::default(),
        );
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].stamp.phase,
            UiInteractionPhase::BeforeRawPointer
        );
        editor_scene
            .execute(commands.into_iter().next().unwrap().command)
            .unwrap();
        // This mirrors the host order: apply the Hierarchy's new selection
        // only after the old Inspector target consumed its blurred value.
        editor_scene.selected_entity = Some("cube-01".into());
        ui.end_frame();

        let camera = editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "camera-main")
            .unwrap();
        let cube = editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap();
        assert_eq!(camera.name.as_deref(), Some("Edited Camera"));
        assert_eq!(cube.name.as_deref(), Some("Cube"));

        let mut play_session = EditorPlaySession::default();
        let mut loaded = None;
        play_session
            .start(&editor_scene.scene, |scene| {
                loaded = Some(scene);
                Ok::<_, ()>(())
            })
            .unwrap();
        let played_camera = loaded
            .as_ref()
            .unwrap()
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "camera-main")
            .unwrap();
        assert_eq!(played_camera.name.as_deref(), Some("Edited Camera"));
    }

    #[test]
    fn play_input_routing_excludes_editor_ui_and_text_focus() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.block_pointer_rect(0.0, 0.0, 200.0, 100.0);
        assert!(!should_route_event_to_gameplay(
            &ui,
            &PlatformEvent::MousePressed {
                button: platform::MouseButton::Left,
                x: 50.0,
                y: 50.0,
            },
        ));
        assert!(should_route_event_to_gameplay(
            &ui,
            &PlatformEvent::MousePressed {
                button: platform::MouseButton::Left,
                x: 300.0,
                y: 200.0,
            },
        ));
        assert!(should_route_event_to_gameplay(
            &ui,
            &PlatformEvent::MouseReleased {
                button: platform::MouseButton::Left,
                x: 50.0,
                y: 50.0,
            },
        ));
        ui.end_frame();

        let mut text_ui = EditorUi::new();
        text_ui.set_pointer(10.0, 10.0);
        text_ui.begin_frame();
        let _ = text_ui.text_field("Name", "Player");
        text_ui.end_frame();
        text_ui.set_mouse_pressed();
        text_ui.begin_frame();
        let _ = text_ui.text_field("Name", "Player");
        text_ui.end_frame();
        text_ui.set_mouse_released();
        text_ui.begin_frame();
        let _ = text_ui.text_field("Name", "Player");
        assert!(text_ui.has_active_text_edit());
        assert!(!should_route_event_to_gameplay(
            &text_ui,
            &PlatformEvent::KeyPressed {
                key: platform::KeyCode::W,
                modifiers: platform::Modifiers::default(),
            },
        ));
        assert!(should_route_event_to_gameplay(
            &text_ui,
            &PlatformEvent::KeyReleased {
                key: platform::KeyCode::W,
                modifiers: platform::Modifiers::default(),
            },
        ));
        text_ui.end_frame();

        let mut pending_focus_ui = EditorUi::new();
        pending_focus_ui.begin_frame();
        let _ = pending_focus_ui.text_field("Name", "Player");
        pending_focus_ui.end_frame();
        pending_focus_ui.set_pointer(10.0, 10.0);
        pending_focus_ui.set_mouse_pressed();
        pending_focus_ui.set_mouse_released();
        assert!(!should_route_event_to_gameplay(
            &pending_focus_ui,
            &PlatformEvent::KeyPressed {
                key: platform::KeyCode::W,
                modifiers: platform::Modifiers::default(),
            },
        ));
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

    #[derive(Debug)]
    struct OrderedReplayProbe {
        value: i32,
        saved_values: Vec<i32>,
        mode: GizmoMode,
        drag_mode: Option<GizmoMode>,
        camera_distance: f32,
        drag_camera_distance: Option<f32>,
        completed_drag_camera_distances: Vec<f32>,
        show_grid: bool,
        trace: Vec<String>,
        stopped_at_play: bool,
    }

    fn ordered_gizmo(sequence: u64, event: GizmoPointerEvent) -> SequencedGizmoPointerEvent {
        SequencedGizmoPointerEvent { sequence, event }
    }

    fn ordered_action(sequence: u64, action: OrderedToolbarAction) -> OrderedAuthoringInput {
        OrderedAuthoringInput::Toolbar { sequence, action }
    }

    fn ordered_scene_view_action(
        sequence: u64,
        action: engine_editor::SceneViewAction,
    ) -> OrderedAuthoringInput {
        OrderedAuthoringInput::SceneView(SequencedSceneViewAction::new(
            engine_editor::UiInteractionStamp {
                sequence,
                phase: UiInteractionPhase::AfterRawPointer,
            },
            action,
        ))
    }

    fn ordered_selection(sequence: u64, selection: Option<&str>) -> OrderedAuthoringInput {
        OrderedAuthoringInput::Selection(SequencedSelection {
            stamp: engine_editor::UiInteractionStamp {
                sequence,
                phase: UiInteractionPhase::AfterRawPointer,
            },
            selection: selection.map(str::to_owned),
        })
    }

    struct NoopPanelCommand;

    impl engine_editor::Command for NoopPanelCommand {
        fn name(&self) -> &str {
            "Noop Panel Command"
        }

        fn execute(&mut self, _scene: &mut Scene) -> Result<(), engine_editor::EditorError> {
            Ok(())
        }

        fn undo(&mut self, _scene: &mut Scene) -> Result<(), engine_editor::EditorError> {
            Ok(())
        }
    }

    fn ordered_panel_command(sequence: u64, phase: UiInteractionPhase) -> OrderedAuthoringInput {
        OrderedAuthoringInput::PanelCommand(SequencedCommand::new(
            engine_editor::UiInteractionStamp { sequence, phase },
            Box::new(NoopPanelCommand),
        ))
    }

    fn ordered_input_labels(inputs: Vec<OrderedAuthoringInput>) -> Vec<&'static str> {
        inputs
            .into_iter()
            .map(|input| match input {
                OrderedAuthoringInput::Gizmo(_) => "gizmo",
                OrderedAuthoringInput::Toolbar { .. } => "toolbar",
                OrderedAuthoringInput::PanelCommand(_) => "panel",
                OrderedAuthoringInput::Selection(_) => "selection",
                OrderedAuthoringInput::SceneView(_) => "scene-view",
            })
            .collect()
    }

    fn complete_ordered_drag(start: u64) -> Vec<SequencedGizmoPointerEvent> {
        vec![
            ordered_gizmo(start, GizmoPointerEvent::Press(Vec2::ZERO)),
            ordered_gizmo(start + 1, GizmoPointerEvent::Move(Vec2::X)),
            ordered_gizmo(start + 2, GizmoPointerEvent::Release(Vec2::X)),
        ]
    }

    #[test]
    fn inspector_text_blur_precedes_same_sequence_gizmo_press() {
        let merged = merge_ordered_authoring_inputs(
            vec![ordered_panel_command(
                10,
                UiInteractionPhase::BeforeRawPointer,
            )],
            vec![ordered_gizmo(10, GizmoPointerEvent::Press(Vec2::ZERO))],
        );
        assert_eq!(ordered_input_labels(merged), ["panel", "gizmo"]);
    }

    #[test]
    fn gizmo_release_precedes_same_sequence_panel_button_command() {
        let merged = merge_ordered_authoring_inputs(
            vec![ordered_panel_command(
                20,
                UiInteractionPhase::AfterRawPointer,
            )],
            vec![ordered_gizmo(20, GizmoPointerEvent::Release(Vec2::ZERO))],
        );
        assert_eq!(ordered_input_labels(merged), ["gizmo", "panel"]);
    }

    #[test]
    fn panel_commands_and_save_replay_in_both_platform_orders() {
        let panel_then_save = merge_ordered_authoring_inputs(
            vec![
                ordered_panel_command(1, UiInteractionPhase::AfterRawPointer),
                ordered_action(2, OrderedToolbarAction::Save),
            ],
            Vec::new(),
        );
        assert_eq!(ordered_input_labels(panel_then_save), ["panel", "toolbar"]);

        let save_then_panel = merge_ordered_authoring_inputs(
            vec![
                ordered_action(1, OrderedToolbarAction::Save),
                ordered_panel_command(2, UiInteractionPhase::AfterRawPointer),
            ],
            Vec::new(),
        );
        assert_eq!(ordered_input_labels(save_then_panel), ["toolbar", "panel"]);
    }

    #[test]
    fn hierarchy_selection_and_complete_drag_replay_in_both_orders() {
        let selection = |sequence| ordered_selection(sequence, Some("cube-01"));
        let select_then_drag =
            merge_ordered_authoring_inputs(vec![selection(1)], complete_ordered_drag(2));
        assert_eq!(
            ordered_input_labels(select_then_drag),
            ["selection", "gizmo", "gizmo", "gizmo"]
        );

        let drag_then_select =
            merge_ordered_authoring_inputs(vec![selection(4)], complete_ordered_drag(1));
        assert_eq!(
            ordered_input_labels(drag_then_select),
            ["gizmo", "gizmo", "gizmo", "selection"]
        );
    }

    #[test]
    fn scene_view_actions_share_raw_pointer_and_semantic_ordering() {
        let same_release = merge_ordered_authoring_inputs(
            vec![ordered_scene_view_action(
                10,
                engine_editor::SceneViewAction::SetDistance(25.0),
            )],
            vec![ordered_gizmo(10, GizmoPointerEvent::Release(Vec2::ZERO))],
        );
        assert_eq!(ordered_input_labels(same_release), ["gizmo", "scene-view"]);

        let setting_then_toolbar = merge_ordered_authoring_inputs(
            vec![
                ordered_scene_view_action(1, engine_editor::SceneViewAction::SetShowGrid(false)),
                ordered_action(2, OrderedToolbarAction::ToggleGizmoSnapping),
            ],
            Vec::new(),
        );
        assert_eq!(
            ordered_input_labels(setting_then_toolbar),
            ["scene-view", "toolbar"]
        );
        let toolbar_then_setting = merge_ordered_authoring_inputs(
            vec![
                ordered_action(1, OrderedToolbarAction::ToggleGizmoSnapping),
                ordered_scene_view_action(2, engine_editor::SceneViewAction::SetShowGrid(false)),
            ],
            Vec::new(),
        );
        assert_eq!(
            ordered_input_labels(toolbar_then_setting),
            ["toolbar", "scene-view"]
        );

        let setting_then_selection = merge_ordered_authoring_inputs(
            vec![
                ordered_scene_view_action(1, engine_editor::SceneViewAction::SetYaw(45.0)),
                ordered_selection(2, Some("cube-01")),
            ],
            Vec::new(),
        );
        assert_eq!(
            ordered_input_labels(setting_then_selection),
            ["scene-view", "selection"]
        );
        let selection_then_setting = merge_ordered_authoring_inputs(
            vec![
                ordered_selection(1, Some("cube-01")),
                ordered_scene_view_action(2, engine_editor::SceneViewAction::SetYaw(45.0)),
            ],
            Vec::new(),
        );
        assert_eq!(
            ordered_input_labels(selection_then_setting),
            ["selection", "scene-view"]
        );
    }

    fn probe_ordered_authoring_replay(
        toolbar: Vec<OrderedAuthoringInput>,
        gizmo: Vec<SequencedGizmoPointerEvent>,
    ) -> OrderedReplayProbe {
        let mut probe = OrderedReplayProbe {
            value: 1,
            saved_values: Vec::new(),
            mode: GizmoMode::Translate,
            drag_mode: None,
            camera_distance: 10.0,
            drag_camera_distance: None,
            completed_drag_camera_distances: Vec::new(),
            show_grid: true,
            trace: Vec::new(),
            stopped_at_play: false,
        };
        for input in merge_ordered_authoring_inputs(toolbar, gizmo) {
            if probe.stopped_at_play {
                continue;
            }
            match input {
                OrderedAuthoringInput::Gizmo(event) => match event.event {
                    GizmoPointerEvent::Press(_) => {
                        probe.drag_mode = Some(probe.mode);
                        probe.drag_camera_distance = Some(probe.camera_distance);
                    }
                    GizmoPointerEvent::Move(_) => {}
                    GizmoPointerEvent::Release(_) => {
                        if let Some(mode) = probe.drag_mode.take() {
                            probe
                                .completed_drag_camera_distances
                                .push(probe.drag_camera_distance.take().unwrap());
                            probe.value += match mode {
                                GizmoMode::Translate => 1,
                                GizmoMode::Rotate => 10,
                                GizmoMode::Scale => 100,
                            };
                            probe.trace.push(format!("drag:{mode:?}"));
                        }
                    }
                    GizmoPointerEvent::Cancel => {
                        probe.drag_mode = None;
                        probe.drag_camera_distance = None;
                    }
                },
                OrderedAuthoringInput::Toolbar { action, .. } => match action {
                    OrderedToolbarAction::Save => {
                        probe.saved_values.push(probe.value);
                        probe.trace.push("save".into());
                    }
                    OrderedToolbarAction::Undo => {
                        probe.value -= 1;
                        probe.trace.push("undo".into());
                    }
                    OrderedToolbarAction::Redo => {
                        probe.value += 1;
                        probe.trace.push("redo".into());
                    }
                    OrderedToolbarAction::StartPlay => {
                        probe.trace.push("play".into());
                        probe.stopped_at_play = true;
                    }
                    OrderedToolbarAction::SetGizmoMode(mode) => {
                        probe.mode = mode;
                        probe.trace.push(format!("mode:{mode:?}"));
                    }
                    OrderedToolbarAction::ToggleGizmoSpace => {
                        probe.trace.push("space".into());
                    }
                    OrderedToolbarAction::ToggleGizmoSnapping => {
                        probe.trace.push("snap".into());
                    }
                },
                OrderedAuthoringInput::PanelCommand(_) => {
                    probe.trace.push("panel".into());
                }
                OrderedAuthoringInput::Selection(selection) => {
                    probe.trace.push(format!(
                        "select:{}",
                        selection.selection.as_deref().unwrap_or("none")
                    ));
                }
                OrderedAuthoringInput::SceneView(action) => match action.action {
                    engine_editor::SceneViewAction::SetPitch(value) => {
                        probe.trace.push(format!("pitch:{value}"));
                    }
                    engine_editor::SceneViewAction::SetYaw(value) => {
                        probe.trace.push(format!("yaw:{value}"));
                    }
                    engine_editor::SceneViewAction::SetDistance(value) => {
                        probe.camera_distance = value;
                        probe.trace.push(format!("distance:{value}"));
                    }
                    engine_editor::SceneViewAction::SetShowGrid(show_grid) => {
                        probe.show_grid = show_grid;
                        probe.trace.push(format!("grid:{show_grid}"));
                    }
                },
            }
        }
        probe
    }

    #[test]
    fn scene_view_camera_and_complete_drag_replay_in_both_platform_orders() {
        let camera_then_drag = probe_ordered_authoring_replay(
            vec![ordered_scene_view_action(
                1,
                engine_editor::SceneViewAction::SetDistance(25.0),
            )],
            complete_ordered_drag(2),
        );
        assert_eq!(camera_then_drag.trace, ["distance:25", "drag:Translate"]);
        assert_eq!(camera_then_drag.completed_drag_camera_distances, [25.0]);

        let drag_then_camera = probe_ordered_authoring_replay(
            vec![ordered_scene_view_action(
                4,
                engine_editor::SceneViewAction::SetDistance(25.0),
            )],
            complete_ordered_drag(1),
        );
        assert_eq!(drag_then_camera.trace, ["drag:Translate", "distance:25"]);
        assert_eq!(drag_then_camera.completed_drag_camera_distances, [10.0]);
        assert_eq!(drag_then_camera.camera_distance, 25.0);
    }

    #[test]
    fn active_drag_keeps_press_camera_when_scene_view_changes_mid_gesture() {
        let probe = probe_ordered_authoring_replay(
            vec![ordered_scene_view_action(
                2,
                engine_editor::SceneViewAction::SetDistance(25.0),
            )],
            vec![
                ordered_gizmo(1, GizmoPointerEvent::Press(Vec2::ZERO)),
                ordered_gizmo(3, GizmoPointerEvent::Release(Vec2::X)),
            ],
        );
        assert_eq!(probe.trace, ["distance:25", "drag:Translate"]);
        assert_eq!(probe.completed_drag_camera_distances, [10.0]);
        assert_eq!(probe.camera_distance, 25.0);
    }

    #[test]
    fn grid_visualization_and_toolbar_replay_in_both_platform_orders() {
        let grid_then_toolbar = probe_ordered_authoring_replay(
            vec![
                ordered_scene_view_action(1, engine_editor::SceneViewAction::SetShowGrid(false)),
                ordered_action(2, OrderedToolbarAction::ToggleGizmoSnapping),
            ],
            Vec::new(),
        );
        assert_eq!(grid_then_toolbar.trace, ["grid:false", "snap"]);
        assert!(!grid_then_toolbar.show_grid);

        let toolbar_then_grid = probe_ordered_authoring_replay(
            vec![
                ordered_action(1, OrderedToolbarAction::ToggleGizmoSnapping),
                ordered_scene_view_action(2, engine_editor::SceneViewAction::SetShowGrid(false)),
            ],
            Vec::new(),
        );
        assert_eq!(toolbar_then_grid.trace, ["snap", "grid:false"]);
        assert!(!toolbar_then_grid.show_grid);
    }

    #[test]
    fn completed_drag_then_undo_replays_in_platform_order() {
        let probe = probe_ordered_authoring_replay(
            vec![ordered_action(4, OrderedToolbarAction::Undo)],
            complete_ordered_drag(1),
        );
        assert_eq!(probe.trace, ["drag:Translate", "undo"]);
        assert_eq!(probe.value, 1);
    }

    #[test]
    fn undo_then_completed_drag_replays_in_platform_order() {
        let probe = probe_ordered_authoring_replay(
            vec![ordered_action(1, OrderedToolbarAction::Undo)],
            complete_ordered_drag(2),
        );
        assert_eq!(probe.trace, ["undo", "drag:Translate"]);
        assert_eq!(probe.value, 1);
    }

    #[test]
    fn undo_can_enable_a_later_redo_in_the_same_redraw() {
        let probe = probe_ordered_authoring_replay(
            vec![
                ordered_action(1, OrderedToolbarAction::Undo),
                ordered_action(2, OrderedToolbarAction::Redo),
            ],
            Vec::new(),
        );
        assert_eq!(probe.trace, ["undo", "redo"]);
        assert_eq!(probe.value, 1);
    }

    #[test]
    fn completed_drag_then_save_captures_post_drag_scene() {
        let probe = probe_ordered_authoring_replay(
            vec![ordered_action(4, OrderedToolbarAction::Save)],
            complete_ordered_drag(1),
        );
        assert_eq!(probe.trace, ["drag:Translate", "save"]);
        assert_eq!(probe.saved_values, [2]);
    }

    #[test]
    fn save_then_completed_drag_leaves_new_change_dirty() {
        let probe = probe_ordered_authoring_replay(
            vec![ordered_action(1, OrderedToolbarAction::Save)],
            complete_ordered_drag(2),
        );
        assert_eq!(probe.trace, ["save", "drag:Translate"]);
        assert_eq!(probe.saved_values, [1]);
        assert_eq!(probe.value, 2);
    }

    #[test]
    fn completed_drag_then_mode_change_uses_old_mode_for_drag() {
        let probe = probe_ordered_authoring_replay(
            vec![ordered_action(
                4,
                OrderedToolbarAction::SetGizmoMode(GizmoMode::Rotate),
            )],
            complete_ordered_drag(1),
        );
        assert_eq!(probe.trace, ["drag:Translate", "mode:Rotate"]);
        assert_eq!(probe.value, 2);
        assert_eq!(probe.mode, GizmoMode::Rotate);
    }

    #[test]
    fn mode_change_then_completed_drag_uses_new_mode() {
        let probe = probe_ordered_authoring_replay(
            vec![ordered_action(
                1,
                OrderedToolbarAction::SetGizmoMode(GizmoMode::Rotate),
            )],
            complete_ordered_drag(2),
        );
        assert_eq!(probe.trace, ["mode:Rotate", "drag:Rotate"]);
        assert_eq!(probe.value, 11);
    }

    #[test]
    fn play_click_is_an_ordered_authoring_barrier() {
        let before_drag = probe_ordered_authoring_replay(
            vec![ordered_action(1, OrderedToolbarAction::StartPlay)],
            complete_ordered_drag(2),
        );
        assert_eq!(before_drag.trace, ["play"]);

        let after_drag = probe_ordered_authoring_replay(
            vec![ordered_action(4, OrderedToolbarAction::StartPlay)],
            complete_ordered_drag(1),
        );
        assert_eq!(after_drag.trace, ["drag:Translate", "play"]);
    }

    #[test]
    fn queued_gizmo_drag_undo_save_reload_roundtrip() {
        let (mut editor_scene, runtime) = gizmo_test_scene_and_runtime();
        let view = runtime_gizmo_view(&runtime, "cube-01", 0, Vec2::splat(800.0)).unwrap();
        let center = project_gizmo_test_point(view, view.world_position);
        let x_tip = project_gizmo_test_point(view, view.world_position + Vec3::X);
        let press = center.lerp(x_tip, 0.55);
        let moved = press + (x_tip - center).normalize() * 60.0;
        let mut gizmo = GizmoSystem::new();
        let ui = EditorUi::new();

        let processed = process_gizmo_pointer_events(
            vec![
                GizmoPointerEvent::Press(press),
                GizmoPointerEvent::Move(moved),
                GizmoPointerEvent::Release(moved),
            ],
            &mut editor_scene,
            &mut gizmo,
            &ui,
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

        let overlay_view = runtime_gizmo_view(&runtime, "cube-01", 1, Vec2::splat(800.0)).unwrap();
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
        let view = runtime_gizmo_view(&runtime, "cube-01", 0, Vec2::splat(800.0)).unwrap();
        let center = project_gizmo_test_point(view, view.world_position);
        let x_tip = project_gizmo_test_point(view, view.world_position + Vec3::X);
        let press = center.lerp(x_tip, 0.55);
        let released = press + (x_tip - center).normalize() * 60.0;
        let mut gizmo = GizmoSystem::new();
        let ui = EditorUi::new();

        assert!(process_gizmo_pointer_events(
            vec![
                GizmoPointerEvent::Press(press),
                GizmoPointerEvent::Release(released),
            ],
            &mut editor_scene,
            &mut gizmo,
            &ui,
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
        let full_view = runtime_gizmo_view(&runtime, "cube-01", 0, Vec2::splat(800.0)).unwrap();
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
            &EditorUi::new(),
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
        let view = runtime_gizmo_view(&runtime, "cube-01", 0, Vec2::splat(800.0)).unwrap();
        let center = project_gizmo_test_point(view, view.world_position);
        let x_tip = project_gizmo_test_point(view, view.world_position + Vec3::X);
        let press = center.lerp(x_tip, 0.5);
        let moved = press + (x_tip - center).normalize() * 50.0;
        let mut gizmo = GizmoSystem::new();
        let ui = EditorUi::new();

        assert!(process_gizmo_pointer_events(
            vec![
                GizmoPointerEvent::Press(press),
                GizmoPointerEvent::Move(moved),
                GizmoPointerEvent::Cancel,
            ],
            &mut editor_scene,
            &mut gizmo,
            &ui,
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
    fn widget_press_does_not_start_gizmo_behind_editor_ui() {
        let (mut editor_scene, runtime) = gizmo_test_scene_and_runtime();
        let view = runtime_gizmo_view(&runtime, "cube-01", 0, Vec2::splat(800.0)).unwrap();
        let center = project_gizmo_test_point(view, view.world_position);
        let x_tip = project_gizmo_test_point(view, view.world_position + Vec3::X);
        let press = center.lerp(x_tip, 0.5);
        let moved = press + (x_tip - center).normalize() * 50.0;
        let mut gizmo = GizmoSystem::new();
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.set_panel_rect(press.x - 8.0, press.y - 8.0, 100.0);
        let _ = ui.button("Blocking Inspector Control");

        assert!(!process_gizmo_pointer_events(
            vec![
                GizmoPointerEvent::Press(press),
                GizmoPointerEvent::Move(moved),
                GizmoPointerEvent::Release(moved),
            ],
            &mut editor_scene,
            &mut gizmo,
            &ui,
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
    }

    #[test]
    fn blank_panel_region_does_not_start_gizmo_behind_editor_ui() {
        let (mut editor_scene, runtime) = gizmo_test_scene_and_runtime();
        let view = runtime_gizmo_view(&runtime, "cube-01", 0, Vec2::splat(800.0)).unwrap();
        let center = project_gizmo_test_point(view, view.world_position);
        let x_tip = project_gizmo_test_point(view, view.world_position + Vec3::X);
        let press = center.lerp(x_tip, 0.5);
        let moved = press + (x_tip - center).normalize() * 50.0;
        let mut gizmo = GizmoSystem::new();
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.block_pointer_rect(press.x - 20.0, press.y - 20.0, 100.0, 100.0);

        assert!(!process_gizmo_pointer_events(
            vec![
                GizmoPointerEvent::Press(press),
                GizmoPointerEvent::Move(moved),
                GizmoPointerEvent::Release(moved),
            ],
            &mut editor_scene,
            &mut gizmo,
            &ui,
            &runtime,
            "cube-01",
            view,
        ));
        assert!(!editor_scene.history.can_undo());
        assert!(!gizmo.dragging);
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

        let view = runtime_gizmo_view(&runtime, "camera-main", 0, Vec2::splat(800.0)).unwrap();
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
        assert!((editor_camera_translation - Vec3::new(25.0, 0.0, 0.0)).length() < 1.0e-5);
    }
}
