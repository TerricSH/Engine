use engine_serialize::{AssetId, SchemaVersion};
use serde::{Deserialize, Serialize};

/// The schema version for source manifest files.
pub const CURRENT_MANIFEST_VERSION: SchemaVersion = SchemaVersion::new(0, 1, 0);

/// A source manifest file describing assets to cook.
///
/// Manifests are JSON (or RON) files stored under `assets/source/` that list
/// the source assets, their types, and cooking rules.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceManifest {
    /// Schema version for forward-compatibility.
    pub schema_version: SchemaVersion,
    /// The list of source asset entries in this manifest.
    pub assets: Vec<SourceAssetEntry>,
}

/// A single source asset entry in a manifest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceAssetEntry {
    /// The unique asset identifier.
    pub id: AssetId,
    /// The type of asset (determines which cooker to use).
    pub asset_type: AssetType,
    /// Relative or absolute path to the source file.
    pub source_path: String,
    /// Cooking rules and options.
    pub cook_rules: CookRules,
}

/// The type of an asset, determining which cooker to dispatch to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetType {
    Mesh,
    Texture,
    Shader,
    Scene,
    Material,
    Pipeline,
    Script,
    Audio,
    Font,
    Animation,
    Skeleton,
    NavMesh,
    /// Logic asset (behavior graph / state machine / skill graph / quest dialogue).
    Logic,
    /// Catch-all for unknown or user-defined asset types.
    Unknown,
}

impl AssetType {
    /// Return a numeric kind code used in the [`CookedAssetHeader`].
    ///
    /// These codes are stable across engine versions.
    pub fn kind_code(&self) -> u16 {
        match self {
            AssetType::Mesh => 1,
            AssetType::Texture => 2,
            AssetType::Shader => 3,
            AssetType::Scene => 4,
            AssetType::Material => 5,
            AssetType::Pipeline => 6,
            AssetType::Script => 7,
            AssetType::Audio => 8,
            AssetType::Font => 9,
            AssetType::Animation => 10,
            AssetType::Skeleton => 11,
            AssetType::NavMesh => 12,
            AssetType::Logic => 13,
            AssetType::Unknown => 0xFFFF,
        }
    }

    /// Parse an `AssetType` from its kind code.
    pub fn from_kind_code(code: u16) -> Self {
        match code {
            1 => AssetType::Mesh,
            2 => AssetType::Texture,
            3 => AssetType::Shader,
            4 => AssetType::Scene,
            5 => AssetType::Material,
            6 => AssetType::Pipeline,
            7 => AssetType::Script,
            8 => AssetType::Audio,
            9 => AssetType::Font,
            10 => AssetType::Animation,
            11 => AssetType::Skeleton,
            12 => AssetType::NavMesh,
            13 => AssetType::Logic,
            _ => AssetType::Unknown,
        }
    }
}

/// Cooking rules and options for a source asset.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CookRules {
    /// Variant keys for shader variants (FD-040).
    pub variant_keys: Vec<String>,
    /// Per-platform options (e.g. "mobile", "desktop").
    pub platform_overrides: Vec<String>,
    /// Optional compression codec name (e.g. "zstd", "lz4").
    pub compression: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_manifest_serialize_roundtrip() {
        let manifest = SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets: vec![SourceAssetEntry {
                id: AssetId::new("mesh-cube"),
                asset_type: AssetType::Mesh,
                source_path: "models/cube.gltf".into(),
                cook_rules: CookRules::default(),
            }],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let restored: SourceManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.schema_version, CURRENT_MANIFEST_VERSION);
        assert_eq!(restored.assets.len(), 1);
        assert_eq!(restored.assets[0].id.id, "mesh-cube");
        assert_eq!(restored.assets[0].asset_type, AssetType::Mesh);
    }

    #[test]
    fn empty_manifest() {
        let manifest = SourceManifest {
            schema_version: CURRENT_MANIFEST_VERSION,
            assets: vec![],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let restored: SourceManifest = serde_json::from_str(&json).unwrap();
        assert!(restored.assets.is_empty());
    }

    #[test]
    fn asset_type_kind_code_roundtrip() {
        let cases = vec![
            AssetType::Mesh,
            AssetType::Texture,
            AssetType::Shader,
            AssetType::Scene,
            AssetType::Material,
            AssetType::Pipeline,
            AssetType::Script,
            AssetType::Audio,
            AssetType::Font,
            AssetType::Animation,
            AssetType::Skeleton,
            AssetType::NavMesh,
            AssetType::Logic,
            AssetType::Unknown,
        ];
        for ty in cases {
            let code = ty.kind_code();
            let restored = AssetType::from_kind_code(code);
            assert_eq!(ty, restored, "kind code {code} roundtrip failed");
        }
    }

    #[test]
    fn cook_rules_default() {
        let rules = CookRules::default();
        assert!(rules.variant_keys.is_empty());
        assert!(rules.platform_overrides.is_empty());
        assert!(rules.compression.is_none());
    }

    #[test]
    fn cook_rules_serialize() {
        let rules = CookRules {
            variant_keys: vec!["forward".into(), "deferred".into()],
            platform_overrides: vec!["mobile".into()],
            compression: Some("zstd".into()),
        };
        let json = serde_json::to_string(&rules).unwrap();
        let restored: CookRules = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.variant_keys.len(), 2);
        assert_eq!(restored.platform_overrides.len(), 1);
        assert_eq!(restored.compression, Some("zstd".into()));
    }
}
