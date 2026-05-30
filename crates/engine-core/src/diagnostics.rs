use engine_renderer::FrameStats;
use engine_scene::{validate_scene, Scene};
use engine_serialize::Diagnostic;

/// Snapshot of per-frame GPU statistics for diagnostics display.
#[derive(Clone, Debug)]
pub struct FrameStatsSnapshot {
    pub frame: u64,
    pub draw_calls: u32,
    pub triangles: u64,
    pub gpu_ms: f32,
    pub visible_drawables: u32,
    pub culled_drawables: u32,
}

/// Snapshot of the asset reload queue (placeholder for future reload system).
#[derive(Clone, Debug, Default)]
pub struct ReloadQueueSnapshot {
    pub queued: u32,
    pub in_flight: u32,
}

/// Aggregate runtime diagnostics exposed to editor / tooling.
#[derive(Clone, Debug)]
pub struct RuntimeDiagnostics {
    pub collector: DiagnosticsCollector,
    pub reload_queue: Option<ReloadQueueSnapshot>,
    pub script_engine_state: String,
}

// ---------------------------------------------------------------------------
// DiagnosticsCollector
// ---------------------------------------------------------------------------

/// Central aggregator for all runtime diagnostics.
///
/// Collects frame statistics, scene validation results, script errors,
/// reload status, and asset-level diagnostics.  The collector retains
/// a bounded history of frame stats and separates transient (per-frame)
/// diagnostics from structural (scene/asset) diagnostics.
#[derive(Clone, Debug)]
pub struct DiagnosticsCollector {
    pub frame_stats: Vec<FrameStatsSnapshot>,
    pub scene_diagnostics: Vec<Diagnostic>,
    pub script_diagnostics: Vec<Diagnostic>,
    pub reload_diagnostics: Vec<Diagnostic>,
    pub asset_diagnostics: Vec<Diagnostic>,
    max_history: usize,
}

impl DiagnosticsCollector {
    /// Create a new collector with a default history limit of 256 frames.
    pub fn new() -> Self {
        Self {
            frame_stats: Vec::new(),
            scene_diagnostics: Vec::new(),
            script_diagnostics: Vec::new(),
            reload_diagnostics: Vec::new(),
            asset_diagnostics: Vec::new(),
            max_history: 256,
        }
    }

    /// Record GPU frame statistics after a completed render frame.
    ///
    /// The snapshot is appended and the history is automatically trimmed
    /// to [`max_history`] entries, discarding the oldest frames.
    pub fn record_frame(&mut self, frame: u64, stats: &FrameStats) {
        if self.frame_stats.len() >= self.max_history {
            self.frame_stats.remove(0);
        }
        self.frame_stats.push(FrameStatsSnapshot {
            frame,
            draw_calls: stats.draw_calls,
            triangles: stats.triangles,
            gpu_ms: stats.gpu_frame_ms,
            visible_drawables: stats.visible_drawables,
            culled_drawables: stats.culled_drawables,
        });
    }

    /// Push a batch of diagnostics from the script system.
    pub fn push_script_diags(&mut self, diags: Vec<Diagnostic>) {
        self.script_diagnostics.extend(diags);
    }

    /// Push a batch of diagnostics from the asset reload system.
    pub fn push_reload_diags(&mut self, diags: Vec<Diagnostic>) {
        self.reload_diagnostics.extend(diags);
    }

    /// Push a batch of diagnostics from scene validation.
    pub fn push_scene_diags(&mut self, diags: Vec<Diagnostic>) {
        self.scene_diagnostics.extend(diags);
    }

    /// Push a batch of asset-level diagnostics.
    pub fn push_asset_diags(&mut self, diags: Vec<Diagnostic>) {
        self.asset_diagnostics.extend(diags);
    }

    /// Return references to **all** current diagnostics (across all categories).
    pub fn all(&self) -> Vec<&Diagnostic> {
        let mut result: Vec<&Diagnostic> = Vec::new();
        result.extend(self.scene_diagnostics.iter());
        result.extend(self.script_diagnostics.iter());
        result.extend(self.reload_diagnostics.iter());
        result.extend(self.asset_diagnostics.iter());
        result
    }

    /// Clear per-frame transient diagnostics (script + reload) while keeping
    /// structural diagnostics (scene + asset) intact.
    pub fn clear_frame(&mut self) {
        self.script_diagnostics.clear();
        self.reload_diagnostics.clear();
    }
}

impl Default for DiagnosticsCollector {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Scene validation
// ---------------------------------------------------------------------------

/// Validate the current scene every `interval` frames.
///
/// When `frame % interval == 0` the full scene validation is run and any
/// diagnostics are returned.  At other frames an empty `Vec` is returned.
pub fn validate_scene_periodic(scene: &Scene, frame: u64, interval: u64) -> Vec<Diagnostic> {
    if interval == 0 || frame % interval != 0 {
        return Vec::new();
    }
    validate_scene(scene)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::DiagnosticSeverity;

    // Helper — create a minimal FrameStats with the fields we care about.
    fn make_stats(
        draw_calls: u32,
        triangles: u64,
        gpu_ms: f32,
        visible_drawables: u32,
        culled_drawables: u32,
    ) -> FrameStats {
        FrameStats {
            visible_drawables,
            culled_drawables,
            visible_lights: 0,
            culled_lights: 0,
            draw_calls,
            triangles,
            gpu_frame_ms: gpu_ms,
        }
    }

    // ── DiagnosticsCollector tests ────────────────────────────────────────

    #[test]
    fn record_frame_and_retrieve() {
        let mut collector = DiagnosticsCollector::new();
        let stats = make_stats(42, 12000, 16.7, 100, 30);
        collector.record_frame(1, &stats);

        assert_eq!(collector.frame_stats.len(), 1);
        let snapshot = &collector.frame_stats[0];
        assert_eq!(snapshot.frame, 1);
        assert_eq!(snapshot.draw_calls, 42);
        assert_eq!(snapshot.triangles, 12000);
        assert!((snapshot.gpu_ms - 16.7).abs() < f32::EPSILON);
        assert_eq!(snapshot.visible_drawables, 100);
        assert_eq!(snapshot.culled_drawables, 30);
    }

    #[test]
    fn record_frame_trims_history() {
        let mut collector = DiagnosticsCollector::new();
        collector.max_history = 3;

        for i in 0..5u64 {
            let stats = make_stats(i as u32, 0, 0.0, 0, 0);
            collector.record_frame(i, &stats);
        }

        assert_eq!(collector.frame_stats.len(), 3);
        assert_eq!(collector.frame_stats[0].frame, 2);
        assert_eq!(collector.frame_stats[1].frame, 3);
        assert_eq!(collector.frame_stats[2].frame, 4);
    }

    #[test]
    fn push_script_diags_and_all() {
        let mut collector = DiagnosticsCollector::new();

        let diag1 = Diagnostic::new("SCR001", DiagnosticSeverity::Error, "script", "parse error");
        let diag2 = Diagnostic::new(
            "SCR002",
            DiagnosticSeverity::Warning,
            "script",
            "unused var",
        );
        collector.push_script_diags(vec![diag1, diag2]);

        let all = collector.all();
        assert_eq!(all.len(), 2);

        let codes: Vec<&str> = all.iter().map(|d| d.code.as_str()).collect();
        assert!(codes.contains(&"SCR001"));
        assert!(codes.contains(&"SCR002"));
    }

    #[test]
    fn clear_frame_removes_transient_only() {
        let mut collector = DiagnosticsCollector::new();

        collector.push_script_diags(vec![Diagnostic::new(
            "SCR001",
            DiagnosticSeverity::Error,
            "script",
            "err",
        )]);
        collector.push_reload_diags(vec![Diagnostic::new(
            "RLD001",
            DiagnosticSeverity::Info,
            "reload",
            "reloaded",
        )]);
        collector.push_scene_diags(vec![Diagnostic::new(
            "SC001",
            DiagnosticSeverity::Error,
            "scene",
            "bad",
        )]);
        collector.push_asset_diags(vec![Diagnostic::new(
            "AST001",
            DiagnosticSeverity::Warning,
            "asset",
            "missing",
        )]);

        assert_eq!(collector.all().len(), 4);

        collector.clear_frame();

        // Transient diagnostics cleared
        assert!(collector.script_diagnostics.is_empty());
        assert!(collector.reload_diagnostics.is_empty());

        // Structural diagnostics preserved
        assert_eq!(collector.scene_diagnostics.len(), 1);
        assert_eq!(collector.asset_diagnostics.len(), 1);

        assert_eq!(collector.all().len(), 2);
    }

    #[test]
    fn clear_frame_empty_collector() {
        let mut collector = DiagnosticsCollector::new();
        collector.clear_frame(); // should not panic
        assert!(collector.all().is_empty());
    }

    #[test]
    fn push_multiple_batches_accumulates() {
        let mut collector = DiagnosticsCollector::new();

        collector.push_script_diags(vec![Diagnostic::new(
            "SCR001",
            DiagnosticSeverity::Error,
            "script",
            "err",
        )]);
        collector.push_script_diags(vec![Diagnostic::new(
            "SCR002",
            DiagnosticSeverity::Warning,
            "script",
            "warn",
        )]);

        assert_eq!(collector.script_diagnostics.len(), 2);
    }

    #[test]
    fn default_collector_is_empty() {
        let collector = DiagnosticsCollector::new();
        assert!(collector.frame_stats.is_empty());
        assert!(collector.all().is_empty());
    }

    #[test]
    fn push_diags_all_categories() {
        let mut collector = DiagnosticsCollector::new();

        collector.push_scene_diags(vec![Diagnostic::new(
            "SC001",
            DiagnosticSeverity::Error,
            "scene",
            "dup id",
        )]);
        collector.push_script_diags(vec![Diagnostic::new(
            "SCR001",
            DiagnosticSeverity::Error,
            "script",
            "type",
        )]);
        collector.push_reload_diags(vec![Diagnostic::new(
            "RLD001",
            DiagnosticSeverity::Info,
            "reload",
            "ok",
        )]);
        collector.push_asset_diags(vec![Diagnostic::new(
            "AST001",
            DiagnosticSeverity::Warning,
            "asset",
            "miss",
        )]);

        assert_eq!(collector.all().len(), 4);
    }

    // ── validate_scene_periodic ───────────────────────────────────────────

    #[test]
    fn validate_scene_periodic_skips_when_interval_mismatch() {
        let scene = engine_scene::sample_scene();
        let diags = validate_scene_periodic(&scene, 1, 5);
        assert!(diags.is_empty());
    }

    #[test]
    fn validate_scene_periodic_runs_on_interval() {
        let scene = engine_scene::sample_scene();
        let diags = validate_scene_periodic(&scene, 10, 5);
        // sample_scene is valid, so validation should return no errors
        assert!(diags.is_empty());
    }

    #[test]
    fn validate_scene_periodic_zero_interval_skips() {
        let scene = engine_scene::sample_scene();
        let diags = validate_scene_periodic(&scene, 0, 0);
        assert!(diags.is_empty());
    }

    // ── RuntimeDiagnostics ────────────────────────────────────────────────

    #[test]
    fn runtime_diagnostics_construction() {
        let rd = RuntimeDiagnostics {
            collector: DiagnosticsCollector::new(),
            reload_queue: None,
            script_engine_state: "idle".to_string(),
        };
        assert_eq!(rd.script_engine_state, "idle");
        assert!(rd.reload_queue.is_none());
        assert!(rd.collector.all().is_empty());
    }
}
