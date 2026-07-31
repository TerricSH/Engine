use super::*;

type SkinAttributes = (Vec<[u32; 4]>, Vec<[f32; 4]>);

pub(super) fn read_morph_targets<'a, 's, F>(
    reader: &gltf::mesh::Reader<'a, 's, F>,
    vertex_count: usize,
    mesh_index: usize,
    primitive_index: usize,
) -> Result<Vec<GltfMorphTarget>, GltfImportError>
where
    F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'s [u8]>,
{
    let target_count = reader.clone().read_morph_targets().len();
    if target_count > MAX_GLTF_MORPH_TARGETS {
        return Err(GltfImportError::TooManyMorphTargets {
            mesh_index,
            primitive_index,
            target_count,
        });
    }
    reader
        .clone()
        .read_morph_targets()
        .enumerate()
        .map(|(target_index, (positions, normals, _tangents))| {
            let position_deltas = positions
                .map(|values| values.map(Vec3::from_array).collect::<Vec<_>>())
                .unwrap_or_else(|| vec![Vec3::ZERO; vertex_count]);
            let normal_deltas = normals
                .map(|values| values.map(Vec3::from_array).collect::<Vec<_>>())
                .unwrap_or_else(|| vec![Vec3::ZERO; vertex_count]);
            if position_deltas.len() != vertex_count
                || normal_deltas.len() != vertex_count
                || position_deltas
                    .iter()
                    .chain(&normal_deltas)
                    .any(|delta| !delta.is_finite())
            {
                return Err(GltfImportError::MorphTargetCountMismatch {
                    mesh_index,
                    primitive_index,
                    target_index,
                    vertex_count,
                });
            }
            Ok(GltfMorphTarget {
                name: format!("target-{target_index}"),
                position_deltas,
                normal_deltas,
            })
        })
        .collect()
}

pub(super) fn read_skin_attributes<'a, 's, F>(
    reader: &gltf::mesh::Reader<'a, 's, F>,
    position_count: usize,
    mesh_index: usize,
    primitive_index: usize,
) -> Result<SkinAttributes, GltfImportError>
where
    F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'s [u8]>,
{
    let (joints, weights) = match (reader.read_joints(0), reader.read_weights(0)) {
        (None, None) => return Ok((Vec::new(), Vec::new())),
        (Some(joints), Some(weights)) => (joints, weights),
        _ => {
            return Err(GltfImportError::IncompleteSkinAttributes {
                mesh_index,
                primitive_index,
            });
        }
    };

    let joints = joints
        .into_u16()
        .map(|joint| joint.map(u32::from))
        .collect::<Vec<_>>();
    let mut weights = weights.into_f32().collect::<Vec<_>>();
    if joints.len() != position_count || weights.len() != position_count {
        return Err(GltfImportError::SkinAttributeCountMismatch {
            mesh_index,
            primitive_index,
            positions: position_count,
            joints: joints.len(),
            weights: weights.len(),
        });
    }

    // glTF permits quantised weights whose decoded sum is only approximately
    // one. Normalise once during import so every backend receives stable,
    // well-formed skinning input.
    for (vertex_index, weight) in weights.iter_mut().enumerate() {
        if !weight
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
        {
            return Err(GltfImportError::InvalidSkinWeights {
                mesh_index,
                primitive_index,
                vertex_index,
            });
        }
        let sum = weight.iter().sum::<f32>();
        if !sum.is_finite() || sum <= f32::EPSILON {
            return Err(GltfImportError::InvalidSkinWeights {
                mesh_index,
                primitive_index,
                vertex_index,
            });
        }
        for value in weight {
            *value /= sum;
        }
    }

    Ok((joints, weights))
}
