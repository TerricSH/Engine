//! Mesh cooking pipeline.
//!
//! Loads glTF 2.0 files via the existing `crate::mesh` module and serialises
//! the extracted [`MeshData`] as a cooked artifact.

use std::path::Path;

use engine_serialize::SchemaVersion;

use super::error::CookError;
use super::{write_cooked_artifact, AssetType, CookResult};

/// Cook a mesh from a glTF 2.0 source file.
///
/// The source path should point to a `.gltf` or `.glb` file.  The first
/// mesh primitive is extracted using [`crate::mesh::load_mesh_from_gltf`],
/// serialized with bincode, and written as a cooked artifact.
pub fn cook_mesh(source: &Path, output: &Path) -> Result<CookResult, CookError> {
    // 1. Load via existing glTF loader.
    let mesh_data = crate::mesh::load_mesh_from_gltf(source).map_err(|e| match e {
        crate::mesh::MeshError::GltfLoad(msg) => CookError::Parse(msg),
        crate::mesh::MeshError::UnsupportedFormat(msg) => CookError::UnsupportedFormat(msg),
        crate::mesh::MeshError::NoPositions => {
            CookError::InvalidAsset("mesh has no positions".into())
        }
        crate::mesh::MeshError::JointsWeightsMismatch => {
            CookError::InvalidAsset("joints and weights count mismatch".into())
        }
    })?;

    // 2. Serialize with bincode.
    let payload =
        bincode::serialize(&mesh_data).map_err(|e| CookError::InvalidAsset(e.to_string()))?;

    // 3. Write cooked artifact with header.
    let result = write_cooked_artifact(
        output,
        AssetType::Mesh.kind_code(),
        &payload,
        SchemaVersion::new(0, 1, 0),
    )?;

    Ok(result)
}

/// Cook all meshes from a glTF file, writing each as a separate cooked
/// artifact.  Returns a vector of [`CookResult`] values.
///
/// Output names are derived from the base output path with a suffix:
/// `<output_stem>_<mesh_name>.cooked`.
pub fn cook_meshes(source: &Path, output_base: &Path) -> Result<Vec<CookResult>, CookError> {
    let meshes = crate::mesh::load_meshes_from_gltf(source).map_err(|e| match e {
        crate::mesh::MeshError::GltfLoad(msg) => CookError::Parse(msg),
        crate::mesh::MeshError::UnsupportedFormat(msg) => CookError::UnsupportedFormat(msg),
        crate::mesh::MeshError::NoPositions => {
            CookError::InvalidAsset("mesh has no positions".into())
        }
        crate::mesh::MeshError::JointsWeightsMismatch => {
            CookError::InvalidAsset("joints and weights count mismatch".into())
        }
    })?;

    let mut results = Vec::new();
    let parent = output_base.parent().unwrap_or(Path::new(""));
    let stem = output_base
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();

    for (i, (name, mesh_data)) in meshes.iter().enumerate() {
        let safe_name = if name.is_empty() {
            format!("{stem}_{i}")
        } else {
            format!("{stem}_{name}")
        };
        let output_path = parent.join(format!("{safe_name}.cooked"));

        let payload =
            bincode::serialize(mesh_data).map_err(|e| CookError::InvalidAsset(e.to_string()))?;

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
    use crate::mesh::MeshData;
    use glam::Vec3;

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
}
