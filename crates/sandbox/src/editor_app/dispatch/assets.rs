//! Asset-browser, import, prefab, and material request handling.

use super::*;

impl EditorApp {
    pub(super) fn dispatch_asset_request(
        &mut self,
        request: EditorRequest,
    ) -> Result<DispatchOutcome, BridgeError> {
        match request {
            EditorRequest::SelectAsset(params) => {
                if !self.asset_browser.select_asset(params.asset_id) {
                    return Err(BridgeError::new(
                        EditorErrorCode::NotFound,
                        "The selected asset is not in the current project catalog",
                    ));
                }
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetAssetBrowser(params) => {
                if params.query.is_none()
                    && params.folder.is_none()
                    && params.kind.is_none()
                    && params.page.is_none()
                    && params.view.is_none()
                {
                    return Err(BridgeError::new(
                        EditorErrorCode::InvalidRequest,
                        "Asset browser update must include at least one field",
                    ));
                }
                self.set_asset_browser_state(params)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::RefreshAssets => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let job_id = self
                    .start_editor_job("Asset reimport", true, move || {
                        super::super::super::project_cli::cook_project(&project)
                            .map(|_| EditorJobOutput::None)
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::RevealProject => {
                reveal_in_file_manager(&self.project.root).map_err(io_error)?;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::RevealAssetFolder(params) => {
                let relative = PathBuf::from(
                    params
                        .folder
                        .trim()
                        .trim_matches(['/', '\\'])
                        .replace('\\', "/"),
                );
                if relative
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
                    && !relative.as_os_str().is_empty()
                {
                    return Err(BridgeError::new(
                        EditorErrorCode::ValidationFailed,
                        "Asset folder must be a normalized project-relative path",
                    ));
                }
                let source_root = self
                    .project
                    .asset_source
                    .canonicalize()
                    .map_err(|error| io_error(error.to_string()))?;
                let folder = self
                    .project
                    .asset_source
                    .join(relative)
                    .canonicalize()
                    .map_err(|error| io_error(error.to_string()))?;
                if !folder.starts_with(&source_root) || !folder.is_dir() {
                    return Err(BridgeError::new(
                        EditorErrorCode::ValidationFailed,
                        "Asset folder is outside the project source tree or is not a directory",
                    ));
                }
                reveal_in_file_manager(&folder).map_err(io_error)?;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::RevealAsset(params) => {
                let path = self.asset_path(&params.asset_id)?;
                reveal_in_file_manager(&path).map_err(io_error)?;
                Ok(DispatchOutcome::accepted(false))
            }
            EditorRequest::OpenAsset(params) => {
                let entry = self
                    .asset_browser
                    .catalog_assets()
                    .iter()
                    .find(|entry| entry.id.id == params.asset_id)
                    .cloned()
                    .ok_or_else(|| not_found("asset", &params.asset_id))?;
                if entry.kind == engine_editor::asset_browser::AssetKind::Material {
                    self.open_material(params.asset_id);
                    Ok(DispatchOutcome::accepted(true))
                } else {
                    reveal_in_file_manager(&self.asset_path(&params.asset_id)?)
                        .map_err(io_error)?;
                    Ok(DispatchOutcome::accepted(false))
                }
            }
            EditorRequest::ImportAsset(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let imported_id = params.asset_id.clone();
                let job_id = self
                    .start_editor_job("Asset import", true, move || {
                        super::super::super::project_cli::import_project_asset_from(
                            project,
                            PathBuf::from(params.source),
                            params.asset_id,
                            params.asset_type,
                            PathBuf::from(params.folder),
                        )?;
                        Ok(EditorJobOutput::SelectAsset(imported_id))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::CreateAssetFolder(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let folder_name = params.folder;
                let folder = PathBuf::from(&folder_name);
                let job_id = self
                    .start_editor_job("Create asset folder", false, move || {
                        super::super::super::editor_asset_ops::create_asset_folder(
                            &project, &folder,
                        )?;
                        Ok(EditorJobOutput::SelectFolder(folder_name))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::RenameAssetFolder(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let folder = PathBuf::from(params.folder);
                let new_folder_name = params.new_folder;
                let new_folder = PathBuf::from(&new_folder_name);
                let job_id = self
                    .start_editor_job("Rename asset folder", false, move || {
                        super::super::super::editor_asset_ops::rename_asset_folder(
                            &project,
                            &folder,
                            &new_folder,
                        )?;
                        Ok(EditorJobOutput::SelectFolder(new_folder_name))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::DeleteAssetFolder(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let folder = PathBuf::from(params.folder);
                let parent_folder = folder
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(""))
                    .to_string_lossy()
                    .replace('\\', "/");
                let job_id = self
                    .start_editor_job("Delete asset folder", false, move || {
                        super::super::super::editor_asset_ops::delete_asset_folder(
                            &project, &folder,
                        )?;
                        Ok(EditorJobOutput::SelectFolder(parent_folder))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::CreateMaterial(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let folder = PathBuf::from(params.folder);
                let job_id = self
                    .start_editor_job("Create material", true, move || {
                        super::super::super::editor_asset_ops::create_material_asset(
                            &project,
                            &folder,
                            &params.name,
                            &super::super::super::editor_asset_ops::MaterialTemplate::default(),
                        )
                        .map(|mutation| EditorJobOutput::SelectAsset(mutation.asset_id.id))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::CreatePrefab(params) => {
                self.require_editing()?;
                self.create_prefab_from_selection(
                    params.asset_id,
                    PathBuf::from(params.relative_source_path),
                    PathBuf::from(params.manifest_name),
                )
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::InstantiatePrefab(params) => {
                self.require_editing()?;
                self.instantiate_prefab_asset(params.asset_id, params.parent_id)
                    .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::UnpackPrefab(params) => {
                self.require_editing()?;
                self.unpack_prefab_instance(
                    params.entity_id,
                    match params.mode {
                        UnpackModeDto::Instance => PrefabUnpackMode::Instance,
                        UnpackModeDto::Completely => PrefabUnpackMode::Completely,
                    },
                )
                .map_err(validation_error)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::DuplicateAsset(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let asset = AssetId::new(params.asset_id);
                let job_id = self
                    .start_editor_job("Duplicate asset", true, move || {
                        super::super::super::editor_asset_ops::duplicate_project_asset(
                            &project, &asset,
                        )
                        .map(|mutation| EditorJobOutput::SelectAsset(mutation.asset_id.id))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::MoveAsset(params) => {
                self.require_editing()?;
                let project = self.project.manifest_path.clone();
                let asset = AssetId::new(params.asset_id);
                let new_path = PathBuf::from(params.new_source_path);
                let job_id = self
                    .start_editor_job("Move asset", true, move || {
                        super::super::super::editor_asset_ops::move_project_asset(
                            &project, &asset, &new_path,
                        )
                        .map(|mutation| EditorJobOutput::SelectAsset(mutation.asset_id.id))
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::DeleteAsset(params) => {
                self.require_editing()?;
                self.reject_current_scene_asset_reference(&params.asset_id)?;
                let project = self.project.manifest_path.clone();
                let asset = AssetId::new(params.asset_id);
                let job_id = self
                    .start_editor_job("Delete asset", true, move || {
                        super::super::super::editor_asset_ops::delete_project_asset(
                            &project, &asset,
                        )?;
                        Ok(EditorJobOutput::ClearAssetSelection)
                    })
                    .map_err(job_conflict)?;
                Ok(DispatchOutcome::job(job_id))
            }
            EditorRequest::AssignAsset(params) => {
                self.asset_browser
                    .select_asset(Some(AssetId::new(params.asset_id)));
                if let Some(scene) = self.editor_scene.as_mut() {
                    scene.selected_entity = Some(params.entity_id);
                }
                let command = self
                    .editor_scene
                    .as_ref()
                    .and_then(|scene| scene.selected_entity.clone())
                    .and_then(|entity| self.asset_browser.selected_assignment_command(entity))
                    .ok_or_else(|| {
                        BridgeError::new(
                            EditorErrorCode::ValidationFailed,
                            "Select a Mesh or Material and an entity with a Renderable component",
                        )
                    })?;
                self.execute_command(Box::new(command))?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::OpenMaterial(params) => {
                self.open_material(params.asset_id);
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SetMaterialParameter(params) => {
                self.require_editing()?;
                self.set_material_parameter(params)?;
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::SaveMaterial => {
                self.require_editing()?;
                self.material_editor.request_save();
                self.process_material_save();
                Ok(DispatchOutcome::accepted(true))
            }
            EditorRequest::AssignOpenMaterial => {
                self.require_editing()?;
                self.assign_open_material()?;
                Ok(DispatchOutcome::accepted(true))
            }
            _ => unreachable!("request routed to the wrong editor IPC domain"),
        }
    }
}
