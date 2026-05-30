//! Hot-reload: connects [`FileWatcher`](crate::watcher::FileWatcher) events
//! to [`AssetRegistry::reload`](crate::registry::AssetRegistry::reload).
//!
//! # Usage
//!
//! ```ignore
//! use engine_asset::{AssetRegistry, HotReload};
//! use std::path::Path;
//!
//! let mut registry = AssetRegistry::new();
//! let mut hot = HotReload::watch(Path::new("assets"), &mut registry).unwrap();
//!
// loop {
//!     hot.poll(); // call each frame
//! }
//! ```

use std::path::Path;

use engine_serialize::AssetId;
use notify::Event as NotifyEvent;

use crate::registry::AssetRegistry;
use crate::watcher::FileWatcher;
use crate::AssetError;

// ---------------------------------------------------------------------------
// HotReload
// ---------------------------------------------------------------------------

/// Poll-based hot-reload coordinator.
///
/// Owns a [`FileWatcher`] and a mutable reference to an [`AssetRegistry`].
/// Call [`poll`](Self::poll) each frame to drain file-change events and
/// re-load affected assets from disk.
///
/// # Limitations
///
/// Only assets whose [`AssetId`] follows the standard convention
/// (`"category-name"` → `assets/{category_plural}/{name}.asset`) can be
/// mapped back from a filesystem event.  Assets with an explicit
/// `logical_path` are not automatically reloaded.
pub struct HotReload<'a> {
    watcher: FileWatcher,
    registry: &'a mut AssetRegistry,

    /// Whether hot-reload is enabled.
    enabled: bool,

    /// Total number of files reloaded since construction.
    stats_reloaded: u64,
}

impl<'a> HotReload<'a> {
    /// Start watching `asset_dir` for changes.
    ///
    /// The returned [`HotReload`] borrows the registry; call
    /// [`poll`](Self::poll) periodically to process file-system events.
    pub fn watch(asset_dir: &Path, registry: &'a mut AssetRegistry) -> Result<Self, AssetError> {
        let watcher = FileWatcher::watch(asset_dir)?;
        tracing::info!(dir = %asset_dir.display(), "hot-reload watcher started");
        Ok(Self {
            watcher,
            registry,
            enabled: true,
            stats_reloaded: 0,
        })
    }

    /// Poll for file changes and re-load affected assets.
    ///
    /// Call this once per frame (or at a throttled rate).  All pending
    /// events are drained from the channel during a single poll.
    pub fn poll(&mut self) {
        if !self.enabled {
            return;
        }
        while let Ok(event) = self.watcher.event_receiver().try_recv() {
            self.handle_event(event);
        }
    }

    /// Enable or disable hot-reload.
    ///
    /// When disabled [`poll`](Self::poll) is a no-op.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Number of assets reloaded since creation.
    pub fn stats_reloaded(&self) -> u64 {
        self.stats_reloaded
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Process a single notify event, re-loading any assets whose paths
    /// match a known [`AssetId`] in the registry.
    fn handle_event(&mut self, event: NotifyEvent) {
        use notify::EventKind;

        // Only react to modifications and new files – ignore removals,
        // access notifications, and meta-events.
        let should_reload = matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_));
        if !should_reload {
            return;
        }

        for path in &event.paths {
            // Convert the changed file path to a conventional AssetId.
            let Some(asset_id) = path_to_asset_id(path) else {
                continue;
            };

            // Only reload assets that are already known to the registry.
            if !self.registry.contains(&asset_id) {
                continue;
            }

            match self.registry.reload(&asset_id) {
                Ok(()) => {
                    self.stats_reloaded += 1;
                    tracing::info!(
                        asset_id = %asset_id.id,
                        path = %path.display(),
                        "hot-reload triggered"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        asset_id = %asset_id.id,
                        error = %e,
                        "hot-reload failed"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Path → AssetId conversion
// ---------------------------------------------------------------------------

/// Convert a filesystem path to an [`AssetId`] using the same convention as
/// [`asset_path`](crate::path::asset_path).
///
/// # Convention
///
/// | Changed file                          | Resulting `AssetId`  |
/// |---------------------------------------|----------------------|
/// | `assets/meshes/cube.asset`            | `mesh-cube`          |
/// | `assets/textures/floor.asset`         | `texture-floor`      |
/// | `assets/shaders/standard.asset`       | `shader-standard`    |
/// | `assets/scenes/gate04.asset`          | `scene-gate04`       |
/// | `C:\game\assets\prefabs\enemy.asset`  | `prefab-enemy`       |
/// | `assets/cube.asset`                   | `cube`               |
///
/// The function normalises path separators so it works identically on
/// Windows and Unix.
///
/// Assets with a `logical_path` (i.e. a non-standard path layout) cannot
/// be mapped back and return [`None`].
pub(crate) fn path_to_asset_id(path: &Path) -> Option<AssetId> {
    let path_str = path.to_string_lossy();
    // Normalise to forward slashes so the same logic works on Windows.
    let normalized = path_str.replace('\\', "/");

    // Find the "assets/" directory component in the path.  This handles
    // both relative paths (assets/meshes/…) and absolute paths
    // (C:/game/assets/meshes/…).
    let after_assets = if let Some(idx) = normalized.find("/assets/") {
        &normalized[idx + "/assets/".len()..]
    } else if let Some(rest) = normalized.strip_prefix("assets/") {
        rest
    } else {
        return None;
    };

    let relative = Path::new(after_assets);
    let stem = relative.file_stem()?.to_str()?;
    let parent = relative.parent()?;

    if parent.as_os_str().is_empty() {
        // Single file at the root of the assets directory, e.g.
        // assets/cube.asset → AssetId { id: "cube" }
        Some(AssetId::new(stem.to_string()))
    } else {
        // Nested one level: assets/{dir}/{name}.asset
        let dir_name = parent.file_name()?.to_str()?;
        let category = singularize(dir_name);
        Some(AssetId::new(format!("{}-{}", category, stem)))
    }
}

/// Reverse of the pluralisation mapping in [`asset_path`](crate::path::asset_path).
///
/// Maps known plural directory names back to their singular category:
///
/// | Directory       | Category   |
/// |-----------------|------------|
/// | `meshes`        | `mesh`     |
/// | `materials`     | `material` |
/// | `textures`      | `texture`  |
/// | …              | …          |
/// | `audio`         | `audio`    | (no change)
/// | `*`             | `*`        | (pass-through)
fn singularize(s: &str) -> &str {
    match s {
        "meshes" => "mesh",
        "materials" => "material",
        "textures" => "texture",
        "shaders" => "shader",
        "scenes" => "scene",
        "prefabs" => "prefab",
        "animations" => "animation",
        "audio" => "audio",
        "fonts" => "font",
        "pipelines" => "pipeline",
        "navmeshes" => "navmesh",
        "scripts" => "script",
        "skeletons" => "skeleton",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // Helper: on Windows, build a path with backslashes to test cross-platform
    // handling; on Unix, use forward slashes.
    fn native_path(unix: &str) -> PathBuf {
        let mut buf = PathBuf::new();
        for part in unix.split('/') {
            buf = buf.join(part);
        }
        buf
    }

    #[test]
    fn standard_mesh_convention() {
        let id = path_to_asset_id(&native_path("assets/meshes/cube.asset"));
        assert_eq!(id, Some(AssetId::new("mesh-cube")));
    }

    #[test]
    fn standard_texture_convention() {
        let id = path_to_asset_id(&native_path("assets/textures/floor.asset"));
        assert_eq!(id, Some(AssetId::new("texture-floor")));
    }

    #[test]
    fn standard_shader_convention() {
        let id = path_to_asset_id(&native_path("assets/shaders/standard.asset"));
        assert_eq!(id, Some(AssetId::new("shader-standard")));
    }

    #[test]
    fn standard_scene_convention() {
        let id = path_to_asset_id(&native_path("assets/scenes/gate04.asset"));
        assert_eq!(id, Some(AssetId::new("scene-gate04")));
    }

    #[test]
    fn standard_prefab_convention() {
        let id = path_to_asset_id(&native_path("assets/prefabs/enemy.asset"));
        assert_eq!(id, Some(AssetId::new("prefab-enemy")));
    }

    #[test]
    fn standard_animation_convention() {
        let id = path_to_asset_id(&native_path("assets/animations/walk.asset"));
        assert_eq!(id, Some(AssetId::new("animation-walk")));
    }

    #[test]
    fn audio_no_plural() {
        let id = path_to_asset_id(&native_path("assets/audio/theme.asset"));
        assert_eq!(id, Some(AssetId::new("audio-theme")));
    }

    #[test]
    fn root_level_file() {
        let id = path_to_asset_id(&native_path("assets/simpleid.asset"));
        assert_eq!(id, Some(AssetId::new("simpleid")));
    }

    #[test]
    fn no_hyphen_id() {
        let id = path_to_asset_id(&native_path("assets/data.asset"));
        assert_eq!(id, Some(AssetId::new("data")));
    }

    #[test]
    fn unknown_category_passthrough() {
        let id = path_to_asset_id(&native_path("assets/custom/data.asset"));
        assert_eq!(id, Some(AssetId::new("custom-data")));
    }

    #[test]
    fn absolute_windows_path() {
        // Simulate the kind of absolute path notify delivers on Windows.
        let raw = "C:\\Projects\\Game\\assets\\meshes\\cube.asset";
        let id = path_to_asset_id(Path::new(raw));
        assert_eq!(id, Some(AssetId::new("mesh-cube")));
    }

    #[test]
    fn absolute_unix_path() {
        let raw = "/home/user/project/assets/meshes/cube.asset";
        let id = path_to_asset_id(Path::new(raw));
        assert_eq!(id, Some(AssetId::new("mesh-cube")));
    }

    #[test]
    fn non_asset_path_returns_none() {
        let id = path_to_asset_id(Path::new("config/settings.toml"));
        assert_eq!(id, None);
    }

    #[test]
    fn trailing_slash_directory_returns_none() {
        // A bare directory path (no filename) has no file stem.
        let id = path_to_asset_id(&native_path("assets/"));
        assert_eq!(id, None);
    }
}
