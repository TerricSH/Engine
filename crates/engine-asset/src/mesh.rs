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
    crate::gltf::load_gltf_scene(path)
        .map_err(|error| MeshError::GltfLoad(error.to_string()))?
        .primitives
        .into_iter()
        .next()
        .map(|primitive| primitive.mesh)
        .ok_or_else(|| MeshError::UnsupportedFormat("no primitives found".into()))
}

/// Load all meshes from a glTF file, returning (name, MeshData) pairs.
pub fn load_meshes_from_gltf(path: &std::path::Path) -> Result<Vec<(String, MeshData)>, MeshError> {
    let out: Vec<_> = crate::gltf::load_gltf_scene(path)
        .map_err(|error| MeshError::GltfLoad(error.to_string()))?
        .primitives
        .into_iter()
        .map(|primitive| (primitive.name, primitive.mesh))
        .collect();

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
    if mesh.joints.is_empty()
        || mesh.weights.is_empty()
        || mesh.joints.len() != mesh.positions.len()
        || mesh.weights.len() != mesh.positions.len()
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skinned_bytes_returns_none_for_non_skinned_mesh() {
        let mesh = MeshData {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            normals: vec![Vec3::Z, Vec3::Z, Vec3::Z],
            uvs: vec![],
            indices: vec![0, 1, 2],
            bounds: (Vec3::ZERO, Vec3::ONE),
            joints: vec![],
            weights: vec![],
        };
        assert!(mesh_data_to_skinned_bytes(&mesh).is_none());
    }

    #[test]
    fn skinned_bytes_produces_correct_stride() {
        let mesh = MeshData {
            positions: vec![Vec3::ZERO; 2],
            normals: vec![Vec3::Z; 2],
            uvs: vec![Vec2::ZERO; 2],
            indices: vec![0, 1],
            bounds: (Vec3::ZERO, Vec3::ONE),
            joints: vec![[0, 1, 2, 3]; 2],
            weights: vec![[1.0, 0.0, 0.0, 0.0]; 2],
        };
        let (vb, ib, ic, u16_fmt) = mesh_data_to_skinned_bytes(&mesh).unwrap();
        assert_eq!(vb.len(), 128);
        assert_eq!(ib.len(), 8);
        assert_eq!(ic, 2);
        assert!(!u16_fmt);
    }

    #[test]
    fn skinned_bytes_joints_weights_must_match() {
        let mesh = MeshData {
            positions: vec![Vec3::ZERO; 3],
            normals: vec![Vec3::Z; 3],
            uvs: vec![Vec2::ZERO; 3],
            indices: vec![0, 1, 2],
            bounds: (Vec3::ZERO, Vec3::ONE),
            joints: vec![[0; 4]; 3],
            weights: vec![[1.0, 0.0, 0.0, 0.0]; 3],
        };
        assert!(mesh_data_to_skinned_bytes(&mesh).is_some());

        let mut invalid = mesh;
        invalid.weights.pop();
        assert!(mesh_data_to_skinned_bytes(&invalid).is_none());
    }
}
