use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use engine_asset::cook::{
    decode_cooked_environment_map, decode_cooked_material, decode_cooked_mesh,
    decode_cooked_morph_target_set, decode_cooked_texture, read_cooked_artifact,
    registered_asset_type_id, AssetType,
};
use engine_renderer::{
    AssetId, AxisAlignedBox, ColorSpace, EnvironmentCubeMip, EnvironmentMapFormat,
    EnvironmentMapUpload, IndexFormat, MaterialUpload, MeshUpload, MeshVertexFormat, MorphTarget,
    MorphTargetSetUpload, SamplerDescriptor, TextureMipLevel, TextureUpload, TextureUploadFormat,
    Transparency,
};
use engine_scene::registry::AssetTypeRegistry;
use engine_serialize::{Diagnostic, DiagnosticSeverity};

use crate::EngineRuntime;

/// Deterministic summary of project cooked assets installed into a runtime.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CookedAssetLoadReport {
    pub discovered_assets: usize,
    pub loaded_meshes: usize,
    pub loaded_textures: usize,
    pub loaded_materials: usize,
    pub loaded_environment_maps: usize,
    pub loaded_morph_target_sets: usize,
    pub loaded_extension_assets: BTreeMap<String, usize>,
    pub skipped_assets: Vec<String>,
    /// Additive installs only: assets whose ID was already present with an
    /// identical decoded payload. They are no-op successes, not reloads, so
    /// they are counted here instead of in the `loaded_*` fields.
    pub identical_assets: usize,
}

impl CookedAssetLoadReport {
    pub fn loaded_render_assets(&self) -> usize {
        self.loaded_meshes
            + self.loaded_textures
            + self.loaded_materials
            + self.loaded_environment_maps
            + self.loaded_morph_target_sets
    }

    pub fn loaded_extension_assets(&self) -> usize {
        self.loaded_extension_assets.values().sum()
    }

    pub fn loaded_assets(&self) -> usize {
        self.loaded_render_assets() + self.loaded_extension_assets()
    }
}

/// How a validated batch of cooked assets is installed into the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookedCommitMode {
    /// Unload the previously installed cooked batch, then install the new
    /// one. This is the transactional whole-directory behaviour of
    /// [`EngineRuntime::load_cooked_assets`].
    Replace,
    /// Install without unloading anything. An asset whose ID is already
    /// present is a no-op success when its decoded payload is identical, and
    /// a validation error naming the ID when the payload differs or a
    /// different asset kind owns the ID.
    Additive,
}

/// Cooked artifacts decoded and structurally validated, but not yet checked
/// against a live asset registry.
///
/// Every payload is owned plain data (`Vec<u8>`, floats, strings), so the
/// batch is `Send` and can be produced on a background thread; see
/// [`crate::AssetStreamLoader`].
pub struct DecodedBatch {
    pub(crate) discovered_assets: usize,
    pub(crate) skipped_assets: Vec<String>,
    pub(crate) meshes: Vec<MeshUpload>,
    pub(crate) textures: Vec<TextureUpload>,
    pub(crate) materials: Vec<(PathBuf, MaterialUpload)>,
    pub(crate) environment_maps: Vec<EnvironmentMapUpload>,
    pub(crate) morph_target_sets: Vec<MorphTargetSetUpload>,
    pub(crate) extensions: Vec<DecodedExtensionAsset>,
}

impl DecodedBatch {
    /// Number of `.cooked` files the decode stage was asked to read.
    pub fn discovered_assets(&self) -> usize {
        self.discovered_assets
    }

    /// Artifacts intentionally skipped (shader, scene, pipeline, script, …).
    pub fn skipped_assets(&self) -> &[String] {
        &self.skipped_assets
    }

    /// Number of runtime-installable assets in the batch.
    pub fn decoded_assets(&self) -> usize {
        self.meshes.len()
            + self.textures.len()
            + self.materials.len()
            + self.environment_maps.len()
            + self.morph_target_sets.len()
            + self.extensions.len()
    }

    /// Flatten into commit order: textures, materials, meshes, extensions.
    /// Textures precede materials so a same-batch material → texture
    /// dependency is satisfied even when a commit budget splits the batch
    /// across drains.
    pub(crate) fn into_commit_order(self) -> Vec<DecodedCookedAsset> {
        let mut items = Vec::with_capacity(self.decoded_assets());
        items.extend(
            self.environment_maps
                .into_iter()
                .map(DecodedCookedAsset::EnvironmentMap),
        );
        items.extend(self.textures.into_iter().map(DecodedCookedAsset::Texture));
        items.extend(
            self.materials
                .into_iter()
                .map(|(path, upload)| DecodedCookedAsset::Material(path, upload)),
        );
        items.extend(self.meshes.into_iter().map(DecodedCookedAsset::Mesh));
        items.extend(
            self.morph_target_sets
                .into_iter()
                .map(DecodedCookedAsset::MorphTargetSet),
        );
        items.extend(
            self.extensions
                .into_iter()
                .map(DecodedCookedAsset::Extension),
        );
        items
    }
}

/// A [`DecodedBatch`] whose cross-references — and, for additive installs,
/// asset-ID conflicts — were validated against a specific runtime. Committing
/// it cannot fail.
pub struct ValidatedBatch {
    pub(crate) decoded: DecodedBatch,
    pub(crate) mode: CookedCommitMode,
    /// Additive mode: asset IDs already installed with an identical payload.
    pub(crate) identical_ids: BTreeSet<AssetId>,
}

impl ValidatedBatch {
    /// Commit mode this batch was validated for.
    pub fn mode(&self) -> CookedCommitMode {
        self.mode
    }

    /// Additive mode: number of assets that will be skipped as identical.
    pub fn identical_assets(&self) -> usize {
        self.identical_ids.len()
    }
}

/// Decode and structurally validate a set of cooked artifacts.
///
/// This is the first stage of [`EngineRuntime::load_cooked_assets`], reused
/// by background loaders: it performs all file I/O and per-artifact checks
/// but never touches an asset registry, so it is safe to run off the main
/// thread. Paths are processed in the given order; any broken artifact fails
/// the whole batch with one diagnostic per failure.
pub fn decode_cooked_batch(
    paths: &[PathBuf],
    asset_type_registry: &AssetTypeRegistry,
) -> Result<DecodedBatch, Vec<Diagnostic>> {
    let mut batch = DecodedBatch {
        discovered_assets: paths.len(),
        skipped_assets: Vec::new(),
        meshes: Vec::new(),
        textures: Vec::new(),
        materials: Vec::new(),
        environment_maps: Vec::new(),
        morph_target_sets: Vec::new(),
        extensions: Vec::new(),
    };
    let mut diagnostics = Vec::new();
    for path in paths {
        match decode_cooked_asset(path, asset_type_registry) {
            Ok(DecodedCookedAsset::Mesh(upload)) => batch.meshes.push(upload),
            Ok(DecodedCookedAsset::Texture(upload)) => batch.textures.push(upload),
            Ok(DecodedCookedAsset::Material(path, upload)) => {
                batch.materials.push((path, upload));
            }
            Ok(DecodedCookedAsset::EnvironmentMap(upload)) => {
                batch.environment_maps.push(upload);
            }
            Ok(DecodedCookedAsset::MorphTargetSet(upload)) => {
                batch.morph_target_sets.push(upload);
            }
            Ok(DecodedCookedAsset::Extension(asset)) => batch.extensions.push(asset),
            Ok(DecodedCookedAsset::Skipped(kind)) => {
                batch.skipped_assets.push(format!(
                    "{} ({kind:?})",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            Err(error) => diagnostics.push(cooked_error(path, error)),
        }
    }
    if diagnostics.is_empty() {
        Ok(batch)
    } else {
        Err(diagnostics)
    }
}

/// How one asset installs in additive mode: freshly, as an identical-payload
/// no-op, or not at all because the ID is taken by a different payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstallPlan {
    Install,
    NoOp,
    Conflict,
}

/// What [`EngineRuntime::install_decoded_item`] installed, for reporting.
pub(crate) enum InstalledItemKind {
    Mesh,
    Texture,
    Material,
    EnvironmentMap,
    MorphTargetSet,
    Extension(String),
}

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
                self.register_material_asset(upload);
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

pub(crate) enum DecodedCookedAsset {
    Mesh(MeshUpload),
    Texture(TextureUpload),
    Material(PathBuf, MaterialUpload),
    EnvironmentMap(EnvironmentMapUpload),
    MorphTargetSet(MorphTargetSetUpload),
    Extension(DecodedExtensionAsset),
    Skipped(AssetType),
}

impl DecodedCookedAsset {
    pub(crate) fn asset_id(&self) -> &AssetId {
        match self {
            DecodedCookedAsset::Mesh(upload) => &upload.mesh_id,
            DecodedCookedAsset::Texture(upload) => &upload.texture_id,
            DecodedCookedAsset::Material(_, upload) => &upload.material_id,
            DecodedCookedAsset::EnvironmentMap(upload) => &upload.environment_id,
            DecodedCookedAsset::MorphTargetSet(upload) => &upload.target_set_id,
            DecodedCookedAsset::Extension(asset) => &asset.id,
            DecodedCookedAsset::Skipped(_) => {
                unreachable!("skipped artifacts are never queued for commit")
            }
        }
    }
}

pub(crate) struct DecodedExtensionAsset {
    pub(crate) type_id: String,
    pub(crate) id: AssetId,
    pub(crate) payload: Vec<u8>,
    pub(crate) value: Box<dyn Any + Send + Sync>,
}

pub(crate) fn additive_conflict_error(id: &AssetId, kind: &str) -> Diagnostic {
    Diagnostic::new(
        "AS0003",
        DiagnosticSeverity::Error,
        "engine-core.cooked-assets",
        format!(
            "additive install of {kind} asset '{}' conflicts with a different asset already \
             installed under the same ID; unload it explicitly or use a replace-mode load",
            id.id
        ),
    )
}

fn decode_cooked_asset(
    path: &Path,
    asset_type_registry: &AssetTypeRegistry,
) -> Result<DecodedCookedAsset, String> {
    let id = cooked_asset_id(path)?;
    let artifact = read_cooked_artifact(path).map_err(|error| error.to_string())?;
    let asset_type = AssetType::from_kind_code(artifact.header.asset_kind);
    if asset_type == AssetType::Unknown {
        return Err(format!(
            "unsupported cooked asset kind code {}",
            artifact.header.asset_kind
        ));
    }
    match asset_type {
        AssetType::Mesh => {
            let mesh = decode_cooked_mesh(&artifact).map_err(|error| error.to_string())?;
            if mesh.positions.is_empty() {
                return Err("cooked mesh has no vertices".into());
            }
            let (vertex_format, vertex_bytes, index_bytes, index_count) = if mesh.joints.is_empty()
                && mesh.weights.is_empty()
            {
                let (vertex_bytes, index_bytes, index_count, _) =
                    engine_asset::mesh::mesh_data_to_upload_bytes(&mesh);
                (
                    MeshVertexFormat::Pbr32,
                    vertex_bytes,
                    index_bytes,
                    index_count,
                )
            } else {
                let (vertex_bytes, index_bytes, index_count, _) =
                        engine_asset::mesh::mesh_data_to_skinned_bytes(&mesh).ok_or_else(|| {
                            "cooked skinned mesh must provide exactly four joints and weights per vertex"
                                .to_string()
                        })?;
                (
                    MeshVertexFormat::Skinned64,
                    vertex_bytes,
                    index_bytes,
                    index_count,
                )
            };
            Ok(DecodedCookedAsset::Mesh(MeshUpload {
                mesh_id: id,
                vertex_format,
                vertex_count: u32::try_from(mesh.positions.len())
                    .map_err(|_| "cooked mesh vertex count exceeds u32".to_string())?,
                vertex_bytes,
                index_format: IndexFormat::U32,
                index_count,
                index_bytes,
                bounds: AxisAlignedBox {
                    min: mesh.bounds.0.to_array(),
                    max: mesh.bounds.1.to_array(),
                },
                content_hash: artifact.header.content_hash,
            }))
        }
        AssetType::Texture => {
            let texture = decode_cooked_texture(&artifact).map_err(|error| error.to_string())?;
            if texture.format != engine_asset::cook::TextureFormat::Rgba8Unorm {
                return Err(format!(
                    "unsupported cooked texture format: {:?}",
                    texture.format
                ));
            }
            let mip_levels = split_rgba8_mips(
                texture.width,
                texture.height,
                texture.mip_count,
                &texture.data,
            )?;
            Ok(DecodedCookedAsset::Texture(TextureUpload {
                texture_id: id,
                width: texture.width,
                height: texture.height,
                format: TextureUploadFormat::Rgba8,
                color_space: ColorSpace::Srgb,
                mip_levels,
                sampler: SamplerDescriptor::default(),
                content_hash: artifact.header.content_hash,
            }))
        }
        AssetType::Material => {
            let material = decode_cooked_material(&artifact).map_err(|error| error.to_string())?;
            for (field, value) in [
                ("base_color[0]", material.base_color[0]),
                ("base_color[1]", material.base_color[1]),
                ("base_color[2]", material.base_color[2]),
                ("base_color[3]", material.base_color[3]),
                ("metallic", material.metallic),
                ("roughness", material.roughness),
                ("ambient_occlusion", material.ambient_occlusion),
                ("emissive[0]", material.emissive[0]),
                ("emissive[1]", material.emissive[1]),
                ("emissive[2]", material.emissive[2]),
            ] {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(format!(
                        "cooked material field '{field}' must be finite and in the range 0..=1"
                    ));
                }
            }
            let transparency = match material.transparency {
                engine_asset::cook::MaterialTransparency::Opaque => Transparency::Opaque,
                engine_asset::cook::MaterialTransparency::Masked { cutoff } => {
                    if !cutoff.is_finite() || !(0.0..=1.0).contains(&cutoff) {
                        return Err(
                            "cooked material alpha cutoff must be finite and in the range 0..=1"
                                .into(),
                        );
                    }
                    Transparency::Masked { cutoff }
                }
                engine_asset::cook::MaterialTransparency::Blend => Transparency::Blend,
                engine_asset::cook::MaterialTransparency::Additive => Transparency::Additive,
            };
            Ok(DecodedCookedAsset::Material(
                path.to_path_buf(),
                MaterialUpload {
                    material_id: id,
                    base_color: material.base_color,
                    metallic: material.metallic,
                    roughness: material.roughness,
                    ambient_occlusion: material.ambient_occlusion,
                    emissive: material.emissive,
                    base_color_texture: material.base_color_texture,
                    normal_texture: material.normal_texture,
                    metallic_roughness_texture: material.metallic_roughness_texture,
                    occlusion_texture: material.occlusion_texture,
                    emissive_texture: material.emissive_texture,
                    advanced: engine_renderer::AdvancedMaterialParameters {
                        clearcoat: material.advanced.clearcoat,
                        clearcoat_roughness: material.advanced.clearcoat_roughness,
                        subsurface: material.advanced.subsurface,
                        subsurface_color: material.advanced.subsurface_color,
                        anisotropy: material.advanced.anisotropy,
                        sheen_color: material.advanced.sheen_color,
                        rim_color: material.advanced.rim_color,
                        rim_power: material.advanced.rim_power,
                    },
                    transparency,
                    double_sided: material.double_sided,
                    content_hash: artifact.header.content_hash,
                },
            ))
        }
        AssetType::EnvironmentMap => {
            let environment =
                decode_cooked_environment_map(&artifact).map_err(|error| error.to_string())?;
            Ok(DecodedCookedAsset::EnvironmentMap(EnvironmentMapUpload {
                environment_id: id,
                format: EnvironmentMapFormat::Rgba16Float,
                mip_levels: environment
                    .mip_levels
                    .into_iter()
                    .map(|mip| EnvironmentCubeMip {
                        face_size: mip.face_size,
                        faces: mip.faces,
                    })
                    .collect(),
                content_hash: artifact.header.content_hash,
            }))
        }
        AssetType::MorphTargetSet => {
            let morph =
                decode_cooked_morph_target_set(&artifact).map_err(|error| error.to_string())?;
            Ok(DecodedCookedAsset::MorphTargetSet(MorphTargetSetUpload {
                target_set_id: id,
                vertex_count: morph.vertex_count,
                targets: morph
                    .targets
                    .into_iter()
                    .map(|target| MorphTarget {
                        name: target.name,
                        position_deltas: target.position_deltas,
                        normal_deltas: target.normal_deltas,
                    })
                    .collect(),
                content_hash: artifact.header.content_hash,
            }))
        }
        kind @ (AssetType::Audio
        | AssetType::Animation
        | AssetType::Skeleton
        | AssetType::NavMesh
        | AssetType::Prefab
        | AssetType::Logic) => {
            let type_id = registered_asset_type_id(&kind)
                .expect("extension-owned asset types have a stable registry mapping");
            let extension = asset_type_registry.get(type_id).ok_or_else(|| {
                format!("cooked {kind:?} asset requires registered extension '{type_id}'")
            })?;
            let loader = extension
                .loader
                .ok_or_else(|| format!("registered extension '{type_id}' has no runtime loader"))?;
            let value = loader(&artifact.payload).map_err(|error| {
                format!("extension loader '{type_id}' rejected cooked payload: {error}")
            })?;
            Ok(DecodedCookedAsset::Extension(DecodedExtensionAsset {
                type_id: type_id.to_string(),
                id,
                payload: artifact.payload,
                value,
            }))
        }
        AssetType::Font => {
            Err("cooked Font assets have no registered runtime loader mapping".into())
        }
        kind @ (AssetType::Shader | AssetType::Scene | AssetType::Pipeline | AssetType::Script) => {
            Ok(DecodedCookedAsset::Skipped(kind))
        }
        AssetType::Unknown => unreachable!("unknown kind was rejected above"),
    }
}

fn validate_material_texture_dependencies(
    runtime: &EngineRuntime,
    textures: &[TextureUpload],
    materials: &[(PathBuf, MaterialUpload)],
    replaced_asset_ids: &BTreeSet<AssetId>,
) -> Vec<Diagnostic> {
    let batch_texture_ids = textures
        .iter()
        .map(|upload| upload.texture_id.clone())
        .collect::<BTreeSet<_>>();

    materials
        .iter()
        .flat_map(|(path, upload)| {
            upload
                .texture_references()
                .into_iter()
                .filter_map(|texture_id| {
                    let texture_id = texture_id?;
                    (!material_texture_available(
                        runtime,
                        &batch_texture_ids,
                        replaced_asset_ids,
                        texture_id,
                    ))
                    .then(|| missing_texture_error(path, &upload.material_id, texture_id))
                })
        })
        .collect()
}

pub(crate) fn missing_texture_error(
    path: &Path,
    material_id: &AssetId,
    texture_id: &AssetId,
) -> Diagnostic {
    cooked_error(
        path,
        format!(
            "cooked material '{}' references missing texture '{}'",
            material_id.id, texture_id.id
        ),
    )
}

/// A material's base-color texture resolves when it is decoded in the same
/// batch or already installed as a typed texture that the commit will not
/// unload.
pub(crate) fn material_texture_available(
    runtime: &EngineRuntime,
    batch_texture_ids: &BTreeSet<AssetId>,
    replaced_asset_ids: &BTreeSet<AssetId>,
    texture_id: &AssetId,
) -> bool {
    batch_texture_ids.contains(texture_id)
        || (!replaced_asset_ids.contains(texture_id)
            && runtime
                .asset_registry()
                .get::<TextureUpload>(texture_id)
                .is_some())
}

pub(crate) fn cooked_asset_id(path: &Path) -> Result<AssetId, String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| format!("cooked asset has no UTF-8 file stem: {}", path.display()))?;
    Ok(AssetId::new(stem))
}

fn split_rgba8_mips(
    width: u32,
    height: u32,
    mip_count: u8,
    data: &[u8],
) -> Result<Vec<TextureMipLevel>, String> {
    if width == 0 || height == 0 || mip_count == 0 {
        return Err("cooked texture dimensions and mip count must be non-zero".into());
    }
    let mut levels = Vec::with_capacity(mip_count as usize);
    let mut offset = 0usize;
    let mut mip_width = width;
    let mut mip_height = height;
    for _ in 0..mip_count {
        let byte_count = (mip_width as usize)
            .checked_mul(mip_height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "cooked texture mip size overflow".to_string())?;
        let end = offset
            .checked_add(byte_count)
            .ok_or_else(|| "cooked texture mip offset overflow".to_string())?;
        let bytes = data
            .get(offset..end)
            .ok_or_else(|| "cooked texture mip chain is truncated".to_string())?;
        levels.push(TextureMipLevel {
            width: mip_width,
            height: mip_height,
            bytes: bytes.to_vec(),
        });
        offset = end;
        mip_width = (mip_width / 2).max(1);
        mip_height = (mip_height / 2).max(1);
    }
    if offset != data.len() {
        return Err(format!(
            "cooked texture contains {} trailing bytes",
            data.len() - offset
        ));
    }
    Ok(levels)
}

fn cooked_error(path: &Path, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(
        "AS0002",
        DiagnosticSeverity::Error,
        "engine-core.cooked-assets",
        message,
    )
    .path(path.to_string_lossy())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn cooked_case(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engine_core_cooked_material_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(crate) fn cook_test_material(dir: &Path, id: &str, texture: Option<&str>) {
        cook_test_material_with_color(dir, id, texture, [0.8, 0.7, 0.6, 1.0]);
    }

    pub(crate) fn cook_test_material_with_color(
        dir: &Path,
        id: &str,
        texture: Option<&str>,
        base_color: [f32; 4],
    ) {
        let texture_field = texture
            .map(|texture| format!(r#", "base_color_texture": "{texture}""#))
            .unwrap_or_default();
        let source = dir.join(format!("{id}.material.json"));
        std::fs::write(
            &source,
            format!(
                r#"{{
                    "schema": "MaterialSource-v0",
                    "base_color": [{}, {}, {}, {}],
                    "metallic": 0.25,
                    "roughness": 0.5,
                    "ambient_occlusion": 1.0{texture_field},
                    "transparency": "Opaque",
                    "double_sided": false
                }}"#,
                base_color[0], base_color[1], base_color[2], base_color[3]
            ),
        )
        .unwrap();
        engine_asset::cook::cook_material(&source, &dir.join(format!("{id}.cooked"))).unwrap();
    }

    fn cook_test_surface_material(
        dir: &Path,
        id: &str,
        transparency: &str,
        alpha_cutoff: f32,
        double_sided: bool,
    ) {
        let source = dir.join(format!("{id}.material.json"));
        std::fs::write(
            &source,
            format!(
                r#"{{
                    "schema": "MaterialSource-v0",
                    "base_color": [0.8, 0.7, 0.6, 0.5],
                    "metallic": 0.25,
                    "roughness": 0.5,
                    "ambient_occlusion": 1.0,
                    "transparency": "{transparency}",
                    "alpha_cutoff": {alpha_cutoff},
                    "double_sided": {double_sided}
                }}"#
            ),
        )
        .unwrap();
        engine_asset::cook::cook_material(&source, &dir.join(format!("{id}.cooked"))).unwrap();
    }

    fn texture_upload(id: &str) -> TextureUpload {
        TextureUpload {
            texture_id: AssetId::new(id),
            width: 1,
            height: 1,
            format: TextureUploadFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            mip_levels: vec![TextureMipLevel {
                width: 1,
                height: 1,
                bytes: vec![255; 4],
            }],
            sampler: SamplerDescriptor::default(),
            content_hash: [1; 32],
        }
    }

    fn material_upload(id: &str, texture: Option<&str>) -> MaterialUpload {
        MaterialUpload {
            material_id: AssetId::new(id),
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 1.0,
            ambient_occlusion: 1.0,
            emissive: [0.0; 3],
            base_color_texture: texture.map(AssetId::new),
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            advanced: engine_renderer::AdvancedMaterialParameters::default(),
            transparency: Transparency::Opaque,
            double_sided: false,
            content_hash: [2; 32],
        }
    }

    fn drain_until_idle(
        runtime: &mut EngineRuntime,
        max_iterations: usize,
    ) -> crate::StreamDrainReport {
        let mut last = crate::StreamDrainReport::default();
        for _ in 0..max_iterations {
            last = runtime.drain_cooked_asset_stream();
            if last.is_complete() {
                return last;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        last
    }

    #[test]
    fn rgba8_mip_split_rejects_truncated_and_trailing_data() {
        assert!(split_rgba8_mips(2, 2, 2, &[0; 19]).is_err());
        assert!(split_rgba8_mips(2, 2, 2, &[0; 21]).is_err());
        let levels = split_rgba8_mips(2, 2, 2, &[0; 20]).unwrap();
        assert_eq!(levels.len(), 2);
        assert_eq!((levels[1].width, levels[1].height), (1, 1));
    }

    #[test]
    fn cooked_skinned_mesh_reaches_the_runtime_as_skinned64() {
        let dir = cooked_case("skinned_mesh");
        let mesh = engine_asset::mesh::MeshData {
            positions: vec![glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y],
            normals: vec![glam::Vec3::Z; 3],
            uvs: vec![glam::Vec2::ZERO; 3],
            indices: vec![0, 1, 2],
            bounds: (glam::Vec3::ZERO, glam::Vec3::ONE),
            joints: vec![[0, 1, 0, 0]; 3],
            weights: vec![[0.75, 0.25, 0.0, 0.0]; 3],
        };
        engine_asset::cook::write_cooked_artifact(
            &dir.join("mesh.skinned.cooked"),
            AssetType::Mesh.kind_code(),
            &bincode::serialize(&mesh).unwrap(),
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();

        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        runtime.load_cooked_assets(&dir).unwrap();
        let upload = runtime
            .asset_registry()
            .get::<MeshUpload>(&AssetId::new("mesh.skinned"))
            .expect("skinned mesh upload");
        assert_eq!(upload.get().vertex_format, MeshVertexFormat::Skinned64);
        assert_eq!(upload.get().vertex_bytes.len(), 3 * 64);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_cooked_directory_is_an_empty_load() {
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        let missing = std::path::PathBuf::from("definitely-missing-cooked-assets");
        let report = runtime.load_cooked_assets(&missing).unwrap();
        assert_eq!(report, CookedAssetLoadReport::default());
    }

    #[test]
    fn material_texture_dependency_accepts_batch_or_typed_registry_texture() {
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        runtime.register_texture_asset(texture_upload("texture.registry"));
        let materials = vec![
            (
                PathBuf::from("batch.cooked"),
                material_upload("material.batch", Some("texture.batch")),
            ),
            (
                PathBuf::from("registry.cooked"),
                material_upload("material.registry", Some("texture.registry")),
            ),
        ];

        assert!(validate_material_texture_dependencies(
            &runtime,
            &[texture_upload("texture.batch")],
            &materials,
            &BTreeSet::new(),
        )
        .is_empty());
    }

    #[test]
    fn material_texture_dependency_requires_a_typed_texture() {
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        runtime.register_material_asset(material_upload("texture.wrong-type", None));
        let path = PathBuf::from("missing-dependency.cooked");
        let materials = vec![(
            path.clone(),
            material_upload("material.invalid", Some("texture.wrong-type")),
        )];

        let diagnostics =
            validate_material_texture_dependencies(&runtime, &[], &materials, &BTreeSet::new());

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path.as_deref(), path.to_str());
        assert!(diagnostics[0].message.contains("texture.wrong-type"));
    }

    #[test]
    fn auxiliary_material_texture_dependencies_are_validated() {
        let runtime = EngineRuntime::new(crate::EngineConfig::default());
        let path = PathBuf::from("missing-normal-dependency.cooked");
        let mut upload = material_upload("material.invalid-normal", None);
        upload.normal_texture = Some(AssetId::new("texture.normal-missing"));

        let diagnostics = validate_material_texture_dependencies(
            &runtime,
            &[],
            &[(path.clone(), upload)],
            &BTreeSet::new(),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path.as_deref(), path.to_str());
        assert!(diagnostics[0].message.contains("texture.normal-missing"));
    }

    #[test]
    fn cooked_material_is_registered_and_counted() {
        let dir = cooked_case("load");
        cook_test_material(&dir, "material.plain", None);
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

        let report = runtime.load_cooked_assets(&dir).unwrap();

        assert_eq!(report.discovered_assets, 1);
        assert_eq!(report.loaded_materials, 1);
        assert_eq!(report.loaded_render_assets(), 1);
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.plain"))
            .is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cooked_surface_materials_preserve_alpha_and_culling_state() {
        let dir = cooked_case("surface_states");
        cook_test_surface_material(&dir, "material.masked", "Masked", 0.37, true);
        cook_test_surface_material(&dir, "material.blended", "Blend", 0.5, false);
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

        runtime.load_cooked_assets(&dir).unwrap();

        let masked = runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.masked"))
            .unwrap();
        assert_eq!(
            masked.get().transparency,
            Transparency::Masked { cutoff: 0.37 }
        );
        assert!(masked.get().double_sided);
        let blended = runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.blended"))
            .unwrap();
        assert_eq!(blended.get().transparency, Transparency::Blend);
        assert!(!blended.get().double_sided);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_material_texture_prevents_partial_batch_registration() {
        let dir = cooked_case("atomic_dependency_failure");
        cook_test_material(&dir, "material.valid", None);
        cook_test_material(&dir, "material.invalid", Some("texture.missing"));
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

        let diagnostics = runtime.load_cooked_assets(&dir).unwrap_err();

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("texture.missing"));
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.valid"))
            .is_none());
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.invalid"))
            .is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn staged_pipeline_matches_legacy_whole_directory_load() {
        let dir = cooked_case("staged_equivalence");
        cook_test_material(&dir, "material.alpha", None);
        cook_test_material(&dir, "material.beta", None);

        let mut legacy = EngineRuntime::new(crate::EngineConfig::default());
        let legacy_report = legacy.load_cooked_assets(&dir).unwrap();

        let mut staged = EngineRuntime::new(crate::EngineConfig::default());
        let paths = vec![
            dir.join("material.alpha.cooked"),
            dir.join("material.beta.cooked"),
        ];
        let decoded =
            decode_cooked_batch(&paths, staged.asset_type_registry()).expect("decode stage");
        assert_eq!(decoded.discovered_assets(), 2);
        assert_eq!(decoded.decoded_assets(), 2);
        let validated = staged
            .validate_cooked_batch(decoded, CookedCommitMode::Replace)
            .expect("validate stage");
        assert_eq!(validated.mode(), CookedCommitMode::Replace);
        let staged_report = staged.commit_cooked_batch(validated);

        assert_eq!(legacy_report, staged_report);
        for id in ["material.alpha", "material.beta"] {
            let id = AssetId::new(id);
            assert_eq!(
                legacy
                    .asset_registry()
                    .get::<MaterialUpload>(&id)
                    .map(|h| h.get().clone()),
                staged
                    .asset_registry()
                    .get::<MaterialUpload>(&id)
                    .map(|h| h.get().clone()),
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn additive_install_merges_without_unloading_and_tracks_for_later_replace() {
        let dir_a = cooked_case("additive_base");
        cook_test_material(&dir_a, "material.base", None);
        let dir_b = cooked_case("additive_extra");
        cook_test_material(&dir_b, "material.extra", None);
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

        runtime.load_cooked_assets(&dir_a).unwrap();
        let report = runtime
            .install_cooked_assets_additive(&[dir_b.join("material.extra.cooked")])
            .unwrap();

        assert_eq!(report.loaded_materials, 1);
        assert_eq!(report.identical_assets, 0);
        for id in ["material.base", "material.extra"] {
            assert!(runtime
                .asset_registry()
                .get::<MaterialUpload>(&AssetId::new(id))
                .is_some());
        }

        // A later whole-directory replace unloads additively installed assets too.
        let empty = cooked_case("additive_empty");
        runtime.load_cooked_assets(&empty).unwrap();
        for id in ["material.base", "material.extra"] {
            assert!(runtime
                .asset_registry()
                .get::<MaterialUpload>(&AssetId::new(id))
                .is_none());
        }
        let _ = std::fs::remove_dir_all(dir_a);
        let _ = std::fs::remove_dir_all(dir_b);
        let _ = std::fs::remove_dir_all(empty);
    }

    #[test]
    fn additive_identical_payload_is_a_noop_success() {
        let dir = cooked_case("additive_identical");
        cook_test_material(&dir, "material.same", None);
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        let paths = [dir.join("material.same.cooked")];

        let first = runtime.install_cooked_assets_additive(&paths).unwrap();
        assert_eq!(first.loaded_materials, 1);
        assert_eq!(first.identical_assets, 0);

        let second = runtime.install_cooked_assets_additive(&paths).unwrap();
        assert_eq!(second.loaded_materials, 0);
        assert_eq!(second.loaded_assets(), 0);
        assert_eq!(second.identical_assets, 1);
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.same"))
            .is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn additive_differing_payload_is_a_validation_error_naming_the_id() {
        let dir_a = cooked_case("additive_conflict_a");
        cook_test_material_with_color(&dir_a, "material.dup", None, [0.8, 0.7, 0.6, 1.0]);
        let dir_b = cooked_case("additive_conflict_b");
        cook_test_material_with_color(&dir_b, "material.dup", None, [0.1, 0.2, 0.3, 1.0]);
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

        runtime
            .install_cooked_assets_additive(&[dir_a.join("material.dup.cooked")])
            .unwrap();
        let diagnostics = runtime
            .install_cooked_assets_additive(&[dir_b.join("material.dup.cooked")])
            .unwrap_err();

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "AS0003");
        assert!(diagnostics[0].message.contains("material.dup"));
        // The original payload survives the rejected install.
        let installed = runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.dup"))
            .expect("original material remains");
        assert_eq!(installed.get().base_color, [0.8, 0.7, 0.6, 1.0]);
        let _ = std::fs::remove_dir_all(dir_a);
        let _ = std::fs::remove_dir_all(dir_b);
    }

    #[test]
    fn additive_validation_failure_leaves_prior_batch_active() {
        let dir = cooked_case("additive_prior_batch");
        cook_test_material(&dir, "material.prior", None);
        let broken = cooked_case("additive_broken");
        cook_test_material(&broken, "material.broken", Some("texture.missing"));
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        runtime.load_cooked_assets(&dir).unwrap();

        let diagnostics = runtime
            .install_cooked_assets_additive(&[broken.join("material.broken.cooked")])
            .unwrap_err();

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("texture.missing"));
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.prior"))
            .is_some());
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.broken"))
            .is_none());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(broken);
    }

    #[test]
    fn background_stream_installs_assets_at_the_frame_boundary() {
        let dir = cooked_case("stream_roundtrip");
        for index in 0..3 {
            cook_test_material(&dir, &format!("material.stream{index}"), None);
        }
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        let paths = (0..3)
            .map(|index| dir.join(format!("material.stream{index}.cooked")))
            .collect::<Vec<_>>();

        assert_eq!(runtime.enqueue_cooked_asset_stream(paths), 3);
        assert_eq!(runtime.asset_registry().pending_loads(), 3);
        assert_eq!(runtime.cooked_asset_stream_pending(), 3);
        assert_eq!(
            runtime
                .asset_registry()
                .asset_state(&AssetId::new("material.stream1")),
            Some(engine_asset::AssetState::Loading),
        );

        let report = drain_until_idle(&mut runtime, 1_000);
        assert!(report.is_ok(), "diagnostics: {:?}", report.diagnostics);
        assert_eq!(report.committed, 3);
        assert_eq!(runtime.cooked_asset_stream_pending(), 0);
        assert_eq!(runtime.asset_registry().pending_loads(), 0);
        for index in 0..3 {
            let id = AssetId::new(format!("material.stream{index}"));
            assert!(runtime
                .asset_registry()
                .get::<MaterialUpload>(&id)
                .is_some());
            assert_eq!(
                runtime.asset_registry().asset_state(&id),
                Some(engine_asset::AssetState::Ready),
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stream_drain_respects_the_commit_budget() {
        let dir = cooked_case("stream_budget");
        for index in 0..5 {
            cook_test_material(&dir, &format!("material.budget{index}"), None);
        }
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        runtime.set_cooked_asset_stream_budget(2);
        assert_eq!(runtime.cooked_asset_stream_budget(), 2);
        let paths = (0..5)
            .map(|index| dir.join(format!("material.budget{index}.cooked")))
            .collect::<Vec<_>>();
        runtime.enqueue_cooked_asset_stream(paths);

        let mut productive_drains = 0;
        let mut total_committed = 0;
        for _ in 0..1_000 {
            let report = runtime.drain_cooked_asset_stream();
            assert!(report.committed <= 2, "budget exceeded: {report:?}");
            if report.committed > 0 {
                productive_drains += 1;
                total_committed += report.committed;
            }
            if report.is_complete() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        assert_eq!(total_committed, 5);
        assert_eq!(productive_drains, 3, "5 assets at budget 2 commit 2+2+1");
        for index in 0..5 {
            assert!(runtime
                .asset_registry()
                .get::<MaterialUpload>(&AssetId::new(format!("material.budget{index}")))
                .is_some());
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stream_commit_conflict_discards_the_failed_batch_and_keeps_prior_state() {
        let dir_a = cooked_case("stream_conflict_a");
        cook_test_material_with_color(&dir_a, "material.keep", None, [0.8, 0.7, 0.6, 1.0]);
        let dir_b = cooked_case("stream_conflict_b");
        cook_test_material_with_color(&dir_b, "material.keep", None, [0.1, 0.2, 0.3, 1.0]);
        cook_test_material(&dir_b, "material.sibling", None);
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        runtime.load_cooked_assets(&dir_a).unwrap();

        runtime.enqueue_cooked_asset_stream(vec![
            dir_b.join("material.keep.cooked"),
            dir_b.join("material.sibling.cooked"),
        ]);
        let report = drain_until_idle(&mut runtime, 1_000);

        assert!(!report.is_ok());
        assert_eq!(report.failed_batches, 1);
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "AS0003"
                && diagnostic.message.contains("material.keep")));
        // The conflicting batch was discarded entirely; prior state is intact.
        let installed = runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.keep"))
            .expect("prior material remains");
        assert_eq!(installed.get().base_color, [0.8, 0.7, 0.6, 1.0]);
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.sibling"))
            .is_none());
        assert_eq!(runtime.asset_registry().pending_loads(), 0);
        let _ = std::fs::remove_dir_all(dir_a);
        let _ = std::fs::remove_dir_all(dir_b);
    }

    #[test]
    fn stream_decode_failure_reports_and_clears_loading_marks() {
        let dir = cooked_case("stream_decode_failure");
        cook_test_material(&dir, "material.good", None);
        engine_asset::cook::write_cooked_artifact(
            &dir.join("broken.cooked"),
            4_242,
            b"valid outer artifact with unknown kind",
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

        runtime.enqueue_cooked_asset_stream(vec![
            dir.join("material.good.cooked"),
            dir.join("broken.cooked"),
        ]);
        let report = drain_until_idle(&mut runtime, 1_000);

        assert!(!report.is_ok());
        assert_eq!(report.failed_batches, 1);
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("kind code 4242")));
        // Decode is all-or-nothing per batch: the good sibling never installs.
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.good"))
            .is_none());
        assert_eq!(runtime.asset_registry().pending_loads(), 0);
        assert_eq!(runtime.cooked_asset_stream_pending(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn drain_without_enqueue_is_a_noop() {
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        let report = runtime.drain_cooked_asset_stream();
        assert!(report.is_complete());
        assert!(report.is_ok());
        assert_eq!(report.committed, 0);
        assert_eq!(runtime.cooked_asset_stream_pending(), 0);
    }

    #[cfg(not(feature = "subsystem-audio"))]
    fn test_extension_loader(cooked: &[u8]) -> Result<Box<dyn Any + Send + Sync>, String> {
        String::from_utf8(cooked.to_vec())
            .map(|value| Box::new(value) as Box<dyn Any + Send + Sync>)
            .map_err(|error| error.to_string())
    }

    #[cfg(not(feature = "subsystem-audio"))]
    #[test]
    fn registered_extension_assets_share_the_typed_cache_and_reload_atomically() {
        use engine_scene::registry::{AssetTypeExtension, AssetTypeMeta};

        let dir = cooked_case("extension_transaction");
        let id = AssetId::new("audio.custom");
        engine_asset::cook::write_cooked_artifact(
            &dir.join("audio.custom.cooked"),
            AssetType::Audio.kind_code(),
            b"first payload",
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
        let mut builder = crate::EngineRuntime::builder(crate::EngineConfig::default());
        builder
            .asset_type_registry_mut()
            .register(AssetTypeExtension {
                meta: AssetTypeMeta {
                    type_id: "audio_clip",
                    source_extensions: vec!["custom"],
                    display_name: "Custom Audio",
                },
                cooker: None,
                loader: Some(test_extension_loader),
            })
            .unwrap();
        let mut runtime = builder.build();

        let report = runtime.load_cooked_assets(&dir).unwrap();

        assert_eq!(report.loaded_extension_assets(), 1);
        assert_eq!(report.loaded_assets(), 1);
        assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
        assert_eq!(
            runtime
                .extension_asset::<String>("audio_clip", &id)
                .expect("extension asset")
                .get(),
            "first payload"
        );
        assert_eq!(
            runtime
                .asset_registry_mut()
                .load(&id)
                .expect("raw payload")
                .get(),
            b"first payload"
        );

        engine_asset::cook::write_cooked_artifact(
            &dir.join("broken.cooked"),
            4_242,
            b"valid outer artifact with unknown kind",
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
        let diagnostics = runtime.load_cooked_assets(&dir).unwrap_err();
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("kind code 4242")));
        assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
        assert_eq!(
            runtime
                .extension_asset::<String>("audio_clip", &id)
                .expect("previous batch remains installed")
                .get(),
            "first payload"
        );

        std::fs::remove_file(dir.join("broken.cooked")).unwrap();
        std::fs::remove_file(dir.join("audio.custom.cooked")).unwrap();
        let empty_report = runtime.load_cooked_assets(&dir).unwrap();
        assert_eq!(empty_report.loaded_assets(), 0);
        assert_eq!(runtime.extension_asset_count("audio_clip"), 0);
        assert!(runtime.asset_registry().get::<String>(&id).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(not(feature = "subsystem-audio"))]
    #[test]
    fn additive_extension_assets_noop_on_identical_payload_and_reject_conflicts() {
        use engine_scene::registry::{AssetTypeExtension, AssetTypeMeta};

        let dir = cooked_case("additive_extension");
        let id = AssetId::new("audio.custom");
        let mut builder = crate::EngineRuntime::builder(crate::EngineConfig::default());
        builder
            .asset_type_registry_mut()
            .register(AssetTypeExtension {
                meta: AssetTypeMeta {
                    type_id: "audio_clip",
                    source_extensions: vec!["custom"],
                    display_name: "Custom Audio",
                },
                cooker: None,
                loader: Some(test_extension_loader),
            })
            .unwrap();
        let mut runtime = builder.build();

        let paths = [dir.join("audio.custom.cooked")];
        engine_asset::cook::write_cooked_artifact(
            &paths[0],
            AssetType::Audio.kind_code(),
            b"first payload",
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
        let first = runtime.install_cooked_assets_additive(&paths).unwrap();
        assert_eq!(first.loaded_extension_assets(), 1);

        // Identical cooked payload: no-op success.
        let second = runtime.install_cooked_assets_additive(&paths).unwrap();
        assert_eq!(second.loaded_extension_assets(), 0);
        assert_eq!(second.identical_assets, 1);
        assert_eq!(
            runtime
                .extension_asset::<String>("audio_clip", &id)
                .expect("extension asset")
                .get(),
            "first payload"
        );

        // Differing cooked payload under the same ID: validation error.
        engine_asset::cook::write_cooked_artifact(
            &paths[0],
            AssetType::Audio.kind_code(),
            b"second payload",
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
        let diagnostics = runtime.install_cooked_assets_additive(&paths).unwrap_err();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "AS0003");
        assert!(diagnostics[0].message.contains("audio.custom"));
        assert_eq!(
            runtime
                .extension_asset::<String>("audio_clip", &id)
                .expect("original payload remains")
                .get(),
            "first payload"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(all(
        feature = "subsystem-animation",
        feature = "subsystem-audio",
        feature = "subsystem-navigation"
    ))]
    fn write_registered_extension_source(
        runtime: &EngineRuntime,
        dir: &Path,
        id: &str,
        kind: AssetType,
        source: &[u8],
    ) {
        let type_id = registered_asset_type_id(&kind).expect("mapped extension kind");
        let extension = runtime
            .asset_type_registry()
            .get(type_id)
            .expect("registered runtime extension");
        let mut payload = Vec::new();
        extension.cooker.expect("registered extension cooker")(source, &mut payload).unwrap();
        engine_asset::cook::write_cooked_artifact(
            &dir.join(format!("{id}.cooked")),
            kind.kind_code(),
            &payload,
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
    }

    #[cfg(all(
        feature = "subsystem-animation",
        feature = "subsystem-audio",
        feature = "subsystem-navigation"
    ))]
    fn minimal_pcm_wav() -> Vec<u8> {
        let samples = [0i16; 80];
        let data_size = u32::try_from(samples.len() * 2).unwrap();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8_000u32.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }

    #[cfg(all(
        feature = "subsystem-animation",
        feature = "subsystem-audio",
        feature = "subsystem-navigation"
    ))]
    #[test]
    fn runtime_subsystem_cookers_and_loaders_roundtrip_all_mapped_asset_kinds() {
        use engine_animation::{AnimationClip, Joint, JointTransform, Skeleton};
        use engine_nav::NavMesh;
        use glam::Vec3;

        let dir = cooked_case("real_runtime_extensions");
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        write_registered_extension_source(
            &runtime,
            &dir,
            "audio.real",
            AssetType::Audio,
            &minimal_pcm_wav(),
        );
        let skeleton = Skeleton {
            joints: vec![Joint {
                name: "root".into(),
                parent_index: None,
                local_transform: JointTransform::IDENTITY,
            }],
            inverse_bind_matrices: vec![[
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]],
        };
        write_registered_extension_source(
            &runtime,
            &dir,
            "skeleton.real",
            AssetType::Skeleton,
            &bincode::serialize(&skeleton).unwrap(),
        );
        let clip = AnimationClip {
            name: "idle".into(),
            duration: 1.0,
            channels: vec![],
            joint_indices: vec![],
        };
        write_registered_extension_source(
            &runtime,
            &dir,
            "animation.real",
            AssetType::Animation,
            &bincode::serialize(&clip).unwrap(),
        );
        let mut navmesh = NavMesh::new();
        let a = navmesh.add_vertex(Vec3::new(0.0, 0.0, 0.0));
        let b = navmesh.add_vertex(Vec3::new(1.0, 0.0, 0.0));
        let c = navmesh.add_vertex(Vec3::new(0.0, 0.0, 1.0));
        navmesh.add_polygon(&[a, b, c], 1.0);
        navmesh.rebuild_bvh();
        write_registered_extension_source(
            &runtime,
            &dir,
            "navmesh.real",
            AssetType::NavMesh,
            &bincode::serialize(&navmesh).unwrap(),
        );
        write_registered_extension_source(
            &runtime,
            &dir,
            "logic.real",
            AssetType::Logic,
            br#"{
                "schema_version":{"major":0,"minor":1,"patch":0},
                "asset_id":"logic.real",
                "kind":"SkillGraph",
                "nodes":[{"id":"root","node_type":"ability","label":null,"transitions":[],"properties":{},"children":[]}],
                "parameters":{},
                "metadata":{"author":null,"description":null,"tags":["test"],"version":"1.0.0"}
            }"#,
        );

        let report = runtime.load_cooked_assets(&dir).unwrap();

        assert_eq!(report.discovered_assets, 5);
        assert_eq!(report.loaded_extension_assets(), 5);
        assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
        assert_eq!(runtime.extension_asset_count("skeleton"), 1);
        assert_eq!(runtime.extension_asset_count("animation_clip"), 1);
        assert_eq!(runtime.extension_asset_count("navmesh"), 1);
        assert_eq!(runtime.extension_asset_count("logic"), 1);
        assert_eq!(
            runtime
                .extension_asset::<engine_audio::AudioClip>(
                    "audio_clip",
                    &AssetId::new("audio.real"),
                )
                .expect("audio clip")
                .get()
                .sample_rate(),
            8_000
        );
        assert_eq!(
            runtime
                .extension_asset::<Skeleton>("skeleton", &AssetId::new("skeleton.real"))
                .expect("skeleton")
                .get()
                .joint_count(),
            1
        );
        assert_eq!(
            runtime
                .extension_asset::<AnimationClip>(
                    "animation_clip",
                    &AssetId::new("animation.real"),
                )
                .expect("animation clip")
                .get()
                .name(),
            "idle"
        );
        assert!(runtime
            .extension_asset::<NavMesh>("navmesh", &AssetId::new("navmesh.real"))
            .is_some());
        assert!(runtime
            .extension_asset::<engine_asset::cook::LogicAsset>("logic", &AssetId::new("logic.real"))
            .is_some());

        engine_asset::cook::write_cooked_artifact(
            &dir.join("audio.real.cooked"),
            AssetType::Audio.kind_code(),
            b"not a valid cooked audio payload",
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
        let diagnostics = runtime.load_cooked_assets(&dir).unwrap_err();
        assert!(diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("extension loader 'audio_clip'") }));
        assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
        assert_eq!(
            runtime
                .extension_asset::<engine_audio::AudioClip>(
                    "audio_clip",
                    &AssetId::new("audio.real"),
                )
                .expect("previous audio remains installed")
                .get()
                .sample_rate(),
            8_000
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
