//! Gate 5 Asset Cook Pipeline.
//!
//! Transforms raw source assets (glTF, GLSL, PNG, etc.) into optimised
//! cooked artifacts in the `.cooked` format (see [`CookedAssetHeader`]).
//!

pub mod cooked_shader;
pub mod dependency;
pub mod environment;
pub mod error;
pub mod gltf_material;
pub mod hlod;
pub mod logic_asset;
pub mod manifest;
pub mod material;
pub mod mesh;
pub mod morph_target;
pub mod prefab;
pub mod scene;
pub mod texture;
pub mod validate;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use engine_scene::registry::AssetTypeRegistry;
use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity, HashDigest, SchemaVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use cooked_shader::{
    cook_shader, CookedShader, DescriptorBinding, ShaderReflection, VertexInputReflection,
};
pub use dependency::{CookState, DependencyGraph, DependencyNode};
pub use environment::{
    cook_environment_map, decode_cooked_environment_map, CookedEnvironmentMap,
    CookedEnvironmentMip, COOKED_ENVIRONMENT_SCHEMA_VERSION,
};
pub use error::{AssetCookError, CookError};
pub use gltf_material::material_source_from_gltf;
pub use hlod::{
    apply_hlod_bake_to_scene, bake_hlod_proxies, bake_hlod_scene, write_hlod_proxy_artifacts,
    HlodBakeOutput, HlodBakeSettings, HlodBakeSource, HlodProxyBake, HLOD_PROXY_PREFIX,
};
pub use logic_asset::{
    cook_logic_asset, decode_logic_asset_cooked_compatible, encode_logic_asset_cooked_v2,
    logic_asset_cooker, logic_asset_loader, parse_logic_asset_json_compatible,
    register_logic_asset_type, CompareOp, ComparisonOp, LogicAsset, LogicAssetKind,
    LogicAssetMigration, LogicAssetMigrationError, LogicAssetSourceSchema, LogicCondition,
    LogicKind, LogicMetadata, LogicNode, LogicParam, LogicParamType, LogicParameter,
    LogicParameterType, LogicTransition, LogicValue, LOGIC_ASSET_COOKED_V2_MAGIC,
    LOGIC_ASSET_SCHEMA_V1, LOGIC_ASSET_SCHEMA_V2, LOGIC_ASSET_TYPE_ID,
};
pub use manifest::{AssetType, CookRules, SourceAssetEntry, SourceManifest};
pub use material::{
    cook_material, decode_cooked_material, AdvancedMaterialSource, CookedMaterial, MaterialSource,
    MaterialTransparency, COOKED_MATERIAL_SCHEMA_VERSION, MATERIAL_SOURCE_SCHEMA,
};
pub use mesh::{cook_mesh, cook_mesh_with_options, GltfMeshCookOptions};
pub use morph_target::{
    cook_morph_target_set, decode_cooked_morph_target_set, CookedMorphTarget, CookedMorphTargetSet,
    COOKED_MORPH_TARGET_SCHEMA_VERSION,
};
pub use prefab::cook_prefab;
pub use scene::cook_scene;
pub use texture::{cook_texture, CookedTexture, CookedTextureFormat};
/// Compatibility name for the cooked-asset schema format. New code should
/// use [`CookedTextureFormat`] to distinguish it from an RHI texture format.
pub type TextureFormat = CookedTextureFormat;
pub use validate::validate_assets;

/// Magic bytes at the start of every cooked asset file.
pub const COOKED_MAGIC: &[u8; 8] = b"ENGCOOK\0";

/// Current version of the cooked asset header format.
pub const COOKED_HEADER_VERSION: u16 = 1;

/// On-disk header written before every cooked payload.
///
/// Layout (74 bytes total):
///
/// | Offset | Size | Field              |
/// |--------|------|--------------------|
/// | 0      | 8    | magic              |
/// | 8      | 2    | header_version     |
/// | 10     | 2    | asset_kind         |
/// | 12     | 6    | schema_version     |
/// | 18     | 32   | content_hash       |
/// | 50     | 8    | uncompressed_size  |
/// | 58     | 8    | compressed_size    |
/// | 66     | 1    | compression        |
/// | 67     | 7    | reserved           |
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CookedAssetHeader {
    /// Magic identifier: `"ENGCOOK\0"`.
    pub magic: [u8; 8],
    /// Header format version (currently 1).
    pub header_version: u16,
    /// Asset kind code (see [`AssetType::kind_code`]).
    pub asset_kind: u16,
    /// Schema version of the payload.
    pub schema_version: SchemaVersion,
    /// SHA-256 of the payload (after decompression, if compressed).
    pub content_hash: HashDigest,
    /// Size of the payload before compression.
    pub uncompressed_size: u64,
    /// Size of the payload after compression (0 = no compression).
    pub compressed_size: u64,
    /// Compression codec (0 = none).
    pub compression: u8,
    /// Reserved for future use.
    pub reserved: [u8; 7],
}

impl CookedAssetHeader {
    /// Create a new header for the given asset kind and payload.
    pub fn new(
        asset_kind: u16,
        schema_version: SchemaVersion,
        content_hash: HashDigest,
        uncompressed_size: u64,
    ) -> Self {
        Self {
            magic: *COOKED_MAGIC,
            header_version: COOKED_HEADER_VERSION,
            asset_kind,
            schema_version,
            content_hash,
            uncompressed_size,
            compressed_size: 0,
            compression: 0,
            reserved: [0u8; 7],
        }
    }

    /// Validate that the header has the correct magic and supported version.
    pub fn is_valid(&self) -> bool {
        &self.magic == COOKED_MAGIC && self.header_version == COOKED_HEADER_VERSION
    }
}

/// Fully validated cooked artifact read from disk.
#[derive(Clone, Debug)]
pub struct CookedArtifact {
    pub header: CookedAssetHeader,
    pub payload: Vec<u8>,
}

/// Read and validate a cooked artifact header, payload length, compression,
/// and SHA-256 content hash.
pub fn read_cooked_artifact(path: &Path) -> Result<CookedArtifact, CookError> {
    let bytes = std::fs::read(path)?;
    let header: CookedAssetHeader = bincode::deserialize(&bytes).map_err(|error| {
        CookError::InvalidAsset(format!(
            "could not decode cooked header from {}: {error}",
            path.display()
        ))
    })?;
    if !header.is_valid() {
        return Err(CookError::InvalidAsset(format!(
            "unsupported cooked header in {}",
            path.display()
        )));
    }
    if header.compression != 0 || header.compressed_size != 0 {
        return Err(CookError::UnsupportedFormat(format!(
            "compressed cooked artifacts are not supported: {}",
            path.display()
        )));
    }
    let header_size = bincode::serialized_size(&header)
        .map_err(|error| CookError::InvalidAsset(error.to_string()))?
        as usize;
    let payload = bytes.get(header_size..).ok_or_else(|| {
        CookError::InvalidAsset(format!("truncated cooked artifact: {}", path.display()))
    })?;
    if payload.len() as u64 != header.uncompressed_size {
        return Err(CookError::InvalidAsset(format!(
            "cooked payload length mismatch in {}: header={}, actual={}",
            path.display(),
            header.uncompressed_size,
            payload.len()
        )));
    }
    let actual_hash: HashDigest = Sha256::digest(payload).into();
    if actual_hash != header.content_hash {
        return Err(CookError::InvalidAsset(format!(
            "cooked payload hash mismatch in {}",
            path.display()
        )));
    }
    Ok(CookedArtifact {
        header,
        payload: payload.to_vec(),
    })
}

/// Decode a validated mesh artifact into its authoring-independent mesh data.
pub fn decode_cooked_mesh(artifact: &CookedArtifact) -> Result<crate::mesh::MeshData, CookError> {
    if artifact.header.asset_kind != AssetType::Mesh.kind_code() {
        return Err(CookError::InvalidAsset(format!(
            "expected mesh kind {}, found {}",
            AssetType::Mesh.kind_code(),
            artifact.header.asset_kind
        )));
    }
    bincode::deserialize(&artifact.payload)
        .map_err(|error| CookError::InvalidAsset(format!("invalid cooked mesh payload: {error}")))
}

/// Decode a validated texture artifact.
pub fn decode_cooked_texture(artifact: &CookedArtifact) -> Result<CookedTexture, CookError> {
    if artifact.header.asset_kind != AssetType::Texture.kind_code() {
        return Err(CookError::InvalidAsset(format!(
            "expected texture kind {}, found {}",
            AssetType::Texture.kind_code(),
            artifact.header.asset_kind
        )));
    }
    bincode::deserialize(&artifact.payload).map_err(|error| {
        CookError::InvalidAsset(format!("invalid cooked texture payload: {error}"))
    })
}

/// The result of cooking a single asset.
#[derive(Clone, Debug, Serialize)]
pub struct CookResult {
    /// Asset identifier string.
    pub asset_id: String,
    /// The type of asset that was cooked.
    pub asset_type: AssetType,
    /// Path to the output cooked file.
    pub output_path: PathBuf,
    /// Path to the source file.
    pub source_path: PathBuf,
    /// Whether cooking succeeded.
    pub success: bool,
    /// Diagnostics produced during cooking.
    pub diagnostics: Vec<Diagnostic>,
}

/// Deterministic, machine-readable result of a complete asset cook run.
#[derive(Clone, Debug, Serialize)]
pub struct CookReport {
    /// Report contract used by CI and release packaging.
    pub schema: String,
    /// Whether the requested source directory existed.
    pub source_directory_present: bool,
    /// Number of manifest files discovered.
    pub manifest_count: usize,
    /// Number of manifests that could not be read, parsed, or validated.
    pub failed_manifest_count: usize,
    /// Number of assets declared by valid manifests.
    pub declared_asset_count: usize,
    /// Number of assets cooked successfully.
    pub succeeded_asset_count: usize,
    /// Number of assets rejected or unsuccessfully cooked.
    pub failed_asset_count: usize,
    /// Per-asset outcomes in deterministic manifest/name order.
    pub results: Vec<CookResult>,
    /// Manifest, validation, cooker, and dependency diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

impl CookReport {
    fn new(source_directory_present: bool) -> Self {
        Self {
            schema: "AssetCookReport-v0".into(),
            source_directory_present,
            manifest_count: 0,
            failed_manifest_count: 0,
            declared_asset_count: 0,
            succeeded_asset_count: 0,
            failed_asset_count: 0,
            results: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Return `true` when no manifest or asset failed and no error diagnostic exists.
    pub fn is_success(&self) -> bool {
        self.failed_manifest_count == 0
            && self.failed_asset_count == 0
            && !self.diagnostics.iter().any(|diagnostic| {
                matches!(
                    diagnostic.severity,
                    DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
                )
            })
    }
}

/// Write a payload as a cooked artifact with its header.
///
/// # Parameters
///
/// * `output` - path for the `.cooked` output file.
/// * `asset_kind` - numeric kind code (see [`AssetType::kind_code`]).
/// * `payload` - serialized asset data.
/// * `schema_version` - schema version for the payload format.
///
/// # Returns
///
/// A [`CookResult`] describing the outcome.  Errors are propagated via
/// [`CookError`].
pub fn write_cooked_artifact(
    output: &Path,
    asset_kind: u16,
    payload: &[u8],
    schema_version: SchemaVersion,
) -> Result<CookResult, CookError> {
    // Compute content hash.
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let content_hash: HashDigest = hasher.finalize().into();

    let header = CookedAssetHeader::new(
        asset_kind,
        schema_version,
        content_hash,
        payload.len() as u64,
    );

    // Ensure parent directory exists.
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Serialise header with bincode.
    let header_bytes = bincode::serialize(&header)
        .map_err(|e| CookError::InvalidAsset(format!("header serialization failed: {e}")))?;

    let mut file = std::fs::File::create(output)?;
    file.write_all(&header_bytes)?;
    file.write_all(payload)?;

    Ok(CookResult {
        asset_id: output
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        asset_type: AssetType::from_kind_code(asset_kind),
        output_path: output.to_path_buf(),
        source_path: PathBuf::new(),
        success: true,
        diagnostics: vec![],
    })
}

/// Cook all manifests and retain every success and failure in a deterministic report.
///
/// This entry point never hides partial failures and is therefore the required API
/// for CI and release packaging.
pub fn cook_orchestrate_checked(
    source_dir: &Path,
    cooked_dir: &Path,
    graph: &mut DependencyGraph,
) -> CookReport {
    let mut registry = AssetTypeRegistry::new();
    engine_scene::register_prefab_asset_type(&mut registry);
    cook_orchestrate_checked_with_registry(source_dir, cooked_dir, graph, &registry)
}

/// Cook all manifests with additional subsystem-owned asset cookers.
///
/// Built-in render/scene/logic assets keep their specialised cookers. Asset
/// types owned by optional runtime subsystems are resolved through the same
/// [`AssetTypeRegistry`] that the runtime uses for loading, then wrapped in the
/// standard [`CookedAssetHeader`] by this crate. A registered loader is also
/// required and is run against the generated payload before the artifact is
/// committed, preventing a cooker from producing data its runtime cannot read.
pub fn cook_orchestrate_checked_with_registry(
    source_dir: &Path,
    cooked_dir: &Path,
    graph: &mut DependencyGraph,
    asset_type_registry: &AssetTypeRegistry,
) -> CookReport {
    let source_path_exists = source_dir.exists();
    let source_directory_present = source_dir.is_dir();
    let mut report = CookReport::new(source_directory_present);

    if let Err(error) = std::fs::create_dir_all(cooked_dir) {
        report.diagnostics.push(cook_diagnostic(
            "COOK_OUTPUT_CREATE_FAILED",
            format!("could not create the cooked output directory: {error}"),
            None,
            None,
        ));
        return report;
    }

    if source_path_exists && !source_directory_present {
        report.diagnostics.push(cook_diagnostic(
            "COOK_SOURCE_NOT_DIRECTORY",
            "the configured source asset path is not a directory".into(),
            None,
            None,
        ));
        return report;
    }

    if !source_directory_present {
        return report;
    }

    let read_dir = match std::fs::read_dir(source_dir) {
        Ok(read_dir) => read_dir,
        Err(error) => {
            report.diagnostics.push(cook_diagnostic(
                "COOK_SOURCE_READ_FAILED",
                format!("could not read the source asset directory: {error}"),
                None,
                None,
            ));
            return report;
        }
    };

    let mut entries = Vec::new();
    for entry in read_dir {
        match entry {
            Ok(entry) => entries.push(entry),
            Err(error) => report.diagnostics.push(cook_diagnostic(
                "COOK_SOURCE_ENTRY_READ_FAILED",
                format!("could not enumerate a source directory entry: {error}"),
                None,
                None,
            )),
        }
    }
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());

    let mut declared_assets = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("manifest"))
        {
            continue;
        }

        report.manifest_count += 1;
        let manifest_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                report.failed_manifest_count += 1;
                report.diagnostics.push(cook_diagnostic(
                    "COOK_MANIFEST_READ_FAILED",
                    format!("could not read manifest '{manifest_name}': {error}"),
                    Some(manifest_name),
                    None,
                ));
                continue;
            }
        };
        let manifest: SourceManifest = match serde_json::from_str(&content) {
            Ok(manifest) => manifest,
            Err(error) => {
                report.failed_manifest_count += 1;
                report.diagnostics.push(cook_diagnostic(
                    "COOK_MANIFEST_PARSE_FAILED",
                    format!("could not parse manifest '{manifest_name}': {error}"),
                    Some(manifest_name),
                    None,
                ));
                continue;
            }
        };

        report.declared_asset_count += manifest.assets.len();
        if manifest.schema_version != manifest::CURRENT_MANIFEST_VERSION {
            report.failed_manifest_count += 1;
            report.diagnostics.push(cook_diagnostic(
                "COOK_MANIFEST_VERSION_UNSUPPORTED",
                format!(
                    "manifest '{manifest_name}' uses schema {}.{}.{}; expected {}.{}.{}",
                    manifest.schema_version.major,
                    manifest.schema_version.minor,
                    manifest.schema_version.patch,
                    manifest::CURRENT_MANIFEST_VERSION.major,
                    manifest::CURRENT_MANIFEST_VERSION.minor,
                    manifest::CURRENT_MANIFEST_VERSION.patch,
                ),
                Some(manifest_name),
                None,
            ));
            continue;
        }

        declared_assets.extend(manifest.assets);
    }

    declared_assets.sort_by(|left, right| {
        left.id
            .id
            .cmp(&right.id.id)
            .then_with(|| left.source_path.cmp(&right.source_path))
    });
    let mut portable_id_counts = BTreeMap::<String, usize>::new();
    for asset_entry in &declared_assets {
        if validate_manifest_asset_id(&asset_entry.id.id).is_ok() {
            *portable_id_counts
                .entry(asset_entry.id.id.to_ascii_lowercase())
                .or_default() += 1;
        }
    }

    for asset_entry in &declared_assets {
        if let Err(message) = validate_manifest_asset_id(&asset_entry.id.id) {
            record_asset_failure(
                &mut report,
                asset_entry,
                PathBuf::from("invalid-asset-id.cooked"),
                "COOK_ASSET_ID_INVALID",
                message,
            );
            continue;
        }

        let relative_output = PathBuf::from(format!("{}.cooked", asset_entry.id.id));
        if portable_id_counts
            .get(&asset_entry.id.id.to_ascii_lowercase())
            .is_some_and(|count| *count > 1)
        {
            record_asset_failure(
                &mut report,
                asset_entry,
                relative_output,
                "COOK_ASSET_ID_DUPLICATE",
                format!(
                    "asset id '{}' is duplicated or differs only by case",
                    asset_entry.id.id
                ),
            );
            continue;
        }

        graph.register(asset_entry.id.clone());
        graph.mark_cooking(&asset_entry.id);

        match cook_source_entry_atomic(source_dir, cooked_dir, asset_entry, asset_type_registry) {
            Ok(cooked) => {
                graph.mark_cooked(&asset_entry.id, cooked.source_hash);
                report.succeeded_asset_count += 1;
                report.results.push(cooked.result);
            }
            Err(error) => {
                graph.mark_failed(&asset_entry.id, error.message.clone());
                record_asset_failure(
                    &mut report,
                    asset_entry,
                    relative_output,
                    error.code,
                    error.message,
                );
            }
        }
    }

    report.diagnostics.extend(graph.to_diagnostics());
    report
}

static COOK_STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Successful output of the authoritative single-asset cook path.
pub(crate) struct CookedSourceEntry {
    pub result: CookResult,
    pub source_hash: HashDigest,
}

/// Categorized failure from the authoritative single-asset cook path.
pub(crate) struct SourceEntryCookError {
    pub code: &'static str,
    pub message: String,
}

/// Cook one manifest entry through its real built-in or registered cooker,
/// validate the staged artifact, and atomically replace the prior artifact.
///
/// Full project cooking and incremental reload both call this function. A
/// failed source read, cooker, validation, or rename leaves the previous
/// cooked artifact untouched.
pub(crate) fn cook_source_entry_atomic(
    source_dir: &Path,
    cooked_dir: &Path,
    entry: &SourceAssetEntry,
    asset_type_registry: &AssetTypeRegistry,
) -> Result<CookedSourceEntry, SourceEntryCookError> {
    validate_manifest_asset_id(&entry.id.id).map_err(|message| SourceEntryCookError {
        code: "COOK_ASSET_ID_INVALID",
        message,
    })?;
    if !entry.cook_rules.variant_keys.is_empty()
        || !entry.cook_rules.platform_overrides.is_empty()
        || entry.cook_rules.compression.is_some()
    {
        return Err(SourceEntryCookError {
            code: "COOK_RULE_UNSUPPORTED",
            message: format!(
                "asset '{}' requests cook rules that are not implemented by the current cooker",
                entry.id.id
            ),
        });
    }

    let source_path = resolve_source_path(source_dir, &entry.source_path).map_err(|error| {
        SourceEntryCookError {
            code: "COOK_SOURCE_PATH_INVALID",
            message: error.to_string(),
        }
    })?;
    let source_hash =
        compute_file_hash_checked(&source_path).map_err(|error| SourceEntryCookError {
            code: "COOK_SOURCE_HASH_FAILED",
            message: error.to_string(),
        })?;
    std::fs::create_dir_all(cooked_dir).map_err(|error| SourceEntryCookError {
        code: "COOK_OUTPUT_CREATE_FAILED",
        message: format!("could not create cooked output directory: {error}"),
    })?;

    let relative_output = PathBuf::from(format!("{}.cooked", entry.id.id));
    let output_path = cooked_dir.join(&relative_output);
    let sequence = COOK_STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let staging_path = cooked_dir.join(format!(
        ".{}.{}.{}.cooked.tmp",
        entry.id.id,
        std::process::id(),
        sequence
    ));

    let cooked = dispatch_source_entry(
        &source_path,
        &staging_path,
        &entry.asset_type,
        &entry.cook_rules,
        asset_type_registry,
    );
    let mut result = match cooked {
        Ok(result) => result,
        Err(error) => {
            let _ = std::fs::remove_file(&staging_path);
            return Err(SourceEntryCookError {
                code: "COOK_ASSET_FAILED",
                message: error.to_string(),
            });
        }
    };
    if let Err(error) = read_cooked_artifact(&staging_path) {
        let _ = std::fs::remove_file(&staging_path);
        return Err(SourceEntryCookError {
            code: "COOK_STAGED_ARTIFACT_INVALID",
            message: error.to_string(),
        });
    }
    if let Err(error) = std::fs::rename(&staging_path, &output_path) {
        let _ = std::fs::remove_file(&staging_path);
        return Err(SourceEntryCookError {
            code: "COOK_OUTPUT_REPLACE_FAILED",
            message: format!(
                "could not atomically replace {}: {error}",
                output_path.display()
            ),
        });
    }

    result.asset_id.clone_from(&entry.id.id);
    result.asset_type = entry.asset_type.clone();
    result.source_path = PathBuf::from(&entry.source_path);
    result.output_path = relative_output;
    result.success = true;
    result.diagnostics.clear();
    Ok(CookedSourceEntry {
        result,
        source_hash,
    })
}

fn dispatch_source_entry(
    source_path: &Path,
    output_path: &Path,
    asset_type: &AssetType,
    cook_rules: &CookRules,
    asset_type_registry: &AssetTypeRegistry,
) -> Result<CookResult, CookError> {
    match asset_type {
        AssetType::Mesh => mesh::cook_mesh_with_options(
            source_path,
            output_path,
            mesh::GltfMeshCookOptions {
                primitive_index: cook_rules.gltf_primitive_index,
                merge_primitives: cook_rules.gltf_merge_primitives,
                bake_node_transforms: cook_rules.gltf_bake_node_transforms,
            },
        ),
        AssetType::Texture => texture::cook_texture(source_path, output_path),
        AssetType::EnvironmentMap => environment::cook_environment_map(source_path, output_path),
        AssetType::Material => material::cook_material(source_path, output_path),
        AssetType::MorphTargetSet => morph_target::cook_morph_target_set(
            source_path,
            output_path,
            cook_rules.gltf_primitive_index,
        ),
        AssetType::Shader => {
            let stage = determine_shader_stage(source_path);
            cooked_shader::cook_shader(source_path, output_path, 0, &stage)
        }
        AssetType::Scene => scene::cook_scene(source_path, output_path, 0),
        AssetType::Logic => logic_asset::cook_logic_asset(source_path, output_path),
        AssetType::Prefab => prefab::cook_prefab(source_path, output_path),
        AssetType::Audio | AssetType::Animation | AssetType::Skeleton | AssetType::NavMesh => {
            cook_registered_extension_asset(
                source_path,
                output_path,
                asset_type,
                asset_type_registry,
            )
        }
        other => Err(CookError::UnsupportedFormat(format!(
            "asset type {other:?} has no authoritative cooker"
        ))),
    }
}

/// Return the extension-registry type ID associated with a manifest asset
/// kind. These mappings are part of the cooked project contract and are shared
/// by cooking, project validation, and runtime loading.
pub fn registered_asset_type_id(asset_type: &AssetType) -> Option<&'static str> {
    match asset_type {
        AssetType::Audio => Some("audio_clip"),
        AssetType::Animation => Some("animation_clip"),
        AssetType::Skeleton => Some("skeleton"),
        AssetType::NavMesh => Some("navmesh"),
        AssetType::Prefab => Some("prefab"),
        AssetType::Logic => Some(LOGIC_ASSET_TYPE_ID),
        _ => None,
    }
}

fn cook_registered_extension_asset(
    source_path: &Path,
    output_path: &Path,
    asset_type: &AssetType,
    registry: &AssetTypeRegistry,
) -> Result<CookResult, CookError> {
    let type_id = registered_asset_type_id(asset_type).ok_or_else(|| {
        CookError::UnsupportedFormat(format!(
            "asset type {asset_type:?} has no built-in or registered cooker mapping"
        ))
    })?;
    let extension = registry.get(type_id).ok_or_else(|| {
        CookError::UnsupportedFormat(format!(
            "asset type {asset_type:?} requires registered extension '{type_id}'"
        ))
    })?;
    let cooker = extension.cooker.ok_or_else(|| {
        CookError::UnsupportedFormat(format!(
            "registered extension '{type_id}' does not provide a cooker"
        ))
    })?;
    let loader = extension.loader.ok_or_else(|| {
        CookError::UnsupportedFormat(format!(
            "registered extension '{type_id}' does not provide a runtime loader"
        ))
    })?;
    let source = std::fs::read(source_path)?;
    let mut payload = Vec::new();
    cooker(&source, &mut payload).map_err(|error| {
        CookError::InvalidAsset(format!(
            "extension cooker '{type_id}' rejected {}: {error}",
            source_path.display()
        ))
    })?;
    loader(&payload).map_err(|error| {
        CookError::InvalidAsset(format!(
            "extension loader '{type_id}' rejected the cooked payload for {}: {error}",
            source_path.display()
        ))
    })?;
    write_cooked_artifact(
        output_path,
        asset_type.kind_code(),
        &payload,
        SchemaVersion::new(0, 1, 0),
    )
}

fn cook_diagnostic(
    code: &str,
    message: String,
    path: Option<String>,
    asset: Option<AssetId>,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(code, DiagnosticSeverity::Error, "asset-cook", message);
    diagnostic.path = path;
    diagnostic.asset = asset;
    diagnostic
}

fn record_asset_failure(
    report: &mut CookReport,
    asset_entry: &SourceAssetEntry,
    output_path: PathBuf,
    code: &str,
    message: String,
) {
    let diagnostic = cook_diagnostic(
        code,
        message,
        Some(asset_entry.source_path.clone()),
        Some(asset_entry.id.clone()),
    );
    report.failed_asset_count += 1;
    report.results.push(CookResult {
        asset_id: asset_entry.id.id.clone(),
        asset_type: asset_entry.asset_type.clone(),
        output_path,
        source_path: PathBuf::from(&asset_entry.source_path),
        success: false,
        diagnostics: vec![diagnostic.clone()],
    });
    report.diagnostics.push(diagnostic);
}

pub(crate) fn validate_manifest_asset_id(asset_id: &str) -> Result<(), String> {
    if asset_id.is_empty() || asset_id.len() > 128 {
        return Err("asset id must contain between 1 and 128 ASCII characters".into());
    }
    if !asset_id
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
    {
        return Err("asset id must start with an ASCII letter or digit".into());
    }
    if !asset_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(
            "asset id may contain only ASCII letters, digits, hyphens, underscores, and dots"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn resolve_source_path(
    source_dir: &Path,
    relative_path: &str,
) -> Result<PathBuf, CookError> {
    let relative_path = Path::new(relative_path);
    if relative_path.as_os_str().is_empty()
        || !relative_path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(CookError::InvalidAsset(
            "source_path must be a non-empty relative path without '.' or '..' components".into(),
        ));
    }

    let source_root = std::fs::canonicalize(source_dir)?;
    let source_path = std::fs::canonicalize(source_dir.join(relative_path))?;
    if !source_path.starts_with(&source_root) {
        return Err(CookError::InvalidAsset(
            "source_path resolves outside the source asset directory".into(),
        ));
    }
    if !source_path.is_file() {
        return Err(CookError::InvalidAsset(
            "source_path does not identify a regular file".into(),
        ));
    }
    Ok(source_path)
}

/// Determine the shader stage from a file extension.
fn determine_shader_stage(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("vert") => "vertex".into(),
        Some("frag") => "fragment".into(),
        Some("comp") => "compute".into(),
        Some("geom") => "geometry".into(),
        Some("tesc") => "tess_control".into(),
        Some("tese") => "tess_eval".into(),
        _ => "vertex".into(), // default
    }
}

fn compute_file_hash_checked(path: &Path) -> Result<HashDigest, CookError> {
    let data = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(hasher.finalize().into())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
