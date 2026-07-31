use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use engine_renderer::{
    AssetId, EnvironmentMapUpload, MaterialUpload, MeshUpload, MorphTargetSetUpload, TextureUpload,
};
use engine_scene::registry::AssetTypeRegistry;
use engine_serialize::Diagnostic;

use super::decode::decode_cooked_asset;
use super::decoded::{DecodedCookedAsset, DecodedExtensionAsset};
use super::validation::cooked_error;

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
    /// [`crate::EngineRuntime::load_cooked_assets`].
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
                .map(|(path, upload)| DecodedCookedAsset::Material(path, Box::new(upload))),
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
/// This is the first stage of [`crate::EngineRuntime::load_cooked_assets`], reused
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
                batch.materials.push((path, *upload));
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
