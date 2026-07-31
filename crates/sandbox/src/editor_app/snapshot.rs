//! Serializable, UI-framework-independent editor state snapshots.

use std::collections::BTreeSet;

use engine_editor::asset_browser::{AssetEntry, AssetKind};
use engine_editor::component_catalog::ComponentCatalog;
use engine_editor::material_editor::{MaterialSaveAccess, ShaderParamType};
use engine_scene::{Component, EntityRecord, PrefabInstanceRef};
use engine_serialize::{DiagnosticSeverity, Value};
use glam::{EulerRot, Quat};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};

use super::protocol::EDITOR_PROTOCOL_VERSION;
use super::*;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorSnapshot {
    pub protocol_version: u32,
    pub session_id: String,
    pub revision: u64,
    // Kept at the top level so the first React shell can consume the snapshot
    // without an adapter while richer domain panels use the nested sections.
    pub project_name: String,
    pub project_path: String,
    pub active_scene_name: String,
    pub scene_dirty: bool,
    pub runtime_mode: &'static str,
    pub hierarchy: Vec<HierarchyNodeDto>,
    pub selection: SelectionDto,
    pub clipboard: ClipboardDto,
    pub assets: Vec<AssetDto>,
    pub console: Vec<ConsoleEntryDto>,
    pub build_targets: Vec<BuildTargetDto>,
    pub document: DocumentDto,
    pub workspace: WorkspaceDto,
    pub viewport: ViewportDto,
    pub catalog: CatalogDto,
    pub asset_browser: AssetBrowserDto,
    pub material: MaterialDto,
    pub animation: AnimationDto,
    pub build: BuildDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_operation: Option<BackgroundOperationDto>,
    pub background_operations: Vec<BackgroundOperationDto>,
    pub settings: SettingsDto,
    pub performance: PerformanceDto,
    pub terrain: TerrainDto,
    pub capabilities: CapabilitiesDto,
}

/// Complete high-frequency domains sent between authoritative project snapshots.
///
/// These are replacements, not field-level deltas, so dropping an older telemetry event is safe.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorTelemetry {
    pub performance: PerformanceDto,
    pub animation: AnimationDto,
    pub build: BuildDto,
    pub terrain: TerrainDto,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyNodeDto {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub expanded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefab: Option<String>,
    pub children: Vec<HierarchyNodeDto>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionDto {
    pub entity_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<TransformDto>,
    pub components: Vec<ComponentDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardDto {
    pub entity_root_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_type: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformDto {
    pub position: Vec3Dto,
    pub rotation_euler: Vec3Dto,
    pub scale: Vec3Dto,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Vec3Dto {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl From<[f32; 3]> for Vec3Dto {
    fn from(value: [f32; 3]) -> Self {
        Self {
            x: value[0],
            y: value[1],
            z: value[2],
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDto {
    pub type_id: String,
    pub display_name: String,
    pub enabled: bool,
    pub removable: bool,
    pub resettable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remove_blocked_reason: Option<String>,
    pub fields: Vec<ComponentFieldDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentFieldDto {
    pub path: String,
    pub label: String,
    pub value: JsonValue,
    pub value_type: &'static str,
    pub engine_value: Value,
    pub accepted_asset_kinds: &'static [&'static str],
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDto {
    pub id: String,
    pub asset_id: AssetId,
    pub name: String,
    pub path: String,
    pub kind: &'static str,
    pub loaded: bool,
    pub cooked: bool,
    pub manifest_declared: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleEntryDto {
    pub id: String,
    pub timestamp: String,
    pub level: &'static str,
    pub source: String,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildTargetDto {
    pub id: &'static str,
    pub name: &'static str,
    pub platform: &'static str,
    pub architecture: &'static str,
    pub active: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentDto {
    pub current_scene_id: String,
    pub current_scene_path: String,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_switch: Option<String>,
    pub pending_recovery: bool,
    pub close_confirmation: bool,
    pub scenes: Vec<SceneDocumentDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneDocumentDto {
    pub id: String,
    pub path: String,
    pub startup: bool,
    pub current: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDto {
    pub react_layout: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportDto {
    pub scene_camera: SceneCameraDto,
    pub gizmos_visible: bool,
    pub snapping_enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneCameraDto {
    pub pitch: f32,
    pub yaw: f32,
    pub distance: f32,
    pub target: [f32; 3],
    pub orthographic: bool,
    pub speed: f32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDto {
    pub components: Vec<ComponentDescriptorDto>,
    pub entity_templates: Vec<EntityTemplateDto>,
    pub verified_script_classes: Vec<ScriptClassDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDescriptorDto {
    pub type_id: String,
    pub display_name: String,
    pub category: String,
    pub removable: bool,
    pub required_components: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityTemplateDto {
    pub id: String,
    pub display_name: String,
    pub category: String,
    pub component_types: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptClassDto {
    pub assembly_id: String,
    pub class_name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetBrowserDto {
    pub query: String,
    pub folder: String,
    pub kind_filter: String,
    pub view: String,
    pub page: usize,
    pub page_size: usize,
    pub page_count: usize,
    pub total: usize,
    pub visible_asset_ids: Vec<String>,
    pub folders: Vec<AssetFolderDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_asset: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetFolderDto {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub direct_asset_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_material: Option<String>,
    pub parameters: Vec<MaterialParameterDto>,
    pub writable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_status: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialParameterDto {
    pub name: String,
    pub kind: &'static str,
    pub value: JsonValue,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimationDto {
    pub available_skeletons: Vec<String>,
    pub available_clips: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_skeleton: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_clip: Option<String>,
    pub playback_time: f32,
    pub duration: f32,
    pub playing: bool,
    pub looping: bool,
    pub speed: f32,
    pub events: Vec<AnimationEventDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimationEventDto {
    pub time: f32,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildDto {
    pub active: bool,
    pub cancellable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub output: String,
    pub package_version: String,
    pub package_output_root: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundOperationDto {
    pub id: u64,
    pub label: String,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn background_operation_snapshot(operation: &EditorOperationStatus) -> BackgroundOperationDto {
    let (state, error) = match &operation.state {
        EditorOperationState::Running => ("running", None),
        EditorOperationState::Succeeded => ("succeeded", None),
        EditorOperationState::CommittedWithWarning(warning) => {
            ("committedWithWarning", Some(warning.clone()))
        }
        EditorOperationState::Failed(error) => ("failed", Some(error.clone())),
    };
    BackgroundOperationDto {
        id: operation.id,
        label: operation.label.clone(),
        state,
        error,
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub window_title: String,
    pub window_width: u32,
    pub window_height: u32,
    pub scene_settings: engine_scene::SceneSettings,
    pub camera_entities: Vec<EntityOptionDto>,
    pub input_map: engine_gameplay::input::InputActionMap,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityOptionDto {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceDto {
    pub current: FrameStatsDto,
    pub history: Vec<FrameStatsDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerrainDto {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    pub enabled: bool,
    /// Decimal string preserves all u64 seed bits in the React shell.
    pub seed: String,
    pub chunk_size: f32,
    pub base_resolution: u32,
    pub height_scale: f32,
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub domain_warp_amplitude: f32,
    pub domain_warp_frequency: f32,
    pub skirt_depth: f32,
    pub collision_enabled: bool,
    pub lod_distances: Vec<f32>,
    pub lod_hysteresis: f32,
    pub runtime: TerrainRuntimeStatsDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerrainRuntimeStatsDto {
    pub queued: usize,
    pub generating: usize,
    pub ready_to_commit: usize,
    pub resident: usize,
    pub failed: usize,
    pub resident_bytes: usize,
    pub stale_results_discarded: u64,
    pub cancelled: u64,
    pub generated: u64,
    pub committed: u64,
    pub evicted: u64,
    pub last_tick_committed_bytes: usize,
    pub last_generation_micros: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameStatsDto {
    pub frame_time_ms: f32,
    pub draw_calls: u32,
    pub triangles: u32,
    pub physics_bodies: u32,
    pub animation_count: u32,
    pub nav_agents: u32,
    pub asset_count: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesDto {
    pub editing: bool,
    pub has_selection: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub can_save: bool,
    pub can_start_play: bool,
    pub can_pause: bool,
    pub can_resume: bool,
    pub can_step: bool,
    pub can_stop: bool,
    pub build_busy: bool,
}

mod helpers;
mod overview;
mod panels;

#[cfg(test)]
use helpers::{asset_kind_name, component_field_snapshot};
use helpers::{
    asset_snapshot, diagnostics_snapshot, frame_stats_snapshot, hierarchy_snapshot,
    selection_snapshot,
};

#[cfg(test)]
mod tests {
    include!("snapshot/tests.rs");
}
