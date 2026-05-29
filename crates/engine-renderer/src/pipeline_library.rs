use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use render_core::{Device, PipelineDescriptor, PipelineHandle, PipelineVariantKey, RhiError};

/// Key for identifying a unique pipeline in the cache.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PipelineCacheKey {
    pub shader_asset_id: String,
    pub vertex_layout_hash: u64,
    pub variant_key: PipelineVariantKey,
}

/// A cached pipeline entry with LRU tracking.
struct PipelineEntry {
    handle: PipelineHandle,
    last_used: Instant,
}

/// Pipeline library that lazily creates and caches pipelines.
///
/// Maps `(shader_asset_id, vertex_layout_hash, variant_key)` to a
/// `PipelineHandle`.  On cache miss the caller-provided `Device` is used
/// to create a new pipeline; the result is stored for future lookups.
pub struct PipelineLibrary {
    cache: BTreeMap<PipelineCacheKey, PipelineEntry>,
    max_entries: usize,
}

impl PipelineLibrary {
    /// Create a new pipeline library.
    ///
    /// `max_entries` controls how many pipelines are retained before the
    /// least-recently-used entry is evicted.
    pub fn new(max_entries: usize) -> Self {
        Self {
            cache: BTreeMap::new(),
            max_entries,
        }
    }

    /// Get or create a pipeline for the given key and descriptor.
    ///
    /// Returns a cached handle if one exists, otherwise calls
    /// `device.create_pipeline(desc)` and stores the result.
    pub fn get_or_create(
        &mut self,
        device: &mut dyn Device,
        key: PipelineCacheKey,
        desc: &PipelineDescriptor,
    ) -> Result<PipelineHandle, RhiError> {
        // Cache hit — update last_used and return
        if let Some(entry) = self.cache.get_mut(&key) {
            entry.last_used = Instant::now();
            return Ok(entry.handle);
        }

        // Cache miss — create a new pipeline
        let handle = device.create_pipeline(desc)?;

        // Evict if we are at capacity (before inserting, so we never exceed max)
        if self.cache.len() >= self.max_entries {
            self.evict_lru();
        }

        self.cache.insert(
            key,
            PipelineEntry {
                handle,
                last_used: Instant::now(),
            },
        );

        // After insertion, handle the edge case where max_entries == 0
        // (we just inserted into a zero-capacity map — evict immediately)
        if self.max_entries == 0 {
            self.evict_lru();
        }

        // Re-fetch the handle from the entry we just inserted
        // (safe unwrap: we just inserted it, or evict_lru removed it and we need
        //  to return the handle anyway)
        Ok(handle)
    }

    /// Evict the least-recently-used entry (the one with the oldest
    /// `last_used` timestamp).
    pub fn evict_lru(&mut self) {
        if let Some(oldest_key) = self
            .cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone())
        {
            self.cache.remove(&oldest_key);
        }
    }

    /// Clear all cached pipelines.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Number of cached pipelines.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

/// Compute a hash for a `VertexLayout` (for use in `PipelineCacheKey`).
///
/// Uses `std::collections::hash_map::DefaultHasher` (SipHash-2-4) so the
/// result is stable within a single process execution but **not** stable
/// across different versions of the compiler / standard library.
pub fn hash_vertex_layout(layout: &render_core::VertexLayout) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    layout.stride_bytes.hash(&mut hasher);
    for attr in &layout.attributes {
        attr.semantic.hash(&mut hasher);
        attr.format.hash(&mut hasher);
        attr.offset_bytes.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_core::{VertexAttribute, VertexLayout};

    // ── Mock device that returns sequential handles ──

    struct MockDevice {
        next_index: u32,
        fail: bool,
    }

    impl MockDevice {
        fn new() -> Self {
            Self {
                next_index: 1,
                fail: false,
            }
        }
    }

    impl Device for MockDevice {
        fn adapter_info(&self) -> &render_core::AdapterInfo {
            unimplemented!("not needed in tests")
        }

        fn create_pipeline(
            &mut self,
            _desc: &PipelineDescriptor,
        ) -> Result<PipelineHandle, RhiError> {
            if self.fail {
                return Err(RhiError::Backend {
                    detail: "mock failure".into(),
                });
            }
            let handle = PipelineHandle::new(self.next_index, 1);
            self.next_index += 1;
            Ok(handle)
        }
    }

    fn dummy_desc() -> PipelineDescriptor {
        PipelineDescriptor::default()
    }

    fn dummy_key(label: &str) -> PipelineCacheKey {
        PipelineCacheKey {
            shader_asset_id: label.to_string(),
            vertex_layout_hash: 42,
            variant_key: PipelineVariantKey::NONE,
        }
    }

    // ── Tests ──

    #[test]
    fn cache_miss_creates_pipeline() {
        let mut lib = PipelineLibrary::new(16);
        let mut device = MockDevice::new();
        let key = dummy_key("miss");
        let handle = lib
            .get_or_create(&mut device, key.clone(), &dummy_desc())
            .expect("create should succeed");
        assert_eq!(handle.index, 1);
        assert_eq!(lib.len(), 1);
    }

    #[test]
    fn cache_hit_returns_same_handle() {
        let mut lib = PipelineLibrary::new(16);
        let mut device = MockDevice::new();
        let key = dummy_key("hit");
        let h1 = lib
            .get_or_create(&mut device, key.clone(), &dummy_desc())
            .expect("first create");
        let h2 = lib
            .get_or_create(&mut device, key.clone(), &dummy_desc())
            .expect("second get");
        assert_eq!(h1, h2, "cache hit should return identical handle");
        assert_eq!(lib.len(), 1);
    }

    #[test]
    fn different_keys_produce_different_handles() {
        let mut lib = PipelineLibrary::new(16);
        let mut device = MockDevice::new();
        let k1 = dummy_key("a");
        let k2 = dummy_key("b");
        let h1 = lib
            .get_or_create(&mut device, k1, &dummy_desc())
            .expect("first");
        let h2 = lib
            .get_or_create(&mut device, k2, &dummy_desc())
            .expect("second");
        assert_ne!(h1, h2);
        assert_eq!(lib.len(), 2);
    }

    #[test]
    fn evict_lru_removes_oldest() {
        let mut lib = PipelineLibrary::new(3);
        let mut device = MockDevice::new();

        let k1 = dummy_key("k1");
        let k2 = dummy_key("k2");
        let k3 = dummy_key("k3");

        // Fill the cache
        lib.get_or_create(&mut device, k1.clone(), &dummy_desc())
            .expect("k1");
        lib.get_or_create(&mut device, k2.clone(), &dummy_desc())
            .expect("k2");
        lib.get_or_create(&mut device, k3.clone(), &dummy_desc())
            .expect("k3");
        assert_eq!(lib.len(), 3);

        // Touch k1 to make it most recent, then k2 is now oldest
        lib.get_or_create(&mut device, k1.clone(), &dummy_desc())
            .expect("k1 touch");

        // Evict one — should remove k2
        lib.evict_lru();
        assert_eq!(lib.len(), 2);
        assert!(
            lib.cache.contains_key(&k1),
            "k1 should be present (most recent)"
        );
        assert!(
            lib.cache.contains_key(&k3),
            "k3 should be present"
        );
        assert!(
            !lib.cache.contains_key(&k2),
            "k2 should have been evicted (oldest)"
        );
    }

    #[test]
    fn evict_when_over_capacity() {
        let mut lib = PipelineLibrary::new(2);
        let mut device = MockDevice::new();

        let k1 = dummy_key("k1");
        let k2 = dummy_key("k2");
        let k3 = dummy_key("k3");

        lib.get_or_create(&mut device, k1.clone(), &dummy_desc())
            .expect("k1");
        lib.get_or_create(&mut device, k2.clone(), &dummy_desc())
            .expect("k2");
        // Insertion of k3 should evict k1 (oldest)
        lib.get_or_create(&mut device, k3.clone(), &dummy_desc())
            .expect("k3");

        assert_eq!(lib.len(), 2);
        assert!(!lib.cache.contains_key(&k1), "k1 should be evicted");
        assert!(lib.cache.contains_key(&k2), "k2 should remain");
        assert!(lib.cache.contains_key(&k3), "k3 should remain");
    }

    #[test]
    fn clear_empties_cache() {
        let mut lib = PipelineLibrary::new(16);
        let mut device = MockDevice::new();

        lib.get_or_create(&mut device, dummy_key("x"), &dummy_desc())
            .expect("create");
        assert_eq!(lib.len(), 1);

        lib.clear();
        assert_eq!(lib.len(), 0);
        assert!(lib.is_empty());
    }

    #[test]
    fn create_propagation_error() {
        let mut lib = PipelineLibrary::new(16);
        let mut device = MockDevice::new();
        device.fail = true;

        let result = lib.get_or_create(&mut device, dummy_key("fail"), &dummy_desc());
        assert!(
            result.is_err(),
            "should propagate device error"
        );
        assert_eq!(lib.len(), 0);
    }

    #[test]
    fn hash_vertex_layout_consistency() {
        let layout_a = VertexLayout {
            stride_bytes: 32,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "NORMAL".into(),
                    format: "float32x3".into(),
                    offset_bytes: 12,
                },
            ],
        };
        let layout_b = VertexLayout {
            stride_bytes: 32,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "NORMAL".into(),
                    format: "float32x3".into(),
                    offset_bytes: 12,
                },
            ],
        };
        let layout_c = VertexLayout {
            stride_bytes: 48,
            attributes: vec![
                VertexAttribute {
                    semantic: "POSITION".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "TEXCOORD".into(),
                    format: "float32x2".into(),
                    offset_bytes: 12,
                },
            ],
        };

        assert_eq!(
            hash_vertex_layout(&layout_a),
            hash_vertex_layout(&layout_b),
            "identical layouts should produce the same hash"
        );
        assert_ne!(
            hash_vertex_layout(&layout_a),
            hash_vertex_layout(&layout_c),
            "different layouts should produce different hashes (collision unlikely)"
        );
    }

    #[test]
    fn zero_capacity_evicts_immediately() {
        let mut lib = PipelineLibrary::new(0);
        let mut device = MockDevice::new();

        let handle = lib
            .get_or_create(&mut device, dummy_key("z"), &dummy_desc())
            .expect("create should still work");
        assert_eq!(
            lib.len(),
            0,
            "zero-capacity cache should be empty after insert"
        );
        // The handle is still valid even though the entry was evicted
        assert_eq!(handle.index, 1);
    }
}
