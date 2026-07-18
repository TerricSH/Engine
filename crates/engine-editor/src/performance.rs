//! Performance inspector panel for the editor.
//!
//! Displays real-time frame statistics including rendering, physics,
//! animation, navigation, and asset counts. Historical frame
//! data is maintained as a rolling 60-frame buffer for graphing.

use tracing;

use engine_animation::components::AnimationPlayer;
use engine_nav::components::AiAgent;
use engine_physics::components::RigidBody;
use engine_renderer::FrameStats as RendererFrameStats;
use engine_scene::World;

// ---------------------------------------------------------------------------
// FrameStats
// ---------------------------------------------------------------------------

/// Per-frame performance statistics snapshot.
#[derive(Clone, Debug)]
pub struct FrameStats {
    /// Total frame time in milliseconds.
    pub frame_time_ms: f32,
    /// Number of draw calls submitted to the GPU this frame.
    pub draw_calls: u32,
    /// Number of triangles rasterised this frame.
    pub triangles: u32,
    /// Number of active physics bodies.
    pub physics_bodies: u32,
    /// Number of active animation players.
    pub animation_count: u32,
    /// Number of active navigation agents.
    pub nav_agents: u32,
    /// Number of loaded assets.
    pub asset_count: u32,
}

impl FrameStats {
    /// All-zero placeholder.
    pub const ZERO: Self = Self {
        frame_time_ms: 0.0,
        draw_calls: 0,
        triangles: 0,
        physics_bodies: 0,
        animation_count: 0,
        nav_agents: 0,
        asset_count: 0,
    };

    /// Return a colour-coded label for the frame time.
    ///
    /// Returns `("label", R, G, B)` where:
    /// - green (`(0.0, 1.0, 0.0)`) when `< 8 ms`
    /// - yellow (`(1.0, 1.0, 0.0)`) when `8 – 13 ms`
    /// - red   (`(1.0, 0.3, 0.0)`) when `> 13 ms`
    pub fn frame_time_color(&self) -> (&'static str, [f32; 3]) {
        if self.frame_time_ms <= 8.0 {
            ("good", [0.0, 1.0, 0.0])
        } else if self.frame_time_ms <= 13.0 {
            ("warn", [1.0, 1.0, 0.0])
        } else {
            ("bad", [1.0, 0.3, 0.0])
        }
    }
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::ZERO
    }
}

// ---------------------------------------------------------------------------
// PerformancePanel
// ---------------------------------------------------------------------------

/// Panel state for the performance inspector.
pub struct PerformancePanel {
    /// Latest frame statistics.
    pub frame_stats: FrameStats,
    /// Rolling 60-frame history.
    pub history: Vec<FrameStats>,
}

impl PerformancePanel {
    /// Create a new performance panel with default state.
    pub fn new() -> Self {
        Self {
            frame_stats: FrameStats::ZERO,
            history: Vec::with_capacity(60),
        }
    }

    /// Push the current stats into the rolling history buffer.
    ///
    /// The buffer is capped at 60 entries — older entries are dropped.
    pub fn commit_frame(&mut self) {
        if self.history.len() >= 60 {
            self.history.remove(0);
        }
        self.history.push(self.frame_stats.clone());
    }

    /// Return a slice of the full history buffer.
    pub fn history(&self) -> &[FrameStats] {
        &self.history
    }
}

impl Default for PerformancePanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// record_frame
// ---------------------------------------------------------------------------

/// Populate `stats` with a snapshot of the current engine performance data.
///
/// Queries the ECS `World` for component counts (physics bodies, animation
/// players, nav agents) and — when `renderer_stats` is provided — merges
/// GPU-side statistics (draw calls, triangles, frame time) into `stats`.
pub fn record_frame(
    stats: &mut FrameStats,
    world: &World,
    renderer_stats: Option<&RendererFrameStats>,
) {
    // ── Physics ──────────────────────────────────────────────────────
    stats.physics_bodies = world.query::<RigidBody>().count() as u32;

    // ── Animation ────────────────────────────────────────────────────
    stats.animation_count = world.query::<AnimationPlayer>().count() as u32;

    // ── Navigation ───────────────────────────────────────────────────
    stats.nav_agents = world.query::<AiAgent>().count() as u32;

    // ── GPU stats (from renderer) ────────────────────────────────────
    // Reset renderer-origin fields to zero so stale data from a
    // previous frame doesn't persist when renderer_stats is None.
    stats.draw_calls = 0;
    stats.triangles = 0;
    stats.frame_time_ms = 0.0;

    if let Some(rs) = renderer_stats {
        stats.draw_calls = rs.draw_calls;
        stats.triangles = u32::try_from(rs.triangles).unwrap_or(u32::MAX);
        stats.frame_time_ms = rs.gpu_frame_ms;
    }

    // ── Assets ───────────────────────────────────────────────────────
    // `asset_count` should be set externally from the AssetRegistry.

    tracing::trace!(
        physics = stats.physics_bodies,
        anim = stats.animation_count,
        nav = stats.nav_agents,
        draw_calls = stats.draw_calls,
        gpu_ms = stats.frame_time_ms,
        "PerformancePanel: frame recorded"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── FrameStats defaults ──────────────────────────────────────────────

    #[test]
    fn frame_stats_default_is_zero() {
        let s = FrameStats::default();
        assert_eq!(s.frame_time_ms, 0.0);
        assert_eq!(s.draw_calls, 0);
        assert_eq!(s.triangles, 0);
        assert_eq!(s.physics_bodies, 0);
        assert_eq!(s.animation_count, 0);
        assert_eq!(s.nav_agents, 0);
        assert_eq!(s.asset_count, 0);
    }

    #[test]
    fn frame_stats_zero_constant() {
        let s = FrameStats::ZERO;
        assert_eq!(s.frame_time_ms, 0.0);
        assert_eq!(s.draw_calls, 0);
    }

    // ── Frame time colour coding ────────────────────────────────────────

    #[test]
    fn frame_time_green_below_8() {
        let s = FrameStats {
            frame_time_ms: 5.0,
            ..FrameStats::ZERO
        };
        let (tag, _) = s.frame_time_color();
        assert_eq!(tag, "good");
    }

    #[test]
    fn frame_time_yellow_8_to_13() {
        let s = FrameStats {
            frame_time_ms: 10.0,
            ..FrameStats::ZERO
        };
        let (tag, _) = s.frame_time_color();
        assert_eq!(tag, "warn");
    }

    #[test]
    fn frame_time_red_above_13() {
        let s = FrameStats {
            frame_time_ms: 20.0,
            ..FrameStats::ZERO
        };
        let (tag, _) = s.frame_time_color();
        assert_eq!(tag, "bad");
    }

    #[test]
    fn frame_time_boundary_8_is_green() {
        let s = FrameStats {
            frame_time_ms: 8.0,
            ..FrameStats::ZERO
        };
        let (tag, _) = s.frame_time_color();
        assert_eq!(tag, "good");
    }

    #[test]
    fn frame_time_boundary_13_is_yellow() {
        let s = FrameStats {
            frame_time_ms: 13.0,
            ..FrameStats::ZERO
        };
        let (tag, _) = s.frame_time_color();
        assert_eq!(tag, "warn");
    }

    // ── Recording frame pushes to history ───────────────────────────────

    #[test]
    fn commit_frame_pushes_to_history() {
        let mut panel = PerformancePanel::new();
        assert!(panel.history.is_empty());

        panel.frame_stats.frame_time_ms = 16.5;
        panel.commit_frame();

        assert_eq!(panel.history.len(), 1);
        assert!((panel.history[0].frame_time_ms - 16.5).abs() < 0.001);
    }

    #[test]
    fn commit_frame_caps_at_60() {
        let mut panel = PerformancePanel::new();

        // Push 65 frames.
        for i in 0..65 {
            panel.frame_stats.frame_time_ms = i as f32;
            panel.commit_frame();
        }

        assert_eq!(panel.history.len(), 60);
        // The oldest frame should be frame 5 (0..4 were evicted).
        assert!((panel.history[0].frame_time_ms - 5.0).abs() < 0.001);
        // The newest should be frame 64.
        assert!((panel.history[59].frame_time_ms - 64.0).abs() < 0.001);
    }

    #[test]
    fn multiple_commits_maintain_order() {
        let mut panel = PerformancePanel::new();
        panel.frame_stats.frame_time_ms = 1.0;
        panel.commit_frame();
        panel.frame_stats.frame_time_ms = 2.0;
        panel.commit_frame();
        panel.frame_stats.frame_time_ms = 3.0;
        panel.commit_frame();

        assert_eq!(panel.history.len(), 3);
        assert!((panel.history[0].frame_time_ms - 1.0).abs() < 0.001);
        assert!((panel.history[1].frame_time_ms - 2.0).abs() < 0.001);
        assert!((panel.history[2].frame_time_ms - 3.0).abs() < 0.001);
    }

    // ── renderer parameter accepted (no-op) ─────────────────────────────

    #[test]
    fn record_frame_accepts_missing_renderer_stats() {
        let world = World::new();
        let mut stats = FrameStats::ZERO;

        // Should not panic.  Pass None for renderer stats (no GPU data in test).
        record_frame(&mut stats, &world, None);

        // World is empty → all counts stay zero.
        assert_eq!(stats.physics_bodies, 0);
        assert_eq!(stats.animation_count, 0);
        assert_eq!(stats.nav_agents, 0);
    }

    #[test]
    fn record_frame_counts_world_components() {
        let mut world = World::new();

        // Add a few rigid bodies.
        let e1 = world.create_entity();
        world.add_component(e1, RigidBody::default());
        let e2 = world.create_entity();
        world.add_component(e2, RigidBody::default());

        // Add one animation player.
        let e3 = world.create_entity();
        world.add_component(e3, AnimationPlayer::new());

        let mut stats = FrameStats::ZERO;
        record_frame(&mut stats, &world, None);

        assert_eq!(stats.physics_bodies, 2);
        assert_eq!(stats.animation_count, 1);
        assert_eq!(stats.nav_agents, 0);
    }

    // ── history() accessor ──────────────────────────────────────────────

    #[test]
    fn history_accessor_returns_slice() {
        let mut panel = PerformancePanel::new();
        assert!(panel.history().is_empty());

        panel.frame_stats.frame_time_ms = 7.0;
        panel.commit_frame();

        assert_eq!(panel.history().len(), 1);
        assert!((panel.history()[0].frame_time_ms - 7.0).abs() < 0.001);
    }
}
