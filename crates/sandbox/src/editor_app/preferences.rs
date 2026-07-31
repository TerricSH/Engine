use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProjectAssetView {
    #[default]
    Grid,
    List,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct EditorWorkspacePreferences {
    pub(super) scene_pitch: f32,
    pub(super) scene_yaw: f32,
    pub(super) scene_distance: f32,
    pub(super) scene_target: [f32; 3],
    pub(super) scene_orthographic: bool,
    pub(super) scene_camera_speed: f32,
    pub(super) gizmos_visible: bool,
    pub(super) snapping_enabled: bool,
    pub(super) project_asset_view: ProjectAssetView,
    pub(super) project_asset_folder: String,
    pub(super) react_layout: Option<String>,
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

pub(super) fn workspace_preferences_path(project: &GameProject) -> PathBuf {
    project.root.join(".engine/editor-workspace.json")
}

pub(super) fn scene_recovery_path(project: &GameProject, scene_id: &str) -> PathBuf {
    project
        .root
        .join(".engine/recovery")
        .join(format!("{scene_id}.scene.ron"))
}

pub(super) fn newer_recovery_snapshot(
    project: &GameProject,
    scene_id: &str,
    scene_path: &Path,
) -> Option<PathBuf> {
    let recovery = scene_recovery_path(project, scene_id);
    let recovery_modified = std::fs::metadata(&recovery).ok()?.modified().ok()?;
    let scene_modified = std::fs::metadata(scene_path).ok()?.modified().ok()?;
    (recovery_modified > scene_modified).then_some(recovery)
}

pub(super) fn load_workspace_preferences(project: &GameProject) -> EditorWorkspacePreferences {
    let path = workspace_preferences_path(project);
    let Ok(bytes) = std::fs::read(&path) else {
        return EditorWorkspacePreferences::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        tracing::warn!(path = %path.display(), %error, "ignored invalid editor workspace preferences");
        EditorWorkspacePreferences::default()
    })
}

pub(super) fn save_workspace_preferences(
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
