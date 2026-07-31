use super::image::decode_gltf_image;
use super::*;

pub(super) fn extract_material(material: &gltf::Material<'_>) -> GltfMaterial {
    let pbr = material.pbr_metallic_roughness();
    GltfMaterial {
        material_index: material
            .index()
            .expect("document.materials() never yields the default material"),
        name: material.name().unwrap_or("material").to_string(),
        base_color: pbr.base_color_factor(),
        base_color_texture: pbr.base_color_texture().map(|info| info.texture().index()),
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        metallic_roughness_texture: pbr
            .metallic_roughness_texture()
            .map(|info| info.texture().index()),
        emissive: material.emissive_factor(),
        emissive_texture: material
            .emissive_texture()
            .map(|info| info.texture().index()),
        normal_texture: material.normal_texture().map(|info| info.texture().index()),
        occlusion_texture: material
            .occlusion_texture()
            .map(|info| info.texture().index()),
        occlusion_strength: material
            .occlusion_texture()
            .map_or(1.0, |info| info.strength()),
        alpha_mode: material.alpha_mode(),
        alpha_cutoff: material.alpha_cutoff(),
        double_sided: material.double_sided(),
    }
}

pub(super) fn load_textures(
    document: &gltf::Document,
    base: &Path,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<GltfTexture>, GltfImportError> {
    let mut textures = Vec::with_capacity(document.textures().len());
    for texture in document.textures() {
        let texture_index = texture.index();
        let image = texture.source();
        let image_index = image.index();
        let source = image.source();
        let source_label = image_source_label(&source, base);
        let decoded =
            gltf::image::Data::from_source(source, Some(base), buffers).map_err(|error| {
                GltfImportError::TextureDecode {
                    texture_index,
                    image_index,
                    image_source: source_label,
                    detail: error.to_string(),
                }
            })?;
        let sampler = texture.sampler();
        textures.push(decode_gltf_image(
            decoded,
            texture_index,
            image_index,
            GltfSampler {
                sampler_index: sampler.index(),
                mag_filter: sampler.mag_filter(),
                min_filter: sampler.min_filter(),
                wrap_s: sampler.wrap_s(),
                wrap_t: sampler.wrap_t(),
            },
        ));
    }
    Ok(textures)
}

fn image_source_label(source: &gltf::image::Source<'_>, base: &Path) -> String {
    match source {
        gltf::image::Source::Uri { uri, .. } if uri.starts_with("data:") => {
            "embedded data URI".to_string()
        }
        gltf::image::Source::Uri { uri, .. } => base.join(uri).display().to_string(),
        gltf::image::Source::View { view, .. } => format!("bufferView {}", view.index()),
    }
}
