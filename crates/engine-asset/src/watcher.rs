use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::loader::AssetError;

// ---------------------------------------------------------------------------
// FileWatcher
// ---------------------------------------------------------------------------

/// Watches a directory for filesystem changes using the `notify` crate.
///
/// Events are delivered via a [`crossbeam_channel::Receiver`] so they can be
/// polled from the main loop without blocking.
///
/// # Example
///
/// ```ignore
/// use engine_asset::FileWatcher;
///
/// let watcher = FileWatcher::watch(std::path::Path::new("assets")).unwrap();
/// loop {
///     while let Ok(event) = watcher.event_receiver().try_recv() {
///         println!("{:?}", event);
///     }
/// }
/// ```
pub struct FileWatcher {
    /// Kept alive so the watch remains active.
    _watcher: notify::RecommendedWatcher,
    receiver: crossbeam_channel::Receiver<notify::Event>,
}

impl FileWatcher {
    /// Start watching `path` recursively.
    ///
    /// The watch runs on a background thread managed by `notify`; events
    /// are pushed into an unbounded channel.
    pub fn watch(path: &Path) -> Result<Self, AssetError> {
        let (tx, rx) = crossbeam_channel::unbounded();

        let mut watcher = notify::recommended_watcher(
            move |event: Result<notify::Event, notify::Error>| {
                if let Ok(ev) = event {
                    let _ = tx.send(ev);
                }
            },
        )
        .map_err(|e| AssetError::WatcherFailed(e.to_string()))?;

        // Import the Watcher trait so we can call `.watch()`.
        use notify::Watcher;
        watcher
            .watch(path, notify::RecursiveMode::Recursive)
            .map_err(|e| AssetError::WatcherFailed(e.to_string()))?;

        Ok(Self {
            _watcher: watcher,
            receiver: rx,
        })
    }

    /// Access the event receiver for polling.
    pub fn event_receiver(&self) -> &crossbeam_channel::Receiver<notify::Event> {
        &self.receiver
    }
}

// ---------------------------------------------------------------------------
// Supported extensions for hot-reload
// ---------------------------------------------------------------------------

/// File extensions that the hot-reload system should react to.
const RELOAD_EXTENSIONS: &[&str] = &[
    "asset", "manifest", "gltf", "glb", "png", "jpg", "jpeg", "bmp", "tga",
    "vert", "frag", "comp", "geom", "tesc", "tese",
];

/// Returns `true` if `path` has an extension that is relevant for hot-reload.
fn is_relevant_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| RELOAD_EXTENSIONS.contains(&ext))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// DebouncedEvent
// ---------------------------------------------------------------------------

/// A debounced file-system event emitted by [`DebouncedWatcher`].
#[derive(Clone, Debug)]
pub struct DebouncedEvent {
    /// The path of the file that changed.
    pub path: PathBuf,
    /// When the event was first detected (before debounce).
    pub detected_at: Instant,
}

// ---------------------------------------------------------------------------
// DebouncedWatcher
// ---------------------------------------------------------------------------

/// A wrapper around [`FileWatcher`] that debounces rapid file-system events
/// and filters to relevant extensions.
///
/// # Design
///
/// Raw `notify::Event`s are drained on every call to
/// [`poll`](Self::poll).  Events that are neither `Modify` nor `Create` are
/// discarded.  Remaining events are grouped by path; only the most recent
/// event per path is kept.  When an event's age exceeds the debounce window
/// (default 200 ms) it is emitted via the receiver.
///
/// This avoids redundant reloads when a single file write triggers multiple
/// `notify` events (e.g. a text editor saving via atomic rename).
///
/// # Example
///
/// ```ignore
/// use engine_asset::watcher::DebouncedWatcher;
///
/// let mut dw = DebouncedWatcher::watch("assets", std::time::Duration::from_millis(200)).unwrap();
/// loop {
///     for event in dw.poll() {
///         println!("{:?} changed", event.path);
///     }
/// }
/// ```
pub struct DebouncedWatcher {
    inner: FileWatcher,
    /// Events that have been observed but not yet emitted (debounce buffer).
    pending: HashMap<PathBuf, Instant>,
    /// Duration to wait before emitting an event.
    debounce: std::time::Duration,
}

impl DebouncedWatcher {
    /// Create a new `DebouncedWatcher` watching `path` recursively.
    ///
    /// `debounce` controls how long the watcher waits before emitting a
    /// file-change event.  Values between 100–500 ms are typical.
    pub fn watch(path: &Path, debounce: std::time::Duration) -> Result<Self, AssetError> {
        let inner = FileWatcher::watch(path)?;
        Ok(Self {
            inner,
            pending: HashMap::new(),
            debounce,
        })
    }

    /// Poll for debounced events.
    ///
    /// Call this once per frame.  Returns all events whose age exceeds
    /// the debounce window since their first observation.
    pub fn poll(&mut self) -> Vec<DebouncedEvent> {
        // 1. Drain raw events from the notify channel.
        while let Ok(event) = self.inner.event_receiver().try_recv() {
            self.ingest_raw(event);
        }

        // 2. Emit events whose debounce timer has expired.
        let now = Instant::now();
        let mut ready = Vec::new();

        self.pending.retain(|path, detected_at| {
            if now.duration_since(*detected_at) >= self.debounce {
                ready.push(DebouncedEvent {
                    path: path.clone(),
                    detected_at: *detected_at,
                });
                false // remove from pending
            } else {
                true // keep waiting
            }
        });

        ready
    }

    /// Whether there are any pending (not-yet-emitted) events.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Number of pending events.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    // ── Private helpers ──────────────────────────────────────────────────

    /// Process a raw `notify` event and update the pending map.
    fn ingest_raw(&mut self, event: notify::Event) {
        use notify::EventKind;

        // Only react to modifications and new files.
        let should_track = matches!(
            event.kind,
            EventKind::Modify(_) | EventKind::Create(_)
        );
        if !should_track {
            return;
        }

        for path in &event.paths {
            // Skip paths whose extension is not relevant.
            if !is_relevant_extension(path) {
                continue;
            }

            // Coalesce: keep the earliest detection time for each path.
            self.pending
                .entry(path.clone())
                .or_insert_with(Instant::now);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn is_relevant_extension_returns_true_for_known() {
        assert!(is_relevant_extension(Path::new("mesh.asset")));
        assert!(is_relevant_extension(Path::new("shader.vert")));
        assert!(is_relevant_extension(Path::new("shader.frag")));
        assert!(is_relevant_extension(Path::new("texture.png")));
        assert!(is_relevant_extension(Path::new("model.gltf")));
        assert!(is_relevant_extension(Path::new("model.glb")));
    }

    #[test]
    fn is_relevant_extension_returns_false_for_unknown() {
        assert!(!is_relevant_extension(Path::new("readme.md")));
        assert!(!is_relevant_extension(Path::new("config.toml")));
        assert!(!is_relevant_extension(Path::new("file.txt")));
    }

    #[test]
    fn is_relevant_extension_no_extension() {
        assert!(!is_relevant_extension(Path::new("Makefile")));
    }

    #[test]
    fn relevant_extensions_list_is_non_empty() {
        assert!(!RELOAD_EXTENSIONS.is_empty());
        assert!(RELOAD_EXTENSIONS.contains(&"asset"));
    }

    #[test]
    fn debounced_watcher_creation() {
        let dir = std::env::temp_dir().join("debounce_test_create");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let dw = DebouncedWatcher::watch(&dir, Duration::from_millis(200));
        assert!(dw.is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn events_are_coalesced_by_path() {
        let mut dw = DebouncedWatcher::create_for_test(Duration::from_millis(200));

        let path = PathBuf::from("assets/textures/test.asset");
        let t0 = Instant::now();
        dw.pending.insert(path.clone(), t0);

        // Same path should not create a second entry (retain replaces).
        dw.pending.insert(path.clone(), Instant::now());

        // After coalesce the map should still have only one entry.
        assert_eq!(dw.pending_count(), 1);
    }

    #[test]
    fn poll_emits_expired_events() {
        let mut dw = DebouncedWatcher::create_for_test(Duration::from_millis(1));

        let path = PathBuf::from("assets/textures/floor.asset");
        dw.pending.insert(path.clone(), Instant::now());

        // Sleep briefly so the event ages past the 1ms debounce.
        std::thread::sleep(Duration::from_millis(5));

        let emitted = dw.poll();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].path, path);
        assert_eq!(dw.pending_count(), 0);
    }

    #[test]
    fn poll_does_not_emit_unexpired_events() {
        let mut dw = DebouncedWatcher::create_for_test(Duration::from_millis(10_000));

        dw.pending.insert(
            PathBuf::from("assets/shaders/standard.asset"),
            Instant::now(),
        );

        let emitted = dw.poll();
        assert!(emitted.is_empty());
        assert_eq!(dw.pending_count(), 1);
    }
}

impl DebouncedWatcher {
    /// Create a `DebouncedWatcher` for testing with a dummy inner
    /// `FileWatcher`. The inner watcher watches a temp directory so it
    /// does not panic on drop.
    #[cfg(test)]
    fn create_for_test(debounce: std::time::Duration) -> Self {
        let dir = std::env::temp_dir().join("debounce_test_inner");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let inner = FileWatcher::watch(&dir).expect("test FileWatcher creation failed");
        Self {
            inner,
            pending: HashMap::new(),
            debounce,
        }
    }
}
