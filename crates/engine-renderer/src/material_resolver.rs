use render_core::{
    BindGroupLayoutDescriptor, BlendState, DepthState, PipelineDescriptor, PipelineLayoutHandle,
    PipelineVariantKey, RasterState, RenderPassHandle, ShaderModuleHandle, TextureFormat,
    VertexLayout,
};

use crate::pipeline_library::{hash_vertex_layout, PipelineCacheKey, PipelineLibrary};
use crate::{MaterialBinding, Transparency};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialPipelineContext {
    pub shader_modules: Vec<ShaderModuleHandle>,
    pub vertex_layout: VertexLayout,
    pub bind_layouts: Vec<BindGroupLayoutDescriptor>,
    pub pipeline_layout: PipelineLayoutHandle,
    pub render_pass: RenderPassHandle,
    pub render_targets: Vec<TextureFormat>,
    pub depth_format: Option<TextureFormat>,
    pub depth_write_enabled: bool,
    pub depth_compare: Option<String>,
    pub front_face: Option<String>,
    pub topology: Option<String>,
    pub polygon_mode: Option<String>,
    pub sample_count: u8,
}

/// Resolved material → pipeline mapping.
pub struct MaterialResolver {
    library: PipelineLibrary,
}

impl MaterialResolver {
    /// Create a new material resolver.
    pub fn new(max_pipelines: usize) -> Self {
        Self {
            library: PipelineLibrary::new(max_pipelines),
        }
    }

    /// Resolve a pipeline descriptor and cache key for the given material.
    pub fn resolve(
        &self,
        material: &MaterialBinding,
        context: &MaterialPipelineContext,
        variant_key: PipelineVariantKey,
    ) -> (PipelineCacheKey, PipelineDescriptor) {
        let combined_variant = PipelineVariantKey::new(material.variant_key).with(variant_key);
        let blend_mode = match material.transparency {
            Transparency::Blend => Some("Alpha".to_string()),
            Transparency::Opaque | Transparency::Masked { .. } => None,
        };
        let cull_mode = if material.double_sided {
            Some("none".to_string())
        } else {
            Some("back".to_string())
        };

        let desc = PipelineDescriptor {
            shader_modules: context.shader_modules.clone(),
            vertex_layout: context.vertex_layout.clone(),
            bind_layouts: context.bind_layouts.clone(),
            pipeline_layout: Some(context.pipeline_layout),
            raster_state: RasterState {
                cull_mode,
                front_face: context.front_face.clone(),
            },
            depth_state: DepthState {
                format: context.depth_format,
                write_enabled: context.depth_write_enabled,
                compare: context.depth_compare.clone(),
            },
            blend_state: BlendState {
                mode: blend_mode.clone(),
            },
            render_targets: context.render_targets.clone(),
            debug_label: Some(format!(
                "material:{} variant:{:016x}",
                material.pipeline.id,
                combined_variant.bits()
            )),
            topology: context.topology.clone(),
            polygon_mode: context.polygon_mode.clone(),
            sample_count: Some(context.sample_count),
            render_pass: Some(context.render_pass),
            specialization: Vec::new(),
        };

        let key = PipelineCacheKey::from_descriptor(
            material.pipeline.id.clone(),
            combined_variant,
            hash_vertex_layout(&context.vertex_layout),
            &desc,
        );

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
    use render_core::{RenderPassHandle, VertexAttribute};

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

    fn dummy_context() -> MaterialPipelineContext {
        MaterialPipelineContext {
            shader_modules: Vec::new(),
            vertex_layout: VertexLayout {
                stride_bytes: 24,
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
            },
            bind_layouts: Vec::new(),
            pipeline_layout: PipelineLayoutHandle::new(3, 1),
            render_pass: RenderPassHandle::new(5, 1),
            render_targets: vec![TextureFormat::Bgra8Unorm],
            depth_format: Some(TextureFormat::Depth32Float),
            depth_write_enabled: true,
            depth_compare: Some("less".into()),
            front_face: Some("counter_clockwise".into()),
            topology: Some("triangle_list".into()),
            polygon_mode: Some("fill".into()),
            sample_count: 1,
        }
    }

    #[test]
    fn resolve_produces_complete_key_and_descriptor() {
        let resolver = MaterialResolver::new(16);
        let material = dummy_material();
        let context = dummy_context();

        let (key, desc) = resolver.resolve(&material, &context, PipelineVariantKey::SKINNED);

        assert_eq!(key.shader_asset_id, "shader_pbr");
        assert_eq!(
            key.vertex_layout_hash,
            hash_vertex_layout(&context.vertex_layout)
        );
        assert_eq!(key.pipeline_layout, Some(context.pipeline_layout));
        assert_eq!(key.render_pass, Some(context.render_pass));
        assert_eq!(key.variant_key, PipelineVariantKey::SKINNED);
        assert_eq!(desc.pipeline_layout, Some(context.pipeline_layout));
        assert_eq!(desc.render_pass, Some(context.render_pass));
        assert_eq!(desc.render_targets, context.render_targets);
        assert_eq!(desc.raster_state.cull_mode.as_deref(), Some("back"));
        assert!(desc
            .debug_label
            .as_ref()
            .is_some_and(|label| label.contains("shader_pbr")));
    }

    #[test]
    fn resolve_combines_material_and_callsite_variant_flags() {
        let resolver = MaterialResolver::new(16);
        let mut material = dummy_material();
        material.variant_key = PipelineVariantKey::INSTANCED.bits();
        let context = dummy_context();

        let (key, _desc) = resolver.resolve(&material, &context, PipelineVariantKey::SHADOW_PASS);

        assert!(key.variant_key.contains(PipelineVariantKey::INSTANCED));
        assert!(key.variant_key.contains(PipelineVariantKey::SHADOW_PASS));
    }

    #[test]
    fn transparent_double_sided_material_changes_pipeline_state() {
        let resolver = MaterialResolver::new(16);
        let mut material = dummy_material();
        material.transparency = crate::Transparency::Blend;
        material.double_sided = true;
        let context = dummy_context();

        let (key, desc) = resolver.resolve(&material, &context, PipelineVariantKey::NONE);

        assert_eq!(desc.blend_state.mode.as_deref(), Some("Alpha"));
        assert_eq!(desc.raster_state.cull_mode.as_deref(), Some("none"));
        assert_eq!(key.blend_state.mode.as_deref(), Some("Alpha"));
    }

    #[test]
    fn different_materials_produce_different_keys() {
        let resolver = MaterialResolver::new(16);
        let context = dummy_context();

        let mat_a = MaterialBinding {
            pipeline: crate::AssetId::new("shader_a"),
            ..dummy_material()
        };
        let mat_b = MaterialBinding {
            pipeline: crate::AssetId::new("shader_b"),
            ..dummy_material()
        };

        let (key_a, _) = resolver.resolve(&mat_a, &context, PipelineVariantKey::NONE);
        let (key_b, _) = resolver.resolve(&mat_b, &context, PipelineVariantKey::NONE);

        assert_ne!(key_a, key_b);
    }

    #[test]
    fn library_access_returns_underlying_cache() {
        let resolver = MaterialResolver::new(32);
        assert_eq!(resolver.library().len(), 0);
        assert!(resolver.library().is_empty());
    }
}
