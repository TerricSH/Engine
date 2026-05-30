use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;

use engine_serialize::AssetId;
use tracing::{debug, info};

use crate::loader::{AssetError, AssetHandle, AssetLoader};
use crate::path::asset_path;

// ---------------------------------------------------------------------------
// AssetState
// ---------------------------------------------------------------------------

/// The lifecycle state of an asset in the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetState {
    /// The asset is currently being loaded (fetch / deserialize in progress).
    Loading,
    /// The asset has been loaded and is available in the cache.
    Ready,
    /// Loading the asset failed.
    Failed(String),
}

// ---------------------------------------------------------------------------
// AssetInfo
// ---------------------------------------------------------------------------

/// Metadata about a loaded or pending asset.
#[derive(Clone, Debug)]
pub struct AssetInfo {
    /// The unique identifier for this asset.
    pub id: AssetId,
    /// The filesystem path the asset was loaded from, if known.
    pub path: Option<String>,
    /// The current lifecycle state of the asset.
    pub state: AssetState,
}

// ---------------------------------------------------------------------------
// Cache internals
// ---------------------------------------------------------------------------

pub struct CachedEntry {
    pub(crate) _info: AssetInfo,
    pub(crate) raw_bytes: Arc<Vec<u8>>,
    pub(crate) typed: Option<Arc<dyn Any + Send + Sync + 'static>>,
}

// ---------------------------------------------------------------------------
// AssetRegistry
// ---------------------------------------------------------------------------

/// Central registry for loading, caching, and managing assets.
///
/// Loaders are registered by file extension.  When
/// [`load_typed`](Self::load_typed) is called the registry reads the file,
/// finds the matching loader, deserializes, and caches the result.  Raw
/// byte access is provided by [`load`](Self::load).
///
/// # Example
///
/// ```ignore
/// use engine_asset::{AssetRegistry, BincodeLoader};
/// use engine_serialize::AssetId;
///
/// #[derive(serde::Deserialize)]
/// struct MyAsset { name: String }
///
/// let mut reg = AssetRegistry::new();
/// reg.register_loader(BincodeLoader::<MyAsset>::new(vec!["my"]));
///
/// let handle = reg.load_typed::<MyAsset>(&AssetId::new("my-example")).unwrap();
/// assert_eq!(handle.get().name, "example");
/// ```
pub struct AssetRegistry {
    loaders: BTreeMap<String, Arc<dyn AssetLoader>>,
    cache: BTreeMap<AssetId, CachedEntry>,
}

impl AssetRegistry {
    /// Create an empty registry.
    ///
    /// No loaders are registered by default; call
    /// [`register_loader`](Self::register_loader) to add them before
    /// attempting typed loads.
    pub fn new() -> Self {
        Self {
            loaders: BTreeMap::new(),
            cache: BTreeMap::new(),
        }
    }

    /// Register a loader for every file extension it declares.
    ///
    /// The loader is wrapped in [`Arc`] and shared across all its
    /// extensions so a single instance handles every extension returned by
    /// [`AssetLoader::extensions`].
    pub fn register_loader<L: AssetLoader + 'static>(&mut self, loader: L) {
        let shared: Arc<dyn AssetLoader> = Arc::new(loader);
        let exts: Vec<String> = shared.extensions().iter().map(|e| e.to_string()).collect();
        for ext in exts {
            info!(extension = %ext, "registering asset loader");
            self.loaders.insert(ext, Arc::clone(&shared));
        }
    }

    /// Load an asset as raw bytes.
    ///
    /// Resolves the filesystem path via [`asset_path`], reads the file,
    /// caches it, and returns an [`AssetHandle<Vec<u8>>`].
    ///
    /// Subsequent calls return the cached handle without re-reading disk.
    pub fn load(&mut self, id: &AssetId) -> Result<AssetHandle<Vec<u8>>, AssetError> {
        // Fast path – already cached.
        if let Some(entry) = self.cache.get(id) {
            debug!(asset_id = %id.id, "load (raw) cache hit");
            return Ok(AssetHandle {
                id: id.clone(),
                inner: Arc::clone(&entry.raw_bytes),
            });
        }

        let path = asset_path(id).ok_or_else(|| AssetError::NotFound(id.clone()))?;
        info!(asset_id = %id.id, path = %path.display(), "loading raw asset");

        let bytes = std::fs::read(&path).map_err(|e| AssetError::LoadFailed {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;

        let raw_bytes = Arc::new(bytes);

        self.cache.insert(
            id.clone(),
            CachedEntry {
                _info: AssetInfo {
                    id: id.clone(),
                    path: Some(path.display().to_string()),
                    state: AssetState::Ready,
                },
                raw_bytes: Arc::clone(&raw_bytes),
                typed: None,
            },
        );

        Ok(AssetHandle {
            id: id.clone(),
            inner: raw_bytes,
        })
    }

    /// Load (or retrieve from cache) an asset deserialized to `T`.
    ///
    /// A registered loader whose extensions match the file extension must
    /// exist; otherwise [`AssetError::UnsupportedFormat`] is returned.
    ///
    /// If the raw bytes are already cached the disk read is skipped.
    pub fn load_typed<T: Send + Sync + 'static>(
        &mut self,
        id: &AssetId,
    ) -> Result<AssetHandle<T>, AssetError> {
        // Fast path – typed data already cached and matches the requested type.
        if let Some(entry) = self.cache.get(id) {
            if let Some(ref typed) = entry.typed {
                if let Ok(arc) = Arc::downcast::<T>(Arc::clone(typed)) {
                    debug!(asset_id = %id.id, "load_typed cache hit");
                    return Ok(AssetHandle {
                        id: id.clone(),
                        inner: arc,
                    });
                }
            }
        }

        let path = asset_path(id).ok_or_else(|| AssetError::NotFound(id.clone()))?;

        let (raw_bytes, raw_arc) = if let Some(entry) = self.cache.get(id) {
            // Raw bytes are already cached – avoid a second disk read.
            (Vec::clone(&entry.raw_bytes), Arc::clone(&entry.raw_bytes))
        } else {
            let bytes = std::fs::read(&path).map_err(|e| AssetError::LoadFailed {
                path: path.display().to_string(),
                detail: e.to_string(),
            })?;
            let arc = Arc::new(bytes);
            (Vec::clone(&arc), arc)
        };

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();

        let loader = self
            .loaders
            .get(&ext)
            .ok_or(AssetError::UnsupportedFormat)?;

        info!(asset_id = %id.id, extension = %ext, "loading typed asset");

        let boxed = loader.load(id, &raw_bytes)?;
        let typed: Box<T> = boxed
            .downcast::<T>()
            .map_err(|_| AssetError::TypeMismatch)?;
        let typed_arc = Arc::from(typed);

        self.cache.insert(
            id.clone(),
            CachedEntry {
                _info: AssetInfo {
                    id: id.clone(),
                    path: Some(path.display().to_string()),
                    state: AssetState::Ready,
                },
                raw_bytes: raw_arc,
                typed: Some(Arc::clone(&typed_arc) as Arc<dyn Any + Send + Sync>),
            },
        );

        Ok(AssetHandle {
            id: id.clone(),
            inner: typed_arc,
        })
    }

    /// Retrieve a previously loaded typed asset from the cache.
    ///
    /// Returns `None` if:
    /// - The asset has not been loaded via [`load_typed`](Self::load_typed), or
    /// - The type parameter `T` does not match the type the asset was
    ///   originally loaded as.
    pub fn get<T: Send + Sync + 'static>(&self, id: &AssetId) -> Option<AssetHandle<T>> {
        let entry = self.cache.get(id)?;
        let typed = entry.typed.as_ref()?;
        let arc = Arc::downcast::<T>(Arc::clone(typed)).ok()?;
        Some(AssetHandle {
            id: id.clone(),
            inner: arc,
        })
    }

    /// Returns `true` if the asset is currently in the cache.
    pub fn contains(&self, id: &AssetId) -> bool {
        self.cache.contains_key(id)
    }

    /// Remove an asset from the cache.
    ///
    /// Returns `true` if the asset was present and has been removed.
    ///
    /// Existing [`AssetHandle`]s remain valid because they hold their own
    /// [`Arc`] reference to the data.
    pub fn unload(&mut self, id: &AssetId) -> bool {
        self.cache.remove(id).is_some()
    }

    /// Remove and re-load an asset from disk (raw bytes only).
    ///
    /// After a successful reload the raw bytes are cached, but any typed
    /// data is discarded and must be re-requested via
    /// [`load_typed`](Self::load_typed).
    pub fn reload(&mut self, id: &AssetId) -> Result<(), AssetError> {
        self.cache.remove(id);
        self.load(id)?;
        Ok(())
    }

    /// Return all cached asset IDs (for editor browsing).
    pub fn cached_ids(&self) -> Vec<AssetId> {
        self.cache.keys().cloned().collect()
    }

    /// Return the number of assets currently being loaded asynchronously.
    ///
    /// All loads in this implementation are synchronous, so this always
    /// returns `0`.  When asynchronous / background loading is added in a
    /// future gate this will track in-flight requests.
    pub fn pending_loads(&self) -> usize {
        0
    }
}

impl Default for AssetRegistry {
    fn default() -> Self {
        Self::new()
    }
}
