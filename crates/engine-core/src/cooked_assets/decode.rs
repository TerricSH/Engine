use std::path::Path;

use engine_asset::cook::{
    decode_cooked_environment_map, decode_cooked_material, decode_cooked_mesh,
    decode_cooked_morph_target_set, decode_cooked_texture, read_cooked_artifact,
    registered_asset_type_id, AssetType,
};
use engine_renderer::{
    AxisAlignedBox, ColorSpace, EnvironmentCubeMip, EnvironmentMapFormat, EnvironmentMapUpload,
    IndexFormat, MaterialUpload, MeshUpload, MeshVertexFormat, MorphTarget, MorphTargetSetUpload,
    SamplerDescriptor, TextureUpload, TextureUploadFormat, Transparency,
};
use engine_scene::registry::AssetTypeRegistry;

use super::decoded::{DecodedCookedAsset, DecodedExtensionAsset};
use super::validation::{cooked_asset_id, split_rgba8_mips};

pub(super) fn decode_cooked_asset(
    path: &Path,
    asset_type_registry: &AssetTypeRegistry,
) -> Result<DecodedCookedAsset, String> {
    let id = cooked_asset_id(path)?;
    let artifact = read_cooked_artifact(path).map_err(|error| error.to_string())?;
    let asset_type = AssetType::from_kind_code(artifact.header.asset_kind);
    if asset_type == AssetType::Unknown {
        return Err(format!(
            "unsupported cooked asset kind code {}",
            artifact.header.asset_kind
        ));
    }
    match asset_type {
        AssetType::Mesh => {
            let mesh = decode_cooked_mesh(&artifact).map_err(|error| error.to_string())?;
            if mesh.positions.is_empty() {
                return Err("cooked mesh has no vertices".into());
            }
            let (vertex_format, vertex_bytes, index_bytes, index_count) = if mesh.joints.is_empty()
                && mesh.weights.is_empty()
            {
                let (vertex_bytes, index_bytes, index_count, _) =
                    engine_asset::mesh::mesh_data_to_upload_bytes(&mesh);
                (
                    MeshVertexFormat::Pbr32,
                    vertex_bytes,
                    index_bytes,
                    index_count,
                )
            } else {
                let (vertex_bytes, index_bytes, index_count, _) =
                        engine_asset::mesh::mesh_data_to_skinned_bytes(&mesh).ok_or_else(|| {
                            "cooked skinned mesh must provide exactly four joints and weights per vertex"
                                .to_string()
                        })?;
                (
                    MeshVertexFormat::Skinned64,
                    vertex_bytes,
                    index_bytes,
                    index_count,
                )
            };
            Ok(DecodedCookedAsset::Mesh(MeshUpload {
                mesh_id: id,
                vertex_format,
                vertex_count: u32::try_from(mesh.positions.len())
                    .map_err(|_| "cooked mesh vertex count exceeds u32".to_string())?,
                vertex_bytes,
                index_format: IndexFormat::U32,
                index_count,
                index_bytes,
                bounds: AxisAlignedBox {
                    min: mesh.bounds.0.to_array(),
                    max: mesh.bounds.1.to_array(),
                },
                content_hash: artifact.header.content_hash,
            }))
        }
        AssetType::Texture => {
            let texture = decode_cooked_texture(&artifact).map_err(|error| error.to_string())?;
            if texture.format != engine_asset::cook::CookedTextureFormat::Rgba8Unorm {
                return Err(format!(
                    "unsupported cooked texture format: {:?}",
                    texture.format
                ));
            }
            let mip_levels = split_rgba8_mips(
                texture.width,
                texture.height,
                texture.mip_count,
                &texture.data,
            )?;
            Ok(DecodedCookedAsset::Texture(TextureUpload {
                texture_id: id,
                width: texture.width,
                height: texture.height,
                format: TextureUploadFormat::Rgba8,
                color_space: ColorSpace::Srgb,
                mip_levels,
                sampler: SamplerDescriptor::default(),
                content_hash: artifact.header.content_hash,
            }))
        }
        AssetType::Material => {
            let material = decode_cooked_material(&artifact).map_err(|error| error.to_string())?;
            for (field, value) in [
                ("base_color[0]", material.base_color[0]),
                ("base_color[1]", material.base_color[1]),
                ("base_color[2]", material.base_color[2]),
                ("base_color[3]", material.base_color[3]),
                ("metallic", material.metallic),
                ("roughness", material.roughness),
                ("ambient_occlusion", material.ambient_occlusion),
                ("emissive[0]", material.emissive[0]),
                ("emissive[1]", material.emissive[1]),
                ("emissive[2]", material.emissive[2]),
            ] {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(format!(
                        "cooked material field '{field}' must be finite and in the range 0..=1"
                    ));
                }
            }
            let transparency = match material.transparency {
                engine_asset::cook::MaterialTransparency::Opaque => Transparency::Opaque,
                engine_asset::cook::MaterialTransparency::Masked { cutoff } => {
                    if !cutoff.is_finite() || !(0.0..=1.0).contains(&cutoff) {
                        return Err(
                            "cooked material alpha cutoff must be finite and in the range 0..=1"
                                .into(),
                        );
                    }
                    Transparency::Masked { cutoff }
                }
                engine_asset::cook::MaterialTransparency::Blend => Transparency::Blend,
                engine_asset::cook::MaterialTransparency::Additive => Transparency::Additive,
            };
            Ok(DecodedCookedAsset::Material(
                path.to_path_buf(),
                Box::new(MaterialUpload {
                    material_id: id,
                    base_color: material.base_color,
                    metallic: material.metallic,
                    roughness: material.roughness,
                    ambient_occlusion: material.ambient_occlusion,
                    emissive: material.emissive,
                    base_color_texture: material.base_color_texture,
                    normal_texture: material.normal_texture,
                    metallic_roughness_texture: material.metallic_roughness_texture,
                    occlusion_texture: material.occlusion_texture,
                    emissive_texture: material.emissive_texture,
                    advanced: engine_renderer::AdvancedMaterialParameters {
                        clearcoat: material.advanced.clearcoat,
                        clearcoat_roughness: material.advanced.clearcoat_roughness,
                        subsurface: material.advanced.subsurface,
                        subsurface_color: material.advanced.subsurface_color,
                        anisotropy: material.advanced.anisotropy,
                        sheen_color: material.advanced.sheen_color,
                        rim_color: material.advanced.rim_color,
                        rim_power: material.advanced.rim_power,
                    },
                    transparency,
                    double_sided: material.double_sided,
                    content_hash: artifact.header.content_hash,
                }),
            ))
        }
        AssetType::EnvironmentMap => {
            let environment =
                decode_cooked_environment_map(&artifact).map_err(|error| error.to_string())?;
            Ok(DecodedCookedAsset::EnvironmentMap(EnvironmentMapUpload {
                environment_id: id,
                format: EnvironmentMapFormat::Rgba16Float,
                mip_levels: environment
                    .mip_levels
                    .into_iter()
                    .map(|mip| EnvironmentCubeMip {
                        face_size: mip.face_size,
                        faces: mip.faces,
                    })
                    .collect(),
                content_hash: artifact.header.content_hash,
            }))
        }
        AssetType::MorphTargetSet => {
            let morph =
                decode_cooked_morph_target_set(&artifact).map_err(|error| error.to_string())?;
            Ok(DecodedCookedAsset::MorphTargetSet(MorphTargetSetUpload {
                target_set_id: id,
                vertex_count: morph.vertex_count,
                targets: morph
                    .targets
                    .into_iter()
                    .map(|target| MorphTarget {
                        name: target.name,
                        position_deltas: target.position_deltas,
                        normal_deltas: target.normal_deltas,
                    })
                    .collect(),
                content_hash: artifact.header.content_hash,
            }))
        }
        kind @ (AssetType::Audio
        | AssetType::Animation
        | AssetType::Skeleton
        | AssetType::NavMesh
        | AssetType::Prefab
        | AssetType::Logic) => {
            let type_id = registered_asset_type_id(&kind)
                .expect("extension-owned asset types have a stable registry mapping");
            let extension = asset_type_registry.get(type_id).ok_or_else(|| {
                format!("cooked {kind:?} asset requires registered extension '{type_id}'")
            })?;
            let loader = extension
                .loader
                .ok_or_else(|| format!("registered extension '{type_id}' has no runtime loader"))?;
            let value = loader(&artifact.payload).map_err(|error| {
                format!("extension loader '{type_id}' rejected cooked payload: {error}")
            })?;
            Ok(DecodedCookedAsset::Extension(DecodedExtensionAsset {
                type_id: type_id.to_string(),
                id,
                payload: artifact.payload,
                value,
            }))
        }
        AssetType::Font => {
            Err("cooked Font assets have no registered runtime loader mapping".into())
        }
        kind @ (AssetType::Shader | AssetType::Scene | AssetType::Pipeline | AssetType::Script) => {
            Ok(DecodedCookedAsset::Skipped(kind))
        }
        AssetType::Unknown => unreachable!("unknown kind was rejected above"),
    }
}
