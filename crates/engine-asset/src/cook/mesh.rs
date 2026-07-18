//! Mesh cooking pipeline.
//!
//! Loads the canonical glTF primitive graph and serialises selected
//! [`crate::mesh::MeshData`] values as cooked artifacts.

use std::path::Path;

use engine_serialize::SchemaVersion;

use super::error::CookError;
use super::{write_cooked_artifact, AssetType, CookResult};

/// Cook a mesh from a glTF 2.0 source file.
///
/// The source path should point to a `.gltf` or `.glb` file containing exactly
/// one primitive. Multi-primitive sources must use [`cook_meshes`] so cooking
/// never silently aliases the first primitive.
pub fn cook_mesh(source: &Path, output: &Path) -> Result<CookResult, CookError> {
    let scene = crate::gltf::load_gltf_scene(source)
        .map_err(|error| CookError::Parse(error.to_string()))?;
    if scene.primitives.len() != 1 {
        return Err(CookError::InvalidAsset(format!(
            "single mesh asset requires exactly one glTF primitive, found {}; use multi-mesh import for this source",
            scene.primitives.len()
        )));
    }
    let mesh_data = scene
        .primitives
        .into_iter()
        .next()
        .expect("primitive count validated")
        .mesh;

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
