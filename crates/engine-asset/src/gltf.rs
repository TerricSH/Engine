//! Strict glTF 2.0 scene importer.
//!
//! The importer preserves document indices, expands every mesh primitive into
//! an explicit [`GltfPrimitive`], resolves the selected scene's complete world
//! transforms, and preserves `JOINTS_0` / `WEIGHTS_0` data for skinned meshes.
//! Triangle lists are the only supported topology and morph targets remain an
//! explicit unsupported feature.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use glam::{Mat4, Vec2, Vec3};
use thiserror::Error;

use crate::mesh::MeshData;

/// PBR material properties extracted from a glTF material.
#[derive(Clone, Debug)]
pub struct GltfMaterial {
    /// Original glTF material index.
    pub material_index: usize,
    pub name: String,
    pub base_color: [f32; 4],
    /// Original glTF texture index, or `None`.
    pub base_color_texture: Option<usize>,
    pub metallic: f32,
    pub roughness: f32,
    /// Original glTF texture index, or `None`.
    pub metallic_roughness_texture: Option<usize>,
    pub emissive: [f32; 3],
    /// Original glTF texture index, or `None`.
    pub emissive_texture: Option<usize>,
    /// Original glTF texture index, or `None`.
    pub normal_texture: Option<usize>,
    /// Original glTF texture index, or `None`.
    pub occlusion_texture: Option<usize>,
    pub occlusion_strength: f32,
    pub alpha_mode: gltf::material::AlphaMode,
    pub alpha_cutoff: Option<f32>,
    pub double_sided: bool,
}

/// Pixel storage produced by the importer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GltfTextureFormat {
    Rgba8,
}

/// Sampler state attached to one glTF texture object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GltfSampler {
    /// Original sampler index. `None` means the glTF default sampler.
    pub sampler_index: Option<usize>,
    pub mag_filter: Option<gltf::texture::MagFilter>,
    pub min_filter: Option<gltf::texture::MinFilter>,
    pub wrap_s: gltf::texture::WrappingMode,
    pub wrap_t: gltf::texture::WrappingMode,
}

/// One decoded glTF texture object.
///
/// A texture is kept separate from its source image so two texture objects
/// sharing one image but using different samplers remain distinct.
#[derive(Clone, Debug)]
pub struct GltfTexture {
    /// Original glTF texture index.
    pub texture_index: usize,
    /// Original glTF image index.
    pub image_index: usize,
    pub sampler: GltfSampler,
    pub format: GltfTextureFormat,
    /// Tightly packed RGBA8 pixels.
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// One glTF mesh primitive with stable source mappings.
#[derive(Clone, Debug)]
pub struct GltfPrimitive {
    pub name: String,
    pub mesh: MeshData,
    pub material_index: Option<usize>,
    pub topology: gltf::mesh::Mode,
    pub source_mesh_index: usize,
    pub source_primitive_index: usize,
}

/// A node belonging to the selected glTF scene.
#[derive(Clone, Debug)]
pub struct GltfNode {
    /// Original glTF node index.
    pub source_node_index: usize,
    pub name: String,
    /// World-space transform accumulated from the complete parent chain.
    pub transform: Mat4,
    /// All primitive indices referenced by this node.
    pub primitive_indices: Vec<usize>,
    /// Original glTF skin index for a skinned mesh node.
    pub skin_index: Option<usize>,
    /// Child indices into the owning scene's `nodes` vector.
    pub children: Vec<usize>,
}

/// One joint extracted from a glTF skin.
///
/// Joints are stored parent-before-child. `source_joint_slot` is the original
/// index used by `JOINTS_0`; the importer remaps vertex indices to this
/// topological order before returning the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct GltfSkinJoint {
    pub source_node_index: usize,
    pub source_joint_slot: usize,
    pub name: String,
    pub parent_index: Option<u32>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub inverse_bind_matrix: [[f32; 4]; 4],
}

/// A glTF skin ready to convert into the engine animation asset.
#[derive(Clone, Debug, PartialEq)]
pub struct GltfSkin {
    pub source_skin_index: usize,
    pub name: String,
    pub skeleton_root_node: Option<usize>,
    pub joints: Vec<GltfSkinJoint>,
    /// Original glTF joint slot -> topological `joints` index.
    pub joint_remap: Vec<u32>,
}

/// One animation track property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GltfAnimationProperty {
    Translation,
    Rotation,
    Scale,
}

/// Values for one animation channel.
#[derive(Clone, Debug, PartialEq)]
pub enum GltfAnimationValues {
    Translations(Vec<[f32; 3]>),
    Rotations(Vec<[f32; 4]>),
    Scales(Vec<[f32; 3]>),
}

/// One animation channel converted to the engine's linear keyframe format.
///
/// STEP channels are expanded with held keys and CUBICSPLINE channels are
/// deterministically resampled during import.
#[derive(Clone, Debug, PartialEq)]
pub struct GltfAnimationChannel {
    pub target_node_index: usize,
    pub property: GltfAnimationProperty,
    pub times: Vec<f32>,
    pub values: GltfAnimationValues,
}

/// One named animation from the document.
#[derive(Clone, Debug, PartialEq)]
pub struct GltfAnimation {
    pub source_animation_index: usize,
    pub name: String,
    pub duration: f32,
    pub channels: Vec<GltfAnimationChannel>,
}

/// The complete contents of a selected glTF scene after import.
#[derive(Clone, Debug)]
pub struct GltfScene {
    /// The selected default scene index, or the first scene when no default is declared.
    pub selected_scene_index: Option<usize>,
    pub primitives: Vec<GltfPrimitive>,
    pub materials: Vec<GltfMaterial>,
    /// One entry per glTF texture, in original document order.
    pub textures: Vec<GltfTexture>,
    /// Only nodes reachable from the selected scene.
    pub nodes: Vec<GltfNode>,
    pub roots: Vec<usize>,
    /// Every skin in original document order.
    pub skins: Vec<GltfSkin>,
    /// Every animation in original document order.
    pub animations: Vec<GltfAnimation>,
}

/// Structured failures from the strict glTF importer.
#[derive(Debug, Error)]
pub enum GltfImportError {
    #[error("failed to open glTF {path}: {detail}", path = .path.display())]
    Open { path: PathBuf, detail: String },

    #[error("failed to load buffers for glTF {path}: {detail}", path = .path.display())]
    BufferLoad { path: PathBuf, detail: String },

    #[error(
        "glTF texture {texture_index} references image {image_index} at {image_source}, but decoding failed: {detail}"
    )]
    TextureDecode {
        texture_index: usize,
        image_index: usize,
        image_source: String,
        detail: String,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} uses unsupported topology {topology:?}; only triangle-list is supported"
    )]
    UnsupportedTopology {
        mesh_index: usize,
        primitive_index: usize,
        topology: gltf::mesh::Mode,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} uses morph targets, which are unsupported by the importer"
    )]
    UnsupportedMorphTargets {
        mesh_index: usize,
        primitive_index: usize,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} must provide JOINTS_0 and WEIGHTS_0 together"
    )]
    IncompleteSkinAttributes {
        mesh_index: usize,
        primitive_index: usize,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} has {positions} positions, {joints} joint tuples, and {weights} weight tuples"
    )]
    SkinAttributeCountMismatch {
        mesh_index: usize,
        primitive_index: usize,
        positions: usize,
        joints: usize,
        weights: usize,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} vertex {vertex_index} has invalid skin weights"
    )]
    InvalidSkinWeights {
        mesh_index: usize,
        primitive_index: usize,
        vertex_index: usize,
    },

    #[error(
        "glTF skin {skin_index} inverse-bind accessor has {matrices} matrices for {joints} joints"
    )]
    InverseBindCountMismatch {
        skin_index: usize,
        joints: usize,
        matrices: usize,
    },

    #[error("glTF skin {skin_index} joint {joint_slot} has invalid transform data")]
    InvalidJointTransform {
        skin_index: usize,
        joint_slot: usize,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} has skin attributes but is not instantiated by a selected-scene node with a skin"
    )]
    MissingPrimitiveSkin {
        mesh_index: usize,
        primitive_index: usize,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} is instantiated with incompatible skins {skin_indices:?}"
    )]
    AmbiguousPrimitiveSkin {
        mesh_index: usize,
        primitive_index: usize,
        skin_indices: Vec<usize>,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} vertex {vertex_index} references joint slot {joint_slot}, but skin {skin_index} has only {joint_count} joints"
    )]
    JointIndexOutOfRange {
        mesh_index: usize,
        primitive_index: usize,
        vertex_index: usize,
        joint_slot: u32,
        skin_index: usize,
        joint_count: usize,
    },

    #[error(
        "glTF animation {animation_index} channel {channel_index} targets morph weights, which are unsupported"
    )]
    UnsupportedAnimationWeights {
        animation_index: usize,
        channel_index: usize,
    },

    #[error(
        "glTF animation {animation_index} channel {channel_index} has {inputs} input times and {outputs} output values"
    )]
    AnimationKeyCountMismatch {
        animation_index: usize,
        channel_index: usize,
        inputs: usize,
        outputs: usize,
    },

    #[error(
        "glTF animation {animation_index} channel {channel_index} would produce {keys} keys, exceeding the per-channel limit of {max}"
    )]
    AnimationKeyLimitExceeded {
        animation_index: usize,
        channel_index: usize,
        keys: usize,
        max: usize,
    },

    #[error(
        "glTF animation {animation_index} channel {channel_index} contains invalid or unsorted keyframe data"
    )]
    InvalidAnimationChannel {
        animation_index: usize,
        channel_index: usize,
    },

    #[error("glTF mesh {mesh_index} primitive {primitive_index} has no POSITION attribute")]
    MissingPositions {
        mesh_index: usize,
        primitive_index: usize,
    },

    #[error("glTF mesh {mesh_index} primitive {primitive_index} has an empty POSITION attribute")]
    EmptyPositions {
        mesh_index: usize,
        primitive_index: usize,
    },
}

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
            if primitive.morph_targets().next().is_some() {
                return Err(GltfImportError::UnsupportedMorphTargets {
                    mesh_index: source_mesh_index,
                    primitive_index: source_primitive_index,
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

fn extract_animations(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<GltfAnimation>, GltfImportError> {
    use gltf::animation::{util::ReadOutputs, Property};

    let mut result = Vec::with_capacity(document.animations().len());
    for animation in document.animations() {
        let animation_index = animation.index();
        let mut duration = 0.0f32;
        let mut channels = Vec::new();
        for (channel_index, channel) in animation.channels().enumerate() {
            let interpolation = channel.sampler().interpolation();
            let target = channel.target();
            if target.property() == Property::MorphTargetWeights {
                return Err(GltfImportError::UnsupportedAnimationWeights {
                    animation_index,
                    channel_index,
                });
            }
            let reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));
            let Some(inputs) = reader.read_inputs() else {
                return Err(GltfImportError::InvalidAnimationChannel {
                    animation_index,
                    channel_index,
                });
            };
            let source_times = inputs.collect::<Vec<_>>();
            let minimum_baked_key_count = match interpolation {
                gltf::animation::Interpolation::Step => {
                    source_times.len().saturating_mul(2).saturating_sub(1)
                }
                _ => source_times.len(),
            };
            if minimum_baked_key_count > MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL {
                return Err(GltfImportError::AnimationKeyLimitExceeded {
                    animation_index,
                    channel_index,
                    keys: minimum_baked_key_count,
                    max: MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL,
                });
            }
            let source_times_valid = source_times
                .iter()
                .all(|time| time.is_finite() && *time >= 0.0)
                && source_times.windows(2).all(|pair| pair[0] <= pair[1]);
            if !source_times_valid {
                return Err(GltfImportError::InvalidAnimationChannel {
                    animation_index,
                    channel_index,
                });
            }
            let Some(outputs) = reader.read_outputs() else {
                return Err(GltfImportError::InvalidAnimationChannel {
                    animation_index,
                    channel_index,
                });
            };
            let (times, property, values) = match outputs {
                ReadOutputs::Translations(values) => {
                    let values = values.collect::<Vec<_>>();
                    validate_animation_output_count(
                        animation_index,
                        channel_index,
                        source_times.len(),
                        values.len(),
                        interpolation,
                    )?;
                    let (times, values) =
                        bake_vec3_animation_track(&source_times, &values, interpolation);
                    (
                        times,
                        GltfAnimationProperty::Translation,
                        GltfAnimationValues::Translations(values),
                    )
                }
                ReadOutputs::Rotations(values) => {
                    let values = values.into_f32().collect::<Vec<_>>();
                    validate_animation_output_count(
                        animation_index,
                        channel_index,
                        source_times.len(),
                        values.len(),
                        interpolation,
                    )?;
                    let (times, values) =
                        bake_quaternion_animation_track(&source_times, &values, interpolation);
                    (
                        times,
                        GltfAnimationProperty::Rotation,
                        GltfAnimationValues::Rotations(values),
                    )
                }
                ReadOutputs::Scales(values) => {
                    let values = values.collect::<Vec<_>>();
                    validate_animation_output_count(
                        animation_index,
                        channel_index,
                        source_times.len(),
                        values.len(),
                        interpolation,
                    )?;
                    let (times, values) =
                        bake_vec3_animation_track(&source_times, &values, interpolation);
                    (
                        times,
                        GltfAnimationProperty::Scale,
                        GltfAnimationValues::Scales(values),
                    )
                }
                ReadOutputs::MorphTargetWeights(_) => {
                    return Err(GltfImportError::UnsupportedAnimationWeights {
                        animation_index,
                        channel_index,
                    });
                }
            };
            if times.len() > MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL {
                return Err(GltfImportError::AnimationKeyLimitExceeded {
                    animation_index,
                    channel_index,
                    keys: times.len(),
                    max: MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL,
                });
            }
            let times_valid = times.iter().all(|time| time.is_finite() && *time >= 0.0)
                && times.windows(2).all(|pair| pair[0] <= pair[1]);
            let values_valid = match &values {
                GltfAnimationValues::Translations(values) | GltfAnimationValues::Scales(values) => {
                    values.iter().flatten().all(|value| value.is_finite())
                }
                GltfAnimationValues::Rotations(values) => values.iter().all(|value| {
                    value.iter().all(|component| component.is_finite())
                        && value
                            .iter()
                            .map(|component| component * component)
                            .sum::<f32>()
                            > f32::EPSILON
                }),
            };
            if !times_valid || !values_valid {
                return Err(GltfImportError::InvalidAnimationChannel {
                    animation_index,
                    channel_index,
                });
            }
            if let Some(last) = times.last() {
                duration = duration.max(*last);
            }
            channels.push(GltfAnimationChannel {
                target_node_index: target.node().index(),
                property,
                times,
                values,
            });
        }
        result.push(GltfAnimation {
            source_animation_index: animation_index,
            name: animation
                .name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("animation-{animation_index}")),
            duration,
            channels,
        });
    }
    Ok(result)
}

fn validate_animation_output_count(
    animation_index: usize,
    channel_index: usize,
    input_count: usize,
    output_count: usize,
    interpolation: gltf::animation::Interpolation,
) -> Result<(), GltfImportError> {
    let expected = match interpolation {
        gltf::animation::Interpolation::Linear | gltf::animation::Interpolation::Step => {
            input_count
        }
        gltf::animation::Interpolation::CubicSpline => input_count.saturating_mul(3),
    };
    if output_count == expected {
        Ok(())
    } else {
        Err(GltfImportError::AnimationKeyCountMismatch {
            animation_index,
            channel_index,
            inputs: input_count,
            outputs: output_count,
        })
    }
}

fn bake_vec3_animation_track(
    times: &[f32],
    values: &[[f32; 3]],
    interpolation: gltf::animation::Interpolation,
) -> (Vec<f32>, Vec<[f32; 3]>) {
    match interpolation {
        gltf::animation::Interpolation::Linear => (times.to_vec(), values.to_vec()),
        gltf::animation::Interpolation::Step => bake_step_animation_track(times, values),
        gltf::animation::Interpolation::CubicSpline => {
            bake_cubic_vec3_animation_track(times, values)
        }
    }
}

fn bake_quaternion_animation_track(
    times: &[f32],
    values: &[[f32; 4]],
    interpolation: gltf::animation::Interpolation,
) -> (Vec<f32>, Vec<[f32; 4]>) {
    match interpolation {
        gltf::animation::Interpolation::Linear => (
            times.to_vec(),
            values
                .iter()
                .copied()
                .map(normalize_animation_quaternion)
                .collect(),
        ),
        gltf::animation::Interpolation::Step => {
            let values = values
                .iter()
                .copied()
                .map(normalize_animation_quaternion)
                .collect::<Vec<_>>();
            bake_step_animation_track(times, &values)
        }
        gltf::animation::Interpolation::CubicSpline => {
            bake_cubic_quaternion_animation_track(times, values)
        }
    }
}

/// Preserve STEP semantics in a linear-only runtime by inserting a held value
/// immediately before every discontinuity.
fn bake_step_animation_track<T: Copy>(times: &[f32], values: &[T]) -> (Vec<f32>, Vec<T>) {
    if times.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut baked_times = Vec::with_capacity(times.len().saturating_mul(2));
    let mut baked_values = Vec::with_capacity(values.len().saturating_mul(2));
    baked_times.push(times[0]);
    baked_values.push(values[0]);
    for index in 1..times.len() {
        let hold_time = times[index].next_down();
        if hold_time > times[index - 1] {
            baked_times.push(hold_time);
            baked_values.push(values[index - 1]);
        }
        baked_times.push(times[index]);
        baked_values.push(values[index]);
    }
    (baked_times, baked_values)
}

const CUBIC_SPLINE_SAMPLES_PER_SECOND: f32 = 60.0;
const MAX_CUBIC_SPLINE_SAMPLES_PER_SEGMENT: usize = 1024;
const MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL: usize = 65_536;

fn bake_cubic_vec3_animation_track(
    times: &[f32],
    values: &[[f32; 3]],
) -> (Vec<f32>, Vec<[f32; 3]>) {
    if times.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut baked_times = vec![times[0]];
    let mut baked_values = vec![values[1]];
    let segment_count = times.len().saturating_sub(1);
    for segment in 0..segment_count {
        let start_time = times[segment];
        let end_time = times[segment + 1];
        let duration = end_time - start_time;
        if duration <= 0.0 {
            baked_times.push(end_time);
            baked_values.push(values[(segment + 1) * 3 + 1]);
            continue;
        }
        let steps = cubic_segment_steps(duration, segment_count);
        let p0 = glam::Vec3::from_array(values[segment * 3 + 1]);
        let m0 = glam::Vec3::from_array(values[segment * 3 + 2]) * duration;
        let p1 = glam::Vec3::from_array(values[(segment + 1) * 3 + 1]);
        let m1 = glam::Vec3::from_array(values[(segment + 1) * 3]) * duration;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            baked_times.push(start_time + duration * t);
            baked_values.push(hermite_vec3(p0, m0, p1, m1, t).to_array());
        }
    }
    (baked_times, baked_values)
}

fn bake_cubic_quaternion_animation_track(
    times: &[f32],
    values: &[[f32; 4]],
) -> (Vec<f32>, Vec<[f32; 4]>) {
    if times.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut baked_times = vec![times[0]];
    let mut baked_values = vec![normalize_animation_quaternion(values[1])];
    let segment_count = times.len().saturating_sub(1);
    for segment in 0..segment_count {
        let start_time = times[segment];
        let end_time = times[segment + 1];
        let duration = end_time - start_time;
        if duration <= 0.0 {
            baked_times.push(end_time);
            baked_values.push(normalize_animation_quaternion(
                values[(segment + 1) * 3 + 1],
            ));
            continue;
        }
        let steps = cubic_segment_steps(duration, segment_count);
        let p0 = glam::Vec4::from_array(values[segment * 3 + 1]);
        let m0 = glam::Vec4::from_array(values[segment * 3 + 2]) * duration;
        let p1 = glam::Vec4::from_array(values[(segment + 1) * 3 + 1]);
        let m1 = glam::Vec4::from_array(values[(segment + 1) * 3]) * duration;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            baked_times.push(start_time + duration * t);
            baked_values.push(normalize_animation_quaternion(
                hermite_vec4(p0, m0, p1, m1, t).to_array(),
            ));
        }
    }
    (baked_times, baked_values)
}

fn cubic_segment_steps(duration: f32, segment_count: usize) -> usize {
    let total_budget_per_segment = MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL
        .saturating_sub(1)
        .checked_div(segment_count.max(1))
        .unwrap_or(1)
        .max(1);
    ((duration * CUBIC_SPLINE_SAMPLES_PER_SECOND).ceil() as usize).clamp(
        1,
        MAX_CUBIC_SPLINE_SAMPLES_PER_SEGMENT.min(total_budget_per_segment),
    )
}

fn hermite_vec3(
    p0: glam::Vec3,
    m0: glam::Vec3,
    p1: glam::Vec3,
    m1: glam::Vec3,
    t: f32,
) -> glam::Vec3 {
    let t2 = t * t;
    let t3 = t2 * t;
    p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
        + m0 * (t3 - 2.0 * t2 + t)
        + p1 * (-2.0 * t3 + 3.0 * t2)
        + m1 * (t3 - t2)
}

fn hermite_vec4(
    p0: glam::Vec4,
    m0: glam::Vec4,
    p1: glam::Vec4,
    m1: glam::Vec4,
    t: f32,
) -> glam::Vec4 {
    let t2 = t * t;
    let t3 = t2 * t;
    p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
        + m0 * (t3 - 2.0 * t2 + t)
        + p1 * (-2.0 * t3 + 3.0 * t2)
        + m1 * (t3 - t2)
}

fn normalize_animation_quaternion(value: [f32; 4]) -> [f32; 4] {
    let rotation = glam::Quat::from_array(value);
    let length_squared = rotation.length_squared();
    if rotation.is_finite() && length_squared.is_finite() && length_squared > f32::EPSILON {
        (rotation / length_squared.sqrt()).to_array()
    } else {
        value
    }
}

fn read_skin_attributes<'a, 's, F>(
    reader: &gltf::mesh::Reader<'a, 's, F>,
    position_count: usize,
    mesh_index: usize,
    primitive_index: usize,
) -> Result<(Vec<[u32; 4]>, Vec<[f32; 4]>), GltfImportError>
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

fn extract_material(material: &gltf::Material<'_>) -> GltfMaterial {
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

fn load_textures(
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

fn flatten_node(
    node: &gltf::Node<'_>,
    parent_transform: Mat4,
    nodes: &mut Vec<GltfNode>,
    roots: &mut Vec<usize>,
    mesh_to_primitives: &[Vec<usize>],
    is_root: bool,
) -> usize {
    let local_transform = Mat4::from_cols_array_2d(&node.transform().matrix());
    let transform = parent_transform * local_transform;
    let primitive_indices = node
        .mesh()
        .and_then(|mesh| mesh_to_primitives.get(mesh.index()))
        .cloned()
        .unwrap_or_default();
    let node_index = nodes.len();

    nodes.push(GltfNode {
        source_node_index: node.index(),
        name: node.name().unwrap_or("node").to_string(),
        transform,
        primitive_indices,
        skin_index: node.skin().map(|skin| skin.index()),
        children: Vec::new(),
    });
    if is_root {
        roots.push(node_index);
    }

    for child in node.children() {
        let child_index = flatten_node(&child, transform, nodes, roots, mesh_to_primitives, false);
        nodes[node_index].children.push(child_index);
    }
    node_index
}

fn compute_bounds(positions: &[Vec3]) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for position in positions {
        min = min.min(*position);
        max = max.max(*position);
    }
    (min, max)
}

fn generate_vertex_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let (Some(&pa), Some(&pb), Some(&pc)) =
            (positions.get(a), positions.get(b), positions.get(c))
        else {
            continue;
        };
        let face = (pb - pa).cross(pc - pa);
        if !face.is_finite() || face.length_squared() <= f32::EPSILON {
            continue;
        }
        normals[a] += face;
        normals[b] += face;
        normals[c] += face;
    }
    normals
        .into_iter()
        .map(|normal| normal.try_normalize().unwrap_or(Vec3::Y))
        .collect()
}

fn decode_gltf_image(
    image: gltf::image::Data,
    texture_index: usize,
    image_index: usize,
    sampler: GltfSampler,
) -> GltfTexture {
    let pixel_count = image.width as usize * image.height as usize;
    let mut data = Vec::with_capacity(pixel_count * 4);
    match image.format {
        gltf::image::Format::R8 => {
            for &red in &image.pixels {
                data.extend_from_slice(&[red, red, red, 255]);
            }
        }
        gltf::image::Format::R8G8 => {
            for pixel in image.pixels.chunks_exact(2) {
                data.extend_from_slice(&[pixel[0], pixel[1], 0, 255]);
            }
        }
        gltf::image::Format::R8G8B8 => {
            for pixel in image.pixels.chunks_exact(3) {
                data.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        gltf::image::Format::R8G8B8A8 => data = image.pixels,
        gltf::image::Format::R16 => {
            for pixel in image.pixels.chunks_exact(2) {
                let red = u16_to_u8(pixel);
                data.extend_from_slice(&[red, red, red, 255]);
            }
        }
        gltf::image::Format::R16G16 => {
            for pixel in image.pixels.chunks_exact(4) {
                data.extend_from_slice(&[u16_to_u8(pixel), u16_to_u8(&pixel[2..]), 0, 255]);
            }
        }
        gltf::image::Format::R16G16B16 => {
            for pixel in image.pixels.chunks_exact(6) {
                data.extend_from_slice(&[
                    u16_to_u8(pixel),
                    u16_to_u8(&pixel[2..]),
                    u16_to_u8(&pixel[4..]),
                    255,
                ]);
            }
        }
        gltf::image::Format::R16G16B16A16 => {
            for pixel in image.pixels.chunks_exact(8) {
                data.extend_from_slice(&[
                    u16_to_u8(pixel),
                    u16_to_u8(&pixel[2..]),
                    u16_to_u8(&pixel[4..]),
                    u16_to_u8(&pixel[6..]),
                ]);
            }
        }
        gltf::image::Format::R32G32B32FLOAT => {
            for pixel in image.pixels.chunks_exact(12) {
                data.extend_from_slice(&[
                    f32_to_u8(pixel),
                    f32_to_u8(&pixel[4..]),
                    f32_to_u8(&pixel[8..]),
                    255,
                ]);
            }
        }
        gltf::image::Format::R32G32B32A32FLOAT => {
            for pixel in image.pixels.chunks_exact(16) {
                data.extend_from_slice(&[
                    f32_to_u8(pixel),
                    f32_to_u8(&pixel[4..]),
                    f32_to_u8(&pixel[8..]),
                    f32_to_u8(&pixel[12..]),
                ]);
            }
        }
    }
    debug_assert_eq!(data.len(), pixel_count * 4);

    GltfTexture {
        texture_index,
        image_index,
        sampler,
        format: GltfTextureFormat::Rgba8,
        data,
        width: image.width,
        height: image.height,
    }
}

fn u16_to_u8(bytes: &[u8]) -> u8 {
    (u16::from_ne_bytes([bytes[0], bytes[1]]) / 257) as u8
}

fn f32_to_u8(bytes: &[u8]) -> u8 {
    (f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).clamp(0.0, 1.0) * 255.0).round()
        as u8
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use glam::{Quat, Vec3};

    use super::*;

    static TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn model_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/models")
            .join(name)
    }

    fn temp_dir(test_name: &str) -> PathBuf {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "engine-asset-{test_name}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test directory");
        path
    }

    fn assert_mat4_close(actual: Mat4, expected: Mat4) {
        for (actual, expected) in actual
            .to_cols_array()
            .into_iter()
            .zip(expected.to_cols_array())
        {
            assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
        }
    }

    #[test]
    fn step_animation_bakes_held_values_before_discontinuities() {
        let (times, values) = bake_vec3_animation_track(
            &[0.0, 1.0, 2.0],
            &[[0.0; 3], [10.0; 3], [20.0; 3]],
            gltf::animation::Interpolation::Step,
        );
        assert_eq!(times.len(), 5);
        assert_eq!(
            values,
            vec![[0.0; 3], [0.0; 3], [10.0; 3], [10.0; 3], [20.0; 3]]
        );
        assert_eq!(times[0], 0.0);
        assert!(times[1] < 1.0 && times[1] > 0.0);
        assert_eq!(times[2], 1.0);
        assert!(times[3] < 2.0 && times[3] > 1.0);
        assert_eq!(times[4], 2.0);
    }

    #[test]
    fn cubic_vec3_animation_is_resampled_with_hermite_tangents() {
        let (times, values) = bake_vec3_animation_track(
            &[0.0, 1.0],
            &[[0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], [1.0; 3], [0.0; 3]],
            gltf::animation::Interpolation::CubicSpline,
        );
        assert_eq!(times.len(), 61);
        assert!((times[15] - 0.25).abs() < 1.0e-6);
        assert!((values[15][0] - 0.15625).abs() < 1.0e-5);
        assert_eq!(values[60], [1.0; 3]);
    }

    #[test]
    fn cubic_quaternion_animation_normalizes_every_baked_key() {
        let (_, values) = bake_quaternion_animation_track(
            &[0.0, 1.0],
            &[
                [0.0; 4],
                [0.0, 0.0, 0.0, 2.0],
                [0.0; 4],
                [0.0; 4],
                [0.0, 0.0, 2.0, 0.0],
                [0.0; 4],
            ],
            gltf::animation::Interpolation::CubicSpline,
        );
        assert_eq!(values.first().copied(), Some([0.0, 0.0, 0.0, 1.0]));
        assert_eq!(values.last().copied(), Some([0.0, 0.0, 1.0, 0.0]));
        assert!(values.iter().all(|value| {
            let length_squared = value
                .iter()
                .map(|component| component * component)
                .sum::<f32>();
            (length_squared - 1.0).abs() < 1.0e-5
        }));
    }

    #[test]
    fn gltf_cubic_spline_channel_imports_as_baked_linear_keys() {
        let dir = temp_dir("cubic-animation");
        let gltf_path = dir.join("cubic.gltf");
        let bin_path = dir.join("cubic.bin");
        let mut bytes = Vec::new();
        for time in [0.0f32, 1.0] {
            bytes.extend_from_slice(&time.to_le_bytes());
        }
        for value in [
            [0.0f32; 3],
            [0.0f32; 3],
            [0.0f32; 3],
            [0.0f32; 3],
            [1.0f32; 3],
            [0.0f32; 3],
        ] {
            for component in value {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        assert_eq!(bytes.len(), 80);
        fs::write(&bin_path, bytes).expect("write cubic animation buffer");
        fs::write(
            &gltf_path,
            r#"{
                "asset": { "version": "2.0" },
                "buffers": [{ "uri": "cubic.bin", "byteLength": 80 }],
                "bufferViews": [
                    { "buffer": 0, "byteOffset": 0, "byteLength": 8 },
                    { "buffer": 0, "byteOffset": 8, "byteLength": 72 }
                ],
                "accessors": [
                    { "bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1] },
                    { "bufferView": 1, "componentType": 5126, "count": 6, "type": "VEC3" }
                ],
                "nodes": [{ "name": "Animated" }],
                "animations": [{
                    "name": "Ease",
                    "samplers": [{ "input": 0, "output": 1, "interpolation": "CUBICSPLINE" }],
                    "channels": [{ "sampler": 0, "target": { "node": 0, "path": "translation" } }]
                }],
                "scenes": [{ "nodes": [0] }],
                "scene": 0
            }"#,
        )
        .expect("write cubic glTF");

        let scene = load_gltf_scene(&gltf_path).expect("CUBICSPLINE glTF should load");
        let channel = &scene.animations[0].channels[0];
        assert_eq!(channel.times.len(), 61);
        let GltfAnimationValues::Translations(values) = &channel.values else {
            panic!("translation values expected");
        };
        assert!((values[15][0] - 0.15625).abs() < 1.0e-5);
        assert_eq!(values[60], [1.0; 3]);

        fs::remove_dir_all(dir).expect("remove test directory");
    }

    #[test]
    fn load_triangle_gltf_exposes_canonical_primitive_data() {
        let scene =
            load_gltf_scene(&model_path("triangle.gltf")).expect("triangle.gltf should load");
        assert_eq!(scene.primitives.len(), 1);
        assert_eq!(scene.materials.len(), 0);
        assert_eq!(scene.nodes.len(), 1);
        assert_eq!(scene.roots, vec![0]);
        assert_eq!(scene.nodes[0].primitive_indices, vec![0]);
        assert_eq!(scene.primitives[0].mesh.positions.len(), 3);
        assert!((scene.primitives[0].mesh.positions[0].x + 1.0).abs() < 0.001);
    }

    #[test]
    fn missing_normals_are_generated_from_triangle_geometry() {
        let normals = generate_vertex_normals(&[Vec3::ZERO, Vec3::X, Vec3::Y], &[0, 1, 2]);
        assert_eq!(normals.len(), 3);
        assert!(normals.iter().all(|normal| normal.z > 0.99));
    }

    #[test]
    fn skinned_gltf_preserves_joint_weights_and_node_skin_binding() {
        let dir = temp_dir("skinned-mesh");
        let gltf_path = dir.join("skinned.gltf");
        let bin_path = dir.join("skinned.bin");
        let mut bytes = Vec::new();
        for position in [[-1.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for value in position {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for _ in 0..3 {
            bytes.extend_from_slice(&[0, 1, 1, 1]);
        }
        for _ in 0..3 {
            for value in [0.75f32, 0.25, 0.0, 0.0] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for index in [0u16, 1, 2] {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        for _ in 0..2 {
            for column in 0..4 {
                for row in 0..4 {
                    let value = if column == row { 1.0f32 } else { 0.0 };
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        for time in [0.0f32, 1.0] {
            bytes.extend_from_slice(&time.to_le_bytes());
        }
        for translation in [[0.0f32, 1.0, 0.0], [0.0, 2.0, 0.0]] {
            for value in translation {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        assert_eq!(bytes.len(), 264);
        fs::write(&bin_path, bytes).expect("write skinned buffer");
        fs::write(
            &gltf_path,
            r#"{
                "asset": { "version": "2.0" },
                "buffers": [{ "uri": "skinned.bin", "byteLength": 264 }],
                "bufferViews": [
                    { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
                    { "buffer": 0, "byteOffset": 36, "byteLength": 12 },
                    { "buffer": 0, "byteOffset": 48, "byteLength": 48 },
                    { "buffer": 0, "byteOffset": 96, "byteLength": 6 },
                    { "buffer": 0, "byteOffset": 104, "byteLength": 128 },
                    { "buffer": 0, "byteOffset": 232, "byteLength": 8 },
                    { "buffer": 0, "byteOffset": 240, "byteLength": 24 }
                ],
                "accessors": [
                    { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-1, 0, 0], "max": [1, 1, 0] },
                    { "bufferView": 1, "componentType": 5121, "count": 3, "type": "VEC4" },
                    { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4" },
                    { "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" },
                    { "bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4" },
                    { "bufferView": 5, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1] },
                    { "bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC3" }
                ],
                "meshes": [{
                    "name": "SkinnedTriangle",
                    "primitives": [{
                        "attributes": { "POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2 },
                        "indices": 3
                    }]
                }],
                "nodes": [
                    { "name": "Mesh", "mesh": 0, "skin": 0 },
                    { "name": "RootJoint", "children": [2] },
                    { "name": "ChildJoint" }
                ],
                "skins": [{ "name": "Rig", "joints": [2, 1], "skeleton": 1, "inverseBindMatrices": 4 }],
                "animations": [{
                    "name": "Raise",
                    "samplers": [{ "input": 5, "output": 6, "interpolation": "LINEAR" }],
                    "channels": [{ "sampler": 0, "target": { "node": 2, "path": "translation" } }]
                }],
                "scenes": [{ "nodes": [0, 1] }],
                "scene": 0
            }"#,
        )
        .expect("write skinned glTF");

        let scene = load_gltf_scene(&gltf_path).expect("skinned glTF should load");
        assert_eq!(scene.primitives.len(), 1);
        assert_eq!(scene.primitives[0].mesh.joints, vec![[1, 0, 0, 0]; 3]);
        assert_eq!(
            scene.primitives[0].mesh.weights,
            vec![[0.75, 0.25, 0.0, 0.0]; 3]
        );
        assert_eq!(scene.nodes[0].skin_index, Some(0));
        assert_eq!(scene.nodes[1].skin_index, None);
        assert_eq!(scene.skins.len(), 1);
        assert_eq!(scene.skins[0].name, "Rig");
        assert_eq!(scene.skins[0].joint_remap, vec![1, 0]);
        assert_eq!(scene.skins[0].joints[0].name, "RootJoint");
        assert_eq!(scene.skins[0].joints[0].parent_index, None);
        assert_eq!(scene.skins[0].joints[1].name, "ChildJoint");
        assert_eq!(scene.skins[0].joints[1].parent_index, Some(0));
        assert_eq!(scene.animations.len(), 1);
        assert_eq!(scene.animations[0].name, "Raise");
        assert_eq!(scene.animations[0].duration, 1.0);
        assert_eq!(
            scene.animations[0].channels[0].values,
            GltfAnimationValues::Translations(vec![[0.0, 1.0, 0.0], [0.0, 2.0, 0.0]])
        );

        fs::remove_dir_all(dir).expect("remove test directory");
    }

    #[test]
    fn resource_chain_preserves_all_indices_and_instances() {
        let scene = load_gltf_scene(&model_path("resource-chain.gltf"))
            .expect("resource-chain.gltf should load");

        assert_eq!(scene.selected_scene_index, Some(1));
        assert_eq!(scene.primitives.len(), 2);
        assert_eq!(scene.materials.len(), 2);
        assert_eq!(scene.textures.len(), 2);
        assert_eq!(scene.nodes.len(), 2, "scene 0 decoy must not be selected");
        assert_eq!(scene.roots, vec![0]);

        for (index, primitive) in scene.primitives.iter().enumerate() {
            assert_eq!(primitive.source_mesh_index, 0);
            assert_eq!(primitive.source_primitive_index, index);
            assert_eq!(primitive.material_index, Some(index));
            assert_eq!(primitive.topology, gltf::mesh::Mode::Triangles);
            assert_eq!(primitive.mesh.positions.len(), 3);
            assert_eq!(primitive.mesh.uvs.len(), 3);
        }

        let material0 = &scene.materials[0];
        assert_eq!(material0.material_index, 0);
        assert_eq!(material0.base_color_texture, Some(0));
        assert_eq!(material0.metallic, 0.25);
        assert_eq!(material0.roughness, 0.75);
        assert_eq!(material0.alpha_mode, gltf::material::AlphaMode::Mask);
        assert_eq!(material0.alpha_cutoff, Some(0.4));
        assert!(material0.double_sided);

        let material1 = &scene.materials[1];
        assert_eq!(material1.material_index, 1);
        assert_eq!(material1.base_color_texture, Some(1));
        assert_eq!(material1.alpha_mode, gltf::material::AlphaMode::Blend);
        assert!(!material1.double_sided);

        let texture0 = &scene.textures[0];
        let texture1 = &scene.textures[1];
        assert_eq!((texture0.texture_index, texture0.image_index), (0, 0));
        assert_eq!((texture1.texture_index, texture1.image_index), (1, 0));
        assert_eq!(texture0.format, GltfTextureFormat::Rgba8);
        assert_eq!(texture0.data, texture1.data);
        assert_eq!(texture0.data, [255, 0, 0, 255, 0, 255, 0, 255]);
        assert_eq!(texture0.sampler.sampler_index, Some(0));
        assert_eq!(texture1.sampler.sampler_index, Some(1));
        assert_eq!(
            texture0.sampler.mag_filter,
            Some(gltf::texture::MagFilter::Nearest)
        );
        assert_eq!(
            texture1.sampler.mag_filter,
            Some(gltf::texture::MagFilter::Linear)
        );
        assert_ne!(texture0.sampler.wrap_s, texture1.sampler.wrap_s);

        assert_eq!(scene.nodes[0].name, "RootInstance");
        assert_eq!(scene.nodes[1].name, "ChildInstance");
        assert_eq!(scene.nodes[0].primitive_indices, vec![0, 1]);
        assert_eq!(scene.nodes[1].primitive_indices, vec![0, 1]);
        assert_eq!(scene.nodes[0].children, vec![1]);

        let root_transform = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 1.0, 1.0),
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(1.0, 2.0, 3.0),
        );
        let child_local = Mat4::from_scale_rotation_translation(
            Vec3::new(0.5, 2.0, 1.0),
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            Vec3::new(1.0, 1.0, 0.0),
        );
        assert_mat4_close(scene.nodes[0].transform, root_transform);
        assert_mat4_close(scene.nodes[1].transform, root_transform * child_local);
    }

    #[test]
    fn corrupt_second_texture_reports_original_indices_and_source() {
        let dir = temp_dir("corrupt-texture");
        let gltf_path = dir.join("corrupt.gltf");
        let broken_image = dir.join("broken.png");
        let json = r#"{
            "asset": { "version": "2.0" },
            "images": [
                { "uri": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=" },
                { "uri": "broken.png" }
            ],
            "textures": [ { "source": 0 }, { "source": 1 } ]
        }"#;
        fs::write(&gltf_path, json).expect("write test glTF");
        fs::write(&broken_image, b"not a png").expect("write corrupt image");

        let error = load_gltf_scene(&gltf_path).expect_err("second texture must fail");
        match error {
            GltfImportError::TextureDecode {
                texture_index,
                image_index,
                image_source,
                ..
            } => {
                assert_eq!(texture_index, 1);
                assert_eq!(image_index, 1);
                assert!(image_source.ends_with("broken.png"));
            }
            other => panic!("unexpected error: {other}"),
        }
        fs::remove_dir_all(dir).expect("remove test directory");
    }

    #[test]
    fn non_triangle_primitive_is_rejected_structurally() {
        let dir = temp_dir("non-triangle");
        let source = model_path("resource-chain.gltf");
        let gltf_path = dir.join("resource-chain.gltf");
        let json = fs::read_to_string(source).expect("read fixture").replacen(
            "\"mode\": 4",
            "\"mode\": 1",
            1,
        );
        fs::write(&gltf_path, json).expect("write modified fixture");
        fs::copy(
            model_path("resource-chain.bin"),
            dir.join("resource-chain.bin"),
        )
        .expect("copy buffer");
        fs::copy(
            model_path("resource-chain.png"),
            dir.join("resource-chain.png"),
        )
        .expect("copy texture");

        let error = load_gltf_scene(&gltf_path).expect_err("line primitive must fail");
        assert!(matches!(
            error,
            GltfImportError::UnsupportedTopology {
                mesh_index: 0,
                primitive_index: 0,
                topology: gltf::mesh::Mode::Lines,
            }
        ));
        fs::remove_dir_all(dir).expect("remove test directory");
    }
}
