//! Filesystem watch coordination with quiet-period debouncing.
//!
//! A path is emitted only after it receives no new create or modify event for
//! the configured interval. Disabling the coordinator clears both debounced
//! and raw operating-system events.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::watcher::FileWatcher;
use crate::AssetError;

const DEBOUNCE_MS: u64 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEventKind {
    Modified,
    Created,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchEvent {
    pub path: PathBuf,
    pub kind: WatchEventKind,
}

/// Owns the one low-level watcher and turns raw events into debounced changes.
pub struct WatchCoordinator {
    watcher: Option<FileWatcher>,
    watch_dir: PathBuf,
    buffer: HashMap<PathBuf, (Instant, WatchEventKind)>,
    enabled: bool,
    debounce: Duration,
}

impl WatchCoordinator {
    pub fn new(watch_dir: &Path) -> Result<Self, AssetError> {
        let watcher = FileWatcher::watch(watch_dir)?;
        Ok(Self {
            watcher: Some(watcher),
            watch_dir: watch_dir.to_path_buf(),
            buffer: HashMap::new(),
            enabled: true,
            debounce: Duration::from_millis(DEBOUNCE_MS),
        })
    }

    pub fn new_disabled() -> Self {
        Self {
            watcher: None,
            watch_dir: PathBuf::new(),
            buffer: HashMap::new(),
            enabled: false,
            debounce: Duration::from_millis(DEBOUNCE_MS),
        }
    }

    /// Drain raw events and return paths whose quiet period has elapsed.
    pub fn poll_events(&mut self) -> Vec<WatchEvent> {
        let raw_events = self.drain_raw_events();
        if !self.enabled {
            self.buffer.clear();
            return Vec::new();
        }

        for notify_event in raw_events {
            use notify::EventKind;

            let kind = match notify_event.kind {
                EventKind::Create(_) => WatchEventKind::Created,
                EventKind::Modify(_) => WatchEventKind::Modified,
                _ => continue,
            };
            let observed_at = Instant::now();
            for path in notify_event.paths {
                self.buffer_event(path, kind, observed_at);
            }
        }

        self.drain_ready(Instant::now())
    }

    fn drain_raw_events(&self) -> Vec<notify::Event> {
        self.watcher
            .as_ref()
            .map(|watcher| watcher.event_receiver().try_iter().collect())
            .unwrap_or_default()
    }

    fn buffer_event(&mut self, path: PathBuf, kind: WatchEventKind, observed_at: Instant) {
        self.buffer
            .entry(path)
            .and_modify(|(last_observed, buffered_kind)| {
                *last_observed = observed_at;
                if kind == WatchEventKind::Created {
                    *buffered_kind = WatchEventKind::Created;
                }
            })
            .or_insert((observed_at, kind));
    }

    fn drain_ready(&mut self, now: Instant) -> Vec<WatchEvent> {
        let mut ready_paths = self
            .buffer
            .iter()
            .filter(|(_, (last_observed, _))| {
                now.saturating_duration_since(*last_observed) >= self.debounce
            })
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        ready_paths.sort();
        ready_paths
            .into_iter()
            .filter_map(|path| {
                self.buffer
                    .remove(&path)
                    .map(|(_, kind)| WatchEvent { path, kind })
            })
            .collect()
    }

    /// Enable or disable event delivery. Events accumulated while disabled
    /// are discarded before the coordinator can be re-enabled.
    pub fn set_enabled(&mut self, enabled: bool) {
        if !enabled || !self.enabled {
            self.buffer.clear();
            let _ = self.drain_raw_events();
        }
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_debounce_ms(&mut self, milliseconds: u64) {
        self.debounce = Duration::from_millis(milliseconds);
    }

    pub fn watch_dir(&self) -> &Path {
        &self.watch_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_directory_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        assert!(WatchCoordinator::new(&root.path().join("missing")).is_err());
    }

    #[test]
    fn disabling_clears_debounced_queue() {
        let root = tempfile::tempdir().unwrap();
        let mut coordinator = WatchCoordinator::new(root.path()).unwrap();
        coordinator.buffer_event(
            root.path().join("queued.asset"),
            WatchEventKind::Modified,
            Instant::now(),
        );
        coordinator.set_enabled(false);
        assert!(!coordinator.is_enabled());
        assert!(coordinator.poll_events().is_empty());
        assert!(coordinator.buffer.is_empty());
    }

    #[test]
    fn debounce_uses_quiet_period_and_preserves_create() {
        let root = tempfile::tempdir().unwrap();
        let mut coordinator = WatchCoordinator::new(root.path()).unwrap();
        coordinator.set_debounce_ms(200);
        let path = root.path().join("asset.bin");
        let first = Instant::now();
        coordinator.buffer_event(path.clone(), WatchEventKind::Modified, first);
        let second = first + Duration::from_millis(50);
        coordinator.buffer_event(path.clone(), WatchEventKind::Created, second);

        assert!(coordinator
            .drain_ready(second + Duration::from_millis(199))
            .is_empty());
        assert_eq!(
            coordinator.drain_ready(second + Duration::from_millis(200)),
            vec![WatchEvent {
                path,
                kind: WatchEventKind::Created,
            }]
        );
    }

    #[test]
    fn real_watch_directory_emits_after_quiet_period() {
        let root = tempfile::tempdir().unwrap();
        let mut coordinator = WatchCoordinator::new(root.path()).unwrap();
        coordinator.set_debounce_ms(25);
        let changed_path = root.path().join("changed.material.json");
        std::fs::write(&changed_path, b"first").unwrap();
        std::fs::write(&changed_path, b"second").unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut observed = Vec::new();
        while Instant::now() < deadline {
            observed.extend(coordinator.poll_events());
            if observed.iter().any(|event| event.path == changed_path) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            observed.iter().any(|event| event.path == changed_path),
            "the operating-system watcher did not report {}",
            changed_path.display()
        );
    }
}
