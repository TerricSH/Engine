use std::path::PathBuf;

use engine_serialize::AssetId;

// ---------------------------------------------------------------------------
// Asset path resolution
// ---------------------------------------------------------------------------

/// Resolve an [`AssetId`] to a conventional filesystem path.
///
/// # Resolution rules
///
/// 1. If the id has a `logical_path`, return `assets/<logical_path>`.
/// 2. Otherwise split the id on the first hyphen (`-`):
///    - The part before the hyphen is treated as a category and mapped to a
///      plural subdirectory (e.g. `mesh` → `meshes/`).
///    - The part after the hyphen becomes the file stem.
///    - The extension is `.asset`.
/// 3. If there is no hyphen the whole id is used as the file stem.
///
/// # Examples
///
/// | `AssetId` | Resolved path |
/// |-----------|--------------|
/// | `id: "mesh-cube"` | `assets/meshes/cube.asset` |
/// | `id: "mat-default"` | `assets/materials/default.asset` |
/// | `id: "scene-gate04"` | `assets/scenes/gate04.asset` |
/// | `id: "my-thing", logical_path: Some("custom/thing.bin")` | `assets/custom/thing.bin` |
pub fn asset_path(id: &AssetId) -> Option<PathBuf> {
    if let Some(ref logical_path) = id.logical_path {
        return Some(PathBuf::from("assets").join(logical_path));
    }

    let id_str = &id.id;
    if let Some(hyphen_pos) = id_str.find('-') {
        let category = &id_str[..hyphen_pos];
        let name = &id_str[hyphen_pos + 1..];
        let dir = match category {
            "mesh" => "meshes",
            "material" => "materials",
            "texture" => "textures",
            "shader" => "shaders",
            "scene" => "scenes",
            "prefab" => "prefabs",
            "animation" => "animations",
            "audio" => "audio",
            "font" => "fonts",
            "logic" => "logic",
            "pipeline" => "pipelines",
            "navmesh" => "navmeshes",
            "script" => "scripts",
            "skeleton" => "skeletons",
            other => other,
        };
        Some(
            PathBuf::from("assets")
                .join(dir)
                .join(format!("{name}.asset")),
        )
    } else {
        Some(PathBuf::from("assets").join(format!("{}.asset", id_str)))
    }
}
