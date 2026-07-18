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

impl EditorApp {
    pub(super) fn editor_snapshot(&self) -> EditorSnapshot {
        let scene = self.editor_scene.as_ref();
        let editing = self.play_session.is_editing();
        let dirty = scene.is_some_and(EditorScene::is_dirty);
        let can_undo = scene.is_some_and(|scene| scene.history.can_undo());
        let can_redo = scene.is_some_and(|scene| scene.history.can_redo());
        let selected = scene.and_then(|scene| scene.selected_entity.as_deref());
        let scene_records = scene
            .map(|scene| scene.scene.entities.as_slice())
            .unwrap_or_default();
        let active_scene_name = scene
            .map(|scene| scene.scene.name.clone())
            .unwrap_or_else(|| self.current_scene_id.clone());
        let runtime_mode = match self.play_session.mode() {
            EditorPlayMode::Editing => "edit",
            EditorPlayMode::Playing => "play",
            EditorPlayMode::Paused => "paused",
        };
        let build_busy = self.background_job.is_some() || self.editor_build_task.is_some();
        let authoring_available = editing && !build_busy;
        let telemetry = self.editor_telemetry();

        EditorSnapshot {
            protocol_version: EDITOR_PROTOCOL_VERSION,
            session_id: self.session_id.clone(),
            revision: self.editor_revision,
            project_name: self.project.manifest.name.clone(),
            project_path: self.project.root.display().to_string(),
            active_scene_name,
            scene_dirty: dirty,
            runtime_mode,
            hierarchy: hierarchy_snapshot(scene_records),
            selection: selection_snapshot(scene_records, &self.selected_entity_ids, selected),
            clipboard: ClipboardDto {
                entity_root_count: self
                    .entity_clipboard
                    .as_ref()
                    .map_or(0, |clipboard| clipboard.root_ids().len()),
                component_type: self
                    .component_clipboard
                    .as_ref()
                    .map(|clipboard| clipboard.type_id().clone()),
            },
            assets: self
                .asset_browser
                .catalog_assets()
                .iter()
                .map(asset_snapshot)
                .collect(),
            console: diagnostics_snapshot(scene),
            build_targets: vec![BuildTargetDto {
                id: "windows-x64",
                name: "Windows Desktop",
                platform: "Windows",
                architecture: "x86_64",
                active: true,
            }],
            document: self.document_snapshot(dirty, can_undo, can_redo),
            workspace: self.workspace_snapshot(),
            viewport: self.viewport_snapshot(),
            catalog: self.catalog_snapshot(),
            asset_browser: self.asset_browser_snapshot(),
            material: self.material_snapshot(),
            animation: telemetry.animation,
            build: telemetry.build,
            background_operation: self
                .last_editor_operation
                .as_ref()
                .map(background_operation_snapshot),
            background_operations: self
                .recent_editor_operations
                .iter()
                .map(background_operation_snapshot)
                .collect(),
            settings: SettingsDto {
                window_title: self.project_settings_draft.title.clone(),
                window_width: self.project_settings_draft.width,
                window_height: self.project_settings_draft.height,
                scene_settings: self.scene_settings_draft.clone(),
                camera_entities: scene_records
                    .iter()
                    .filter(|entity| {
                        entity.enabled
                            && entity
                                .components
                                .get("engine.camera")
                                .is_some_and(|camera| camera.enabled)
                    })
                    .map(|entity| EntityOptionDto {
                        id: entity.persistent_id.clone(),
                        name: entity
                            .name
                            .clone()
                            .unwrap_or_else(|| entity.persistent_id.clone()),
                    })
                    .collect(),
                input_map: self
                    .game_loop
                    .as_ref()
                    .map(|game_loop| game_loop.input_map.clone())
                    .unwrap_or_else(|| {
                        engine_gameplay::input::InputActionMap::new("player", "gameplay")
                    }),
            },
            performance: telemetry.performance,
            capabilities: CapabilitiesDto {
                editing,
                has_selection: selected.is_some(),
                can_undo: authoring_available && can_undo,
                can_redo: authoring_available && can_redo,
                can_save: authoring_available && dirty,
                can_start_play: editing && !build_busy,
                can_pause: self.play_session.mode() == EditorPlayMode::Playing,
                can_resume: self.play_session.mode() == EditorPlayMode::Paused,
                can_step: self.play_session.mode() == EditorPlayMode::Paused,
                can_stop: !editing,
                build_busy,
            },
        }
    }

    pub(super) fn editor_telemetry(&self) -> EditorTelemetry {
        EditorTelemetry {
            performance: PerformanceDto {
                current: frame_stats_snapshot(&self.performance.frame_stats),
                history: self
                    .performance
                    .history()
                    .iter()
                    .map(frame_stats_snapshot)
                    .collect(),
            },
            animation: self.animation_snapshot(),
            build: BuildDto {
                active: self.background_job.is_some() || self.editor_build_task.is_some(),
                cancellable: self.editor_build_task.is_some(),
                status: self.build_status.clone(),
                output: self.build_output.clone(),
                package_version: self.package_version.clone(),
                package_output_root: self.package_output_root.clone(),
            },
        }
    }

    fn document_snapshot(&self, dirty: bool, can_undo: bool, can_redo: bool) -> DocumentDto {
        let startup = self.project.startup_scene_id();
        let mut scenes = self
            .project
            .scenes()
            .into_iter()
            .map(|(id, path)| SceneDocumentDto {
                current: id == self.current_scene_id,
                startup: id == startup,
                id,
                path: path.display().to_string(),
            })
            .collect::<Vec<_>>();
        scenes.sort_by(|left, right| left.id.cmp(&right.id));
        DocumentDto {
            current_scene_id: self.current_scene_id.clone(),
            current_scene_path: self.current_scene_path.display().to_string(),
            dirty,
            can_undo,
            can_redo,
            status: self.scene_document_status.clone(),
            pending_switch: self.pending_scene_switch.clone(),
            pending_recovery: self.pending_recovery.is_some(),
            close_confirmation: self.close_confirmation_pending,
            scenes,
        }
    }

    fn workspace_snapshot(&self) -> WorkspaceDto {
        WorkspaceDto {
            react_layout: self
                .workspace_preferences
                .react_layout
                .clone()
                .unwrap_or_else(|| DEFAULT_REACT_LAYOUT.to_string()),
        }
    }

    fn viewport_snapshot(&self) -> ViewportDto {
        let (pitch, yaw, distance) = self.scene_view.camera_orbit();
        ViewportDto {
            scene_camera: SceneCameraDto {
                pitch,
                yaw,
                distance,
                target: *self.scene_view.target(),
                orthographic: self.scene_view.orthographic(),
                speed: self.scene_view.camera_speed(),
            },
            gizmos_visible: self.workspace_preferences.gizmos_visible,
            snapping_enabled: self.gizmo.snapping,
        }
    }

    fn catalog_snapshot(&self) -> CatalogDto {
        CatalogDto {
            components: ComponentCatalog::descriptors()
                .iter()
                .map(|descriptor| ComponentDescriptorDto {
                    type_id: descriptor.type_id.into(),
                    display_name: descriptor.display_name.into(),
                    category: descriptor.category.into(),
                    removable: descriptor.removable,
                    required_components: descriptor
                        .required_components
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                })
                .collect(),
            entity_templates: ComponentCatalog::templates()
                .iter()
                .map(|template| EntityTemplateDto {
                    id: template.id.into(),
                    display_name: template.display_name.into(),
                    category: template.category.into(),
                    component_types: template
                        .component_types
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                })
                .collect(),
            verified_script_classes: self
                .game_loop
                .as_ref()
                .map(|game_loop| game_loop.runtime.verified_script_classes())
                .unwrap_or_default()
                .into_iter()
                .map(|class| ScriptClassDto {
                    assembly_id: class.assembly_id,
                    class_name: class.class_name,
                })
                .collect(),
        }
    }

    fn asset_browser_snapshot(&self) -> AssetBrowserDto {
        AssetBrowserDto {
            query: self.asset_browser.search_query().to_string(),
            folder: self.asset_browser.current_folder().to_string(),
            kind_filter: self.asset_browser.kind_filter().label().to_string(),
            view: match self.workspace_preferences.project_asset_view {
                ProjectAssetView::Grid => "grid",
                ProjectAssetView::List => "list",
            }
            .into(),
            page: self.asset_browser.page(),
            page_size: self.asset_browser.page_size(),
            page_count: self.asset_browser.page_count(),
            total: self.asset_browser.assets().len(),
            visible_asset_ids: self
                .asset_browser
                .visible_assets()
                .iter()
                .map(|asset| asset.id.id.clone())
                .collect(),
            folders: self
                .asset_browser
                .folders()
                .iter()
                .map(|folder| AssetFolderDto {
                    path: folder.path.clone(),
                    name: folder.name.clone(),
                    depth: folder.depth,
                    direct_asset_count: folder.direct_asset_count,
                })
                .collect(),
            selected_asset: self
                .asset_browser
                .selected_asset()
                .map(|asset| asset.id.clone()),
        }
    }

    fn material_snapshot(&self) -> MaterialDto {
        let (writable, read_only_reason) = match self.material_editor.save_access() {
            MaterialSaveAccess::Writable => (true, None),
            MaterialSaveAccess::ReadOnly(reason) => (false, Some(reason.clone())),
        };
        MaterialDto {
            selected_material: self.material_editor.selected_material.clone(),
            parameters: self
                .material_editor
                .shader_params
                .iter()
                .map(|parameter| {
                    let (kind, value) = match parameter.param_type {
                        ShaderParamType::Float => ("float", json!(parameter.float_value)),
                        ShaderParamType::Color => ("color", json!(parameter.color_value)),
                        ShaderParamType::Texture => ("texture", json!(parameter.texture_value)),
                    };
                    MaterialParameterDto {
                        name: parameter.name.clone(),
                        kind,
                        value,
                    }
                })
                .collect(),
            writable,
            read_only_reason,
            save_status: self.material_editor.save_status().map(str::to_string),
        }
    }

    fn animation_snapshot(&self) -> AnimationDto {
        AnimationDto {
            available_skeletons: self.animation_preview.available_skeletons.clone(),
            available_clips: self.animation_preview.available_clips.clone(),
            selected_skeleton: self.animation_preview.selected_skeleton.clone(),
            selected_clip: self.animation_preview.selected_clip.clone(),
            playback_time: self.animation_preview.playback_time,
            duration: self
                .animation_preview
                .clip_info()
                .map_or(0.0, |info| info.duration),
            playing: self.animation_preview.playing,
            looping: self.animation_preview.looping,
            speed: self.animation_preview.speed,
            events: self
                .animation_preview
                .events
                .iter()
                .map(|event| AnimationEventDto {
                    time: event.time,
                    name: event.name.clone(),
                })
                .collect(),
        }
    }
}

fn hierarchy_snapshot(entities: &[EntityRecord]) -> Vec<HierarchyNodeDto> {
    fn children(
        entities: &[EntityRecord],
        parent: Option<&str>,
        visiting: &mut BTreeSet<String>,
    ) -> Vec<HierarchyNodeDto> {
        entities
            .iter()
            .filter(|entity| entity.parent.as_deref() == parent)
            .filter_map(|entity| {
                if !visiting.insert(entity.persistent_id.clone()) {
                    return None;
                }
                let children = children(entities, Some(&entity.persistent_id), visiting);
                visiting.remove(&entity.persistent_id);
                Some(HierarchyNodeDto {
                    id: entity.persistent_id.clone(),
                    name: entity
                        .name
                        .clone()
                        .unwrap_or_else(|| entity.persistent_id.clone()),
                    enabled: entity.enabled,
                    expanded: true,
                    prefab: prefab_asset(entity),
                    children,
                })
            })
            .collect()
    }

    children(entities, None, &mut BTreeSet::new())
}

fn prefab_asset(entity: &EntityRecord) -> Option<String> {
    let component = entity.components.get(PrefabInstanceRef::TYPE_ID)?;
    match component.fields.get("source_asset")? {
        Value::Asset(asset) => Some(asset.id.clone()),
        Value::Str(asset) => Some(asset.clone()),
        _ => None,
    }
}

fn selection_snapshot(
    entities: &[EntityRecord],
    selected_ids: &[String],
    selected: Option<&str>,
) -> SelectionDto {
    let Some(selected) = selected else {
        return SelectionDto::default();
    };
    let Some(entity) = entities
        .iter()
        .find(|entity| entity.persistent_id == selected)
    else {
        return SelectionDto::default();
    };
    let mut entity_ids = selected_ids
        .iter()
        .filter(|id| entities.iter().any(|entity| &entity.persistent_id == *id))
        .cloned()
        .collect::<Vec<_>>();
    if !entity_ids.iter().any(|id| id == selected) {
        entity_ids.push(entity.persistent_id.clone());
    }
    let selected_entities = entity_ids
        .iter()
        .filter_map(|id| {
            entities
                .iter()
                .find(|candidate| &candidate.persistent_id == id)
        })
        .collect::<Vec<_>>();
    let transform = selected_entities
        .iter()
        .all(|selected_entity| selected_entity.components.contains_key("engine.transform"))
        .then(|| transform_snapshot(entity))
        .flatten();
    let components = entity
        .components
        .iter()
        .filter(|(type_id, _)| {
            selected_entities
                .iter()
                .all(|selected_entity| selected_entity.components.contains_key(*type_id))
        })
        .map(|(type_id, component)| {
            let descriptor = ComponentCatalog::descriptor(type_id);
            ComponentDto {
                type_id: type_id.clone(),
                display_name: descriptor
                    .map(|descriptor| descriptor.display_name.to_string())
                    .unwrap_or_else(|| type_id.clone()),
                enabled: component.enabled,
                removable: descriptor.is_none_or(|descriptor| descriptor.removable),
                resettable: descriptor.is_some(),
                remove_blocked_reason: selected_entities.iter().find_map(|selected_entity| {
                    selected_entity
                        .components
                        .keys()
                        .find_map(|dependent_type| {
                            let dependent = ComponentCatalog::descriptor(dependent_type)?;
                            dependent
                                .required_components
                                .contains(&type_id.as_str())
                                .then(|| {
                                    format!("{} requires this component", dependent.display_name)
                                })
                        })
                }),
                fields: component
                    .fields
                    .iter()
                    .map(|(name, value)| component_field_snapshot(type_id, name, value))
                    .collect(),
            }
        })
        .collect();
    SelectionDto {
        entity_ids,
        active_entity_id: Some(entity.persistent_id.clone()),
        display_name: Some(
            entity
                .name
                .clone()
                .unwrap_or_else(|| entity.persistent_id.clone()),
        ),
        active: Some(entity.enabled),
        transform,
        components,
    }
}

fn transform_snapshot(entity: &EntityRecord) -> Option<TransformDto> {
    let transform = entity.components.get("engine.transform")?;
    let position = match transform.fields.get("translation")? {
        Value::Vec3(value) => *value,
        _ => return None,
    };
    let rotation = match transform.fields.get("rotation")? {
        Value::Quat(value) => *value,
        _ => return None,
    };
    let scale = match transform.fields.get("scale")? {
        Value::Vec3(value) => *value,
        _ => return None,
    };
    let (x, y, z) = Quat::from_array(rotation).to_euler(EulerRot::XYZ);
    Some(TransformDto {
        position: position.into(),
        rotation_euler: Vec3Dto::from([x.to_degrees(), y.to_degrees(), z.to_degrees()]),
        scale: scale.into(),
    })
}

fn component_field_snapshot(component_type: &str, name: &str, value: &Value) -> ComponentFieldDto {
    let (value_type, plain_value) = match value {
        Value::Bool(value) => ("boolean", json!(value)),
        Value::Int(value) => ("number", json!(value.to_string())),
        Value::UInt(value) => ("number", json!(value.to_string())),
        Value::Float32(value) => ("number", json!(value)),
        Value::Float64(value) => ("number", json!(value)),
        Value::Str(value) => ("string", json!(value)),
        Value::Vec3(value) => ("vec3", json!(Vec3Dto::from(*value))),
        Value::Quat(value) => ("vec4", json!(value)),
        Value::Color(value) => ("color", json!(value)),
        Value::Asset(value) => ("asset", json!(value.id)),
        Value::Entity(value) => ("string", json!(value)),
        Value::Enum(value) => ("enum", json!(value)),
        Value::List(value) => (
            "list",
            serde_json::to_value(value).unwrap_or(JsonValue::Null),
        ),
        Value::Map(value) => (
            "map",
            serde_json::to_value(value).unwrap_or(JsonValue::Null),
        ),
    };
    ComponentFieldDto {
        path: name.to_string(),
        label: humanize(name),
        value: plain_value,
        value_type,
        engine_value: value.clone(),
        accepted_asset_kinds: match (component_type, name) {
            ("engine.renderable", "mesh") => &["model"],
            ("engine.renderable", "material") => &["material"],
            ("engine.audio_source", "clip_asset") => &["audio"],
            ("engine.nav_agent", "navmesh_ref") => &["navmesh"],
            _ => &[],
        },
    }
}

fn humanize(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut uppercase = true;
    for character in value.chars() {
        if matches!(character, '_' | '-') {
            output.push(' ');
            uppercase = true;
        } else if uppercase {
            output.extend(character.to_uppercase());
            uppercase = false;
        } else {
            output.push(character);
        }
    }
    output
}

fn asset_snapshot(entry: &AssetEntry) -> AssetDto {
    AssetDto {
        id: entry.id.id.clone(),
        asset_id: entry.id.clone(),
        name: entry.display_name(),
        path: entry
            .browser_path()
            .map(str::to_string)
            .unwrap_or_else(|| entry.id.id.clone()),
        kind: asset_kind_name(entry.kind),
        loaded: entry.loaded,
        cooked: entry.cooked,
        manifest_declared: entry.manifest_declared,
    }
}

fn asset_kind_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Scene => "scene",
        AssetKind::Prefab => "prefab",
        AssetKind::Mesh | AssetKind::Skeleton | AssetKind::Animation => "model",
        AssetKind::Material => "material",
        AssetKind::Texture => "texture",
        AssetKind::Audio => "audio",
        AssetKind::Script => "script",
        AssetKind::Shader | AssetKind::Pipeline => "shader",
        AssetKind::NavMesh => "navmesh",
        AssetKind::Font | AssetKind::Logic | AssetKind::Unknown => "other",
    }
}

fn diagnostics_snapshot(scene: Option<&EditorScene>) -> Vec<ConsoleEntryDto> {
    scene
        .into_iter()
        .flat_map(|scene| scene.diagnostics.all_entries().iter())
        .enumerate()
        .map(|(index, entry)| {
            let diagnostic = &entry.diagnostic;
            ConsoleEntryDto {
                id: format!("{index}:{}", diagnostic.code),
                timestamp: format!("-{:.3}s", entry.timestamp.elapsed().as_secs_f32()),
                level: match diagnostic.severity {
                    DiagnosticSeverity::Info => "info",
                    DiagnosticSeverity::Warning => "warning",
                    DiagnosticSeverity::Error | DiagnosticSeverity::Fatal => "error",
                },
                source: diagnostic.system.clone(),
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
                path: diagnostic.path.clone(),
                entity: diagnostic.entity.clone(),
                suggested_action: diagnostic.suggested_action.clone(),
            }
        })
        .collect()
}

fn frame_stats_snapshot(stats: &engine_editor::performance::FrameStats) -> FrameStatsDto {
    FrameStatsDto {
        frame_time_ms: stats.frame_time_ms,
        draw_calls: stats.draw_calls,
        triangles: stats.triangles,
        physics_bodies: stats.physics_bodies,
        animation_count: stats.animation_count,
        nav_agents: stats.nav_agents,
        asset_count: stats.asset_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hierarchy_snapshot_preserves_parent_order_and_nesting() {
        let scene = engine_scene::sample_scene();
        let snapshot = hierarchy_snapshot(&scene.entities);
        assert!(!snapshot.is_empty());
        let ids = snapshot
            .iter()
            .map(|entity| entity.id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), snapshot.len());
    }

    #[test]
    fn multi_selection_exposes_only_components_shared_by_every_entity() {
        let scene = engine_scene::sample_scene();
        let first = &scene.entities[0];
        let second = scene
            .entities
            .iter()
            .skip(1)
            .find(|entity| entity.components.keys().ne(first.components.keys()))
            .expect("sample scene must contain entities with different components");
        let selected_ids = vec![first.persistent_id.clone(), second.persistent_id.clone()];

        let snapshot = selection_snapshot(
            &scene.entities,
            &selected_ids,
            Some(first.persistent_id.as_str()),
        );
        let expected = first
            .components
            .keys()
            .filter(|type_id| second.components.contains_key(*type_id))
            .cloned()
            .collect::<BTreeSet<_>>();
        let actual = snapshot
            .components
            .iter()
            .map(|component| component.type_id.clone())
            .collect::<BTreeSet<_>>();

        assert_eq!(snapshot.entity_ids, selected_ids);
        assert_eq!(actual, expected);
    }

    #[test]
    fn integer_inspector_values_keep_full_precision() {
        let field = component_field_snapshot("engine.camera", "mask", &Value::UInt(u64::MAX));
        assert_eq!(field.value, json!(u64::MAX.to_string()));
        assert_eq!(field.engine_value, Value::UInt(u64::MAX));
    }

    #[test]
    fn react_asset_fields_expose_compatible_kinds_and_complete_asset_ids() {
        let mesh = component_field_snapshot(
            "engine.renderable",
            "mesh",
            &Value::Asset(AssetId::with_path("hero", "models/hero.glb")),
        );
        let material = component_field_snapshot(
            "engine.renderable",
            "material",
            &Value::Asset(AssetId::new("hero-material")),
        );
        assert_eq!(mesh.accepted_asset_kinds, &["model"]);
        assert_eq!(material.accepted_asset_kinds, &["material"]);

        let value = serde_json::to_value(AssetDto {
            id: "hero".into(),
            asset_id: AssetId::with_path("hero", "models/hero.glb"),
            name: "Hero".into(),
            path: "models/hero.glb".into(),
            kind: "model",
            loaded: true,
            cooked: true,
            manifest_declared: true,
        })
        .unwrap();
        assert_eq!(value["assetId"]["id"], json!("hero"));
        assert_eq!(value["assetId"]["logical_path"], json!("models/hero.glb"));
    }

    #[test]
    fn every_asset_kind_has_a_stable_react_category() {
        let kinds = [
            AssetKind::Mesh,
            AssetKind::Texture,
            AssetKind::Shader,
            AssetKind::Scene,
            AssetKind::Material,
            AssetKind::Pipeline,
            AssetKind::Script,
            AssetKind::Audio,
            AssetKind::Font,
            AssetKind::Animation,
            AssetKind::Skeleton,
            AssetKind::NavMesh,
            AssetKind::Logic,
            AssetKind::Prefab,
            AssetKind::Unknown,
        ];
        assert!(kinds.iter().all(|kind| !asset_kind_name(*kind).is_empty()));
    }

    #[test]
    fn viewport_snapshot_exposes_complete_scene_camera_and_gizmo_state() {
        let viewport = ViewportDto {
            scene_camera: SceneCameraDto {
                pitch: 20.0,
                yaw: 45.0,
                distance: 10.0,
                target: [1.0, 2.0, 3.0],
                orthographic: false,
                speed: 5.0,
            },
            gizmos_visible: true,
            snapping_enabled: true,
        };
        let value = serde_json::to_value(viewport).unwrap();
        assert_eq!(value["sceneCamera"]["target"], json!([1.0, 2.0, 3.0]));
        assert_eq!(value["sceneCamera"]["speed"], json!(5.0));
        assert_eq!(value["gizmosVisible"], json!(true));
        assert_eq!(value["snappingEnabled"], json!(true));
    }
}
