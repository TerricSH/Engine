//! Transactional project asset operations used by the editor's Project panel.
//!
//! Every operation prepares and cooks against a private copy of the project's
//! source tree. The live project is changed only after validation succeeds, and
//! every live file touched by a commit is snapshotted so an I/O failure can be
//! rolled back without leaving a manifest/source/cooked split brain.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
use engine_asset::cook::{
    cook_orchestrate_checked_with_registry, read_cooked_artifact, AssetType, CookRules,
    DependencyGraph, MaterialSource, SourceAssetEntry, SourceManifest, MATERIAL_SOURCE_SCHEMA,
};
use engine_asset::project::GameProject;
use engine_serialize::{
    AssetId, DiagnosticSeverity, LogicAsset, LogicCondition, LogicValue, Value,
};
use serde::{Deserialize, Serialize};

const TRASH_SCHEMA: &str = "EditorAssetTrash-v0";
static ASSET_OPERATION_MUTEX: Mutex<()> = Mutex::new(());

/// Values used to create a portable metallic-roughness material source asset.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MaterialTemplate {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub ambient_occlusion: f32,
    pub emissive: [f32; 3],
    pub base_color_texture: Option<String>,
    pub normal_texture: Option<String>,
    pub metallic_roughness_texture: Option<String>,
    pub occlusion_texture: Option<String>,
    pub emissive_texture: Option<String>,
    pub transparency: String,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

impl Default for MaterialTemplate {
    fn default() -> Self {
        Self {
            base_color: [1.0, 1.0, 1.0, 1.0],
            metallic: 0.0,
            roughness: 0.5,
            ambient_occlusion: 1.0,
            emissive: [0.0; 3],
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            transparency: "Opaque".into(),
            alpha_cutoff: 0.5,
            double_sided: false,
        }
    }
}

/// Paths and stable identity produced by a successful create/copy/move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AssetMutation {
    pub asset_id: AssetId,
    pub manifest_path: PathBuf,
    pub source_path: PathBuf,
    pub cooked_path: PathBuf,
}

/// Recoverable location produced by a successful delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeletedAsset {
    pub asset_id: AssetId,
    pub trash_directory: PathBuf,
    pub metadata_path: PathBuf,
}

mod assets;
mod catalog;
mod deletion;
mod folders;
mod identity;
mod paths;
mod transaction;

pub(crate) use assets::{
    create_material_asset, delete_project_asset, duplicate_project_asset, move_project_asset,
};
#[cfg(test)]
use assets::{delete_project_asset_impl, move_project_asset_impl};
use catalog::{ManifestCatalog, StagedWorkspace};
use deletion::{allocate_trash_directory, reject_known_references, TrashMetadata};
pub(crate) use folders::{create_asset_folder, delete_asset_folder, rename_asset_folder};
use identity::{
    collect_scene_asset_dependencies, cook_staged_asset, next_duplicate_identity,
    next_material_identity, portable_slug, rewrite_duplicated_source_identity,
    source_asset_dependencies,
};
use paths::{
    cooked_path, copy_directory_tree, copy_file_create_new, ensure_destination_absent,
    ensure_existing_folder, ensure_no_symlink_ancestors, ensure_parent_is_real_directory, io_read,
    load_project, lock_asset_operations, manifest_path_string, map_staged_path,
    normalize_relative_path, portable_path_key, project_relative_string, reject_symlink,
    resolve_case_insensitive, serialize_manifest, unix_nanos, validate_asset_id,
    write_file_create_new,
};
use transaction::{commit_transaction, CommitWrite};

#[cfg(test)]
mod tests {
    include!("editor_asset_ops/tests/common.rs");
    include!("editor_asset_ops/tests/folders_and_duplicates.rs");
    include!("editor_asset_ops/tests/move_and_delete.rs");
    include!("editor_asset_ops/tests/dependencies.rs");
}
