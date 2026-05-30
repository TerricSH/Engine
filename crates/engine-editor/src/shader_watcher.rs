//! File-system watcher for shader source directories.
//!
//! Watches configured shader source paths for changes and emits
//! [`ShaderChange`] events through a crossbeam channel.  Rapid saves
//! (within a 200 ms debounce window) are coalesced so that only one
//! event is sent per asset per window.
//!
//! # Usage
//!
//! ```ignore
//! let (watcher, rx) = ShaderWatcher::new()?;
//! watcher.watch("assets/shaders")?;
//!
//! // In the main/render loop:
//! while let Ok(change) = rx.try_recv() {
//!     println!("{} changed: recompile needed", change.asset_id);
//! }
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

// ============================================================================
// Public types
// ============================================================================

/// A detected shader-source change.
#[derive(Clone, Debug)]
pub struct ShaderChange {
    /// The path to the changed file.
    pub path: String,
    /// A derived asset identifier (e.g., `"shader-my-pass"`) based on the
    /// file stem.  This can be passed directly to
    /// [`GpuReloadCoordinator::queue_reload`].
    pub asset_id: String,
}

/// File-system watcher for shader sources.
///
/// Drop the watcher to stop watching (the background thread is shut down
/// automatically).
pub struct ShaderWatcher {
    _watcher: RecommendedWatcher,
    /// Receiver for debounced change events.
    pub receiver: crossbeam_channel::Receiver<ShaderChange>,
}

impl ShaderWatcher {
    /// Create a new watcher and return the control handle plus a channel
    /// receiver for [`ShaderChange`] events.
    ///
    /// The watcher is **not** watching any directories yet — call
    /// [`watch`](Self::watch) to start watching a path.
    pub fn new() -> Result<Self, String> {
        let (raw_tx, raw_rx) = crossbeam_channel::unbounded::<notify::Event>();
        let (debounce_tx, debounce_rx) = crossbeam_channel::unbounded::<ShaderChange>();

        // Spawn a debounce thread that coalesces rapid events per path.
        let debounce_tx_clone = debounce_tx.clone();
        std::thread::Builder::new()
            .name("shader-watcher-debounce".into())
            .spawn(move || {
                Self::debounce_loop(raw_rx, debounce_tx_clone);
            })
            .map_err(|e| format!("failed to spawn debounce thread: {e}"))?;

        // SAFETY: `notify`'s `RecommendedWatcher` expects a thread-safe
        // event handler.  `crossbeam_channel::Sender` is `Send + Sync`, so
        // this is safe.
        let watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = raw_tx.send(event);
                }
            },
            Config::default(),
        )
        .map_err(|e| format!("failed to create file watcher: {e}"))?;

        Ok(Self {
            _watcher: watcher,
            receiver: debounce_rx,
        })
    }

    /// Watch a directory recursively for shader-source changes.
    ///
    /// `path` should be the root of a directory tree containing `.vert`,
    /// `.frag`, `.comp`, `.glsl`, or `.hlsl` files.
    pub fn watch(&self, path: &Path) -> Result<(), String> {
        self._watcher
            .watch(path, RecursiveMode::Recursive)
            .map_err(|e| format!("failed to watch '{}': {e}", path.display()))
    }

    /// Internal debounce loop that runs on a background thread.
    ///
    /// Groups raw notify events by file path and only emits a
    /// [`ShaderChange`] once at least `DEBOUNCE_MS` have elapsed since
    /// the last event for that path.
    fn debounce_loop(
        raw_rx: crossbeam_channel::Receiver<notify::Event>,
        debounce_tx: crossbeam_channel::Sender<ShaderChange>,
    ) {
        const DEBOUNCE_MS: u64 = 200;

        let mut last_event: HashMap<String, Instant> = HashMap::new();

        while let Ok(event) = raw_rx.recv() {
            // Only care about content-change events (Modify, Create).
            let is_modify = matches!(
                event.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
            );
            if !is_modify {
                continue;
            }

            for path in &event.paths {
                let path_str = path.to_string_lossy().to_string();
                let now = Instant::now();

                // Debounce: skip if we sent an event for this path recently.
                if let Some(last) = last_event.get(&path_str) {
                    if now.duration_since(*last) < Duration::from_millis(DEBOUNCE_MS) {
                        last_event.insert(path_str, now);
                        continue;
                    }
                }

                last_event.insert(path_str.clone(), now);

                // Derive an asset ID from the file stem.
                let asset_id = derive_asset_id(path);
                let change = ShaderChange { path: path_str, asset_id };

                if debounce_tx.send(change).is_err() {
                    // Receiver dropped — stop the loop.
                    return;
                }
            }
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Derive a shader asset ID from a file path.
///
/// Convention: the file stem (e.g. `"my_pass"` from `"my_pass.vert"`) is
/// prefixed with `"shader-"` to produce `"shader-my_pass"`.
fn derive_asset_id(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    format!("shader-{stem}")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_asset_id_from_vert_file() {
        let p = Path::new("assets/shaders/my_pass.vert");
        assert_eq!(derive_asset_id(p), "shader-my_pass");
    }

    #[test]
    fn derive_asset_id_from_frag_file() {
        let p = Path::new("assets/shaders/my_pass.frag");
        assert_eq!(derive_asset_id(p), "shader-my_pass");
    }

    #[test]
    fn derive_asset_id_with_underscores() {
        let p = Path::new("assets/shaders/opaque_pbr_forward.vert");
        assert_eq!(derive_asset_id(p), "shader-opaque_pbr_forward");
    }

    #[test]
    fn derive_asset_id_unknown_stem() {
        let p = Path::new(".hidden");
        assert_eq!(derive_asset_id(p), "shader-unknown");
    }
}
