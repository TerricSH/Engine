use std::collections::HashMap;

use render_core::{
    BindGroupLayoutDescriptor, BlendState, DepthState, Device, PipelineDescriptor,
    PipelineHandle, PipelineLayoutHandle, PipelineVariantKey, RasterState, RenderPassHandle,
    RhiError, ShaderModuleHandle, TextureFormat,
};

/// Key for identifying a unique pipeline in the cache.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PipelineCacheKey {
    pub shader_asset_id: String,
    pub shader_modules: Vec<ShaderModuleHandle>,
    pub vertex_layout_hash: u64,
    pub bind_layouts: Vec<BindGroupLayoutDescriptor>,
    pub pipeline_layout: Option<PipelineLayoutHandle>,
    pub raster_state: RasterState,
    pub depth_state: DepthState,
    pub blend_state: BlendState,
    pub render_targets: Vec<TextureFormat>,
    pub topology: Option<String>,
    pub polygon_mode: Option<String>,
    pub sample_count: Option<u8>,
    pub render_pass: Option<RenderPassHandle>,
    pub variant_key: PipelineVariantKey,
}

impl PipelineCacheKey {
    /// Build a cache key from the descriptor fields that materially affect
    /// pipeline creation on the backend.
    pub fn from_descriptor(
        shader_asset_id: impl Into<String>,
        variant_key: PipelineVariantKey,
        vertex_layout_hash: u64,
        desc: &PipelineDescriptor,
    ) -> Self {
        Self {
            shader_asset_id: shader_asset_id.into(),
            shader_modules: desc.shader_modules.clone(),
            vertex_layout_hash,
            bind_layouts: desc.bind_layouts.clone(),
            pipeline_layout: desc.pipeline_layout,
            raster_state: desc.raster_state.clone(),
            depth_state: desc.depth_state.clone(),
            blend_state: desc.blend_state.clone(),
            render_targets: desc.render_targets.clone(),
            topology: desc.topology.clone(),
            polygon_mode: desc.polygon_mode.clone(),
            sample_count: desc.sample_count,
            render_pass: desc.render_pass,
            variant_key,
        }
    }
}

struct PipelineEntry {
    handle: PipelineHandle,
    last_used_tick: u64,
}

/// Pipeline library that lazily creates and caches pipelines.
pub struct PipelineLibrary {
    cache: HashMap<PipelineCacheKey, PipelineEntry>,
    max_entries: usize,
    next_use_tick: u64,
}

impl PipelineLibrary {
    /// Create a new pipeline library.
    ///
    /// `max_entries == 0` disables caching while still allowing pipeline
    /// creation through this API.
    pub fn new(max_entries: usize) -> Self {
        Self {
            cache: HashMap::new(),
            max_entries,
            next_use_tick: 1,
        }
    }

    fn touch(&mut self) -> u64 {
        let tick = self.next_use_tick;
        self.next_use_tick = self.next_use_tick.wrapping_add(1);
        if self.next_use_tick == 0 {
            self.next_use_tick = 1;
        }
        tick
    }

    fn remove_lru(&mut self) -> Option<PipelineHandle> {
        let oldest_key = self
            .cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_used_tick)
            .map(|(key, _)| key.clone())?;

        self.cache.remove(&oldest_key).map(|entry| entry.handle)
    }

    /// Get or create a pipeline for the given key and descriptor.
    pub fn get_or_create(
        &mut self,
        device: &mut dyn Device,
        key: PipelineCacheKey,
        desc: &PipelineDescriptor,
    ) -> Result<PipelineHandle, RhiError> {
        if self.max_entries == 0 {
            return device.create_pipeline(desc);
        }

        let hit_tick = self.touch();
        if let Some(entry) = self.cache.get_mut(&key) {
            entry.last_used_tick = hit_tick;
            return Ok(entry.handle);
        }

        let handle = device.create_pipeline(desc)?;

        if self.cache.len() >= self.max_entries {
            let _ = self.evict_lru(device);
        }

        let insert_tick = self.touch();
        self.cache.insert(
            key,
            PipelineEntry {
                handle,
                last_used_tick: insert_tick,
            },
        );

        Ok(handle)
    }

    /// Evict the least-recently-used entry and destroy its backend pipeline.
    pub fn evict_lru(&mut self, device: &mut dyn Device) -> Option<PipelineHandle> {
        let handle = self.remove_lru()?;
        device.destroy_pipeline(handle);
        Some(handle)
    }

    /// Clear all cached pipelines and destroy their backend resources.
    pub fn clear(&mut self, device: &mut dyn Device) -> usize {
        let cached = std::mem::take(&mut self.cache);
        let count = cached.len();
        for entry in cached.into_values() {
            device.destroy_pipeline(entry.handle);
        }
        count
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

/// Compute a stable hash for a `VertexLayout`.
pub fn hash_vertex_layout(layout: &render_core::VertexLayout) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    fn hash_bytes(state: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *state ^= u64::from(*byte);
            *state = state.wrapping_mul(FNV_PRIME);
        }
    }

    let mut hash = FNV_OFFSET;
    hash_bytes(&mut hash, &layout.stride_bytes.to_le_bytes());
    for attr in &layout.attributes {
        hash_bytes(&mut hash, attr.semantic.as_bytes());
        hash_bytes(&mut hash, &[0xff]);
        hash_bytes(&mut hash, attr.format.as_bytes());
        hash_bytes(&mut hash, &[0xfe]);
        hash_bytes(&mut hash, &attr.offset_bytes.to_le_bytes());
        hash_bytes(&mut hash, &[0xfd]);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_core::{RenderPassHandle, VertexAttribute, VertexLayout};

    struct MockDevice {
        next_index: u32,
        fail: bool,
        destroyed: Vec<PipelineHandle>,
    }

    impl MockDevice {
        fn new() -> Self {
            Self {
                next_index: 1,
                fail: false,
                destroyed: Vec::new(),
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

        fn destroy_pipeline(&mut self, handle: PipelineHandle) {
            self.destroyed.push(handle);
        }
    }

    fn dummy_desc() -> PipelineDescriptor {
        PipelineDescriptor::default()
    }

    fn dummy_key(label: &str) -> PipelineCacheKey {
        PipelineCacheKey::from_descriptor(label, PipelineVariantKey::NONE, 42, &dummy_desc())
    }

    #[test]
    fn cache_miss_creates_pipeline() {
        let mut lib = PipelineLibrary::new(16);
        let mut device = MockDevice::new();
        let key = dummy_key("miss");

        let handle = lib
            .get_or_create(&mut device, key, &dummy_desc())
            .expect("create should succeed");

        assert_eq!(handle.index, 1);
        assert_eq!(lib.len(), 1);
    }

    #[test]
    fn cache_hit_returns_same_handle() {
        let mut lib = PipelineLibrary::new(16);
        let mut device = MockDevice::new();
        let key = dummy_key("hit");

        let first = lib
            .get_or_create(&mut device, key.clone(), &dummy_desc())
            .expect("first create");
        let second = lib
            .get_or_create(&mut device, key, &dummy_desc())
            .expect("second get");

        assert_eq!(first, second);
        assert_eq!(lib.len(), 1);
    }

    #[test]
    fn different_keys_produce_different_handles() {
        let mut lib = PipelineLibrary::new(16);
        let mut device = MockDevice::new();

        let first = lib
            .get_or_create(&mut device, dummy_key("a"), &dummy_desc())
            .expect("first");
        let second = lib
            .get_or_create(&mut device, dummy_key("b"), &dummy_desc())
            .expect("second");

        assert_ne!(first, second);
        assert_eq!(lib.len(), 2);
    }

    #[test]
    fn key_includes_descriptor_state() {
        let mut opaque = dummy_desc();
        opaque.render_pass = Some(RenderPassHandle::new(1, 1));

        let mut alpha = opaque.clone();
        alpha.blend_state.mode = Some("Alpha".into());

        let opaque_key =
            PipelineCacheKey::from_descriptor("scene", PipelineVariantKey::NONE, 42, &opaque);
        let alpha_key =
            PipelineCacheKey::from_descriptor("scene", PipelineVariantKey::NONE, 42, &alpha);

        assert_ne!(opaque_key, alpha_key);
    }

    #[test]
    fn evict_lru_removes_oldest() {
        let mut lib = PipelineLibrary::new(3);
        let mut device = MockDevice::new();
        let k1 = dummy_key("k1");
        let k2 = dummy_key("k2");
        let k3 = dummy_key("k3");

        let h1 = lib
            .get_or_create(&mut device, k1.clone(), &dummy_desc())
            .expect("k1");
        let h2 = lib
            .get_or_create(&mut device, k2.clone(), &dummy_desc())
            .expect("k2");
        lib.get_or_create(&mut device, k3.clone(), &dummy_desc())
            .expect("k3");

        lib.get_or_create(&mut device, k1.clone(), &dummy_desc())
            .expect("k1 touch");

        let evicted = lib.evict_lru(&mut device);
        assert_eq!(evicted, Some(h2));
        assert_eq!(device.destroyed, vec![h2]);
        assert_eq!(lib.len(), 2);
        assert!(lib.cache.contains_key(&k1));
        assert!(lib.cache.contains_key(&k3));
        assert!(!lib.cache.contains_key(&k2));
        assert!(!device.destroyed.contains(&h1));
    }

    #[test]
    fn evict_when_over_capacity() {
        let mut lib = PipelineLibrary::new(2);
        let mut device = MockDevice::new();

        let first = lib
            .get_or_create(&mut device, dummy_key("k1"), &dummy_desc())
            .expect("k1");
        lib.get_or_create(&mut device, dummy_key("k2"), &dummy_desc())
            .expect("k2");
        lib.get_or_create(&mut device, dummy_key("k3"), &dummy_desc())
            .expect("k3");

        assert_eq!(lib.len(), 2);
        assert_eq!(device.destroyed, vec![first]);
        assert!(!lib.cache.contains_key(&dummy_key("k1")));
    }

    #[test]
    fn clear_empties_cache_and_destroys_pipelines() {
        let mut lib = PipelineLibrary::new(16);
        let mut device = MockDevice::new();
        let first = lib
            .get_or_create(&mut device, dummy_key("x"), &dummy_desc())
            .expect("create");

        let cleared = lib.clear(&mut device);

        assert_eq!(cleared, 1);
        assert_eq!(device.destroyed, vec![first]);
        assert!(lib.is_empty());
    }

    #[test]
    fn create_propagates_device_error() {
        let mut lib = PipelineLibrary::new(16);
        let mut device = MockDevice::new();
        device.fail = true;

        let result = lib.get_or_create(&mut device, dummy_key("fail"), &dummy_desc());

        assert!(result.is_err());
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
        let layout_b = layout_a.clone();
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

        assert_eq!(hash_vertex_layout(&layout_a), hash_vertex_layout(&layout_b));
        assert_ne!(hash_vertex_layout(&layout_a), hash_vertex_layout(&layout_c));
    }

    #[test]
    fn zero_capacity_disables_caching_without_destroying_handles() {
        let mut lib = PipelineLibrary::new(0);
        let mut device = MockDevice::new();

        let first = lib
            .get_or_create(&mut device, dummy_key("z"), &dummy_desc())
            .expect("first create");
        let second = lib
            .get_or_create(&mut device, dummy_key("z"), &dummy_desc())
            .expect("second create");

        assert_ne!(first, second);
        assert!(lib.is_empty());
        assert!(device.destroyed.is_empty());
    }

    #[test]
    fn over_capacity_destroys_evicted_pipeline() {
        let mut lib = PipelineLibrary::new(2);
        let mut device = MockDevice::new();

        let first = lib
            .get_or_create(&mut device, dummy_key("first"), &dummy_desc())
            .expect("first");
        lib.get_or_create(&mut device, dummy_key("second"), &dummy_desc())
            .expect("second");
        lib.get_or_create(&mut device, dummy_key("third"), &dummy_desc())
            .expect("third");

        assert_eq!(device.destroyed, vec![first]);
    }
}
