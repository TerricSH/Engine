//! Full glTF 2.0 importer — meshes, materials, node hierarchy.
//!
//! Builds on the low-level vertex extraction in [`crate::mesh`] and adds
//! PBR material properties, texture references, and a transform hierarchy
//! that maps onto the engine's ECS World.

use glam::Mat4;

use crate::mesh::{load_meshes_from_gltf, MeshData, MeshError};

// ============================================================================
// Exported types
// ============================================================================

/// PBR material properties extracted from a glTF material.
#[derive(Clone, Debug)]
pub struct GltfMaterial {
    /// Linear-space base colour (RGBA).
    pub base_color: [f32; 4],
    /// Index into the owning scene's `textures` list, or `None`.
    pub base_color_texture: Option<usize>,
    /// Metallic factor in [0,1].
    pub metallic: f32,
    /// Roughness factor in [0,1].
    pub roughness: f32,
    /// Index into the owning scene's `textures` list (ORM texture), or `None`.
    pub metallic_roughness_texture: Option<usize>,
    /// Emissive colour (linear RGB).
    pub emissive: [f32; 3],
    /// Index into `textures`, or `None`.
    pub emissive_texture: Option<usize>,
    /// Index into `textures` (normal map), or `None`.
    pub normal_texture: Option<usize>,
    /// Whether the material uses double-sided rendering.
    pub double_sided: bool,
}

/// A decoded texture (RGBA pixel data).
#[derive(Clone, Debug)]
pub struct GltfTexture {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A single node in the glTF scene graph.
#[derive(Clone, Debug)]
pub struct GltfNode {
    pub name: String,
    /// Local transform relative to the parent node.
    pub transform: Mat4,
    /// Index into the owning scene's `meshes` vec, or `None`.
    pub mesh_index: Option<usize>,
    /// Index into the owning scene's `materials` vec, or `None`.
    pub material_index: Option<usize>,
    /// Child node indices (into the owning scene's `nodes` vec).
    pub children: Vec<usize>,
}

/// The complete contents of a glTF file after import.
#[derive(Clone, Debug)]
pub struct GltfScene {
    pub meshes: Vec<MeshData>,
    pub materials: Vec<GltfMaterial>,
    pub textures: Vec<GltfTexture>,
    pub nodes: Vec<GltfNode>,
    /// Root node indices (nodes without a parent).
    pub roots: Vec<usize>,
}

// ============================================================================
// Import
// ============================================================================

/// Load a full scene from a glTF 2.0 file.
pub fn load_gltf_scene(path: &std::path::Path) -> Result<GltfScene, MeshError> {
    let (doc, _buffers, images) = gltf::import(path).map_err(|e| MeshError::GltfLoad(e.to_string()))?;

    // ── Meshes ─────────────────────────────────────────────────────────
    let mesh_pairs = load_meshes_from_gltf(path)?;
    let meshes: Vec<MeshData> = mesh_pairs.into_iter().map(|(_, m)| m).collect();

    // ── Materials ──────────────────────────────────────────────────────
    let materials: Vec<GltfMaterial> = doc.materials().map(|mat| extract_material(&mat)).collect();

    // ── Textures ───────────────────────────────────────────────────────
    let textures: Vec<GltfTexture> = images
        .into_iter()
        .filter_map(|img| decode_gltf_image(img).ok())
        .collect();

    // ── Nodes ──────────────────────────────────────────────────────────
    let mut nodes: Vec<GltfNode> = Vec::new();
    let mut roots: Vec<usize> = Vec::new();
    if let Some(scene) = doc.scenes().next() {
        for node in scene.nodes() {
            flatten_node(&node, &Mat4::IDENTITY, &mut nodes, &mut roots, &materials, true);
        }
    }

    Ok(GltfScene {
        meshes,
        materials,
        textures,
        nodes,
        roots,
    })
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Extract PBR material properties from a glTF material.
fn extract_material(mat: &gltf::Material<'_>) -> GltfMaterial {
    let pbr = mat.pbr_metallic_roughness();
    let base_color: [f32; 4] = {
        let f = pbr.base_color_factor();
        [f[0], f[1], f[2], f[3]]
    };
    let emissive: [f32; 3] = {
        let f = mat.emissive_factor();
        [f[0], f[1], f[2]]
    };

    GltfMaterial {
        base_color,
        base_color_texture: pbr.base_color_texture().map(|t| t.texture().index()),
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        metallic_roughness_texture: pbr.metallic_roughness_texture().map(|t| t.texture().index()),
        emissive,
        emissive_texture: mat.emissive_texture().map(|t| t.texture().index()),
        normal_texture: mat
            .normal_texture()
            .map(|t| t.texture().index()),
        double_sided: mat.double_sided(),
    }
}

/// Recursively flatten a glTF node tree into the `nodes` vec.
/// `parent_transform` is the accumulated world transform of the parent.
fn flatten_node(
    node: &gltf::Node<'_>,
    parent_transform: &Mat4,
    nodes: &mut Vec<GltfNode>,
    roots: &mut Vec<usize>,
    materials: &[GltfMaterial],
    is_root: bool,
) -> usize {
    // Compute local transform.
    let local_mat = {
        let mat = node.transform().matrix();
        Mat4::from_cols_array_2d(&mat)
    };

    let world = *parent_transform * local_mat;

    // Determine mesh and material indices.
    let (mesh_idx, mat_idx) = node.mesh().map(|m| {
        let mesh_idx = m.index();
        let prim_mat = m.primitives().next().and_then(|p| {
            let gltf_mat = p.material();
            materials.iter().position(|em| {
                em.base_color == extract_material(&gltf_mat).base_color
            })
        });
        (Some(mesh_idx), prim_mat)
    }).unwrap_or((None, None));

    let idx = nodes.len();
    let name = node.name().unwrap_or("node").to_string();
    nodes.push(GltfNode {
        name,
        transform: world,
        mesh_index: mesh_idx,
        material_index: mat_idx,
        children: Vec::new(),
    });

    if is_root {
        roots.push(idx);
    }

    // Recurse into children.
    for child in node.children() {
        let child_idx = flatten_node(&child, &world, nodes, roots, materials, false);
        nodes[idx].children.push(child_idx);
    }

    idx
}

/// Decode a glTF image into RGBA pixel data.
fn decode_gltf_image(img: gltf::image::Data) -> Result<GltfTexture, MeshError> {
    let (data, width, height) = match img.format {
        gltf::image::Format::R8 => {
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|&p| vec![p, p, p, 255]).collect();
            (rgba, img.width, img.height)
        }
        gltf::image::Format::R8G8 => {
            let rgba: Vec<u8> = img.pixels.chunks_exact(2)
                .flat_map(|c| vec![c[0], c[1], 0, 255]).collect();
            (rgba, img.width, img.height)
        }
        gltf::image::Format::R8G8B8 => {
            let rgba: Vec<u8> = img.pixels.chunks_exact(3)
                .flat_map(|c| vec![c[0], c[1], c[2], 255]).collect();
            (rgba, img.width, img.height)
        }
        gltf::image::Format::R8G8B8A8 => (img.pixels, img.width, img.height),
        gltf::image::Format::R16 => {
            let rgba: Vec<u8> = img.pixels.chunks_exact(2)
                .map(|c| u16::from_ne_bytes([c[0], c[1]]) as u8)
                .flat_map(|p| vec![p, p, p, 255]).collect();
            (rgba, img.width, img.height)
        }
        gltf::image::Format::R16G16 => {
            let rgba: Vec<u8> = img.pixels.chunks_exact(4)
                .flat_map(|c| {
                    let r = u16::from_ne_bytes([c[0], c[1]]) as u8;
                    let g = u16::from_ne_bytes([c[2], c[3]]) as u8;
                    vec![r, g, 0, 255]
                }).collect();
            (rgba, img.width, img.height)
        }
        gltf::image::Format::R16G16B16 => {
            let rgba: Vec<u8> = img.pixels.chunks_exact(6)
                .flat_map(|c| {
                    let r = u16::from_ne_bytes([c[0], c[1]]) as u8;
                    let g = u16::from_ne_bytes([c[2], c[3]]) as u8;
                    let b = u16::from_ne_bytes([c[4], c[5]]) as u8;
                    vec![r, g, b, 255]
                }).collect();
            (rgba, img.width, img.height)
        }
        gltf::image::Format::R16G16B16A16 => {
            let rgba: Vec<u8> = img.pixels.chunks_exact(8)
                .flat_map(|c| {
                    let r = u16::from_ne_bytes([c[0], c[1]]) as u8;
                    let g = u16::from_ne_bytes([c[2], c[3]]) as u8;
                    let b = u16::from_ne_bytes([c[4], c[5]]) as u8;
                    let a = u16::from_ne_bytes([c[6], c[7]]) as u8;
                    vec![r, g, b, a]
                }).collect();
            (rgba, img.width, img.height)
        }
        // Float formats: cast f32 → u8 (tone-map via clamp).
        gltf::image::Format::R32G32B32FLOAT => {
            let rgba: Vec<u8> = img.pixels.chunks_exact(12)
                .flat_map(|c| {
                    let r = (f32::from_ne_bytes([c[0], c[1], c[2], c[3]]).clamp(0.0, 1.0) * 255.0) as u8;
                    let g = (f32::from_ne_bytes([c[4], c[5], c[6], c[7]]).clamp(0.0, 1.0) * 255.0) as u8;
                    let b = (f32::from_ne_bytes([c[8], c[9], c[10], c[11]]).clamp(0.0, 1.0) * 255.0) as u8;
                    vec![r, g, b, 255]
                }).collect();
            (rgba, img.width, img.height)
        }
        gltf::image::Format::R32G32B32A32FLOAT => {
            let rgba: Vec<u8> = img.pixels.chunks_exact(16)
                .flat_map(|c| {
                    let r = (f32::from_ne_bytes([c[0], c[1], c[2], c[3]]).clamp(0.0, 1.0) * 255.0) as u8;
                    let g = (f32::from_ne_bytes([c[4], c[5], c[6], c[7]]).clamp(0.0, 1.0) * 255.0) as u8;
                    let b = (f32::from_ne_bytes([c[8], c[9], c[10], c[11]]).clamp(0.0, 1.0) * 255.0) as u8;
                    let a = (f32::from_ne_bytes([c[12], c[13], c[14], c[15]]).clamp(0.0, 1.0) * 255.0) as u8;
                    vec![r, g, b, a]
                }).collect();
            (rgba, img.width, img.height)
        }
    };

    Ok(GltfTexture { data, width, height })
}
