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
use engine_core::{EngineConfig, EngineRuntime};
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
use render_vulkan::create_backend_renderer_for_surface;

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
    let mut command = {
        let system_root = std::env::var_os("SystemRoot")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                "Could not locate Windows Explorer because SystemRoot is unset".to_string()
            })?;
        let explorer = PathBuf::from(system_root).join("explorer.exe");
        if !explorer.is_file() {
            return Err(format!(
                "Could not locate Windows Explorer at {}",
                explorer.display()
            ));
        }
        std::process::Command::new(explorer)
    };
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
        .current_dir(&project.root)
        .spawn()
        .map(|child| child.id())
        .map_err(|error| {
            format!(
                "Could not launch an editor for {}: {error}",
                project.manifest_path.display()
            )
        })
}

mod material_io;
mod viewport;

use material_io::*;
use viewport::*;

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

mod preferences;
mod state;

use preferences::*;
use state::*;

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
            package_output_root: "Dist".to_string(),
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
}

mod commands;
mod documents;
mod frame;
mod jobs;
mod play;
mod viewport_runtime;
mod workspace;

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
                surface,
                size,
                scale_factor,
            } => {
                match self.initialize_native_surface(surface, size.width, size.height, scale_factor)
                {
                    Ok(()) => HostDirective::RequestRedraw,
                    Err(error) => {
                        tracing::error!(%error, "editor native surface initialization failed");
                        HostDirective::Exit
                    }
                }
            }
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
                let messages = self.take_frame_bridge_messages(self.frame.is_multiple_of(12));
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
    let script_deployment_error =
        super::project_scripts::deploy_installed_project_script_runtime(&project)
            .err()
            .map(|error| {
                tracing::error!(
                    path = %project_path.display(),
                    %error,
                    "installed project script runtime deployment failed"
                );
                error
            });
    if let Err(error) = super::project_cli::cook_project(&project_path) {
        tracing::error!(path = %project_path.display(), %error, "editor project cook failed");
        std::process::exit(2);
    }
    let title = format!("{} - Engine Editor", project.manifest.name);
    let mut app = EditorApp::new(project);
    if let Some(error) = script_deployment_error {
        app.build_status = Some(format!(
            "Project opened, but the installed script SDK could not be deployed: {error}"
        ));
    }
    let config = EditorHostConfig::new(title, EDITOR_WEB_ASSETS)
        .with_initial_size(1600, 900)
        .with_minimum_size(1120, 680);
    #[cfg(debug_assertions)]
    let config = match std::env::var("ENGINE_EDITOR_WEB_DEV_URL") {
        Ok(url) => {
            tracing::info!(%url, "loading React editor from the loopback development server");
            config.with_development_url(url)
        }
        Err(_) => config,
    };
    if let Err(e) = engine_editor_host::run_editor_host(config, app) {
        tracing::error!("editor: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;
