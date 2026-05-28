//! Asset registry and loading system for the engine.
//!
//! Provides runtime asset management including registration of typed loaders,
//! caching, and filesystem change notification via `notify`.
//!
//! # Architecture
//!
//! - [`AssetRegistry`] – central cache and loader dispatch.
//! - [`AssetLoader`] trait – decodes raw bytes into typed values.
//! - [`RawLoader`] – pass-through that stores [`Vec<u8>`].
//! - [`BincodeLoader<T>`] – bincode-based deserialization for any `T:
//!   DeserializeOwned`.
//! - [`AssetHandle<T>`] – [`Arc`]-backed shared handle to loaded data.
//! - [`FileWatcher`] – directory watch via `notify`, events delivered over
//!   a [`crossbeam_channel`] receiver.

pub mod cook;
pub mod mesh;
mod registry;
mod loader;
mod watcher;
mod path;
pub mod hot_reload;

pub use registry::{AssetRegistry, AssetState, AssetInfo};
pub use loader::{AssetLoader, AssetHandle, BincodeLoader, RawLoader, AssetError, CachedEntry};
pub use watcher::FileWatcher;
pub use path::asset_path;
pub use hot_reload::HotReload;

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::AssetId;

    // ── AssetHandle tests ────────────────────────────────────────────────

    #[test]
    fn asset_handle_new_creates_handle() {
        let id = AssetId::new("mesh-cube");
        let handle = AssetHandle::new(id.clone(), 42u32);
        assert_eq!(*handle.id(), id);
        assert_eq!(*handle.get(), 42);
    }

    #[test]
    fn asset_handle_get_returns_inner() {
        let handle = AssetHandle::new(AssetId::new("data"), "hello".to_string());
        assert_eq!(handle.get(), "hello");
    }

    #[test]
    fn asset_handle_id_returns_id() {
        let id = AssetId::with_path("id", "path/file.txt");
        let handle = AssetHandle::new(id.clone(), ());
        assert_eq!(*handle.id(), id);
    }

    #[test]
    fn asset_handle_clone() {
        let handle = AssetHandle::new(AssetId::new("a"), vec![1, 2, 3]);
        let cloned = handle.clone();
        assert_eq!(*handle.get(), *cloned.get());
        assert_eq!(*handle.id(), *cloned.id());
    }

    // ── AssetError display tests ──────────────────────────────────────────

    #[test]
    fn asset_error_not_found_display() {
        let err = AssetError::NotFound(AssetId::new("missing"));
        assert_eq!(err.to_string(), "asset not found: AssetId { id: \"missing\", logical_path: None }");
    }

    #[test]
    fn asset_error_load_failed_display() {
        let err = AssetError::LoadFailed {
            path: "assets/test.asset".to_string(),
            detail: "file not found".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "failed to load asset at assets/test.asset: file not found"
        );
    }

    #[test]
    fn asset_error_unsupported_format_display() {
        let err = AssetError::UnsupportedFormat;
        assert_eq!(err.to_string(), "unsupported asset format");
    }

    #[test]
    fn asset_error_type_mismatch_display() {
        let err = AssetError::TypeMismatch;
        assert_eq!(err.to_string(), "asset type mismatch");
    }

    #[test]
    fn asset_error_watcher_failed_display() {
        let err = AssetError::WatcherFailed("permission denied".to_string());
        assert_eq!(err.to_string(), "file watcher error: permission denied");
    }

    // ── asset_path tests ─────────────────────────────────────────────────

    fn ap(path: &str) -> Option<std::path::PathBuf> {
        // Build the expected path using the same method as asset_path to handle platform separators
        let parts: Vec<&str> = path.split('/').collect();
        let mut buf = std::path::PathBuf::new();
        for part in parts {
            buf = buf.join(part);
        }
        Some(buf)
    }

    #[test]
    fn asset_path_with_logical_path() {
        let id = AssetId::with_path("anything", "custom/thing.bin");
        let path = asset_path(&id);
        assert_eq!(path, ap("assets/custom/thing.bin"));
    }

    #[test]
    fn asset_path_mesh_category() {
        let id = AssetId::new("mesh-cube");
        let path = asset_path(&id);
        assert_eq!(path, ap("assets/meshes/cube.asset"));
    }

    #[test]
    fn asset_path_material_category() {
        let id = AssetId::new("material-default");
        let path = asset_path(&id);
        assert_eq!(path, ap("assets/materials/default.asset"));
    }

    #[test]
    fn asset_path_texture_category() {
        let id = AssetId::new("texture-floor");
        let path = asset_path(&id);
        assert_eq!(path, ap("assets/textures/floor.asset"));
    }

    #[test]
    fn asset_path_shader_category() {
        let id = AssetId::new("shader-standard");
        let path = asset_path(&id);
        assert_eq!(path, ap("assets/shaders/standard.asset"));
    }

    #[test]
    fn asset_path_scene_category() {
        let id = AssetId::new("scene-gate04");
        let path = asset_path(&id);
        assert_eq!(path, ap("assets/scenes/gate04.asset"));
    }

    #[test]
    fn asset_path_prefab_category() {
        let id = AssetId::new("prefab-enemy");
        let path = asset_path(&id);
        assert_eq!(path, ap("assets/prefabs/enemy.asset"));
    }

    #[test]
    fn asset_path_unknown_category() {
        let id = AssetId::new("custom-data");
        let path = asset_path(&id);
        assert_eq!(path, ap("assets/custom/data.asset"));
    }

    #[test]
    fn asset_path_no_hyphen() {
        let id = AssetId::new("simpleid");
        let path = asset_path(&id);
        assert_eq!(path, ap("assets/simpleid.asset"));
    }

    // ── RawLoader tests ──────────────────────────────────────────────────

    #[test]
    fn raw_loader_extensions() {
        let loader = RawLoader;
        assert_eq!(loader.extensions(), &["asset", "bin"]);
    }

    #[test]
    fn raw_loader_load_returns_bytes() {
        let loader = RawLoader;
        let data = vec![0x01, 0x02, 0x03];
        let id = AssetId::new("test");
        let result = loader.load(&id, &data).unwrap();
        let bytes = result.downcast::<Vec<u8>>().unwrap();
        assert_eq!(*bytes, vec![0x01, 0x02, 0x03]);
    }

    #[test]
    fn raw_loader_load_empty_bytes() {
        let loader = RawLoader;
        let id = AssetId::new("empty");
        let result = loader.load(&id, &[]).unwrap();
        let bytes = result.downcast::<Vec<u8>>().unwrap();
        assert!(bytes.is_empty());
    }

    // ── AssetState tests ─────────────────────────────────────────────────

    #[test]
    fn asset_state_variants() {
        assert_eq!(AssetState::Loading, AssetState::Loading);
        assert_eq!(AssetState::Ready, AssetState::Ready);
        assert_ne!(AssetState::Loading, AssetState::Ready);
    }

    #[test]
    fn asset_state_failed_contains_message() {
        let failed = AssetState::Failed("IO error".to_string());
        assert_eq!(failed, AssetState::Failed("IO error".to_string()));
        assert_ne!(failed, AssetState::Failed("other".to_string()));
    }

    #[test]
    fn asset_state_debug() {
        assert_eq!(format!("{:?}", AssetState::Loading), "Loading");
        assert_eq!(format!("{:?}", AssetState::Ready), "Ready");
        assert!(format!("{:?}", AssetState::Failed("err".to_string())).contains("Failed"));
    }

    // ── BincodeLoader tests ──────────────────────────────────────────────

    #[test]
    fn bincode_loader_extensions() {
        let loader = BincodeLoader::<i32>::new(vec!["custom"]);
        assert_eq!(loader.extensions(), &["custom"]);
    }
}
