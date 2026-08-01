//! Mesh cooking pipeline.
//!
//! Loads the canonical glTF primitive graph and serialises selected
//! [`crate::mesh::MeshData`] values as cooked artifacts.

use std::path::Path;

use engine_serialize::SchemaVersion;
use glam::{Mat4, Vec2, Vec3};

use super::error::CookError;
use super::{write_cooked_artifact, AssetType, CookResult};
use crate::gltf::GltfScene;
use crate::mesh::MeshData;

/// glTF-specific controls for producing one cooked mesh artifact.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GltfMeshCookOptions {
    /// Select one primitive by the importer's stable document order.
    pub primitive_index: Option<u32>,
    /// Concatenate every primitive (or every selected-scene instance when
    /// transforms are baked) into a single vertex/index buffer.
    pub merge_primitives: bool,
    /// Apply selected-scene node world transforms to static mesh vertices.
    pub bake_node_transforms: bool,
}

/// Cook a mesh from a glTF 2.0 source file.
///
/// The source path should point to a `.gltf` or `.glb` file containing exactly
/// one primitive. Multi-primitive sources must use [`cook_meshes`] so cooking
/// never silently aliases the first primitive.
pub fn cook_mesh(source: &Path, output: &Path) -> Result<CookResult, CookError> {
    cook_mesh_with_options(source, output, GltfMeshCookOptions::default())
}

/// Cook one selected primitive from a glTF source. Without an explicit
/// selection the source must contain exactly one primitive.
pub fn cook_mesh_primitive(
    source: &Path,
    output: &Path,
    primitive_index: Option<u32>,
) -> Result<CookResult, CookError> {
    cook_mesh_with_options(
        source,
        output,
        GltfMeshCookOptions {
            primitive_index,
            ..GltfMeshCookOptions::default()
        },
    )
}

/// Cook one glTF source using manifest-level merge and transform controls.
pub fn cook_mesh_with_options(
    source: &Path,
    output: &Path,
    options: GltfMeshCookOptions,
) -> Result<CookResult, CookError> {
    let scene = crate::gltf::load_gltf_scene(source)
        .map_err(|error| CookError::Parse(error.to_string()))?;
    let mesh_data = build_mesh(&scene, options)?;

    let payload =
        bincode::serialize(&mesh_data).map_err(|e| CookError::InvalidAsset(e.to_string()))?;

    write_cooked_artifact(
        output,
        AssetType::Mesh.kind_code(),
        &payload,
        SchemaVersion::new(0, 1, 0),
    )
}

fn build_mesh(scene: &GltfScene, options: GltfMeshCookOptions) -> Result<MeshData, CookError> {
    if scene.primitives.is_empty() {
        return Err(CookError::InvalidAsset(
            "glTF source contains no mesh primitives".into(),
        ));
    }
    if options.merge_primitives && options.primitive_index.is_some() {
        return Err(CookError::InvalidAsset(
            "gltf_merge_primitives and gltf_primitive_index are mutually exclusive".into(),
        ));
    }
    if !options.merge_primitives && options.primitive_index.is_none() && scene.primitives.len() != 1
    {
        return Err(CookError::InvalidAsset(format!(
            "single mesh asset requires exactly one glTF primitive, found {}; enable gltf_merge_primitives or select gltf_primitive_index",
            scene.primitives.len()
        )));
    }

    let selected = options
        .primitive_index
        .map(|index| index as usize)
        .unwrap_or(0);
    if !options.merge_primitives && selected >= scene.primitives.len() {
        return Err(CookError::InvalidAsset(format!(
            "glTF primitive selection {selected} is out of range for {} primitives",
            scene.primitives.len()
        )));
    }

    let instances = if options.bake_node_transforms {
        transformed_instances(scene, options.merge_primitives, selected)?
    } else if options.merge_primitives {
        (0..scene.primitives.len())
            .map(|primitive_index| (primitive_index, Mat4::IDENTITY, None))
            .collect()
    } else {
        vec![(selected, Mat4::IDENTITY, None)]
    };

    let mut merged = empty_mesh();
    let mut merged_skin = None;
    for (primitive_index, transform, instance_skin) in instances {
        let primitive = &scene.primitives[primitive_index];
        let skinned = !primitive.mesh.joints.is_empty() || !primitive.mesh.weights.is_empty();
        if options.bake_node_transforms && skinned {
            return Err(CookError::InvalidAsset(format!(
                "cannot bake node transforms into skinned glTF primitive {primitive_index}; preserve the node transform and skeleton binding"
            )));
        }
        if options.bake_node_transforms && !primitive.morph_targets.is_empty() {
            return Err(CookError::InvalidAsset(format!(
                "cannot bake node transforms into morph-target glTF primitive {primitive_index}; keep the base mesh and morph deltas in the same authored space"
            )));
        }
        if skinned {
            let skin = instance_skin.or_else(|| primitive_skin(scene, primitive_index));
            if let (Some(existing), Some(current)) = (merged_skin, skin) {
                if existing != current {
                    return Err(CookError::InvalidAsset(format!(
                        "cannot merge glTF primitives bound to different skins ({existing} and {current})"
                    )));
                }
            }
            merged_skin = merged_skin.or(skin);
        }
        append_mesh(
            &mut merged,
            &primitive.mesh,
            transform,
            options.bake_node_transforms,
            primitive_index,
        )?;
    }
    recompute_bounds(&mut merged)?;
    Ok(merged)
}

fn transformed_instances(
    scene: &GltfScene,
    merge_primitives: bool,
    selected: usize,
) -> Result<Vec<(usize, Mat4, Option<usize>)>, CookError> {
    let mut instances = Vec::new();
    for node in &scene.nodes {
        for &primitive_index in &node.primitive_indices {
            if merge_primitives || primitive_index == selected {
                instances.push((primitive_index, node.transform, node.skin_index));
            }
        }
    }
    if instances.is_empty() {
        let target = if merge_primitives {
            "any mesh primitive".to_string()
        } else {
            format!("primitive {selected}")
        };
        return Err(CookError::InvalidAsset(format!(
            "cannot bake glTF node transforms because the selected scene does not reference {target}"
        )));
    }
    if !merge_primitives && instances.len() != 1 {
        return Err(CookError::InvalidAsset(format!(
            "cannot bake primitive {selected}: the selected scene contains {} instances; enable gltf_merge_primitives to preserve every instance",
            instances.len()
        )));
    }
    Ok(instances)
}

fn primitive_skin(scene: &GltfScene, primitive_index: usize) -> Option<usize> {
    scene
        .nodes
        .iter()
        .filter(|node| node.primitive_indices.contains(&primitive_index))
        .filter_map(|node| node.skin_index)
        .next()
}

fn empty_mesh() -> MeshData {
    MeshData {
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        bounds: (Vec3::ZERO, Vec3::ZERO),
        joints: Vec::new(),
        weights: Vec::new(),
    }
}

fn append_mesh(
    destination: &mut MeshData,
    source: &MeshData,
    transform: Mat4,
    bake_transform: bool,
    primitive_index: usize,
) -> Result<(), CookError> {
    validate_mesh(source, primitive_index)?;
    let destination_was_empty = destination.positions.is_empty();
    let destination_skinned = !destination.joints.is_empty() || !destination.weights.is_empty();
    let source_skinned = !source.joints.is_empty() || !source.weights.is_empty();
    if !destination_was_empty && destination_skinned != source_skinned {
        return Err(CookError::InvalidAsset(format!(
            "cannot merge static and skinned glTF primitives (primitive {primitive_index})"
        )));
    }

    let vertex_offset = u32::try_from(destination.positions.len()).map_err(|_| {
        CookError::InvalidAsset("merged glTF mesh exceeds the u32 vertex address space".into())
    })?;
    let determinant = transform.determinant();
    if bake_transform && (!determinant.is_finite() || determinant.abs() <= 1.0e-8) {
        return Err(CookError::InvalidAsset(format!(
            "glTF primitive {primitive_index} has a singular or non-finite node transform"
        )));
    }
    let normal_transform = transform.inverse().transpose();
    for (&position, &normal) in source.positions.iter().zip(&source.normals) {
        let transformed_position = if bake_transform {
            transform.transform_point3(position)
        } else {
            position
        };
        let transformed_normal = if bake_transform {
            normal_transform.transform_vector3(normal).try_normalize()
        } else {
            normal.try_normalize()
        }
        .ok_or_else(|| {
            CookError::InvalidAsset(format!(
                "glTF primitive {primitive_index} contains a zero or invalid transformed normal"
            ))
        })?;
        if !transformed_position.is_finite() || !transformed_normal.is_finite() {
            return Err(CookError::InvalidAsset(format!(
                "glTF primitive {primitive_index} produced non-finite transformed vertices"
            )));
        }
        destination.positions.push(transformed_position);
        destination.normals.push(transformed_normal);
    }

    append_uvs(destination, source);
    if source_skinned {
        destination.joints.extend_from_slice(&source.joints);
        destination.weights.extend_from_slice(&source.weights);
    }

    let mut local_indices = source.indices.clone();
    if bake_transform && determinant.is_sign_negative() {
        for triangle in local_indices.chunks_exact_mut(3) {
            triangle.swap(1, 2);
        }
    }
    for index in local_indices {
        destination.indices.push(
            vertex_offset
                .checked_add(index)
                .ok_or_else(|| CookError::InvalidAsset("merged glTF index exceeds u32".into()))?,
        );
    }
    Ok(())
}

fn append_uvs(destination: &mut MeshData, source: &MeshData) {
    let previous_vertices = destination.positions.len() - source.positions.len();
    if destination.uvs.is_empty() && !source.uvs.is_empty() && previous_vertices > 0 {
        destination.uvs.resize(previous_vertices, Vec2::ZERO);
    }
    if !destination.uvs.is_empty() {
        if source.uvs.is_empty() {
            destination
                .uvs
                .resize(destination.positions.len(), Vec2::ZERO);
        } else {
            destination.uvs.extend_from_slice(&source.uvs);
        }
    } else if previous_vertices == 0 && !source.uvs.is_empty() {
        destination.uvs.extend_from_slice(&source.uvs);
    }
}

fn validate_mesh(source: &MeshData, primitive_index: usize) -> Result<(), CookError> {
    let vertex_count = source.positions.len();
    if vertex_count == 0
        || source.normals.len() != vertex_count
        || (!source.uvs.is_empty() && source.uvs.len() != vertex_count)
        || (!source.joints.is_empty() && source.joints.len() != vertex_count)
        || (!source.weights.is_empty() && source.weights.len() != vertex_count)
        || source.joints.is_empty() != source.weights.is_empty()
        || source.indices.is_empty()
        || !source.indices.len().is_multiple_of(3)
        || source
            .indices
            .iter()
            .any(|index| *index as usize >= vertex_count)
    {
        return Err(CookError::InvalidAsset(format!(
            "glTF primitive {primitive_index} has inconsistent vertex or triangle data"
        )));
    }
    Ok(())
}

fn recompute_bounds(mesh: &mut MeshData) -> Result<(), CookError> {
    let Some(first) = mesh.positions.first().copied() else {
        return Err(CookError::InvalidAsset(
            "merged glTF mesh contains no vertices".into(),
        ));
    };
    let mut min = first;
    let mut max = first;
    for position in mesh.positions.iter().copied().skip(1) {
        min = min.min(position);
        max = max.max(position);
    }
    mesh.bounds = (min, max);
    Ok(())
}

/// Cook all meshes from a glTF file, writing each as a separate cooked
/// artifact.  Returns a vector of [`CookResult`] values.
///
/// Output names are derived from the base output path with a suffix:
/// `<output_stem>_<mesh_name>.cooked`.
pub fn cook_meshes(source: &Path, output_base: &Path) -> Result<Vec<CookResult>, CookError> {
    let scene = crate::gltf::load_gltf_scene(source)
        .map_err(|error| CookError::Parse(error.to_string()))?;
    if scene.primitives.is_empty() {
        return Err(CookError::InvalidAsset(
            "glTF source contains no mesh primitives".into(),
        ));
    }

    let mut results = Vec::new();
    let parent = output_base.parent().unwrap_or(Path::new(""));
    let stem = output_base
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();

    for (i, primitive) in scene.primitives.iter().enumerate() {
        let safe_name = if primitive.name.is_empty() {
            format!("{stem}_{i}")
        } else {
            format!("{stem}_{}", primitive.name)
        };
        let output_path = parent.join(format!("{safe_name}.cooked"));

        let payload = bincode::serialize(&primitive.mesh)
            .map_err(|e| CookError::InvalidAsset(e.to_string()))?;

        let result = write_cooked_artifact(
            &output_path,
            AssetType::Mesh.kind_code(),
            &payload,
            SchemaVersion::new(0, 1, 0),
        )?;
        results.push(result);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::{append_mesh, empty_mesh, recompute_bounds};
    use crate::mesh::MeshData;
    use glam::{Mat4, Vec3};

    /// Create a minimal mesh for serialisation roundtrip testing.
    fn make_test_mesh() -> MeshData {
        MeshData {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            normals: vec![Vec3::Z, Vec3::Z, Vec3::Z],
            uvs: vec![],
            indices: vec![0, 1, 2],
            bounds: (Vec3::ZERO, Vec3::ONE),
            joints: vec![],
            weights: vec![],
        }
    }

    #[test]
    fn mesh_data_bincode_roundtrip() {
        let mesh = make_test_mesh();
        let bytes = bincode::serialize(&mesh).unwrap();
        let restored: MeshData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.positions.len(), 3);
        assert_eq!(restored.indices.len(), 3);
        assert_eq!(restored.positions[0], Vec3::ZERO);
    }

    #[test]
    fn mesh_with_uvs_roundtrip() {
        let mut mesh = make_test_mesh();
        mesh.uvs = vec![
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(1.0, 0.0),
            glam::Vec2::new(0.0, 1.0),
        ];
        let bytes = bincode::serialize(&mesh).unwrap();
        let restored: MeshData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.uvs.len(), 3);
    }

    #[test]
    fn merge_rebases_indices_and_pads_partial_uv_sets() {
        let first = make_test_mesh();
        let mut second = make_test_mesh();
        second
            .positions
            .iter_mut()
            .for_each(|position| position.x += 2.0);
        second.uvs = vec![glam::Vec2::ONE; 3];
        let mut merged = empty_mesh();

        append_mesh(&mut merged, &first, Mat4::IDENTITY, false, 0).unwrap();
        append_mesh(&mut merged, &second, Mat4::IDENTITY, false, 1).unwrap();
        recompute_bounds(&mut merged).unwrap();

        assert_eq!(merged.positions.len(), 6);
        assert_eq!(merged.indices, [0, 1, 2, 3, 4, 5]);
        assert_eq!(merged.uvs[..3], [glam::Vec2::ZERO; 3]);
        assert_eq!(merged.uvs[3..], [glam::Vec2::ONE; 3]);
        assert_eq!(merged.bounds.1.x, 3.0);
    }

    #[test]
    fn node_bake_transforms_vertices_normals_and_mirrored_winding() {
        let mut source = make_test_mesh();
        let authored_normal = glam::Vec3::new(1.0, 1.0, 0.0).normalize();
        source.normals.fill(authored_normal);
        let transform = Mat4::from_translation(glam::Vec3::new(0.0, 2.0, 0.0))
            * Mat4::from_scale(glam::Vec3::new(-2.0, 1.0, 1.0));
        let mut baked = empty_mesh();

        append_mesh(&mut baked, &source, transform, true, 0).unwrap();

        assert_eq!(baked.positions[0], glam::Vec3::new(0.0, 2.0, 0.0));
        assert_eq!(baked.positions[1], glam::Vec3::new(-2.0, 2.0, 0.0));
        assert_eq!(baked.indices, [0, 2, 1]);
        let expected_normal = glam::Vec3::new(-0.5, 1.0, 0.0).normalize();
        assert!(baked.normals[0].abs_diff_eq(expected_normal, 1.0e-6));
    }
}
