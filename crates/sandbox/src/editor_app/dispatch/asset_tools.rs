//! Asset, material, animation, diagnostics, and build helper operations.

use super::*;

impl EditorApp {
    pub(super) fn frame_selected(&mut self) -> Result<(), BridgeError> {
        let target = self
            .editor_scene
            .as_ref()
            .and_then(|scene| {
                let selected = scene.selected_entity.as_ref()?;
                let entity = scene
                    .scene
                    .entities
                    .iter()
                    .find(|entity| &entity.persistent_id == selected)?;
                match entity
                    .components
                    .get("engine.transform")?
                    .fields
                    .get("translation")?
                {
                    Value::Vec3(value) => Some(*value),
                    _ => None,
                }
            })
            .ok_or_else(selection_error)?;
        let (pitch, yaw, _) = self.scene_view.camera_orbit();
        self.scene_view.set_target(target);
        self.scene_view.set_camera_orbit(pitch, yaw, 6.0);
        Ok(())
    }

    pub(super) fn set_asset_browser_state(
        &mut self,
        params: AssetBrowserParams,
    ) -> Result<(), BridgeError> {
        if let Some(query) = params.query {
            self.asset_browser.set_search_query(query);
        }
        if let Some(folder) = params.folder {
            self.asset_browser.set_current_folder(folder);
        }
        if let Some(kind) = params.kind {
            let filter = match kind.to_ascii_lowercase().as_str() {
                "all" => AssetKindFilter::All,
                "mesh" | "model" => AssetKindFilter::Mesh,
                "texture" => AssetKindFilter::Texture,
                "shader" => AssetKindFilter::Shader,
                "scene" => AssetKindFilter::Scene,
                "material" => AssetKindFilter::Material,
                "pipeline" => AssetKindFilter::Pipeline,
                "script" => AssetKindFilter::Script,
                "audio" => AssetKindFilter::Audio,
                "font" => AssetKindFilter::Font,
                "animation" => AssetKindFilter::Animation,
                "skeleton" => AssetKindFilter::Skeleton,
                "navmesh" => AssetKindFilter::NavMesh,
                "logic" => AssetKindFilter::Logic,
                "prefab" => AssetKindFilter::Prefab,
                "unknown" | "other" => AssetKindFilter::Unknown,
                _ => {
                    return Err(BridgeError::new(
                        EditorErrorCode::InvalidRequest,
                        format!("Unknown asset kind filter '{kind}'"),
                    ));
                }
            };
            self.asset_browser.set_kind_filter(filter);
        }
        if let Some(page) = params.page {
            while self.asset_browser.page() < page && self.asset_browser.next_page() {}
            while self.asset_browser.page() > page && self.asset_browser.previous_page() {}
        }
        if let Some(view) = params.view {
            self.workspace_preferences.project_asset_view = match view.as_str() {
                "grid" => ProjectAssetView::Grid,
                "list" => ProjectAssetView::List,
                _ => {
                    return Err(BridgeError::new(
                        EditorErrorCode::InvalidRequest,
                        format!("Unknown asset browser view '{view}'"),
                    ));
                }
            };
        }
        self.workspace_preferences.project_asset_folder =
            self.asset_browser.current_folder().to_string();
        Ok(())
    }

    pub(super) fn asset_path(&self, asset_id: &str) -> Result<PathBuf, BridgeError> {
        let entry = self
            .asset_browser
            .catalog_assets()
            .iter()
            .find(|entry| entry.id.id == asset_id)
            .ok_or_else(|| not_found("asset", asset_id))?;
        if let Some(source) = entry.source_path.as_deref() {
            return Ok(self.project.asset_source.join(source));
        }
        Ok(self.project.root.clone())
    }

    pub(super) fn open_material(&mut self, material: String) {
        if let Some(game_loop) = self.game_loop.as_ref() {
            load_material(
                &mut self.material_editor,
                &material,
                game_loop.runtime.asset_registry(),
            );
        }
        self.material_editor
            .set_save_access(project_material_save_access(&self.project, &material));
        self.material_editor_selection = Some(material);
        self.request_ui_open_panel(UiPanel::Material, UiDockZone::Bottom);
    }

    pub(super) fn set_material_parameter(
        &mut self,
        params: MaterialParameterParams,
    ) -> Result<(), BridgeError> {
        let parameter = self
            .material_editor
            .shader_params
            .iter_mut()
            .find(|parameter| parameter.name == params.name)
            .ok_or_else(|| not_found("material parameter", &params.name))?;
        match parameter.param_type {
            ShaderParamType::Float => {
                parameter.float_value = serde_json::from_value(params.value)
                    .map_err(|error| validation_error(error.to_string()))?;
            }
            ShaderParamType::Color => {
                parameter.color_value = serde_json::from_value(params.value)
                    .map_err(|error| validation_error(error.to_string()))?;
            }
            ShaderParamType::Texture => {
                parameter.texture_value = serde_json::from_value(params.value)
                    .map_err(|error| validation_error(error.to_string()))?;
            }
            ShaderParamType::Choice => {
                let value: String = serde_json::from_value(params.value)
                    .map_err(|error| validation_error(error.to_string()))?;
                if !parameter
                    .choice_options
                    .iter()
                    .any(|option| option == &value)
                {
                    return Err(validation_error(format!(
                        "material parameter '{}' must be one of: {}",
                        parameter.name,
                        parameter.choice_options.join(", ")
                    )));
                }
                parameter.choice_value = value;
            }
            ShaderParamType::Bool => {
                parameter.bool_value = serde_json::from_value(params.value)
                    .map_err(|error| validation_error(error.to_string()))?;
            }
        }
        Ok(())
    }

    pub(super) fn assign_open_material(&mut self) -> Result<(), BridgeError> {
        let material = self
            .material_editor
            .selected_material
            .clone()
            .ok_or_else(|| not_found("open material", "selection"))?;
        let loaded = self.game_loop.as_ref().is_some_and(|game_loop| {
            game_loop
                .runtime
                .asset_registry()
                .get::<engine_renderer::MaterialUpload>(&AssetId::new(&material))
                .is_some()
        });
        if !loaded {
            return Err(BridgeError::new(
                EditorErrorCode::ValidationFailed,
                format!("Material '{material}' is not loaded; reimport it before assignment"),
            ));
        }
        let editor_scene = self.editor_scene.as_ref().ok_or_else(runtime_unavailable)?;
        let command = assign_material_to_selected_command(editor_scene, &material)
            .map_err(validation_error)?;
        self.execute_command(command)
    }

    pub(super) fn set_animation_state(&mut self, params: AnimationParams) {
        if let Some(skeleton) = params.skeleton {
            self.animation_preview.selected_skeleton = skeleton;
        }
        if let Some(clip) = params.clip {
            self.animation_preview.selected_clip = clip;
        }
        if let Some(playing) = params.playing {
            self.animation_preview.playing = playing;
        }
        if let Some(looping) = params.looping {
            self.animation_preview.looping = looping;
        }
        if let Some(speed) = params.speed {
            self.animation_preview.speed = speed.clamp(0.05, 4.0);
        }
        if let Some(time) = params.time {
            let duration = self
                .animation_preview
                .clip_info()
                .map_or(f32::MAX, |info| info.duration);
            self.animation_preview.playback_time = time.clamp(0.0, duration);
        }
    }

    pub(super) fn export_diagnostics(&mut self) -> Result<(), BridgeError> {
        let output = self.project.root.join(".engine/logs/editor-console.txt");
        let contents = self
            .editor_scene
            .as_ref()
            .map(|scene| {
                scene
                    .diagnostics
                    .all_entries()
                    .iter()
                    .map(|entry| {
                        format!(
                            "{:?} [{}] {}: {}",
                            entry.diagnostic.severity,
                            entry.diagnostic.code,
                            entry.diagnostic.system,
                            entry.diagnostic.message
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        atomic_write_file(&output, contents.as_bytes()).map_err(io_error)?;
        self.build_status = Some(format!("Console exported to {}.", output.display()));
        Ok(())
    }

    pub(super) fn start_build_request(&mut self, params: BuildParams) -> Result<(), BridgeError> {
        let operation = params
            .operation
            .unwrap_or_else(|| match params.target_id.as_deref() {
                Some("validate") => BuildOperation::Validate,
                Some("windows-x64") if params.version.is_some() => BuildOperation::PackageWindows,
                _ => BuildOperation::CookAndCompile,
            });
        self.run_after_build = params.run_after_build;
        match operation {
            BuildOperation::Validate => self.start_editor_build(
                super::super::super::editor_build_ops::EditorBuildOperation::Validate,
            ),
            BuildOperation::CookAndCompile => self.start_editor_build(
                super::super::super::editor_build_ops::EditorBuildOperation::CookAndCompile,
            ),
            BuildOperation::PackageWindows => {
                let version = params
                    .version
                    .unwrap_or_else(|| self.package_version.clone());
                let output = params
                    .output_root
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(&self.package_output_root));
                self.start_editor_build(
                    super::super::super::editor_build_ops::EditorBuildOperation::PackageWindows(
                        super::super::super::editor_build_ops::PackageWindowsOptions::new(
                            version, output,
                        ),
                    ),
                );
            }
        }
        Ok(())
    }
}
