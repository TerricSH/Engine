use std::path::Path;

use crate::loader::AssetError;

/// Lowest-level recursive filesystem watcher used exclusively by the formal
/// reload coordinator. Raw notify events are intentionally not interpreted at
/// this layer; manifest resolution, debouncing and recooking belong to
/// `reload::WatchCoordinator` and `ReloadCoordinator`.
pub struct FileWatcher {
    /// Kept alive so the operating-system watch remains active.
    _watcher: notify::RecommendedWatcher,
    receiver: crossbeam_channel::Receiver<notify::Event>,
}

impl FileWatcher {
    /// Start watching `path` recursively.
    pub fn watch(path: &Path) -> Result<Self, AssetError> {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut watcher =
            notify::recommended_watcher(move |event: Result<notify::Event, notify::Error>| {
                if let Ok(event) = event {
                    let _ = tx.send(event);
                }
            })
            .map_err(|error| AssetError::WatcherFailed(error.to_string()))?;

        use notify::Watcher;
        watcher
            .watch(path, notify::RecursiveMode::Recursive)
            .map_err(|error| AssetError::WatcherFailed(error.to_string()))?;

        Ok(Self {
            _watcher: watcher,
            receiver: rx,
        })
    }

    /// Raw event receiver consumed by the single formal watch coordinator.
    pub(crate) fn event_receiver(&self) -> &crossbeam_channel::Receiver<notify::Event> {
        &self.receiver
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_rejects_a_missing_directory() {
        let missing = std::env::temp_dir().join(format!(
            "engine_asset_missing_watch_{}_{}",
            std::process::id(),
            line!()
        ));
        assert!(FileWatcher::watch(&missing).is_err());
    }
}
