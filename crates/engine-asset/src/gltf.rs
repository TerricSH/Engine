//! Full glTF 2.0 importer — meshes, materials, node hierarchy.
//!
//! Builds on a single [`gltf::import`] call and produces a [`GltfScene`]
//! with correctly-indexed meshes, materials, textures, and nodes.

use glam::Mat4;
use glam::Vec2;
use glam::Vec3;

use crate::mesh::{MeshData, MeshError};

// ============================================================================
// Exported types
// ============================================================================

/// PBR material properties extracted from a glTF material.
#[derive(Clone, Debug)]
pub struct GltfMaterial {
    pub base_color: [f32; 4],
    /// Index into the owning scene's `textures` list, or `None`.
    pub base_color_texture: Option<usize>,
    pub metallic: f32,
    pub roughness: f32,
    pub metallic_roughness_texture: Option<usize>,
    pub emissive: [f32; 3],
    pub emissive_texture: Option<usize>,
    pub normal_texture: Option<usize>,
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
    /// World-space transform (accumulated from parent chain).
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
    pub roots: Vec<usize>,
}

// ============================================================================
// Import — single-pass, correctly indexed
// ============================================================================

/// Load a full scene from a glTF 2.0 file.
pub fn load_gltf_scene(path: &std::path::Path) -> Result<GltfScene, MeshError> {
    let (doc, buffers, images_raw) =
        gltf::import(path).map_err(|e| MeshError::GltfLoad(e.to_string()))?;

    // ── Materials (document order) ─────────────────────────────────────
    let materials: Vec<GltfMaterial> = doc.materials().map(|mat| extract_material(&mat)).collect();

    // ── Textures (document order — index matches doc.textures()) ───────
    // Build a mapping of glTF image index → texture index list overlap.
    let textures: Vec<GltfTexture> = doc
        .textures()
        .filter_map(|tex| {
            let img_idx = tex.source().index();
            images_raw.get(img_idx).and_then(|img| {
                decode_gltf_image(img.clone())
                    .map_err(|e| tracing::warn!(target: "gltf", index = img_idx, "texture decode: {e}"))
                    .ok()
            })
        })
        .collect();

    // ── Meshes — expand all primitives into a flat vec ────────────────
    // mesh_doc_to_our[(doc_mesh_idx, prim_idx)] = index in our meshes[]
    let mut meshes: Vec<MeshData> = Vec::new();
    let mut mesh_prim_to_our: Vec<(usize, usize, usize)> = Vec::new(); // (doc_mesh, prim_counter, our_idx)

    for doc_mesh in doc.meshes() {
        let doc_mesh_idx = doc_mesh.index();
        for (prim_counter, prim) in doc_mesh.primitives().enumerate() {
            let reader = prim.reader(|buffer| Some(&buffers[buffer.index()]));
            let our_idx = meshes.len();

            let positions: Vec<Vec3> = reader
                .read_positions()
                .ok_or(MeshError::NoPositions)?
                .map(Vec3::from_array)
                .collect();
            let normals: Vec<Vec3> = reader
                .read_normals()
                .map(|iter| iter.map(Vec3::from_array).collect())
                .unwrap_or_else(|| vec![Vec3::Y; positions.len()]);
            let uvs: Vec<Vec2> = reader
                .read_tex_coords(0)
                .map(|iter| iter.into_f32().map(Vec2::from_array).collect())
                .unwrap_or_default();
            let indices: Vec<u32> = reader
                .read_indices()
                .map(|iter| iter.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());

            if positions.is_empty() {
                continue;
            }

            let (min, max) = compute_bounds(&positions);
            meshes.push(MeshData {
                positions,
                normals,
                uvs,
                indices,
                bounds: (min, max),
                joints: vec![],
                weights: vec![],
            });
            mesh_prim_to_our.push((doc_mesh_idx, prim_counter, our_idx));
        }
    }

    // ── Nodes ──────────────────────────────────────────────────────────
    let mut nodes: Vec<GltfNode> = Vec::new();
    let mut roots: Vec<usize> = Vec::new();
    if let Some(scene) = doc.scenes().next() {
        for node in scene.nodes() {
            flatten_node(
                &node,
                &Mat4::IDENTITY,
                &mut nodes,
                &mut roots,
                &materials,
                &mesh_prim_to_our,
                true,
            );
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
        normal_texture: mat.normal_texture().map(|t| t.texture().index()),
        double_sided: mat.double_sided(),
    }
}

fn flatten_node(
    node: &gltf::Node<'_>,
    parent_transform: &Mat4,
    nodes: &mut Vec<GltfNode>,
    roots: &mut Vec<usize>,
    _materials: &[GltfMaterial],
    mesh_prim_to_our: &[(usize, usize, usize)],
    is_root: bool,
) -> usize {
    let local_mat = {
        let mat = node.transform().matrix();
        Mat4::from_cols_array_2d(&mat)
    };
    let world = *parent_transform * local_mat;

    // Look up the correct mesh index in our flat vec.
    let our_mesh = node.mesh().and_then(|m| {
        let doc_mesh_idx = m.index();
        mesh_prim_to_our
            .iter()
            .find(|(doc, _, _)| *doc == doc_mesh_idx)
            .map(|(_, _, our)| *our)
    });
    let mat_idx: Option<usize> = node
        .mesh()
        .and_then(|m| m.primitives().next())
        .and_then(|p| p.material().index());

    let idx = nodes.len();
    let name = node.name().unwrap_or("node").to_string();
    nodes.push(GltfNode {
        name,
        transform: world,
        mesh_index: our_mesh,
        material_index: mat_idx,
        children: Vec::new(),
    });

    if is_root {
        roots.push(idx);
    }

    for child in node.children() {
        let child_idx = flatten_node(&child, &world, nodes, roots, _materials, mesh_prim_to_our, false);
        nodes[idx].children.push(child_idx);
    }

    idx
}

fn compute_bounds(positions: &[Vec3]) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for p in positions {
        min = min.min(*p);
        max = max.max(*p);
    }
    (min, max)
}

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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_triangle_gltf() {
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/models/triangle.gltf"
        ));
        let scene = load_gltf_scene(path).expect("triangle.gltf should load");
        assert_eq!(scene.meshes.len(), 1, "one mesh (1 primitive)");
        assert_eq!(scene.materials.len(), 0, "no materials in triangle.gltf");
        assert_eq!(scene.nodes.len(), 1, "one node");
        assert_eq!(scene.roots.len(), 1, "one root node");

        let mesh = &scene.meshes[0];
        assert_eq!(mesh.positions.len(), 3, "triangle has 3 vertices");
        assert_eq!(mesh.indices.len(), 3, "triangle has 3 indices");
        assert!(mesh.normals.len() == 3, "triangle has normals");

        let node = &scene.nodes[0];
        assert_eq!(node.mesh_index, Some(0), "node points to first mesh");
        assert!(node.material_index.is_none(), "no material");

        // Verify a known vertex position (from the glTF data: [-1,-1,0])
        assert!((mesh.positions[0].x - (-1.0)).abs() < 0.001);
        assert!((mesh.positions[0].y - (-1.0)).abs() < 0.001);
    }
}
