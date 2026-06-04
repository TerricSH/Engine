//! Mesh data types and glTF loading.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A single mesh with vertex/index data, ready for GPU upload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeshData {
    /// Vertex positions (x,y,z).
    pub positions: Vec<Vec3>,
    /// Vertex normals (one per position, normalized).
    pub normals: Vec<Vec3>,
    /// Vertex texture coordinates (u,v) — optional, empty if absent.
    pub uvs: Vec<Vec2>,
    /// Index buffer (triangles).
    pub indices: Vec<u32>,
    /// Bounding box.
    pub bounds: (Vec3, Vec3), // min, max
    /// Skinning joint indices (4 per vertex), empty if not skinned.
    pub joints: Vec<[u32; 4]>,
    /// Skinning blend weights (4 per vertex), empty if not skinned.
    pub weights: Vec<[f32; 4]>,
}

/// Errors from mesh loading.
#[derive(Debug, Error)]
pub enum MeshError {
    #[error("glTF load failed: {0}")]
    GltfLoad(String),
    #[error("unsupported mesh format: {0}")]
    UnsupportedFormat(String),
    #[error("mesh has no positions")]
    NoPositions,
    #[error("joints and weights count mismatch")]
    JointsWeightsMismatch,
}

/// Load a mesh from a glTF 2.0 file.
///
/// Returns the first mesh found in the file.  If the file contains multiple
/// meshes, use [`load_meshes`] instead.
pub fn load_mesh_from_gltf(path: &std::path::Path) -> Result<MeshData, MeshError> {
    let (doc, buffers, _) = gltf::import(path).map_err(|e| MeshError::GltfLoad(e.to_string()))?;

    // Pick the first mesh's first primitive.
    for mesh in doc.meshes() {
        if let Some(prim) = mesh.primitives().next() {
            let reader = prim.reader(|buffer| Some(&buffers[buffer.index()]));

            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .ok_or(MeshError::NoPositions)?
                .collect();

            let normals: Vec<[f32; 3]> = reader
                .read_normals()
                .map(|iter| iter.collect())
                .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);

            let uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|iter| iter.into_f32().collect())
                .unwrap_or_default();

            let indices: Vec<u32> = reader
                .read_indices()
                .map(|iter| iter.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());

            if positions.is_empty() {
                return Err(MeshError::NoPositions);
            }

            let positions: Vec<Vec3> = positions.into_iter().map(Vec3::from_array).collect();
            let normals: Vec<Vec3> = normals.into_iter().map(Vec3::from_array).collect();
            let uvs: Vec<Vec2> = uvs.into_iter().map(Vec2::from_array).collect();

            let joints: Vec<[u16; 4]> = reader
                .read_joints(0)
                .map(|iter| iter.into_u16().collect())
                .unwrap_or_default();
            let weights: Vec<[f32; 4]> = reader
                .read_weights(0)
                .map(|iter| iter.into_f32().collect())
                .unwrap_or_default();
            let joints_u32: Vec<[u32; 4]> = joints
                .iter()
                .map(|&j| [j[0] as u32, j[1] as u32, j[2] as u32, j[3] as u32])
                .collect();
            if !joints_u32.is_empty() && joints_u32.len() != weights.len() {
                return Err(MeshError::JointsWeightsMismatch);
            }

            let (min, max) = compute_bounds(&positions);

            return Ok(MeshData {
                positions,
                normals,
                uvs,
                indices,
                bounds: (min, max),
                joints: joints_u32,
                weights,
            });
        }
    }

    Err(MeshError::UnsupportedFormat("no primitives found".into()))
}

/// Load all meshes from a glTF file, returning (name, MeshData) pairs.
pub fn load_meshes_from_gltf(path: &std::path::Path) -> Result<Vec<(String, MeshData)>, MeshError> {
    let (doc, buffers, _) = gltf::import(path).map_err(|e| MeshError::GltfLoad(e.to_string()))?;

    let mut out = Vec::new();
    for mesh in doc.meshes() {
        for (pi, prim) in mesh.primitives().enumerate() {
            let reader = prim.reader(|buffer| Some(&buffers[buffer.index()]));

            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .ok_or(MeshError::NoPositions)?
                .collect();

            let normals: Vec<[f32; 3]> = reader
                .read_normals()
                .map(|iter| iter.collect())
                .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);

            let uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|iter| iter.into_f32().collect())
                .unwrap_or_default();

            let indices: Vec<u32> = reader
                .read_indices()
                .map(|iter| iter.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());

            if positions.is_empty() {
                continue;
            }

            let joints: Vec<[u16; 4]> = reader
                .read_joints(0)
                .map(|iter| iter.into_u16().collect())
                .unwrap_or_default();
            let weights: Vec<[f32; 4]> = reader
                .read_weights(0)
                .map(|iter| iter.into_f32().collect())
                .unwrap_or_default();
            let joints_u32: Vec<[u32; 4]> = joints
                .iter()
                .map(|&j| [j[0] as u32, j[1] as u32, j[2] as u32, j[3] as u32])
                .collect();

            let name = format!("{}_{}", mesh.name().unwrap_or("mesh"), pi);
            let pos_glam: Vec<Vec3> = positions.iter().map(|&p| Vec3::from_array(p)).collect();
            let data = MeshData {
                positions: pos_glam.clone(),
                normals: normals.iter().map(|&n| Vec3::from_array(n)).collect(),
                uvs: uvs.iter().map(|&u| Vec2::from_array(u)).collect(),
                indices,
                bounds: compute_bounds(&pos_glam),
                joints: joints_u32,
                weights,
            };
            out.push((name, data));
        }
    }

    if out.is_empty() {
        Err(MeshError::UnsupportedFormat("no primitives found".into()))
    } else {
        Ok(out)
    }
}

/// Create a unit cube mesh (useful as a fallback test model).
pub fn create_test_cube() -> MeshData {
    // 24 vertices (4 per face, 6 faces) with unique normals.
    let positions = vec![
        // +X face
        Vec3::new(0.5, -0.5, -0.5),
        Vec3::new(0.5, -0.5, 0.5),
        Vec3::new(0.5, 0.5, 0.5),
        Vec3::new(0.5, 0.5, -0.5),
        // -X face
        Vec3::new(-0.5, -0.5, 0.5),
        Vec3::new(-0.5, -0.5, -0.5),
        Vec3::new(-0.5, 0.5, -0.5),
        Vec3::new(-0.5, 0.5, 0.5),
        // +Y face
        Vec3::new(-0.5, 0.5, -0.5),
        Vec3::new(-0.5, 0.5, 0.5),
        Vec3::new(0.5, 0.5, 0.5),
        Vec3::new(0.5, 0.5, -0.5),
        // -Y face
        Vec3::new(-0.5, -0.5, 0.5),
        Vec3::new(-0.5, -0.5, -0.5),
        Vec3::new(0.5, -0.5, -0.5),
        Vec3::new(0.5, -0.5, 0.5),
        // +Z face
        Vec3::new(-0.5, -0.5, 0.5),
        Vec3::new(0.5, -0.5, 0.5),
        Vec3::new(0.5, 0.5, 0.5),
        Vec3::new(-0.5, 0.5, 0.5),
        // -Z face
        Vec3::new(0.5, -0.5, -0.5),
        Vec3::new(-0.5, -0.5, -0.5),
        Vec3::new(-0.5, 0.5, -0.5),
        Vec3::new(0.5, 0.5, -0.5),
    ];
    let normals = vec![
        Vec3::X,
        Vec3::X,
        Vec3::X,
        Vec3::X,
        Vec3::NEG_X,
        Vec3::NEG_X,
        Vec3::NEG_X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::Y,
        Vec3::Y,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::NEG_Y,
        Vec3::NEG_Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::Z,
        Vec3::Z,
        Vec3::Z,
        Vec3::NEG_Z,
        Vec3::NEG_Z,
        Vec3::NEG_Z,
        Vec3::NEG_Z,
    ];
    // Each face as 2 triangles (6 indices per face), CCW winding.
    let indices = vec![
        0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11, 12, 13, 14, 12, 14, 15, 16, 17,
        18, 16, 18, 19, 20, 21, 22, 20, 22, 23,
    ];
    MeshData {
        positions,
        normals,
        uvs: vec![],
        indices,
        bounds: (Vec3::splat(-0.5), Vec3::splat(0.5)),
        joints: vec![],
        weights: vec![],
    }
}

/// Convert [`MeshData`] with skinning data into the 64-byte stride skinned
/// vertex format used by the skinned forward pipeline:
///
/// - position:  `float32x3`  (offset 0)
/// - normal:    `float32x3`  (offset 12)
/// - texcoords: `float32x2`  (offset 24)
/// - joints:    `uint32x4`   (offset 32)
/// - weights:   `float32x4`  (offset 48)
///
/// Total stride: 64 bytes.
///
/// Returns `None` if the mesh has no joint/weight data.
pub fn mesh_data_to_skinned_bytes(mesh: &MeshData) -> Option<(Vec<u8>, Vec<u8>, u32, bool)> {
    if mesh.joints.is_empty() || mesh.weights.is_empty() {
        return None;
    }
    let vertex_count = mesh.positions.len();
    let stride = 64u64;

    let mut vertex_bytes = Vec::with_capacity(vertex_count * stride as usize);
    for i in 0..vertex_count {
        let pos = mesh.positions.get(i).copied().unwrap_or(Vec3::ZERO);
        let nrm = mesh.normals.get(i).copied().unwrap_or(Vec3::Y);
        let uv = mesh.uvs.get(i).copied().unwrap_or(Vec2::ZERO);
        let joint = mesh.joints.get(i).copied().unwrap_or([0; 4]);
        let weight = mesh.weights.get(i).copied().unwrap_or([0.0; 4]);

        vertex_bytes.extend_from_slice(&pos.x.to_ne_bytes());
        vertex_bytes.extend_from_slice(&pos.y.to_ne_bytes());
        vertex_bytes.extend_from_slice(&pos.z.to_ne_bytes());
        vertex_bytes.extend_from_slice(&nrm.x.to_ne_bytes());
        vertex_bytes.extend_from_slice(&nrm.y.to_ne_bytes());
        vertex_bytes.extend_from_slice(&nrm.z.to_ne_bytes());
        vertex_bytes.extend_from_slice(&uv.x.to_ne_bytes());
        vertex_bytes.extend_from_slice(&uv.y.to_ne_bytes());
        vertex_bytes.extend_from_slice(&joint[0].to_ne_bytes());
        vertex_bytes.extend_from_slice(&joint[1].to_ne_bytes());
        vertex_bytes.extend_from_slice(&joint[2].to_ne_bytes());
        vertex_bytes.extend_from_slice(&joint[3].to_ne_bytes());
        vertex_bytes.extend_from_slice(&weight[0].to_ne_bytes());
        vertex_bytes.extend_from_slice(&weight[1].to_ne_bytes());
        vertex_bytes.extend_from_slice(&weight[2].to_ne_bytes());
        vertex_bytes.extend_from_slice(&weight[3].to_ne_bytes());
    }

    let index_count = mesh.indices.len() as u32;
    let mut index_bytes = Vec::with_capacity(mesh.indices.len() * 4);
    for idx in &mesh.indices {
        index_bytes.extend_from_slice(&idx.to_ne_bytes());
    }

    Some((vertex_bytes, index_bytes, index_count, false))
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

/// Convert [`MeshData`] into interleaved vertex/index bytes suitable for
/// [`engine_renderer::BackendRenderer::upload_mesh`].
///
/// Vertex layout (32-byte stride):
/// - position:  `float32x3`  (offset 0)
/// - normal:    `float32x3`  (offset 12)
/// - texcoords: `float32x2`  (offset 24)
///
/// Index format: `u32` (can be converted to `u16` externally if index count
/// is ≤ 65535).
pub fn mesh_data_to_upload_bytes(mesh: &MeshData) -> (Vec<u8>, Vec<u8>, u32, bool) {
    let vertex_count = mesh.positions.len();
    let stride = 32u64; // 8 floats × 4 bytes

    let mut vertex_bytes = Vec::with_capacity(vertex_count * stride as usize);
    for i in 0..vertex_count {
        let pos = mesh.positions.get(i).copied().unwrap_or(Vec3::ZERO);
        let nrm = mesh.normals.get(i).copied().unwrap_or(Vec3::Y);
        let uv = mesh.uvs.get(i).copied().unwrap_or(Vec2::ZERO);

        vertex_bytes.extend_from_slice(&pos.x.to_ne_bytes());
        vertex_bytes.extend_from_slice(&pos.y.to_ne_bytes());
        vertex_bytes.extend_from_slice(&pos.z.to_ne_bytes());
        vertex_bytes.extend_from_slice(&nrm.x.to_ne_bytes());
        vertex_bytes.extend_from_slice(&nrm.y.to_ne_bytes());
        vertex_bytes.extend_from_slice(&nrm.z.to_ne_bytes());
        vertex_bytes.extend_from_slice(&uv.x.to_ne_bytes());
        vertex_bytes.extend_from_slice(&uv.y.to_ne_bytes());
    }

    let index_count = mesh.indices.len() as u32;
    let index_format_u16 = false; // MeshData uses u32 indices
    let mut index_bytes = Vec::with_capacity(mesh.indices.len() * 4);
    for idx in &mesh.indices {
        index_bytes.extend_from_slice(&idx.to_ne_bytes());
    }

    (vertex_bytes, index_bytes, index_count, index_format_u16)
}

/// Convert [`MeshData`] into vertex/index bytes matching the SceneRenderer's
/// `scene_forward_vertex_layout` (position + color, 32-byte stride).
///
/// Vertex layout:
/// - position: `float32x3` (offset 0)
/// - color:    `float32x4` (offset 12)
/// - pad:      `float32`   (offset 28, set to 0.0)
///
/// The color is derived from the normal vector (`nrm * 0.5 + 0.5`), mapping
/// each normal component into `[0, 1]` range. Alpha is always 1.0.
///
/// This format matches the SceneRenderer's fallback forward pipeline so
/// glTF meshes render immediately without modifying the pipeline setup.
pub fn mesh_data_to_color_bytes(mesh: &MeshData) -> (Vec<u8>, Vec<u8>, u32, bool) {
    let vertex_count = mesh.positions.len();
    let stride = 32u64; // 8 floats × 4 bytes

    let mut vertex_bytes = Vec::with_capacity(vertex_count * stride as usize);
    for i in 0..vertex_count {
        let pos = mesh.positions.get(i).copied().unwrap_or(Vec3::ZERO);
        let nrm = mesh.normals.get(i).copied().unwrap_or(Vec3::Y);
        // Map normal [-1,1] to color [0,1] as a visual debug aid.
        let color = nrm * 0.5 + 0.5;

        vertex_bytes.extend_from_slice(&pos.x.to_ne_bytes());
        vertex_bytes.extend_from_slice(&pos.y.to_ne_bytes());
        vertex_bytes.extend_from_slice(&pos.z.to_ne_bytes());
        vertex_bytes.extend_from_slice(&color.x.to_ne_bytes());
        vertex_bytes.extend_from_slice(&color.y.to_ne_bytes());
        vertex_bytes.extend_from_slice(&color.z.to_ne_bytes());
        vertex_bytes.extend_from_slice(&1.0f32.to_ne_bytes()); // alpha
        vertex_bytes.extend_from_slice(&0.0f32.to_ne_bytes()); // pad
    }

    let index_count = mesh.indices.len() as u32;
    let mut index_bytes = Vec::with_capacity(mesh.indices.len() * 4);
    for idx in &mesh.indices {
        index_bytes.extend_from_slice(&idx.to_ne_bytes());
    }

    (vertex_bytes, index_bytes, index_count, false)
}

