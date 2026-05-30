//! Reload state machine for the hot-reload pipeline.
//!
//! Tracks the lifecycle of each asset through the reload process:
//!
//! ```text
//! Detected → Recooking → Cooked → Queued → Applying → Applied
//!                ↓ (on failure)
//!             Failed(error)
//! ```
//!
//! Reload state transitions are surfaced as [`Diagnostic`] values via
//! [`ReloadTracker::to_diagnostics`] so they can be displayed in editor
//! tools and diagnostic views.

use std::collections::BTreeMap;

use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity};

// ---------------------------------------------------------------------------
// ReloadState
// ---------------------------------------------------------------------------

/// The current state of an asset in the incremental reload pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReloadState {
    /// A file-change event was detected for this asset.
    Detected,
    /// The asset is being recooked (cook pipeline in progress).
    Recooking,
    /// The asset was cooked successfully and validated.
    Cooked,
    /// The cooked asset is queued for application (waiting for a frame
    /// boundary or renderer sync point).
    Queued,
    /// The cooked asset is currently being applied to the runtime.
    Applying,
    /// The cooked asset was applied successfully.
    Applied,
    /// Cooking or application failed with an error message.  The last
    /// valid version of the asset is preserved.
    Failed(String),
}

impl std::fmt::Display for ReloadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReloadState::Detected => write!(f, "Detected"),
            ReloadState::Recooking => write!(f, "Recooking"),
            ReloadState::Cooked => write!(f, "Cooked"),
            ReloadState::Queued => write!(f, "Queued"),
            ReloadState::Applying => write!(f, "Applying"),
            ReloadState::Applied => write!(f, "Applied"),
            ReloadState::Failed(e) => write!(f, "Failed({e})"),
        }
    }
}

// ---------------------------------------------------------------------------
// ReloadInfo
// ---------------------------------------------------------------------------

/// Per-asset reload tracking information.
#[derive(Clone, Debug)]
pub struct ReloadInfo {
    /// Current state in the reload lifecycle.
    pub state: ReloadState,
    /// The unique asset identifier.
    pub asset_id: AssetId,
    /// Optional filesystem path to the source file.
    pub path: Option<String>,
    /// Timestamp of the most recent state transition.
    pub updated_at: std::time::Instant,
    /// Error message if the asset is in the [`ReloadState::Failed`] state.
    pub error_message: Option<String>,
}

impl ReloadInfo {
    fn new(asset_id: AssetId, path: Option<String>) -> Self {
        Self {
            state: ReloadState::Detected,
            asset_id,
            path,
            updated_at: std::time::Instant::now(),
            error_message: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ReloadTracker
// ---------------------------------------------------------------------------

/// Tracks the reload state of assets through the incremental hot-reload
/// pipeline.
///
/// Maintains a map of [`AssetId`] → [`ReloadInfo`] and provides methods to
/// transition states, query pending assets, and produce [`Diagnostic`]
/// values for display.
pub struct ReloadTracker {
    assets: BTreeMap<AssetId, ReloadInfo>,
}

impl ReloadTracker {
    /// Create a new empty tracker.
    pub fn new() -> Self {
        Self {
            assets: BTreeMap::new(),
        }
    }

    /// Register or transition an asset to a new state.
    ///
    /// If the asset is not yet tracked, a new [`ReloadInfo`] is created.
    /// The `updated_at` timestamp is refreshed on every transition.
    pub fn transition(&mut self, id: &AssetId, state: ReloadState) {
        let info = self
            .assets
            .entry(id.clone())
            .or_insert_with(|| ReloadInfo::new(id.clone(), None));
        info.state = state;
        info.updated_at = std::time::Instant::now();
        if let ReloadState::Failed(ref err) = &info.state {
            info.error_message = Some(err.clone());
        }
    }

    /// Register or transition an asset to a new state, also setting its
    /// filesystem path.
    pub fn transition_with_path(&mut self, id: &AssetId, state: ReloadState, path: Option<String>) {
        let info = self
            .assets
            .entry(id.clone())
            .or_insert_with(|| ReloadInfo::new(id.clone(), path.clone()));
        info.state = state;
        info.updated_at = std::time::Instant::now();
        if path.is_some() {
            info.path = path;
        }
        if let ReloadState::Failed(ref err) = &info.state {
            info.error_message = Some(err.clone());
        }
    }

    /// Look up the reload info for an asset.
    pub fn get(&self, id: &AssetId) -> Option<&ReloadInfo> {
        self.assets.get(id)
    }

    /// Iterate over all tracked reload info entries.
    pub fn all(&self) -> Vec<&ReloadInfo> {
        self.assets.values().collect()
    }

    /// Return the list of asset IDs that are in the [`ReloadState::Queued`]
    /// state and ready to be applied.
    pub fn pending_apply(&self) -> Vec<AssetId> {
        self.assets
            .values()
            .filter(|info| info.state == ReloadState::Queued)
            .map(|info| info.asset_id.clone())
            .collect()
    }

    /// Mark an asset as successfully applied (transition to [`Applied`]).
    pub fn mark_completed(&mut self, id: &AssetId) {
        self.transition(id, ReloadState::Applied);
    }

    /// Mark an asset as failed with the given error message.
    pub fn mark_failed(&mut self, id: &AssetId, error: String) {
        self.transition(id, ReloadState::Failed(error));
    }

    /// Remove an asset from the tracker (e.g. when it is unloaded).
    pub fn remove(&mut self, id: &AssetId) {
        self.assets.remove(id);
    }

    /// Number of tracked assets.
    pub fn len(&self) -> usize {
        self.assets.len()
    }

    /// Returns `true` if no assets are tracked.
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }

    /// Produce [`Diagnostic`] values summarising the current reload state
    /// of all tracked assets.
    ///
    /// Each tracked asset generates one diagnostic.  Assets in a terminal
    /// state ([`Applied`], [`Failed`]) are reported with appropriate
    /// severity:
    ///
    /// - [`Applied`] → `Info` (positive signal that reload succeeded)
    /// - [`Failed`]  → `Error` (reload failed, last valid state preserved)
    /// - All other states → `Warning` (in-progress)
    pub fn to_diagnostics(&self) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        for info in self.assets.values() {
            let (severity, code, message) = match &info.state {
                ReloadState::Applied => (
                    DiagnosticSeverity::Info,
                    "RELOAD_APPLIED",
                    format!("reload applied for asset {}", info.asset_id.id),
                ),
                ReloadState::Failed(err) => (
                    DiagnosticSeverity::Error,
                    "RELOAD_FAILED",
                    format!("reload failed for asset {}: {err}", info.asset_id.id),
                ),
                ReloadState::Detected => (
                    DiagnosticSeverity::Warning,
                    "RELOAD_DETECTED",
                    format!("reload detected for asset {}", info.asset_id.id),
                ),
                ReloadState::Recooking => (
                    DiagnosticSeverity::Warning,
                    "RELOAD_RECOOKING",
                    format!("recooking asset {}", info.asset_id.id),
                ),
                ReloadState::Cooked => (
                    DiagnosticSeverity::Warning,
                    "RELOAD_COOKED",
                    format!("asset {} cooked successfully", info.asset_id.id),
                ),
                ReloadState::Queued => (
                    DiagnosticSeverity::Warning,
                    "RELOAD_QUEUED",
                    format!("asset {} queued for apply", info.asset_id.id),
                ),
                ReloadState::Applying => (
                    DiagnosticSeverity::Warning,
                    "RELOAD_APPLYING",
                    format!("applying asset {}", info.asset_id.id),
                ),
            };

            let mut d = Diagnostic::new(code, severity, "reload", message);
            d.asset = Some(info.asset_id.clone());
            diags.push(d);
        }
        diags
    }
}

impl Default for ReloadTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> AssetId {
        AssetId::new(name)
    }

    #[test]
    fn tracker_new_is_empty() {
        let tracker = ReloadTracker::new();
        assert!(tracker.is_empty());
        assert_eq!(tracker.len(), 0);
    }

    #[test]
    fn tracker_transition_creates_entry() {
        let mut tracker = ReloadTracker::new();
        tracker.transition(&id("mesh-cube"), ReloadState::Detected);
        assert_eq!(tracker.len(), 1);
        let info = tracker.get(&id("mesh-cube")).unwrap();
        assert_eq!(info.state, ReloadState::Detected);
    }

    #[test]
    fn tracker_transition_updates_state() {
        let mut tracker = ReloadTracker::new();
        tracker.transition(&id("mesh-cube"), ReloadState::Detected);
        tracker.transition(&id("mesh-cube"), ReloadState::Recooking);
        tracker.transition(&id("mesh-cube"), ReloadState::Cooked);
        let info = tracker.get(&id("mesh-cube")).unwrap();
        assert_eq!(info.state, ReloadState::Cooked);
    }

    #[test]
    fn tracker_pending_apply() {
        let mut tracker = ReloadTracker::new();
        tracker.transition(&id("mesh-cube"), ReloadState::Queued);
        tracker.transition(&id("mat-default"), ReloadState::Queued);
        tracker.transition(&id("shader-std"), ReloadState::Cooked);

        let pending = tracker.pending_apply();
        assert_eq!(pending.len(), 2);
        assert!(pending.contains(&id("mesh-cube")));
        assert!(pending.contains(&id("mat-default")));
    }

    #[test]
    fn tracker_mark_completed() {
        let mut tracker = ReloadTracker::new();
        tracker.transition(&id("mesh-cube"), ReloadState::Queued);
        tracker.mark_completed(&id("mesh-cube"));
        assert_eq!(
            tracker.get(&id("mesh-cube")).unwrap().state,
            ReloadState::Applied
        );
    }

    #[test]
    fn tracker_mark_failed() {
        let mut tracker = ReloadTracker::new();
        tracker.mark_failed(&id("mesh-cube"), "compiler error".into());
        let info = tracker.get(&id("mesh-cube")).unwrap();
        assert_eq!(info.state, ReloadState::Failed("compiler error".into()));
        assert_eq!(info.error_message, Some("compiler error".into()));
    }

    #[test]
    fn tracker_remove() {
        let mut tracker = ReloadTracker::new();
        tracker.transition(&id("mesh-cube"), ReloadState::Applied);
        tracker.remove(&id("mesh-cube"));
        assert!(tracker.is_empty());
    }

    #[test]
    fn tracker_all() {
        let mut tracker = ReloadTracker::new();
        tracker.transition(&id("a"), ReloadState::Applied);
        tracker.transition(&id("b"), ReloadState::Failed("err".into()));
        assert_eq!(tracker.all().len(), 2);
    }

    #[test]
    fn tracker_to_diagnostics_reflects_states() {
        let mut tracker = ReloadTracker::new();
        tracker.transition(&id("good"), ReloadState::Applied);
        tracker.transition(&id("bad"), ReloadState::Failed("oops".into()));
        tracker.transition(&id("wip"), ReloadState::Recooking);

        let diags = tracker.to_diagnostics();
        assert_eq!(diags.len(), 3);

        let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
        assert!(codes.contains(&"RELOAD_APPLIED"));
        assert!(codes.contains(&"RELOAD_FAILED"));
        assert!(codes.contains(&"RELOAD_RECOOKING"));

        // Applied is Info, Failed is Error
        let info_count = diags
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Info)
            .count();
        let error_count = diags
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .count();
        assert_eq!(info_count, 1);
        assert_eq!(error_count, 1);
    }

    #[test]
    fn reload_state_display() {
        assert_eq!(ReloadState::Detected.to_string(), "Detected");
        assert_eq!(ReloadState::Recooking.to_string(), "Recooking");
        assert_eq!(ReloadState::Failed("err".into()).to_string(), "Failed(err)");
    }

    #[test]
    fn transition_with_path_sets_path() {
        let mut tracker = ReloadTracker::new();
        tracker.transition_with_path(
            &id("mesh-cube"),
            ReloadState::Detected,
            Some("assets/meshes/cube.asset".into()),
        );
        let info = tracker.get(&id("mesh-cube")).unwrap();
        assert_eq!(info.path, Some("assets/meshes/cube.asset".into()));
    }
}
