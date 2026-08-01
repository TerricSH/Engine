//! Conversion from imported glTF PBR materials to project authoring sources.

use super::material::{AdvancedMaterialSource, MaterialSource, MATERIAL_SOURCE_SCHEMA};

/// Convert the importer's complete metallic-roughness representation into the
/// project's portable authoring JSON contract. Texture indices are resolved by
/// the caller so project import can choose stable, collision-checked asset IDs.
pub fn material_source_from_gltf(
    material: &crate::gltf::GltfMaterial,
    texture_asset_id: impl Fn(usize) -> String,
) -> MaterialSource {
    let texture = |index: Option<usize>| index.map(&texture_asset_id);
    MaterialSource {
        schema: MATERIAL_SOURCE_SCHEMA.into(),
        base_color: material.base_color,
        metallic: material.metallic,
        roughness: material.roughness,
        ambient_occlusion: material.occlusion_strength,
        emissive: material.emissive,
        base_color_texture: texture(material.base_color_texture),
        normal_texture: texture(material.normal_texture),
        metallic_roughness_texture: texture(material.metallic_roughness_texture),
        occlusion_texture: texture(material.occlusion_texture),
        emissive_texture: texture(material.emissive_texture),
        advanced: AdvancedMaterialSource::default(),
        transparency: match material.alpha_mode {
            gltf::material::AlphaMode::Opaque => "Opaque",
            gltf::material::AlphaMode::Mask => "Masked",
            gltf::material::AlphaMode::Blend => "Blend",
        }
        .into(),
        alpha_cutoff: material.alpha_cutoff.unwrap_or(0.5),
        double_sided: material.double_sided,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_preserves_pbr_surface_and_texture_slots() {
        let gltf = crate::gltf::GltfMaterial {
            material_index: 4,
            name: "paint".into(),
            base_color: [0.2, 0.3, 0.4, 0.8],
            base_color_texture: Some(1),
            metallic: 0.75,
            roughness: 0.25,
            metallic_roughness_texture: Some(2),
            emissive: [0.1, 0.2, 0.3],
            emissive_texture: Some(3),
            normal_texture: Some(4),
            occlusion_texture: Some(5),
            occlusion_strength: 0.6,
            alpha_mode: gltf::material::AlphaMode::Mask,
            alpha_cutoff: Some(0.35),
            double_sided: true,
        };

        let source = material_source_from_gltf(&gltf, |index| format!("model.texture.{index}"));

        assert_eq!(source.base_color, gltf.base_color);
        assert_eq!(source.metallic, 0.75);
        assert_eq!(source.roughness, 0.25);
        assert_eq!(source.ambient_occlusion, 0.6);
        assert_eq!(source.emissive, gltf.emissive);
        assert_eq!(
            source.base_color_texture.as_deref(),
            Some("model.texture.1")
        );
        assert_eq!(
            source.metallic_roughness_texture.as_deref(),
            Some("model.texture.2")
        );
        assert_eq!(source.emissive_texture.as_deref(), Some("model.texture.3"));
        assert_eq!(source.normal_texture.as_deref(), Some("model.texture.4"));
        assert_eq!(source.occlusion_texture.as_deref(), Some("model.texture.5"));
        assert_eq!(source.transparency, "Masked");
        assert_eq!(source.alpha_cutoff, 0.35);
        assert!(source.double_sided);
    }
}
