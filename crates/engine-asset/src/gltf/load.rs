use super::*;
use super::{
    animation::extract_animations,
    material::{extract_material, load_textures},
    mesh::{read_morph_targets, read_skin_attributes},
    node::{compute_bounds, flatten_node, generate_vertex_normals},
};

/// Load one scene from a glTF 2.0 file.
pub fn load_gltf_scene(path: &Path) -> Result<GltfScene, GltfImportError> {
    let parsed = gltf::Gltf::open(path).map_err(|error| GltfImportError::Open {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    let gltf::Gltf { document, blob } = parsed;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let buffers = gltf::import_buffers(&document, Some(base), blob).map_err(|error| {
        GltfImportError::BufferLoad {
            path: path.to_path_buf(),
            detail: error.to_string(),
        }
    })?;

    let materials = document
        .materials()
        .map(|material| extract_material(&material))
        .collect();
    let textures = load_textures(&document, base, &buffers)?;

    let mesh_count = document.meshes().len();
    let mut primitives = Vec::new();
    let mut mesh_to_primitives = vec![Vec::new(); mesh_count];

    for source_mesh in document.meshes() {
        let source_mesh_index = source_mesh.index();
        let source_mesh_name = source_mesh.name().unwrap_or("mesh");
        for (source_primitive_index, primitive) in source_mesh.primitives().enumerate() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                return Err(GltfImportError::UnsupportedTopology {
                    mesh_index: source_mesh_index,
                    primitive_index: source_primitive_index,
                    topology: primitive.mode(),
                });
            }
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
            let positions: Vec<Vec3> = reader
                .read_positions()
                .ok_or(GltfImportError::MissingPositions {
                    mesh_index: source_mesh_index,
                    primitive_index: source_primitive_index,
                })?
                .map(Vec3::from_array)
                .collect();
            if positions.is_empty() {
                return Err(GltfImportError::EmptyPositions {
                    mesh_index: source_mesh_index,
                    primitive_index: source_primitive_index,
                });
            }

            let uvs = reader
                .read_tex_coords(0)
                .map(|values| values.into_f32().map(Vec2::from_array).collect())
                .unwrap_or_default();
            let indices: Vec<u32> = reader
                .read_indices()
                .map(|values| values.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());
            let normals = reader
                .read_normals()
                .map(|values| values.map(Vec3::from_array).collect())
                .unwrap_or_else(|| generate_vertex_normals(&positions, &indices));
            let morph_targets = read_morph_targets(
                &reader,
                positions.len(),
                source_mesh_index,
                source_primitive_index,
            )?;
            let (joints, weights) = read_skin_attributes(
                &reader,
                positions.len(),
                source_mesh_index,
                source_primitive_index,
            )?;
            let material_index = primitive.material().index();
            let primitive_index = primitives.len();
            let bounds = compute_bounds(&positions);

            primitives.push(GltfPrimitive {
                name: format!("{source_mesh_name}_{source_primitive_index}"),
                mesh: MeshData {
                    positions,
                    normals,
                    uvs,
                    indices,
                    bounds,
                    joints,
                    weights,
                },
                material_index,
                topology: gltf::mesh::Mode::Triangles,
                source_mesh_index,
                source_primitive_index,
                morph_targets,
            });
            mesh_to_primitives[source_mesh_index].push(primitive_index);
        }
    }

    let selected_scene = document
        .default_scene()
        .or_else(|| document.scenes().next());
    let selected_scene_index = selected_scene.as_ref().map(gltf::Scene::index);
    let mut nodes = Vec::new();
    let mut roots = Vec::new();
    if let Some(scene) = selected_scene {
        for node in scene.nodes() {
            flatten_node(
                &node,
                Mat4::IDENTITY,
                &mut nodes,
                &mut roots,
                &mesh_to_primitives,
                true,
            );
        }
    }
    let skins = extract_skins(&document, &buffers)?;
    remap_skinned_primitives(&mut primitives, &nodes, &skins)?;
    let animations = extract_animations(&document, &buffers)?;

    Ok(GltfScene {
        selected_scene_index,
        primitives,
        materials,
        textures,
        nodes,
        roots,
        skins,
        animations,
    })
}

fn extract_skins(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<GltfSkin>, GltfImportError> {
    let node_count = document.nodes().len();
    let mut parents = vec![None; node_count];
    let mut local_matrices = vec![Mat4::IDENTITY; node_count];
    for node in document.nodes() {
        local_matrices[node.index()] = Mat4::from_cols_array_2d(&node.transform().matrix());
        for child in node.children() {
            parents[child.index()] = Some(node.index());
        }
    }

    let mut global_cache = vec![None; node_count];
    fn global_matrix(
        node_index: usize,
        parents: &[Option<usize>],
        local_matrices: &[Mat4],
        cache: &mut [Option<Mat4>],
    ) -> Mat4 {
        if let Some(value) = cache[node_index] {
            return value;
        }
        let local = local_matrices[node_index];
        let global = match parents[node_index] {
            Some(parent) => global_matrix(parent, parents, local_matrices, cache) * local,
            None => local,
        };
        cache[node_index] = Some(global);
        global
    }
    for node_index in 0..node_count {
        let _ = global_matrix(node_index, &parents, &local_matrices, &mut global_cache);
    }

    let node_names = document
        .nodes()
        .map(|node| {
            (
                node.index(),
                node.name()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("joint-{}", node.index())),
            )
        })
        .collect::<HashMap<_, _>>();

    let mut result = Vec::with_capacity(document.skins().len());
    for skin in document.skins() {
        let skin_index = skin.index();
        let source_nodes = skin.joints().map(|node| node.index()).collect::<Vec<_>>();
        let source_slot_by_node = source_nodes
            .iter()
            .enumerate()
            .map(|(slot, node)| (*node, slot))
            .collect::<HashMap<_, _>>();
        let reader = skin.reader(|buffer| Some(&buffers[buffer.index()]));
        let inverse_bind_matrices = reader
            .read_inverse_bind_matrices()
            .map(|matrices| matrices.collect::<Vec<_>>())
            .unwrap_or_else(|| vec![Mat4::IDENTITY.to_cols_array_2d(); source_nodes.len()]);
        if inverse_bind_matrices.len() != source_nodes.len() {
            return Err(GltfImportError::InverseBindCountMismatch {
                skin_index,
                joints: source_nodes.len(),
                matrices: inverse_bind_matrices.len(),
            });
        }

        let node_depth = |mut node_index: usize| {
            let mut depth = 0usize;
            while let Some(parent) = parents[node_index] {
                depth += 1;
                node_index = parent;
            }
            depth
        };
        let mut ordered_slots = (0..source_nodes.len()).collect::<Vec<_>>();
        ordered_slots.sort_by_key(|slot| (node_depth(source_nodes[*slot]), *slot));

        let mut joint_remap = vec![0u32; source_nodes.len()];
        for (joint_index, source_slot) in ordered_slots.iter().copied().enumerate() {
            joint_remap[source_slot] = joint_index as u32;
        }

        let mut joints = Vec::with_capacity(source_nodes.len());
        for source_slot in ordered_slots {
            let source_node_index = source_nodes[source_slot];
            let mut ancestor = parents[source_node_index];
            let parent_source_slot = loop {
                match ancestor {
                    Some(node_index) => {
                        if let Some(slot) = source_slot_by_node.get(&node_index) {
                            break Some(*slot);
                        }
                        ancestor = parents[node_index];
                    }
                    None => break None,
                }
            };
            let parent_index = parent_source_slot.map(|slot| joint_remap[slot]);
            let joint_global = global_cache[source_node_index].unwrap_or(Mat4::IDENTITY);
            let local = match parent_source_slot {
                Some(parent_slot) => {
                    let parent_global =
                        global_cache[source_nodes[parent_slot]].unwrap_or(Mat4::IDENTITY);
                    parent_global.inverse() * joint_global
                }
                None => joint_global,
            };
            let (scale, rotation, translation) = local.to_scale_rotation_translation();
            let rotation_length_squared = rotation.length_squared();
            if !local.is_finite()
                || !scale.is_finite()
                || scale.abs().min_element() <= f32::EPSILON
                || !rotation.is_finite()
                || !rotation_length_squared.is_finite()
                || rotation_length_squared <= f32::EPSILON
                || !translation.is_finite()
                || !inverse_bind_matrices[source_slot]
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite())
            {
                return Err(GltfImportError::InvalidJointTransform {
                    skin_index,
                    joint_slot: source_slot,
                });
            }
            joints.push(GltfSkinJoint {
                source_node_index,
                source_joint_slot: source_slot,
                name: node_names[&source_node_index].clone(),
                parent_index,
                translation: translation.to_array(),
                rotation: rotation.normalize().to_array(),
                scale: scale.to_array(),
                inverse_bind_matrix: inverse_bind_matrices[source_slot],
            });
        }
        result.push(GltfSkin {
            source_skin_index: skin_index,
            name: skin
                .name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("skin-{skin_index}")),
            skeleton_root_node: skin.skeleton().map(|node| node.index()),
            joints,
            joint_remap,
        });
    }
    Ok(result)
}

fn remap_skinned_primitives(
    primitives: &mut [GltfPrimitive],
    nodes: &[GltfNode],
    skins: &[GltfSkin],
) -> Result<(), GltfImportError> {
    let mut primitive_skins = vec![HashSet::<usize>::new(); primitives.len()];
    for node in nodes {
        let Some(skin_index) = node.skin_index else {
            continue;
        };
        for primitive_index in &node.primitive_indices {
            if let Some(indices) = primitive_skins.get_mut(*primitive_index) {
                indices.insert(skin_index);
            }
        }
    }

    for (primitive_index, primitive) in primitives.iter_mut().enumerate() {
        if primitive.mesh.joints.is_empty() {
            continue;
        }
        let skin_indices = &primitive_skins[primitive_index];
        if skin_indices.is_empty() {
            return Err(GltfImportError::MissingPrimitiveSkin {
                mesh_index: primitive.source_mesh_index,
                primitive_index: primitive.source_primitive_index,
            });
        }
        if skin_indices.len() != 1 {
            let mut skin_indices = skin_indices.iter().copied().collect::<Vec<_>>();
            skin_indices.sort_unstable();
            return Err(GltfImportError::AmbiguousPrimitiveSkin {
                mesh_index: primitive.source_mesh_index,
                primitive_index: primitive.source_primitive_index,
                skin_indices,
            });
        }
        let skin_index = *skin_indices.iter().next().expect("non-empty checked");
        let Some(skin) = skins.get(skin_index) else {
            return Err(GltfImportError::MissingPrimitiveSkin {
                mesh_index: primitive.source_mesh_index,
                primitive_index: primitive.source_primitive_index,
            });
        };
        for (vertex_index, joint_indices) in primitive.mesh.joints.iter_mut().enumerate() {
            for joint_slot in joint_indices {
                let Some(remapped) = skin.joint_remap.get(*joint_slot as usize) else {
                    return Err(GltfImportError::JointIndexOutOfRange {
                        mesh_index: primitive.source_mesh_index,
                        primitive_index: primitive.source_primitive_index,
                        vertex_index,
                        joint_slot: *joint_slot,
                        skin_index,
                        joint_count: skin.joints.len(),
                    });
                };
                *joint_slot = *remapped;
            }
        }
    }
    Ok(())
}
