//! Gate 5 Asset Cook Pipeline.
//!
//! Transforms raw source assets (glTF, GLSL, PNG, etc.) into optimised
//! cooked artifacts in the `.cooked` format (see [`CookedAssetHeader`]).
//!
//! # Architecture
//!
//! ```text
//! assets/source/*.manifest  ──→  cook_orchestrate()
//!                                       │
//!                          ┌────────────┼────────────┐
//!                          ▼            ▼            ▼
//!                    cook_mesh()  cook_texture()  cook_shader()  …
//!                          │            │            │
//!                          └────────────┼────────────┘
//!                                       ▼
//!                              write_cooked_artifact()
//!                                       │
//!                              assets/cooked/*.cooked
//! ```

pub mod cooked_shader;
pub mod dependency;
pub mod error;
pub mod logic_asset;
pub mod manifest;
pub mod mesh;
pub mod scene;
pub mod texture;
pub mod validate;

use std::io::Write;
use std::path::{Path, PathBuf};

use engine_serialize::{AssetId, Diagnostic, HashDigest, SchemaVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// Re-exports ──────────────────────────────────────────────────────────────

pub use cooked_shader::{
    cook_shader, CookedShader, DescriptorBinding, ShaderReflection, VertexInputReflection,
};
pub use dependency::{CookState, DependencyGraph, DependencyNode};
pub use error::CookError;
pub use logic_asset::{cook_logic_asset, LogicAsset};
pub use manifest::{AssetType, CookRules, SourceAssetEntry, SourceManifest};
pub use mesh::cook_mesh;
pub use scene::cook_scene;
pub use texture::{cook_texture, CookedTexture, TextureFormat};
pub use validate::validate_assets;

// ── Constants ────────────────────────────────────────────────────────────

/// Magic bytes at the start of every cooked asset file.
pub const COOKED_MAGIC: &[u8; 8] = b"ENGCOOK\0";

/// Current version of the cooked asset header format.
pub const COOKED_HEADER_VERSION: u16 = 1;

// ── CookedAssetHeader (FD-006) ───────────────────────────────────────────

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

// ── CookResult ──────────────────────────────────────────────────────────

/// The result of cooking a single asset.
#[derive(Clone, Debug)]
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

// ── write_cooked_artifact ────────────────────────────────────────────────

/// Write a payload as a cooked artifact with its header.
///
/// # Parameters
///
/// * `output`         – path for the `.cooked` output file.
/// * `asset_kind`     – numeric kind code (see [`AssetType::kind_code`]).
/// * `payload`        – the serialised asset data.
/// * `schema_version` – schema version for the payload format.
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

    let header =
        CookedAssetHeader::new(asset_kind, schema_version, content_hash, payload.len() as u64);

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

// ── cook_orchestrate ─────────────────────────────────────────────────────

/// Run the full cook pipeline: scan manifests, dispatch cookers, write
/// artifacts, and populate the dependency graph.
///
/// # Parameters
///
/// * `source_dir` – directory containing source manifests (`.manifest` files)
///                  and referenced source assets.
/// * `cooked_dir` – directory where cooked `.cooked` artifacts are written.
/// * `graph`      – mutable [`DependencyGraph`] that is populated during
///                  cooking.
///
/// # Returns
///
/// A vector of [`CookResult`] values, one per successfully cooked asset.
/// Failures are logged via `tracing` and reflected in the dependency graph
/// via [`DependencyGraph::mark_failed`].
pub fn cook_orchestrate(
    source_dir: &Path,
    cooked_dir: &Path,
    graph: &mut DependencyGraph,
) -> Vec<CookResult> {
    let mut results = Vec::new();

    // Read the source directory.
    let entries = match std::fs::read_dir(source_dir) {
        Ok(e) => e.filter_map(|r| r.ok()).collect::<Vec<_>>(),
        Err(e) => {
            tracing::error!("failed to read source directory {:?}: {e}", source_dir);
            return results;
        }
    };

    for entry in &entries {
        let path = entry.path();

        // Only process .manifest files.
        if path.extension().map_or(true, |e| e != "manifest") {
            continue;
        }

        // Read manifest content.
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("failed to read manifest {:?}: {e}", path);
                continue;
            }
        };

        // Parse manifest (JSON format).
        let manifest: SourceManifest = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("failed to parse manifest {:?}: {e}", path);
                continue;
            }
        };

        tracing::info!(
            manifest = %path.display(),
            asset_count = manifest.assets.len(),
            "processing source manifest"
        );

        // Process each asset entry.
        for asset_entry in &manifest.assets {
            let source_path = source_dir.join(&asset_entry.source_path);
            let output_stem = asset_entry.id.id.replace('-', "_");
            let output_path = cooked_dir.join(format!("{output_stem}.cooked"));

            // Register in dependency graph.
            graph.register(asset_entry.id.clone());

            // Process cook rules for dependency tracking.
            for variant_key in &asset_entry.cook_rules.variant_keys {
                // Variants are tracked by registering a synthetic dependency.
                let variant_id = AssetId::new(format!("{}-variant-{variant_key}", asset_entry.id.id));
                graph.register(variant_id);
            }

            // Dispatch to the appropriate cooker based on asset type.
            let result = match asset_entry.asset_type {
                AssetType::Mesh => match mesh::cook_mesh(&source_path, &output_path) {
                    Ok(r) => {
                        graph.mark_cooked(&asset_entry.id, compute_file_hash(&source_path));
                        r
                    }
                    Err(e) => {
                        graph.mark_failed(&asset_entry.id, e.to_string());
                        tracing::error!("mesh cook failed: {:?}: {e}", asset_entry.id.id);
                        continue;
                    }
                },
                AssetType::Texture => match texture::cook_texture(&source_path, &output_path) {
                    Ok(r) => {
                        graph.mark_cooked(&asset_entry.id, compute_file_hash(&source_path));
                        r
                    }
                    Err(e) => {
                        graph.mark_failed(&asset_entry.id, e.to_string());
                        tracing::error!("texture cook failed: {:?}: {e}", asset_entry.id.id);
                        continue;
                    }
                },
                AssetType::Shader => {
                    let stage = determine_shader_stage(&source_path);
                    match cooked_shader::cook_shader(&source_path, &output_path, 0, &stage) {
                        Ok(r) => {
                            graph.mark_cooked(&asset_entry.id, compute_file_hash(&source_path));
                            r
                        }
                        Err(e) => {
                            graph.mark_failed(&asset_entry.id, e.to_string());
                            tracing::error!("shader cook failed: {:?}: {e}", asset_entry.id.id);
                            continue;
                        }
                    }
                }
                AssetType::Scene => match scene::cook_scene(&source_path, &output_path, 0) {
                    Ok(r) => {
                        graph.mark_cooked(&asset_entry.id, compute_file_hash(&source_path));
                        r
                    }
                    Err(e) => {
                        graph.mark_failed(&asset_entry.id, e.to_string());
                        tracing::error!("scene cook failed: {:?}: {e}", asset_entry.id.id);
                        continue;
                    }
                },
                AssetType::Logic => match logic_asset::cook_logic_asset(&source_path, &output_path) {
                    Ok(r) => {
                        graph.mark_cooked(&asset_entry.id, compute_file_hash(&source_path));
                        r
                    }
                    Err(e) => {
                        graph.mark_failed(&asset_entry.id, e.to_string());
                        tracing::error!("logic asset cook failed: {:?}: {e}", asset_entry.id.id);
                        continue;
                    }
                },
                _ => {
                    // Unsupported asset type — emit diagnostic and skip.
                    graph.mark_failed(
                        &asset_entry.id,
                        format!("unsupported asset type: {:?}", asset_entry.asset_type),
                    );
                    tracing::warn!(
                        "unsupported asset type {:?} for {:?}",
                        asset_entry.asset_type,
                        asset_entry.id.id
                    );
                    continue;
                }
            };

            results.push(result);
        }
    }

    results
}

// ── Helpers ──────────────────────────────────────────────────────────────

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

/// Compute the SHA-256 hash of a file's contents.
fn compute_file_hash(path: &Path) -> HashDigest {
    let data = std::fs::read(path).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&data);
    hasher.finalize().into()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooked_asset_header_creation() {
        let hash = [0xABu8; 32];
        let header = CookedAssetHeader::new(1, SchemaVersion::new(0, 1, 0), hash, 4096);
        assert_eq!(&header.magic, COOKED_MAGIC);
        assert_eq!(header.header_version, 1);
        assert_eq!(header.asset_kind, 1);
        assert_eq!(header.uncompressed_size, 4096);
        assert!(header.is_valid());
    }

    #[test]
    fn cooked_asset_header_invalid_magic() {
        let mut header = CookedAssetHeader::new(1, SchemaVersion::new(0, 1, 0), [0u8; 32], 100);
        header.magic = [0; 8];
        assert!(!header.is_valid());
    }

    #[test]
    fn cooked_asset_header_serde_roundtrip() {
        let hash = [0x42u8; 32];
        let header = CookedAssetHeader::new(3, SchemaVersion::new(1, 0, 0), hash, 8192);
        let bytes = bincode::serialize(&header).unwrap();
        let restored: CookedAssetHeader = bincode::deserialize(&bytes).unwrap();
        assert!(restored.is_valid());
        assert_eq!(restored.asset_kind, 3);
        assert_eq!(restored.schema_version, SchemaVersion::new(1, 0, 0));
        assert_eq!(restored.content_hash, hash);
        assert_eq!(restored.uncompressed_size, 8192);
    }

    #[test]
    fn write_and_read_cooked_artifact() {
        use std::io::Read;

        let dir = std::env::temp_dir().join("cook_test_write_read");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let output = dir.join("test_mesh.cooked");
        let payload = vec![0x01, 0x02, 0x03, 0x04];

        let result = write_cooked_artifact(&output, 1, &payload, SchemaVersion::new(0, 1, 0))
            .unwrap();
        assert!(result.success);
        assert_eq!(result.asset_id, "test_mesh");

        // Read back and verify.
        let mut file = std::fs::File::open(&output).unwrap();
        let mut file_bytes = Vec::new();
        file.read_to_end(&mut file_bytes).unwrap();

        // Header size: bincode serialized size of CookedAssetHeader.
        let header: CookedAssetHeader =
            bincode::deserialize(&file_bytes[..]).unwrap();
        assert!(header.is_valid());
        assert_eq!(header.asset_kind, 1);

        // Payload after header.
        let header_size = bincode::serialized_size(&header).unwrap() as usize;
        let read_payload = &file_bytes[header_size..];
        assert_eq!(read_payload, &payload);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn determine_shader_stage_by_extension() {
        assert_eq!(
            determine_shader_stage(Path::new("shader.vert")),
            "vertex"
        );
        assert_eq!(
            determine_shader_stage(Path::new("shader.frag")),
            "fragment"
        );
        assert_eq!(
            determine_shader_stage(Path::new("shader.comp")),
            "compute"
        );
        assert_eq!(
            determine_shader_stage(Path::new("shader.unknown")),
            "vertex"
        );
    }

    #[test]
    fn asset_type_kind_code_mapping() {
        assert_eq!(AssetType::Mesh.kind_code(), 1);
        assert_eq!(AssetType::from_kind_code(1), AssetType::Mesh);
        assert_eq!(AssetType::Unknown.kind_code(), 0xFFFF);
        assert_eq!(AssetType::from_kind_code(0xFFFF), AssetType::Unknown);
    }
}
