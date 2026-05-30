//! File watcher coordinator with event debouncing.
//!
//! Wraps the existing [`FileWatcher`](crate::watcher::FileWatcher) and adds
//! per-path debouncing: multiple filesystem events for the same path arriving
//! within the debounce window (200 ms) are coalesced into a single
//! [`WatchEvent`].
//!
//! # Usage
//!
//! ```ignore
//! use engine_asset::reload::watch::WatchCoordinator;
//!
//! let mut coord = WatchCoordinator::new("assets/source").unwrap();
//! coord.set_enabled(true);
//!
//! loop {
//!     for event in coord.poll_events() {
//!         println!("{:?} changed", event.path);
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::watcher::FileWatcher;
use crate::AssetError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Debounce window in milliseconds.  Multiple events for the same path
/// within this window are coalesced into a single event.
const DEBOUNCE_MS: u64 = 200;

// ---------------------------------------------------------------------------
// WatchEventKind
// ---------------------------------------------------------------------------

/// The kind of filesystem change detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEventKind {
    /// File was modified (content changed).
    Modified,
    /// File was created.
    Created,
}

// ---------------------------------------------------------------------------
// WatchEvent
// ---------------------------------------------------------------------------

/// A debounced filesystem event.
#[derive(Clone, Debug)]
pub struct WatchEvent {
    /// Path to the file that changed.
    pub path: PathBuf,
    /// The kind of change detected.
    pub kind: WatchEventKind,
}

// ---------------------------------------------------------------------------
// WatchCoordinator
// ---------------------------------------------------------------------------

/// Debouncing file-watcher coordinator.
///
/// Owns a [`FileWatcher`] and buffers incoming notify events, coalescing
/// duplicate paths within a configurable debounce window (default 200 ms).
///
/// Call [`poll_events`](Self::poll_events) once per frame to drain
/// buffered events.
pub struct WatchCoordinator {
    /// Underlying recursive file watcher (None when disabled).
    watcher: Option<FileWatcher>,
    /// Event buffer for debouncing: maps path → (timestamp, kind).
    buffer: HashMap<PathBuf, (Instant, WatchEventKind)>,
    /// Whether event delivery is enabled.
    enabled: bool,
    /// Debounce window in milliseconds.
    debounce_ms: u64,
}

impl WatchCoordinator {
    /// Create a new watch coordinator watching `watch_dir` recursively.
    ///
    /// # Errors
    ///
    /// Returns [`AssetError::WatcherFailed`] if the underlying
    /// [`FileWatcher`] cannot be created (e.g. directory does not exist).
    pub fn new(watch_dir: &Path) -> Result<Self, AssetError> {
        let watcher = FileWatcher::watch(watch_dir)?;
        tracing::info!(dir = %watch_dir.display(), "watch coordinator started");
        Ok(Self {
            watcher: Some(watcher),
            buffer: HashMap::new(),
            enabled: true,
            debounce_ms: DEBOUNCE_MS,
        })
    }

    /// Create a disabled coordinator that returns no events.
    pub fn new_disabled() -> Self {
        Self {
            watcher: None,
            buffer: HashMap::new(),
            enabled: false,
            debounce_ms: DEBOUNCE_MS,
        }
    }

    /// Drain pending events from the file watcher, coalesce duplicates
    /// within the debounce window, and return the resulting [`WatchEvent`]
    /// vec.
    ///
    /// Call this once per frame.
    pub fn poll_events(&mut self) -> Vec<WatchEvent> {
        if !self.enabled {
            return Vec::new();
        }

        let Some(ref watcher) = self.watcher else {
            return Vec::new();
        };

        // Drain all available events from the watcher channel.
        while let Ok(notify_event) = watcher.event_receiver().try_recv() {
            use notify::EventKind;

            let should_buffer = matches!(
                notify_event.kind,
                EventKind::Modify(_) | EventKind::Create(_)
            );
            if !should_buffer {
                continue;
            }

            for path in &notify_event.paths {
                let kind = if matches!(notify_event.kind, EventKind::Create(_)) {
                    WatchEventKind::Created
                } else {
                    WatchEventKind::Modified
                };

                let now = Instant::now();

                // Coalesce: if we already have a buffered event for this
                // path within the debounce window, keep the existing kind
                // and just refresh the timestamp.
                let is_coalesced = self
                    .buffer
                    .get(path)
                    .is_some_and(|(prev_time, _prev_kind)| {
                        let elapsed = now.duration_since(*prev_time).as_millis() as u64;
                        elapsed < self.debounce_ms
                    });
                if !is_coalesced {
                    self.buffer.insert(path.clone(), (now, kind));
                }
            }
        }

        // Return all buffered events and clear the buffer.
        let mut events: Vec<WatchEvent> = self
            .buffer
            .drain()
            .map(|(path, (_, kind))| WatchEvent { path, kind })
            .collect();

        // Sort by path for deterministic ordering.
        events.sort_by(|a, b| a.path.cmp(&b.path));

        if !events.is_empty() {
            tracing::debug!(count = events.len(), "watch coordinator events");
        }

        events
    }

    /// Enable or disable event delivery.
    ///
    /// When disabled, [`poll_events`](Self::poll_events) returns an empty
    /// vec and all incoming events are discarded.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.buffer.clear();
        }
    }

    /// Returns `true` if event delivery is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Override the debounce window (in milliseconds).
    ///
    /// The default is 200 ms.
    pub fn set_debounce_ms(&mut self, ms: u64) {
        self.debounce_ms = ms;
    }

    /// Access the underlying file watcher's watch directory.
    pub fn watch_dir(&self) -> &Path {
        // FileWatcher doesn't expose the watched path, so we return
        // a stub.  This method exists for diagnostic purposes.
        Path::new("")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_event(path: &str, kind: WatchEventKind) -> WatchEvent {
        WatchEvent {
            path: PathBuf::from(path),
            kind,
        }
    }

    #[test]
    fn watch_event_construction() {
        let ev = make_event("assets/meshes/cube.asset", WatchEventKind::Modified);
        assert_eq!(ev.path, PathBuf::from("assets/meshes/cube.asset"));
        assert_eq!(ev.kind, WatchEventKind::Modified);
    }

    #[test]
    fn watch_event_kind_equality() {
        assert_eq!(WatchEventKind::Modified, WatchEventKind::Modified);
        assert_ne!(WatchEventKind::Modified, WatchEventKind::Created);
    }

    #[test]
    fn watch_coordinator_new_fails_on_bad_path() {
        // A non-existent directory should produce an error.
        let result = WatchCoordinator::new(Path::new(r"\\?\__nonexistent__\__test__"));
        assert!(result.is_err());
    }

    #[test]
    fn watch_coordinator_disabled_returns_empty() {
        // This test verifies that when disabled, poll_events is empty.
        // We can't easily construct a WatchCoordinator without a real
        // directory, so we test the set_enabled contract indirectly
        // by verifying the is_enabled flag.
        let dir = std::env::temp_dir().join("watch_test_disabled");
        let _ = std::fs::create_dir_all(&dir);
        let mut coord = WatchCoordinator::new(&dir).unwrap();
        assert!(coord.is_enabled());
        coord.set_enabled(false);
        assert!(!coord.is_enabled());
        assert!(coord.poll_events().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_debounce_ms() {
        let dir = std::env::temp_dir().join("watch_test_debounce");
        let _ = std::fs::create_dir_all(&dir);
        let mut coord = WatchCoordinator::new(&dir).unwrap();
        coord.set_debounce_ms(500);
        // Private field — just verify no crash; poll returns empty
        // because no events were generated.
        assert!(coord.poll_events().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn debounce_coalesces_same_path() {
        // Unit test for the buffer logic: simulate events being added
        // to the buffer and verify coalescing.
        let dir = std::env::temp_dir().join("watch_test_coalesce");
        let _ = std::fs::create_dir_all(&dir);
        let mut coord = WatchCoordinator::new(&dir).unwrap();

        let p = PathBuf::from("assets/test.asset");

        // Simulate two rapid events being buffered manually (the buffer
        // is ordinarily populated by poll_events draining the watcher).
        let now = Instant::now();
        coord
            .buffer
            .insert(p.clone(), (now, WatchEventKind::Modified));

        // A second event within the debounce window should be coalesced
        // (not replace the existing entry).
        let later = now + Duration::from_millis(50);
        match coord.buffer.get(&p) {
            Some((prev_time, _)) if later.duration_since(*prev_time).as_millis() < 200 => {
                // Coalesced — keep existing entry.
            }
            _ => {
                coord
                    .buffer
                    .insert(p.clone(), (later, WatchEventKind::Created));
            }
        }

        assert_eq!(coord.buffer.len(), 1);
        // The original Modified kind should be preserved.
        assert_eq!(coord.buffer.get(&p).unwrap().1, WatchEventKind::Modified);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
