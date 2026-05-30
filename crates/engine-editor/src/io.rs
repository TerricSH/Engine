use std::fs;
use std::io;
use std::path::Path;

use engine_scene::Scene;
use ron;

use crate::EditorError;

/// Default subdirectory under which scene files are stored.
pub const SCENES_DIR: &str = "assets/scenes";

/// Default file extension for scene files.
pub const SCENE_EXT: &str = "scene.ron";

/// Construct the default save path for a scene: `assets/scenes/{scene_id}.scene.ron`.
pub fn default_scene_path(scene: &Scene) -> String {
    format!("{}/{}.{}", SCENES_DIR, scene.scene_id, SCENE_EXT)
}

/// Save `scene` to `path` using the RON (Rusty Object Notation) format.
///
/// Creates parent directories if they do not exist.
pub fn save_scene(scene: &Scene, path: &Path) -> Result<(), EditorError> {
    // Ensure the parent directory exists.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            EditorError::IoFailed(format!(
                "failed to create directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    let pretty = ron::ser::PrettyConfig::default();

    let data = ron::ser::to_string_pretty(scene, pretty)
        .map_err(|e| EditorError::IoFailed(format!("serialization error: {e}")))?;

    fs::write(path, &data)
        .map_err(|e| EditorError::IoFailed(format!("failed to write {}: {e}", path.display())))?;

    Ok(())
}

/// Load a [`Scene`] from a RON file at `path`.
pub fn load_scene(path: &Path) -> Result<Scene, EditorError> {
    let data = fs::read_to_string(path).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            EditorError::SceneNotFound
        } else {
            EditorError::IoFailed(format!("failed to read {}: {e}", path.display()))
        }
    })?;

    let scene: Scene = ron::from_str(&data)
        .map_err(|e| EditorError::IoFailed(format!("deserialization error: {e}")))?;

    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::sample_scene;
    use std::path::PathBuf;

    #[test]
    fn save_load_roundtrip() {
        let dir = std::env::temp_dir().join("engine-editor-test-io");
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("roundtrip.scene.ron");

        let scene = sample_scene();
        save_scene(&scene, &path).expect("save should succeed");

        let loaded = load_scene(&path).expect("load should succeed");
        assert_eq!(scene.scene_id, loaded.scene_id);
        assert_eq!(scene.name, loaded.name);
        assert_eq!(scene.entities.len(), loaded.entities.len());

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_nonexistent_returns_scene_not_found() {
        let path = PathBuf::from("/nonexistent/path/scene.scene.ron");
        match load_scene(&path) {
            Err(EditorError::SceneNotFound) => {} // expected
            other => panic!("expected SceneNotFound, got {:?}", other),
        }
    }

    #[test]
    fn default_scene_path_format() {
        let scene = sample_scene();
        let path = default_scene_path(&scene);
        assert!(path.contains(&scene.scene_id));
        assert!(path.ends_with(".scene.ron"));
    }
}
