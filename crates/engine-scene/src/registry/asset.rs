use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

/// Metadata about a registered asset type.
#[derive(Clone, Debug)]
pub struct AssetTypeMeta {
    pub type_id: &'static str,
    pub source_extensions: Vec<&'static str>,
    pub display_name: &'static str,
}

// ---------------------------------------------------------------------------
// Type aliases for extension hooks
// ---------------------------------------------------------------------------

/// Asset cooker: produces cooked data from source bytes.
pub type CookerFn = fn(source: &[u8], output: &mut Vec<u8>) -> Result<(), String>;

/// Asset loader: loads cooked data into a runtime asset.
pub type LoaderFn = fn(cooked: &[u8]) -> Result<Box<dyn std::any::Any>, String>;

// ---------------------------------------------------------------------------
// AssetTypeExtension
// ---------------------------------------------------------------------------

/// A registered asset type extension.
pub struct AssetTypeExtension {
    pub meta: AssetTypeMeta,
    pub cooker: Option<CookerFn>,
    pub loader: Option<LoaderFn>,
}

impl std::fmt::Debug for AssetTypeExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetTypeExtension")
            .field("meta", &self.meta)
            .field("cooker", &self.cooker.map(|_| "(fn)"))
            .field("loader", &self.loader.map(|_| "(fn)"))
            .finish()
    }
}

// ---------------------------------------------------------------------------
// AssetTypeRegistry
// ---------------------------------------------------------------------------

/// Registry for asset types.
///
/// Allows subsystems to register asset types (mesh, texture, audio, …) with
/// their own cooker and loader functions.
pub struct AssetTypeRegistry {
    extensions: BTreeMap<&'static str, AssetTypeExtension>,
}

impl AssetTypeRegistry {
    pub fn new() -> Self {
        Self {
            extensions: BTreeMap::new(),
        }
    }

    /// Register a new asset type extension.
    ///
    /// Returns `Err` with the type ID if an asset type with the same `type_id`
    /// is already registered.
    pub fn register(&mut self, ext: AssetTypeExtension) -> Result<(), &'static str> {
        let type_id = ext.meta.type_id;
        if self.extensions.contains_key(type_id) {
            return Err(type_id);
        }
        self.extensions.insert(type_id, ext);
        Ok(())
    }

    /// Get extension metadata by type ID.
    pub fn get(&self, type_id: &str) -> Option<&AssetTypeExtension> {
        self.extensions.get(type_id)
    }

    /// Find a cooker function by source file extension (e.g. `"glb"`, `"wav"`).
    ///
    /// Returns the first cooker whose `source_extensions` list contains the
    /// given extension, or `None` if no match is found.
    pub fn cooker_for(&self, extension: &str) -> Option<&CookerFn> {
        self.extensions.values().find_map(|ext| {
            if ext.meta.source_extensions.contains(&extension) {
                ext.cooker.as_ref()
            } else {
                None
            }
        })
    }
}

impl Default for AssetTypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}
