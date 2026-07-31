use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use engine_renderer::AssetId;
use engine_serialize::Diagnostic;

use crate::EngineRuntime;

use super::decoded::{additive_conflict_error, DecodedCookedAsset, DecodedExtensionAsset};
use super::types::{
    decode_cooked_batch, CookedAssetLoadReport, CookedCommitMode, DecodedBatch, InstallPlan,
    InstalledItemKind, ValidatedBatch,
};
use super::validation::{cooked_error, validate_material_texture_dependencies};

impl EngineRuntime {
    /// Validate and register every runtime-loadable cooked asset in `cooked_dir`.
    ///
    /// Meshes, RGBA8 textures, portable opaque materials, and assets owned by
    /// registered runtime extensions are installed transactionally. A corrupt
    /// or unsupported artifact leaves the previous successful batch intact.
    /// Shader, scene, pipeline, and script artifacts are reported as skipped
    /// because their dedicated consumers do not use this cache. Logic graphs
    /// install through the registered `logic` runtime loader.
    ///
    /// This is the staged pipeline
    /// ([`decode_cooked_batch`] → [`validate_cooked_batch`](Self::validate_cooked_batch)
    /// → [`commit_cooked_batch`](Self::commit_cooked_batch)) in
    /// [`CookedCommitMode::Replace`] over the whole directory.
    pub fn load_cooked_assets(
        &mut self,
        cooked_dir: &Path,
    ) -> Result<CookedAssetLoadReport, Vec<Diagnostic>> {
        if !cooked_dir.exists() {
            return Ok(CookedAssetLoadReport::default());
        }
        if !cooked_dir.is_dir() {
            return Err(vec![cooked_error(
                cooked_dir,
                "configured cooked asset path is not a directory",
            )]);
        }

        let entries = match std::fs::read_dir(cooked_dir) {
            Ok(entries) => entries,
            Err(error) => {
                return Err(vec![cooked_error(
                    cooked_dir,
                    format!("could not enumerate cooked assets: {error}"),
                )]);
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(error) => {
                    return Err(vec![cooked_error(
                        cooked_dir,
                        format!("could not enumerate a cooked asset entry: {error}"),
                    )]);
                }
            };
            if path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("cooked"))
            {
                paths.push(path);
            }
        }
        paths.sort();

        let batch = decode_cooked_batch(&paths, &self.asset_type_registry)?;
        let batch = self.validate_cooked_batch(batch, CookedCommitMode::Replace)?;
        Ok(self.commit_cooked_batch(batch))
    }

    /// Decode, validate, and additively install an explicit set of cooked
    /// artifacts, synchronously, without unloading existing assets.
    ///
    /// Conflict rules: an asset ID already present with an identical decoded
    /// payload is a no-op success (counted as
    /// [`CookedAssetLoadReport::identical_assets`]); a differing payload, or
    /// a different asset kind owning the same ID, is a validation error
    /// listing the ID and nothing from the batch is installed. Existing
    /// assets — including earlier batches — are never modified by a failure.
    pub fn install_cooked_assets_additive(
        &mut self,
        paths: &[PathBuf],
    ) -> Result<CookedAssetLoadReport, Vec<Diagnostic>> {
        let batch = decode_cooked_batch(paths, &self.asset_type_registry)?;
        let batch = self.validate_cooked_batch(batch, CookedCommitMode::Additive)?;
        Ok(self.commit_cooked_batch(batch))
    }

    /// Second stage: check a decoded batch against this runtime's registry.
    ///
    /// Material → texture references must resolve inside the batch or against
    /// the registry (in [`CookedCommitMode::Replace`] entries from the
    /// previous cooked batch do not count — they are about to be unloaded).
    /// Additive validation additionally classifies every asset ID as install
    /// or identical-no-op and rejects payload conflicts.
    pub fn validate_cooked_batch(
        &self,
        batch: DecodedBatch,
        mode: CookedCommitMode,
    ) -> Result<ValidatedBatch, Vec<Diagnostic>> {
        let empty = BTreeSet::new();
        let replaced_asset_ids = match mode {
            CookedCommitMode::Replace => &self.loaded_cooked_asset_ids,
            CookedCommitMode::Additive => &empty,
        };
        let mut diagnostics = validate_material_texture_dependencies(
            self,
            &batch.textures,
            &batch.materials,
            replaced_asset_ids,
        );

        // Runtime mesh IDs are reserved while the mesh is live: neither a
        // replace-mode swap (which would overwrite the registry entry and
        // later unload it) nor an additive install may claim them. Additive
        // payload-identity checks below only cover typed equality, so the
        // reservation is enforced explicitly for both modes.
        {
            let runtime_id_conflicts = batch
                .textures
                .iter()
                .map(|upload| (&upload.texture_id, "texture"))
                .chain(
                    batch
                        .materials
                        .iter()
                        .map(|(_, upload)| (&upload.material_id, "material")),
                )
                .chain(batch.meshes.iter().map(|upload| (&upload.mesh_id, "mesh")))
                .chain(
                    batch
                        .environment_maps
                        .iter()
                        .map(|upload| (&upload.environment_id, "environment map")),
                )
                .chain(
                    batch
                        .morph_target_sets
                        .iter()
                        .map(|upload| (&upload.target_set_id, "morph target set")),
                )
                .chain(
                    batch
                        .extensions
                        .iter()
                        .map(|asset| (&asset.id, asset.type_id.as_str())),
                );
            for (id, kind) in runtime_id_conflicts {
                if self.is_runtime_mesh_asset_id(id) {
                    diagnostics.push(crate::runtime_mesh::runtime_mesh_conflict_diagnostic(
                        id, kind,
                    ));
                }
            }
        }

        let mut identical_ids = BTreeSet::new();
        if mode == CookedCommitMode::Additive {
            for upload in &batch.textures {
                Self::classify_additive(
                    &mut identical_ids,
                    &mut diagnostics,
                    &upload.texture_id,
                    "texture",
                    self.additive_typed_plan(&upload.texture_id, upload),
                );
            }
            for (_, upload) in &batch.materials {
                Self::classify_additive(
                    &mut identical_ids,
                    &mut diagnostics,
                    &upload.material_id,
                    "material",
                    self.additive_typed_plan(&upload.material_id, upload),
                );
            }
            for upload in &batch.meshes {
                Self::classify_additive(
                    &mut identical_ids,
                    &mut diagnostics,
                    &upload.mesh_id,
                    "mesh",
                    self.additive_typed_plan(&upload.mesh_id, upload),
                );
            }
            for upload in &batch.environment_maps {
                Self::classify_additive(
                    &mut identical_ids,
                    &mut diagnostics,
                    &upload.environment_id,
                    "environment map",
                    self.additive_typed_plan(&upload.environment_id, upload),
                );
            }
            for upload in &batch.morph_target_sets {
                Self::classify_additive(
                    &mut identical_ids,
                    &mut diagnostics,
                    &upload.target_set_id,
                    "morph target set",
                    self.additive_typed_plan(&upload.target_set_id, upload),
                );
            }
            for asset in &batch.extensions {
                let plan = self.additive_extension_plan(asset);
                Self::classify_additive(
                    &mut identical_ids,
                    &mut diagnostics,
                    &asset.id,
                    &asset.type_id,
                    plan,
                );
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        Ok(ValidatedBatch {
            decoded: batch,
            mode,
            identical_ids,
        })
    }

    fn classify_additive(
        identical_ids: &mut BTreeSet<AssetId>,
        diagnostics: &mut Vec<Diagnostic>,
        id: &AssetId,
        kind: &str,
        plan: InstallPlan,
    ) {
        match plan {
            InstallPlan::NoOp => {
                identical_ids.insert(id.clone());
            }
            InstallPlan::Install => {}
            InstallPlan::Conflict => diagnostics.push(additive_conflict_error(id, kind)),
        }
    }

    /// Third stage: install a validated batch. Infallible — validation has
    /// already proven every install or no-op. Returns the deterministic
    /// install summary. In [`CookedCommitMode::Replace`] the previous cooked
    /// batch is unloaded first; in [`CookedCommitMode::Additive`] the new
    /// assets merge into the tracked cooked set, so a later replace also
    /// unloads them.
    pub fn commit_cooked_batch(&mut self, batch: ValidatedBatch) -> CookedAssetLoadReport {
        let ValidatedBatch {
            mut decoded,
            mode,
            identical_ids,
        } = batch;
        let mut report = CookedAssetLoadReport {
            discovered_assets: decoded.discovered_assets,
            skipped_assets: std::mem::take(&mut decoded.skipped_assets),
            identical_assets: identical_ids.len(),
            ..CookedAssetLoadReport::default()
        };
        if mode == CookedCommitMode::Replace {
            for id in std::mem::take(&mut self.loaded_cooked_asset_ids) {
                self.asset_registry.unload(&id);
            }
            self.loaded_extension_asset_ids.clear();
        }
        for item in decoded.into_commit_order() {
            if identical_ids.contains(item.asset_id()) {
                continue;
            }
            match self.install_decoded_item(item) {
                InstalledItemKind::Mesh => report.loaded_meshes += 1,
                InstalledItemKind::Texture => report.loaded_textures += 1,
                InstalledItemKind::Material => report.loaded_materials += 1,
                InstalledItemKind::EnvironmentMap => report.loaded_environment_maps += 1,
                InstalledItemKind::MorphTargetSet => report.loaded_morph_target_sets += 1,
                InstalledItemKind::Extension(type_id) => {
                    *report.loaded_extension_assets.entry(type_id).or_default() += 1;
                }
            }
        }
        report
    }

    /// Install one decoded asset and record it in the cooked tracking sets.
    pub(crate) fn install_decoded_item(&mut self, item: DecodedCookedAsset) -> InstalledItemKind {
        match item {
            DecodedCookedAsset::Texture(upload) => {
                self.loaded_cooked_asset_ids
                    .insert(upload.texture_id.clone());
                self.register_texture_asset(upload);
                InstalledItemKind::Texture
            }
            DecodedCookedAsset::Material(_, upload) => {
                self.loaded_cooked_asset_ids
                    .insert(upload.material_id.clone());
                self.register_material_asset(*upload);
                InstalledItemKind::Material
            }
            DecodedCookedAsset::Mesh(upload) => {
                self.loaded_cooked_asset_ids.insert(upload.mesh_id.clone());
                self.register_mesh_asset(upload);
                InstalledItemKind::Mesh
            }
            DecodedCookedAsset::EnvironmentMap(upload) => {
                self.loaded_cooked_asset_ids
                    .insert(upload.environment_id.clone());
                self.register_environment_map_asset(upload);
                InstalledItemKind::EnvironmentMap
            }
            DecodedCookedAsset::MorphTargetSet(upload) => {
                self.loaded_cooked_asset_ids
                    .insert(upload.target_set_id.clone());
                self.register_morph_target_set_asset(upload);
                InstalledItemKind::MorphTargetSet
            }
            DecodedCookedAsset::Extension(asset) => {
                self.loaded_cooked_asset_ids.insert(asset.id.clone());
                self.loaded_extension_asset_ids
                    .entry(asset.type_id.clone())
                    .or_default()
                    .insert(asset.id.clone());
                let type_id = asset.type_id.clone();
                self.asset_registry
                    .insert_erased(asset.id, asset.payload, asset.value);
                InstalledItemKind::Extension(type_id)
            }
            DecodedCookedAsset::Skipped(_) => {
                unreachable!("skipped artifacts are never queued for commit")
            }
        }
    }

    /// Additive conflict check for a typed render asset: an identical typed
    /// value already installed is a no-op; any other occupant of the same ID
    /// (different payload or different asset kind) is a conflict.
    pub(crate) fn additive_typed_plan<T>(&self, id: &AssetId, upload: &T) -> InstallPlan
    where
        T: PartialEq + Send + Sync + 'static,
    {
        if let Some(existing) = self.asset_registry.get::<T>(id) {
            return if existing.get() == upload {
                InstallPlan::NoOp
            } else {
                InstallPlan::Conflict
            };
        }
        if self.asset_registry.contains(id) {
            return InstallPlan::Conflict;
        }
        InstallPlan::Install
    }

    /// Additive conflict check for an extension asset: a no-op only when the
    /// same extension type already installed the same cooked payload.
    pub(crate) fn additive_extension_plan(&self, asset: &DecodedExtensionAsset) -> InstallPlan {
        if !self.asset_registry.contains(&asset.id) {
            return InstallPlan::Install;
        }
        let same_extension = self
            .loaded_extension_asset_ids
            .get(&asset.type_id)
            .is_some_and(|ids| ids.contains(&asset.id));
        let same_payload = self
            .asset_registry
            .cached_raw_bytes(&asset.id)
            .is_some_and(|bytes| *bytes == asset.payload);
        if same_extension && same_payload {
            InstallPlan::NoOp
        } else {
            InstallPlan::Conflict
        }
    }
}
