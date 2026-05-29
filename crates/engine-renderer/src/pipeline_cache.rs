//! Material-to-pipeline cache.
//!
//! Maps `(pipeline_asset_id, variant_key)` pairs to concrete
//! [`PipelineHandle`]s, creating them lazily through the [`Device`] trait.
//! Supports eviction and hot-reload by clearing cached pipelines for a given
//! asset.
//!
//! # Hot-reload
//!
//! When a shader asset is modified at runtime, call [`evict_asset`] with the
//! asset ID so the next frame re-creates the pipeline from the updated
//! descriptor.

use std::collections::HashMap;

use render_core::{Device, PipelineDescriptor, PipelineHandle, PipelineVariantKey};

/// Resolution key: (pipeline_asset_id, variant_key).
///
/// Uniquely identifies a pipeline permutation in the cache.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PipelineCacheKey {
    pub pipeline_asset: String,
    pub variant_key: PipelineVariantKey,
}

/// Manages pipeline creation and caching.
///
/// Maintains two maps:
/// - `pipelines`: the live cache of `(asset, variant) → PipelineHandle`
/// - `descriptor_cache`: pre-registered [`PipelineDescriptor`]s keyed by
///   asset ID, used to lazily create pipelines on first access.
pub struct PipelineCache {
    pipelines: HashMap<PipelineCacheKey, PipelineHandle>,
    descriptor_cache: HashMap<String, PipelineDescriptor>,
}

impl PipelineCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            pipelines: HashMap::new(),
            descriptor_cache: HashMap::new(),
        }
    }

    /// Get or create a pipeline for the given key.
    ///
    /// If a pipeline already exists for this key it is returned directly.
    /// Otherwise the descriptor registered via [`register_descriptor`] is
    /// looked up and a new pipeline is created through `device`.
    ///
    /// # Errors
    ///
    /// Returns an error if no descriptor has been registered for the asset ID
    /// or if the underlying [`Device::create_pipeline_variant`] fails.
    pub fn get_or_create(
        &mut self,
        key: &PipelineCacheKey,
        device: &mut dyn Device,
    ) -> Result<PipelineHandle, render_core::RhiError> {
        // Fast path: already cached.
        if let Some(&handle) = self.pipelines.get(key) {
            return Ok(handle);
        }

        // Look up the pipeline descriptor for this asset.
        let descriptor = self
            .descriptor_cache
            .get(&key.pipeline_asset)
            .ok_or_else(|| render_core::RhiError::Backend {
                detail: format!(
                    "PipelineCache: no descriptor registered for asset '{}'",
                    key.pipeline_asset
                ),
            })?;

        // Create the pipeline through the device with the variant key.
        let handle = device.create_pipeline_variant(descriptor, key.variant_key)?;

        self.pipelines.insert(key.clone(), handle);
        Ok(handle)
    }

    /// Pre-register a [`PipelineDescriptor`] for a pipeline asset ID.
    ///
    /// This is typically called during scene initialisation (e.g. in
    /// [`crate::BackendRenderer::begin_frame`]) before any drawables are
    /// processed.  The descriptor is stored and used later by
    /// [`get_or_create`] when a pipeline needs to be created.
    pub fn register_descriptor(
        &mut self,
        asset_id: impl Into<String>,
        descriptor: PipelineDescriptor,
    ) {
        self.descriptor_cache.insert(asset_id.into(), descriptor);
    }

    /// Remove a cached pipeline for a specific key.
    ///
    /// The next call to [`get_or_create`] for the same key will re-create the
    /// pipeline.  This is useful when a pipeline needs to be invalidated
    /// without affecting other variants of the same asset.
    pub fn evict(&mut self, key: &PipelineCacheKey) {
        self.pipelines.remove(key);
    }

    /// Evict all cached pipelines for a given asset (for hot-reload).
    ///
    /// All variants of the asset will be re-created on the next frame.  The
    /// registered descriptor is kept so re-creation can happen immediately.
    pub fn evict_asset(&mut self, asset_id: &str) {
        self.pipelines.retain(|k, _| k.pipeline_asset != asset_id);
    }

    /// Clear all cached pipelines.
    ///
    /// All pipelines will be re-created on subsequent frames.  Registered
    /// descriptors are preserved.
    pub fn clear(&mut self) {
        self.pipelines.clear();
    }

    /// Number of cached pipeline entries.
    pub fn len(&self) -> usize {
        self.pipelines.len()
    }

    /// Returns `true` if the cache contains no pipelines.
    pub fn is_empty(&self) -> bool {
        self.pipelines.is_empty()
    }
}

impl Default for PipelineCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_core::{
        BlendState, DepthState, PipelineDescriptor, PipelineVariantKey, RasterState,
        VertexLayout,
    };

    /// A minimal device implementation that records created pipelines.
    struct FakeDevice {
        next_handle: u32,
        created: Vec<(PipelineDescriptor, PipelineVariantKey)>,
    }

    impl FakeDevice {
        fn new() -> Self {
            Self {
                next_handle: 1,
                created: Vec::new(),
            }
        }
    }

    impl Device for FakeDevice {
        fn adapter_info(&self) -> &render_core::AdapterInfo {
            unimplemented!()
        }

        fn create_pipeline(
            &mut self,
            _desc: &PipelineDescriptor,
        ) -> Result<PipelineHandle, render_core::RhiError> {
            let handle = PipelineHandle::new(self.next_handle, 1);
            self.next_handle += 1;
            self.created.push((_desc.clone(), PipelineVariantKey::NONE));
            Ok(handle)
        }

        fn create_pipeline_variant(
            &mut self,
            desc: &PipelineDescriptor,
            variant_key: PipelineVariantKey,
        ) -> Result<PipelineHandle, render_core::RhiError> {
            let handle = PipelineHandle::new(self.next_handle, 1);
            self.next_handle += 1;
            self.created.push((desc.clone(), variant_key));
            Ok(handle)
        }
    }

    fn dummy_descriptor() -> PipelineDescriptor {
        PipelineDescriptor {
            vertex_layout: VertexLayout::default(),
            raster_state: RasterState::default(),
            depth_state: DepthState::default(),
            blend_state: BlendState::default(),
            render_targets: vec![],
            debug_label: Some("test".into()),
            ..PipelineDescriptor::default()
        }
    }

    #[test]
    fn empty_cache_has_len_zero() {
        let cache = PipelineCache::new();
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn get_or_create_creates_pipeline_on_miss() {
        let mut cache = PipelineCache::new();
        let mut device = FakeDevice::new();

        let desc = dummy_descriptor();
        cache.register_descriptor("test_pipeline", desc.clone());

        let key = PipelineCacheKey {
            pipeline_asset: "test_pipeline".into(),
            variant_key: PipelineVariantKey::SKINNED,
        };

        let handle = cache.get_or_create(&key, &mut device).unwrap();
        assert_eq!(handle.index, 1);
        assert_eq!(cache.len(), 1);

        // Verify device was called with the variant key
        assert_eq!(device.created.len(), 1);
        assert_eq!(device.created[0].1, PipelineVariantKey::SKINNED);
    }

    #[test]
    fn get_or_create_returns_cached_pipeline() {
        let mut cache = PipelineCache::new();
        let mut device = FakeDevice::new();

        let desc = dummy_descriptor();
        cache.register_descriptor("test_pipeline", desc);

        let key = PipelineCacheKey {
            pipeline_asset: "test_pipeline".into(),
            variant_key: PipelineVariantKey::NONE,
        };

        let h1 = cache.get_or_create(&key, &mut device).unwrap();
        let h2 = cache.get_or_create(&key, &mut device).unwrap();
        assert_eq!(h1, h2);
        // Device should only have been called once
        assert_eq!(device.created.len(), 1);
    }

    #[test]
    fn different_variants_produce_different_pipelines() {
        let mut cache = PipelineCache::new();
        let mut device = FakeDevice::new();

        let desc = dummy_descriptor();
        cache.register_descriptor("test_pipeline", desc);

        let key_default = PipelineCacheKey {
            pipeline_asset: "test_pipeline".into(),
            variant_key: PipelineVariantKey::NONE,
        };
        let key_skinned = PipelineCacheKey {
            pipeline_asset: "test_pipeline".into(),
            variant_key: PipelineVariantKey::SKINNED,
        };

        let h1 = cache.get_or_create(&key_default, &mut device).unwrap();
        let h2 = cache.get_or_create(&key_skinned, &mut device).unwrap();
        assert_ne!(h1, h2);
        assert_eq!(device.created.len(), 2);
    }

    #[test]
    fn evict_removes_single_entry() {
        let mut cache = PipelineCache::new();
        let mut device = FakeDevice::new();

        let desc = dummy_descriptor();
        cache.register_descriptor("test_pipeline", desc);

        let key = PipelineCacheKey {
            pipeline_asset: "test_pipeline".into(),
            variant_key: PipelineVariantKey::NONE,
        };

        cache.get_or_create(&key, &mut device).unwrap();
        assert_eq!(cache.len(), 1);

        cache.evict(&key);
        assert_eq!(cache.len(), 0);

        // After eviction, get_or_create should re-create
        cache.get_or_create(&key, &mut device).unwrap();
        assert_eq!(device.created.len(), 2);
    }

    #[test]
    fn evict_asset_removes_all_variants() {
        let mut cache = PipelineCache::new();
        let mut device = FakeDevice::new();

        let desc = dummy_descriptor();
        cache.register_descriptor("test_pipeline", desc);

        let keys = [
            PipelineCacheKey {
                pipeline_asset: "test_pipeline".into(),
                variant_key: PipelineVariantKey::NONE,
            },
            PipelineCacheKey {
                pipeline_asset: "test_pipeline".into(),
                variant_key: PipelineVariantKey::SKINNED,
            },
            PipelineCacheKey {
                pipeline_asset: "test_pipeline".into(),
                variant_key: PipelineVariantKey::INSTANCED,
            },
        ];

        for k in &keys {
            cache.get_or_create(k, &mut device).unwrap();
        }
        assert_eq!(cache.len(), 3);

        cache.evict_asset("test_pipeline");
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn clear_removes_all_entries() {
        let mut cache = PipelineCache::new();
        let mut device = FakeDevice::new();

        let desc = dummy_descriptor();
        cache.register_descriptor("a", desc.clone());
        cache.register_descriptor("b", desc);

        let ka = PipelineCacheKey {
            pipeline_asset: "a".into(),
            variant_key: PipelineVariantKey::NONE,
        };
        let kb = PipelineCacheKey {
            pipeline_asset: "b".into(),
            variant_key: PipelineVariantKey::NONE,
        };

        cache.get_or_create(&ka, &mut device).unwrap();
        cache.get_or_create(&kb, &mut device).unwrap();
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn get_or_create_missing_descriptor_returns_error() {
        let mut cache = PipelineCache::new();
        let mut device = FakeDevice::new();

        let key = PipelineCacheKey {
            pipeline_asset: "unknown".into(),
            variant_key: PipelineVariantKey::NONE,
        };

        let result = cache.get_or_create(&key, &mut device);
        assert!(result.is_err());
    }

    #[test]
    fn register_descriptor_replaces_previous() {
        let mut cache = PipelineCache::new();

        let desc_a = PipelineDescriptor {
            debug_label: Some("a".into()),
            ..dummy_descriptor()
        };
        let desc_b = PipelineDescriptor {
            debug_label: Some("b".into()),
            ..dummy_descriptor()
        };

        cache.register_descriptor("test", desc_a);
        cache.register_descriptor("test", desc_b);

        // Only the last descriptor should be stored
        assert_eq!(
            cache
                .descriptor_cache
                .get("test")
                .unwrap()
                .debug_label
                .as_deref(),
            Some("b")
        );
    }
}
