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
}

/// Load a mesh from a glTF 2.0 file.
///
/// Returns the first mesh found in the file.  If the file contains multiple
/// meshes, use [`load_meshes`] instead.
pub fn load_mesh_from_gltf(path: &std::path::Path) -> Result<MeshData, MeshError> {
    let (doc, buffers, _) = gltf::import(path)
        .map_err(|e| MeshError::GltfLoad(e.to_string()))?;

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

            let (min, max) = compute_bounds(&positions);

            return Ok(MeshData {
                positions,
                normals,
                uvs,
                indices,
                bounds: (min, max),
            });
        }
    }

    Err(MeshError::UnsupportedFormat("no primitives found".into()))
}

/// Load all meshes from a glTF file, returning (name, MeshData) pairs.
pub fn load_meshes_from_gltf(
    path: &std::path::Path,
) -> Result<Vec<(String, MeshData)>, MeshError> {
    let (doc, buffers, _) = gltf::import(path)
        .map_err(|e| MeshError::GltfLoad(e.to_string()))?;

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

            let name = format!("{}_{}", mesh.name().unwrap_or("mesh"), pi);
            let pos_glam: Vec<Vec3> = positions.iter().map(|&p| Vec3::from_array(p)).collect();
            let data = MeshData {
                positions: pos_glam.clone(),
                normals: normals.iter().map(|&n| Vec3::from_array(n)).collect(),
                uvs: uvs.iter().map(|&u| Vec2::from_array(u)).collect(),
                indices,
                bounds: compute_bounds(&pos_glam),
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
        Vec3::new(0.5, -0.5, -0.5), Vec3::new(0.5, 0.5, -0.5),
        Vec3::new(0.5, 0.5, 0.5), Vec3::new(0.5, -0.5, 0.5),
        // -X face
        Vec3::new(-0.5, -0.5, 0.5), Vec3::new(-0.5, 0.5, 0.5),
        Vec3::new(-0.5, 0.5, -0.5), Vec3::new(-0.5, -0.5, -0.5),
        // +Y face
        Vec3::new(-0.5, 0.5, -0.5), Vec3::new(0.5, 0.5, -0.5),
        Vec3::new(0.5, 0.5, 0.5), Vec3::new(-0.5, 0.5, 0.5),
        // -Y face
        Vec3::new(-0.5, -0.5, 0.5), Vec3::new(0.5, -0.5, 0.5),
        Vec3::new(0.5, -0.5, -0.5), Vec3::new(-0.5, -0.5, -0.5),
        // +Z face
        Vec3::new(-0.5, -0.5, 0.5), Vec3::new(-0.5, 0.5, 0.5),
        Vec3::new(0.5, 0.5, 0.5), Vec3::new(0.5, -0.5, 0.5),
        // -Z face
        Vec3::new(0.5, -0.5, -0.5), Vec3::new(0.5, 0.5, -0.5),
        Vec3::new(-0.5, 0.5, -0.5), Vec3::new(-0.5, -0.5, -0.5),
    ];
    let normals = vec![
        Vec3::X, Vec3::X, Vec3::X, Vec3::X,
        Vec3::NEG_X, Vec3::NEG_X, Vec3::NEG_X, Vec3::NEG_X,
        Vec3::Y, Vec3::Y, Vec3::Y, Vec3::Y,
        Vec3::NEG_Y, Vec3::NEG_Y, Vec3::NEG_Y, Vec3::NEG_Y,
        Vec3::Z, Vec3::Z, Vec3::Z, Vec3::Z,
        Vec3::NEG_Z, Vec3::NEG_Z, Vec3::NEG_Z, Vec3::NEG_Z,
    ];
    let _indices: Vec<u32> = (0..24u32).collect();
    // Each face as 2 triangles (6 indices per face)
    let indices = vec![
        0,1,2, 0,2,3, 4,5,6, 4,6,7,
        8,9,10, 8,10,11, 12,13,14, 12,14,15,
        16,17,18, 16,18,19, 20,21,22, 20,22,23,
    ];
    MeshData {
        positions,
        normals,
        uvs: vec![],
        indices,
        bounds: (Vec3::splat(-0.5), Vec3::splat(0.5)),
    }
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
