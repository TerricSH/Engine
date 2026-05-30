//! Sandbox-level diagnostics aggregator.
//!
//! [`SandboxDiagnostics`] consolidates diagnostics from the engine runtime's
//! [`DiagnosticsCollector`], asset reload coordinator, and script engine into
//! a single structured snapshot consumable by the editor diagnostics panel.

use engine_asset::ReloadCoordinator;
use engine_core::{FrameStatsSnapshot, RuntimeDiagnostics};
use engine_serialize::{Diagnostic, DiagnosticSeverity};

/// Aggregated sandbox diagnostics snapshot.
///
/// Collects and caches diagnostics from multiple sources each frame:
/// - Frame-level render statistics (draw calls, culling, GPU time)
/// - Scene validation warnings
/// - Asset reload status
/// - Script lifecycle errors
pub struct SandboxDiagnostics {
    /// Latest frame stats snapshot from the runtime collector.
    pub frame_snapshot: Option<FrameStatsSnapshot>,
    /// Scene validation diagnostics.
    pub scene_validation: Vec<Diagnostic>,
    /// Asset reload status diagnostics.
    pub reload_status: Vec<Diagnostic>,
    /// Script execution diagnostics.
    pub script_diagnostics: Vec<Diagnostic>,
    /// Asset-level diagnostics (e.g. load failures).
    pub asset_diagnostics: Vec<Diagnostic>,
}

impl SandboxDiagnostics {
    /// Create a new empty diagnostics aggregator.
    pub fn new() -> Self {
        Self {
            frame_snapshot: None,
            scene_validation: Vec::new(),
            reload_status: Vec::new(),
            script_diagnostics: Vec::new(),
            asset_diagnostics: Vec::new(),
        }
    }

    /// Refresh the snapshot from the runtime diagnostics and reload coordinator.
    ///
    /// Call this once per frame after rendering to capture the latest state.
    pub fn update(
        &mut self,
        runtime_diags: &RuntimeDiagnostics,
        reload_coordinator: &ReloadCoordinator,
    ) {
        let collector = &runtime_diags.collector;

        // Capture the most recent frame stats snapshot, if any.
        self.frame_snapshot = collector.frame_stats.last().cloned();

        // Copy diagnostics from each category.
        self.scene_validation = collector.scene_diagnostics.clone();
        self.script_diagnostics = collector.script_diagnostics.clone();
        self.asset_diagnostics = collector.asset_diagnostics.clone();

        // Reload diagnostics come from the coordinator's tracker and the collector.
        let mut reload = reload_coordinator.tracker().to_diagnostics();
        reload.extend(collector.reload_diagnostics.clone());
        self.reload_status = reload;
    }

    /// Return all diagnostics from all categories in a single flat list.
    ///
    /// The diagnostics are ordered: frame stats info, scene validation,
    /// reload status, script diagnostics, asset diagnostics.
    pub fn all_diagnostics(&self) -> Vec<Diagnostic> {
        let mut all = Vec::new();

        // Frame stats as info diagnostics (if a snapshot is available).
        if let Some(snap) = &self.frame_snapshot {
            all.push(
                Diagnostic::new(
                    "DIAG_DRAW_CALLS",
                    DiagnosticSeverity::Info,
                    "sandbox",
                    format!("draw calls: {}", snap.draw_calls),
                )
                .path("frame_stats.draw_calls"),
            );
            all.push(
                Diagnostic::new(
                    "DIAG_TRIANGLES",
                    DiagnosticSeverity::Info,
                    "sandbox",
                    format!("triangles: {}", snap.triangles),
                )
                .path("frame_stats.triangles"),
            );
            all.push(
                Diagnostic::new(
                    "DIAG_GPU_TIME",
                    DiagnosticSeverity::Info,
                    "sandbox",
                    format!("GPU frame time: {:.3} ms", snap.gpu_ms),
                )
                .path("frame_stats.gpu_frame_ms"),
            );
            all.push(
                Diagnostic::new(
                    "DIAG_VISIBLE_DRAWABLES",
                    DiagnosticSeverity::Info,
                    "sandbox",
                    format!(
                        "visible: {} | culled: {}",
                        snap.visible_drawables, snap.culled_drawables
                    ),
                )
                .path("frame_stats.visible_drawables"),
            );
        }

        // Scene validation diagnostics
        all.extend(self.scene_validation.iter().cloned());

        // Reload status diagnostics
        all.extend(self.reload_status.iter().cloned());

        // Script error diagnostics
        all.extend(self.script_diagnostics.iter().cloned());

        // Asset diagnostics
        all.extend(self.asset_diagnostics.iter().cloned());

        all
    }

    /// Total number of diagnostics across all categories (excluding frame snapshot).
    #[expect(dead_code)]
    pub fn len(&self) -> usize {
        let mut count = self.scene_validation.len()
            + self.reload_status.len()
            + self.script_diagnostics.len()
            + self.asset_diagnostics.len();
        if self.frame_snapshot.is_some() {
            count += 4; // four frame stats info diagnostics
        }
        count
    }

    /// Returns `true` if no diagnostics are present.
    #[expect(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.scene_validation.is_empty()
            && self.reload_status.is_empty()
            && self.script_diagnostics.is_empty()
            && self.asset_diagnostics.is_empty()
            && self.frame_snapshot.is_none()
    }
}

impl Default for SandboxDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}
