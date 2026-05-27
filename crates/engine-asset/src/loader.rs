use std::any::Any;
use std::marker::PhantomData;
use std::sync::Arc;

use engine_serialize::AssetId;
use serde::de::DeserializeOwned;
use thiserror::Error;

// CachedEntry is defined in the registry module but re-exported here so that
// `engine_asset::CachedEntry` resolves through the `loader` re-export path.
pub use crate::registry::CachedEntry;

// ---------------------------------------------------------------------------
// AssetError
// ---------------------------------------------------------------------------

/// Errors that can occur during asset operations.
#[derive(Error, Debug)]
pub enum AssetError {
    /// The requested asset was not found in the registry or on disk.
    #[error("asset not found: {0:?}")]
    NotFound(AssetId),

    /// Loading the asset failed with a specific path and detail message.
    #[error("failed to load asset at {path}: {detail}")]
    LoadFailed {
        /// Filesystem path that was read.
        path: String,
        /// Human-readable error detail.
        detail: String,
    },

    /// No loader is registered for the asset's file extension.
    #[error("unsupported asset format")]
    UnsupportedFormat,

    /// The loaded data could not be downcast to the requested type parameter.
    #[error("asset type mismatch")]
    TypeMismatch,

    /// An error from the file watcher subsystem.
    #[error("file watcher error: {0}")]
    WatcherFailed(String),

    /// An underlying I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// AssetHandle
// ---------------------------------------------------------------------------

/// A shared handle to a loaded asset.
///
/// Internally wraps [`Arc`] so cloning is cheap and the asset data lives
/// as long as any handle (or the registry cache) holds a reference.
pub struct AssetHandle<T> {
    pub(crate) id: AssetId,
    pub(crate) inner: Arc<T>,
}

impl<T> AssetHandle<T> {
    /// Create a new handle from an id and value.
    pub fn new(id: AssetId, data: T) -> Self {
        Self {
            id,
            inner: Arc::new(data),
        }
    }

    /// Borrow the inner asset data.
    pub fn get(&self) -> &T {
        &self.inner
    }

    /// The [`AssetId`] of this asset.
    pub fn id(&self) -> &AssetId {
        &self.id
    }
}

impl<T> Clone for AssetHandle<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            inner: Arc::clone(&self.inner),
        }
    }
}

// ---------------------------------------------------------------------------
// AssetLoader trait
// ---------------------------------------------------------------------------

/// A loader that converts raw bytes into a typed asset value.
///
/// Each loader declares which file extensions it handles via
/// [`extensions`](Self::extensions).  The [`AssetRegistry`] dispatches to the
/// matching loader when `load_typed` is called.
///
/// Implementations must be [`Send`] + `'static` so they can be stored
/// behind [`Arc`] inside the registry.
pub trait AssetLoader: Send + 'static {
    /// File extensions this loader supports (e.g. `["mesh", "model"]`).
    ///
    /// Extensions are matched case-sensitively against the file extension
    /// of the resolved asset path.
    fn extensions(&self) -> &[&str];

    /// Decode `data` (the full file contents) into a boxed value.
    fn load(&self, id: &AssetId, data: &[u8]) -> Result<Box<dyn Any + Send>, AssetError>;
}

// ---------------------------------------------------------------------------
// RawLoader
// ---------------------------------------------------------------------------

/// A loader that passes through raw bytes as [`Vec<u8>`].
///
/// Handles the `.asset` and `.bin` file extensions by default.
pub struct RawLoader;

impl AssetLoader for RawLoader {
    fn extensions(&self) -> &[&str] {
        &["asset", "bin"]
    }

    fn load(&self, _id: &AssetId, data: &[u8]) -> Result<Box<dyn Any + Send>, AssetError> {
        Ok(Box::new(data.to_vec()))
    }
}

// ---------------------------------------------------------------------------
// BincodeLoader
// ---------------------------------------------------------------------------

/// A loader that deserialises bytes with [`bincode`].
///
/// Generic over any type that implements [`DeserializeOwned`] + [`Send`].
///
/// # Example
///
/// ```ignore
/// use engine_asset::BincodeLoader;
///
/// #[derive(serde::Deserialize)]
/// struct MyAsset { value: i32 }
///
/// let loader = BincodeLoader::<MyAsset>::new(vec!["myasset"]);
/// ```
pub struct BincodeLoader<T> {
    extensions: Vec<&'static str>,
    _phantom: PhantomData<T>,
}

impl<T: DeserializeOwned + Send + 'static> BincodeLoader<T> {
    /// Create a new `BincodeLoader` that handles the given extensions.
    pub fn new(extensions: Vec<&'static str>) -> Self {
        Self {
            extensions,
            _phantom: PhantomData,
        }
    }
}

impl<T: DeserializeOwned + Send + 'static> AssetLoader for BincodeLoader<T> {
    fn extensions(&self) -> &[&str] {
        &self.extensions
    }

    fn load(&self, id: &AssetId, data: &[u8]) -> Result<Box<dyn Any + Send>, AssetError> {
        let value: T = bincode::deserialize(data).map_err(|e| AssetError::LoadFailed {
            path: id.id.clone(),
            detail: format!("bincode deserialize error: {e}"),
        })?;
        Ok(Box::new(value))
    }
}
