use std::path::Path;

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
