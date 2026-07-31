impl EditorApp {
    pub(in crate::editor_app) fn editor_snapshot(&self) -> EditorSnapshot {
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
            terrain: self.terrain_snapshot(scene_records),
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

    pub(in crate::editor_app) fn editor_telemetry(&self) -> EditorTelemetry {
        let scene_records = self
            .editor_scene
            .as_ref()
            .map(|scene| scene.scene.entities.as_slice())
            .unwrap_or_default();
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
            terrain: self.terrain_snapshot(scene_records),
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
}
use super::*;
