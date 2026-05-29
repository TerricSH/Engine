use render_core::{PipelineDescriptor, PipelineVariantKey};

use crate::pipeline_library::{PipelineCacheKey, PipelineLibrary};
use crate::MaterialBinding;

/// Resolved material → pipeline mapping.
///
/// Maps a [`MaterialBinding`] + [`PipelineVariantKey`] pair to a cached
/// pipeline.  The resolver is the bridge between high-level material data
/// and the low-level pipeline cache.
pub struct MaterialResolver {
    library: PipelineLibrary,
}

impl MaterialResolver {
    /// Create a new material resolver.
    ///
    /// `max_pipelines` controls the capacity of the underlying pipeline cache.
    pub fn new(max_pipelines: usize) -> Self {
        Self {
            library: PipelineLibrary::new(max_pipelines),
        }
    }

    /// Resolve a pipeline for the given material binding.
    ///
    /// Returns a `(PipelineCacheKey, PipelineDescriptor)` pair that the
    /// caller should pass to [`get_or_create`](PipelineLibrary::get_or_create).
    ///
    /// The returned key is built from:
    /// - `shader_asset_id`: the material's `pipeline` asset ID
    /// - `vertex_layout_hash`: `0` (caller should replace with the actual
    ///   mesh vertex-layout hash before calling `get_or_create`)
    /// - `variant_key`: the supplied variant flags
    ///
    /// The returned descriptor is a default template with the debug label
    /// set to the material's pipeline asset ID.
    pub fn resolve(
        &self,
        material: &MaterialBinding,
        variant_key: PipelineVariantKey,
    ) -> (PipelineCacheKey, PipelineDescriptor) {
        let key = PipelineCacheKey {
            shader_asset_id: material.pipeline.id.clone(),
            vertex_layout_hash: 0, // caller should override with the mesh's layout hash
            variant_key,
        };

        let mut desc = PipelineDescriptor::default();
        desc.debug_label = Some(format!(
            "material:{} variant:{:016x}",
            material.pipeline.id,
            variant_key.bits()
        ));

        (key, desc)
    }

    /// Access the underlying pipeline library.
    pub fn library(&self) -> &PipelineLibrary {
        &self.library
    }

    /// Mutable access to the underlying pipeline library.
    pub fn library_mut(&mut self) -> &mut PipelineLibrary {
        &mut self.library
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_core::PipelineVariantKey;

    fn dummy_material() -> MaterialBinding {
        MaterialBinding {
            material_id: crate::AssetId::new("mat_wood"),
            pipeline: crate::AssetId::new("shader_pbr"),
            variant_key: 0,
            textures: Vec::new(),
            uniforms: crate::ParamBlock {
                bytes: Vec::new(),
                layout_hash: [0; 32],
            },
            pass_mask: 1,
            transparency: crate::Transparency::Opaque,
            double_sided: false,
        }
    }

    #[test]
    fn resolve_produces_correct_key() {
        let resolver = MaterialResolver::new(16);
        let material = dummy_material();
        let variant = PipelineVariantKey::SKINNED;

        let (key, _desc) = resolver.resolve(&material, variant);

        assert_eq!(key.shader_asset_id, "shader_pbr");
        assert_eq!(key.vertex_layout_hash, 0);
        assert_eq!(key.variant_key, PipelineVariantKey::SKINNED);
    }

    #[test]
    fn resolve_with_multiple_variant_flags() {
        let resolver = MaterialResolver::new(16);
        let material = dummy_material();
        let variant = PipelineVariantKey::NONE
            .with(PipelineVariantKey::INSTANCED)
            .with(PipelineVariantKey::SHADOW_PASS);

        let (key, _desc) = resolver.resolve(&material, variant);

        assert!(key.variant_key.contains(PipelineVariantKey::INSTANCED));
        assert!(key.variant_key.contains(PipelineVariantKey::SHADOW_PASS));
        assert!(!key.variant_key.contains(PipelineVariantKey::SKINNED));
    }

    #[test]
    fn descriptor_has_debug_label() {
        let resolver = MaterialResolver::new(16);
        let material = dummy_material();
        let variant = PipelineVariantKey::new();

        let (_key, desc) = resolver.resolve(&material, variant);

        assert!(
            desc.debug_label.is_some(),
            "resolve should set a debug label"
        );
        let label = desc.debug_label.as_ref().unwrap();
        assert!(label.contains("shader_pbr"), "label should mention the shader asset");
    }

    #[test]
    fn different_materials_produce_different_keys() {
        let resolver = MaterialResolver::new(16);

        let mat_a = MaterialBinding {
            pipeline: crate::AssetId::new("shader_a"),
            ..dummy_material()
        };
        let mat_b = MaterialBinding {
            pipeline: crate::AssetId::new("shader_b"),
            ..dummy_material()
        };

        let (key_a, _) = resolver.resolve(&mat_a, PipelineVariantKey::NONE);
        let (key_b, _) = resolver.resolve(&mat_b, PipelineVariantKey::NONE);

        assert_ne!(key_a.shader_asset_id, key_b.shader_asset_id);
    }

    #[test]
    fn variant_key_bit_operations() {
        // Test the PipelineVariantKey bit operations directly
        let vk = PipelineVariantKey::NONE;
        assert_eq!(vk.bits(), 0);

        let vk = vk.with(PipelineVariantKey::SKINNED);
        assert!(vk.contains(PipelineVariantKey::SKINNED));
        assert!(!vk.contains(PipelineVariantKey::INSTANCED));

        let vk = vk.with(PipelineVariantKey::INSTANCED);
        assert!(vk.contains(PipelineVariantKey::SKINNED));
        assert!(vk.contains(PipelineVariantKey::INSTANCED));

        // SHADOW_PASS is a separate bit
        assert!(!vk.contains(PipelineVariantKey::SHADOW_PASS));

        // Combining multiple flags
        let combined = PipelineVariantKey::NONE
            .with(PipelineVariantKey::SKINNED)
            .with(PipelineVariantKey::SHADOW_PASS);
        assert!(combined.contains(PipelineVariantKey::SKINNED));
        assert!(combined.contains(PipelineVariantKey::SHADOW_PASS));
        assert!(!combined.contains(PipelineVariantKey::INSTANCED));

        // Using `new` with explicit bitmask
        let combined2 = PipelineVariantKey::new(
            PipelineVariantKey::SKINNED.bits() | PipelineVariantKey::INSTANCED.bits(),
        );
        assert!(combined2.contains(PipelineVariantKey::SKINNED));
        assert!(combined2.contains(PipelineVariantKey::INSTANCED));
    }

    #[test]
    fn library_access_returns_underlying_cache() {
        let resolver = MaterialResolver::new(32);
        assert_eq!(resolver.library().len(), 0);
        assert!(resolver.library().is_empty());
    }
}
